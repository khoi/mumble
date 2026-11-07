//! The macOS host of `mumble` control commands ([`crate::control`]). The main
//! thread answers what it owns (the window and settings saves) and hands commands that wait on the listener or the
//! window to a short-lived thread, so a slow answer never stalls the run loop.
//! Commands only the listener answers (`mumble dictation`, `mumble paste-last`)
//! skip the main thread: the control thread hands them over at once
//! ([`answer_off_main`]).

use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::Duration;

use crate::app_settings::AppSettings;
use crate::control::{
    self, CaptureReport, CaptureSnapshot, CaptureState, ControlCall, ControlCommand, ControlError,
    ControlReply, ControlResponse, FeatureReport, ListenerReport, Pane, PermissionReport,
    StatusReport, TranscriptionReport, WindowReport, settings,
};
use crate::desktop::host::{DesktopAction, DesktopUpdateStatus};
use crate::desktop::onboarding::PermissionState;
use crate::recognition::RecognitionControl;
use crate::rune_ui::story_link::{self, StoryLink};
use crate::rune_ui::{Destination, Ui};

use super::{Pump, Request, RuntimeHandle, bring_window_forward, show, window_visible};

/// How long a command waits for the listener's answer.
const LISTENER_TIMEOUT: Duration = Duration::from_secs(3);
const WINDOW_TIMEOUT: Duration = Duration::from_secs(1);
/// How long `model use` waits to confirm, inside
/// the local API's own control deadline.
const CONFIRM_WAIT: Duration = Duration::from_secs(8);
/// Lets the local API write the reply before Mumble quits.
const QUIT_GRACE: Duration = Duration::from_millis(250);

impl Pump {
    pub(super) fn control(&mut self, call: ControlCall) {
        let ControlCall { request, reply } = call;
        let answer = match control::check_version(&request) {
            Ok(()) => self.answer(request.command, &reply),
            Err(error) => Some(error.into()),
        };
        if let Some(answer) = answer {
            let _ = reply.send(answer.into());
        }
    }

    /// Answers now, or returns `None` once a thread or waiter owns `reply`.
    fn answer(
        &mut self,
        command: ControlCommand,
        reply: &SyncSender<ControlResponse>,
    ) -> Option<ControlReply> {
        let runtime = self.runtime.clone();
        let command = match listener_command(&runtime, command, reply) {
            Ok(()) => return None,
            Err(command) => command,
        };
        match command {
            ControlCommand::Status => {
                let window = self.window.clone();
                background(reply, move || status(&runtime, &window));
                None
            }
            ControlCommand::Quit { force } | ControlCommand::Restart { force } => {
                let restart = matches!(command, ControlCommand::Restart { .. });
                let reply = reply.clone();
                spawn(move || {
                    let capture = capture_snapshot(&runtime);
                    let answer = control::check_quit(capture, force).and_then(|()| {
                        if restart && !crate::desktop::onboarding::spawn_relaunch() {
                            return Err(ControlError::new(
                                "unsupported",
                                "only an installed Mumble.app can restart itself; quit it and open \
                                 it again",
                            ));
                        }
                        Ok(())
                    });
                    let quitting = answer.is_ok();
                    let _ = reply.send(
                        match answer {
                            Ok(()) if restart => ControlReply::done("Restarting Mumble"),
                            Ok(()) => ControlReply::done("Quitting Mumble"),
                            Err(error) => error.into(),
                        }
                        .into(),
                    );
                    if quitting {
                        thread::sleep(QUIT_GRACE);
                        super::quit();
                    }
                });
                None
            }
            ControlCommand::Dictation { .. } | ControlCommand::PasteLast => {
                unreachable!("listener_command answers it")
            }
            ControlCommand::SettingsGet { key } => Some(reply_with(load().and_then(|settings| {
                Ok(ControlReply::Settings {
                    settings: match key {
                        Some(key) => settings::get(&settings, &key)?,
                        None => settings::list(&settings),
                    },
                })
            }))),
            ControlCommand::SettingsSet { key, value } => {
                Some(reply_with(self.set_setting(&key, &value)))
            }
            ControlCommand::SettingsApply { values } => {
                Some(reply_with(self.apply_settings(&values)))
            }
            ControlCommand::WindowShow { pane } => Some(reply_with(self.show_window(pane))),
            ControlCommand::WindowHide => Some(if window_visible() {
                self.window.close();
                ControlReply::done("Closed the window")
            } else {
                ControlReply::unchanged("The window isn't open")
            }),
            ControlCommand::WindowStory {
                story,
                variant,
                play,
            } => Some(self.show_story(StoryLink::new(&story, variant, play))),
            ControlCommand::OnboardingReset => {
                show(&self.window, Destination::Settings);
                let window = self.window.clone();
                background(reply, move || restart_onboarding(&window));
                None
            }
            ControlCommand::HistoryClear => Some(reply_with(clear_history(&runtime))),
            ControlCommand::LaunchAtLogin { enabled } => {
                background(reply, move || launch_at_login(enabled));
                None
            }
            ControlCommand::CheckForUpdates => Some(match crate::macos_rune::update_status() {
                DesktopUpdateStatus::Managed => {
                    ControlReply::error("unsupported", "updates are managed by your organization")
                }
                DesktopUpdateStatus::Unavailable | DesktopUpdateStatus::SourceBuild => {
                    ControlReply::error("unavailable", "this Mumble build doesn't update itself")
                }
                _ => match crate::macos_rune::update_action(&DesktopAction::CheckForUpdates) {
                    Ok(()) => {
                        ControlReply::done("Checking for updates; Mumble shows what it finds")
                    }
                    Err(error) => ControlReply::error("unavailable", format!("{error:#}")),
                },
            }),
            ControlCommand::VocabularyList => Some(reply_with(load().map(|settings| {
                ControlReply::Vocabulary {
                    entries: settings.vocabulary,
                }
            }))),
            ControlCommand::VocabularyAdd { .. } | ControlCommand::VocabularyRemove { .. } => {
                Some(reply_with(self.edit_vocabulary(&command)))
            }
        }
    }

    /// Saves a vocabulary edit as the window would, then lets the runtime and
    /// an open window adopt it.
    fn edit_vocabulary(&mut self, command: &ControlCommand) -> Result<ControlReply, ControlError> {
        let mut settings = load()?;
        let reply = control::edit_vocabulary(&mut settings.vocabulary, command)?;
        if matches!(reply, ControlReply::Done { changed: true, .. }) {
            settings.save().map_err(|error| {
                ControlError::new("internal", format!("couldn't save: {error:#}"))
            })?;
            let _ = self.runtime.send(Request::ReloadSettings);
        }
        Ok(reply)
    }

    /// Saves one setting as the window would, then lets the runtime, the
    /// listener, and an open window adopt it.
    fn set_setting(&mut self, key: &str, value: &str) -> Result<ControlReply, ControlError> {
        if let Some(error) = settings::unavailable(key) {
            return Err(error);
        }
        let before = load()?;
        let change = settings::apply(&before, key, value, &crate::rune_ui::shortcut_rules())?;
        self.commit(&before, change, || format!("{key} is already {value}"))
    }

    fn apply_settings(
        &mut self,
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ControlReply, ControlError> {
        let before = load()?;
        let change = settings::apply_all(&before, values, &crate::rune_ui::shortcut_rules())?;
        self.commit(&before, change, || {
            "Every setting already has its value".into()
        })
    }

    /// Saves a validated change, reloads it, and carries out its effects.
    fn commit(
        &mut self,
        before: &AppSettings,
        change: settings::Change,
        unchanged: impl FnOnce() -> String,
    ) -> Result<ControlReply, ControlError> {
        if !change.changed(before) {
            return Ok(ControlReply::unchanged(unchanged()));
        }
        change
            .settings
            .save()
            .map_err(|error| ControlError::new("internal", format!("couldn't save: {error:#}")))?;
        let _ = self.runtime.send(Request::ReloadSettings);
        for effect in change.effects {
            match effect {
                settings::Effect::HistoryRetention(retention) => {
                    if let Some(history) = self.runtime.history()
                        && let Err(error) = history.set_retention(retention)
                    {
                        tracing::warn!(%error, "could not apply history retention");
                    }
                }
                // An open window keeps the Dock icon; closing it applies this.
                settings::Effect::DockIcon if !window_visible() => {
                    crate::app_settings::set_dock_icon_visible(
                        crate::app_settings::dock_icon_visible(
                            change.settings.show_dock_icon,
                            crate::macos_rune::status_item::installed(),
                        ),
                    );
                }
                settings::Effect::DockIcon => {}
                settings::Effect::CheckForUpdates => {
                    let _ = crate::macos_rune::update_action(&DesktopAction::CheckForUpdates);
                }
            }
        }
        Ok(ControlReply::done(change.message))
    }

    fn show_window(&mut self, pane: Option<Pane>) -> Result<ControlReply, ControlError> {
        let destination = match pane {
            None | Some(Pane::Settings) => Destination::Settings,
            Some(Pane::Models) => Destination::Models,
            Some(Pane::Modes) => Destination::Modes,
            Some(Pane::History) => Destination::History,
        };
        show(&self.window, destination);
        Ok(ControlReply::done("Opened the window"))
    }

    /// `mumble window story`: what a `mumble://story/…` link does.
    fn show_story(&mut self, link: StoryLink) -> ControlReply {
        let enabled = crate::rune_ui::playground::enabled();
        let mut message = format!("Opened the playground at {}", link.story);
        if let Some(variant) = &link.variant {
            message.push_str(&format!(", variant {variant}"));
        }
        if link.play {
            message.push_str(", and played it");
        }
        self.window.update(move |_, tx| {
            if let Err(why) = story_link::open(&link, tx) {
                tracing::warn!(story = %link.story, %why, "mumble window story opened only the window");
            }
        });
        bring_window_forward();
        if enabled {
            ControlReply::done(message)
        } else {
            ControlReply::error(
                "feature-off",
                format!("{}; opened the window", story_link::PLAYGROUND_OFF),
            )
        }
    }
}

fn reply_with(result: Result<ControlReply, ControlError>) -> ControlReply {
    result.unwrap_or_else(ControlReply::from)
}

fn spawn(work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new()
        .name("mumble-control".into())
        .spawn(work)
    {
        tracing::error!(%error, "could not start a control thread");
    }
}

/// Answers a call that needs only the listener (`hex dictation`, `mumble
/// paste-last`) from a thread of its own, or returns it for the main thread,
/// whose pump drains its channels on a timer.
pub(super) fn answer_off_main(runtime: &RuntimeHandle, call: ControlCall) -> Option<ControlCall> {
    let ControlCall { mut request, reply } = call;
    if control::check_version(&request).is_err() {
        return Some(ControlCall { request, reply });
    }
    match listener_command(runtime, request.command, &reply) {
        Ok(()) => None,
        Err(command) => {
            request.command = command;
            Some(ControlCall { request, reply })
        }
    }
}

/// Hands a command the listener answers to a thread that waits for it, or
/// gives the command back.
fn listener_command(
    runtime: &RuntimeHandle,
    command: ControlCommand,
    reply: &SyncSender<ControlResponse>,
) -> Result<(), ControlCommand> {
    let runtime = runtime.clone();
    match command {
        ControlCommand::Dictation {
            action,
            print: true,
            raw,
        } => background(reply, move || {
            let action = control::check_flags(action, true, raw)?;
            listener_ready(&runtime)?;
            let (print, printed) = control::PrintReply::channel();
            // The listener answers once the recording stopped; only this
            // command then waits for the transcript.
            match ask(&runtime, |reply| RecognitionControl::Capture {
                action,
                print: Some(print),
                raw: false,
                reply,
            })? {
                ControlReply::Done { .. } => {
                    Ok(control::await_printed(&printed, control::PRINT_WAIT))
                }
                refused => Ok(refused),
            }
        }),
        ControlCommand::Dictation { action, raw, .. } => background(reply, move || {
            let action = control::check_flags(action, false, raw)?;
            listener_ready(&runtime)?;
            ask(&runtime, |reply| RecognitionControl::Capture {
                action,
                print: None,
                raw,
                reply,
            })
        }),
        ControlCommand::PasteLast => background(reply, move || {
            listener_ready(&runtime)?;
            ask(&runtime, |reply| RecognitionControl::PasteLast {
                reply: Some(reply),
            })
        }),
        command => return Err(command),
    }
    Ok(())
}

/// Answers `reply` from a thread; a dropped reply reads as `control-unavailable`.
fn background(
    reply: &SyncSender<ControlResponse>,
    work: impl FnOnce() -> Result<ControlReply, ControlError> + Send + 'static,
) {
    let reply = reply.clone();
    spawn(move || {
        let _ = reply.send(reply_with(work()).into());
    });
}

fn load() -> Result<AppSettings, ControlError> {
    AppSettings::load().map_err(|error| {
        ControlError::new("internal", format!("couldn't read settings: {error:#}"))
    })
}

fn listener_ready(runtime: &RuntimeHandle) -> Result<(), ControlError> {
    let published = runtime.published();
    let listener = published.as_ref().map(|published| &published.listener);
    if listener.is_some_and(|listener| listener.running) {
        return Ok(());
    }
    if let Some(failure) = listener.and_then(|listener| listener.warning.as_ref()) {
        return Err(ControlError::new(
            "listener-stopped",
            format!("the listener stopped ({failure}); restart Mumble"),
        ));
    }
    Err(ControlError::new(
        "not-ready",
        "Mumble is waiting for setup (permissions or the dictation model); run `mumble status`",
    ))
}

/// Sends the listener a control and waits for its answer.
fn ask<T: Send + 'static>(
    runtime: &RuntimeHandle,
    build: impl FnOnce(SyncSender<T>) -> RecognitionControl,
) -> Result<T, ControlError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    runtime
        .recognition(build(sender))
        .map_err(|error| ControlError::new("busy", error))?;
    receiver.recv_timeout(LISTENER_TIMEOUT).map_err(|_| {
        ControlError::new(
            "unavailable",
            "the listener didn't answer in time; check `mumble status`",
        )
    })
}

/// What records now; a listener still waiting for setup records nothing.
fn capture_snapshot(runtime: &RuntimeHandle) -> Result<CaptureSnapshot, ControlError> {
    if listener_ready(runtime).is_err() {
        return Ok(CaptureSnapshot::default());
    }
    ask(runtime, |reply| RecognitionControl::CaptureStatus { reply })
}

fn clear_history(runtime: &RuntimeHandle) -> Result<ControlReply, ControlError> {
    let history = runtime
        .history()
        .ok_or_else(|| ControlError::new("unavailable", "history couldn't be opened"))?;
    let count = history.search("").len();
    if count == 0 {
        return Ok(ControlReply::unchanged("History is already empty"));
    }
    history
        .clear()
        .map_err(|error| ControlError::new("failed", format!("couldn't clear history: {error}")))?;
    Ok(ControlReply::done(format!(
        "Cleared {count} dictation{}",
        if count == 1 { "" } else { "s" }
    )))
}

/// The signed app's `SMAppService` registration; an open Settings window
/// picks the change up on its next status poll.
fn launch_at_login(enabled: Option<bool>) -> Result<ControlReply, ControlError> {
    use crate::desktop::login_item::LoginItemStatus;

    let describe = |status| match status {
        LoginItemStatus::Enabled => "on",
        LoginItemStatus::Disabled => "off",
        LoginItemStatus::RequiresApproval => {
            "waiting for approval in System Settings › General › Login Items"
        }
    };
    let before = crate::desktop::login_item::set_now(None)
        .map_err(|error| ControlError::new("failed", format!("{error:#}")))?;
    let Some(enabled) = enabled else {
        return Ok(ControlReply::unchanged(format!(
            "Launch at login is {}",
            describe(before)
        )));
    };
    if (before != LoginItemStatus::Disabled) == enabled {
        return Ok(ControlReply::unchanged(format!(
            "Launch at login is already {}",
            describe(before)
        )));
    }
    let bundled = std::env::current_exe().is_ok_and(|executable| {
        executable
            .ancestors()
            .nth(3)
            .is_some_and(|path| path.extension().is_some_and(|extension| extension == "app"))
    });
    if !bundled {
        return Err(ControlError::new(
            "unsupported",
            "only an installed Mumble.app can launch at login",
        ));
    }
    let after = crate::desktop::login_item::set_now(Some(enabled)).map_err(|error| {
        ControlError::new(
            "failed",
            format!("couldn't change launch at login: {error:#}"),
        )
    })?;
    Ok(ControlReply::done(format!(
        "Launch at login is {}",
        describe(after)
    )))
}

/// The pane the window shows, asked on the UI thread.
fn current_pane(window: &rune::AppHandle<Ui>) -> Option<String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    window.update(move |ui, _| {
        let _ = sender.send(crate::rune_ui::current_pane(ui));
    });
    receiver.recv_timeout(WINDOW_TIMEOUT).ok().map(String::from)
}

fn restart_onboarding(window: &rune::AppHandle<Ui>) -> Result<ControlReply, ControlError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    window.update(move |ui, tx| {
        let _ = sender.send(crate::rune_ui::restart_onboarding(ui, tx));
    });
    match receiver.recv_timeout(CONFIRM_WAIT) {
        Ok(Ok(())) => Ok(ControlReply::done("Opened onboarding")),
        Ok(Err(why)) => Err(ControlError::new("unsupported", why)),
        Err(_) => Err(ControlError::new(
            "unavailable",
            "the window didn't answer; try again",
        )),
    }
}

fn permission(state: PermissionState) -> String {
    match state {
        PermissionState::Ready => "ready",
        PermissionState::NeedsRequest => "needs-request",
        PermissionState::NeedsSettings => "needs-settings",
        PermissionState::NeedsRestart => "needs-restart",
    }
    .into()
}

fn status(
    runtime: &RuntimeHandle,
    window: &rune::AppHandle<Ui>,
) -> Result<ControlReply, ControlError> {
    let published = runtime
        .published()
        .ok_or_else(|| ControlError::new("not-ready", "Mumble is still starting; try again"))?;
    // A stopped listener cannot say what it was doing.
    let snapshot = match &published.listener.warning {
        Some(failure) => Err(ControlError::new("listener-stopped", failure.clone())),
        None => capture_snapshot(runtime),
    };
    let microphone_unavailable = snapshot
        .as_ref()
        .is_ok_and(|snapshot| snapshot.microphone_unavailable);
    let capture = match snapshot {
        Ok(snapshot) => CaptureReport::from(snapshot),
        Err(_) => CaptureReport {
            state: CaptureState::Unknown,
            ..Default::default()
        },
    };
    let open = window.is_visible();
    let settings = &published.settings;
    let selection = &settings.transcription;
    let setup = crate::desktop::onboarding::status();
    Ok(ControlReply::Status(Box::new(StatusReport {
        app_version: crate::VERSION.into(),
        pid: std::process::id(),
        memory_footprint_bytes: control::memory_footprint(),
        listener: ListenerReport {
            running: published.listener.running,
            status: if microphone_unavailable {
                "No microphone".into()
            } else {
                published.listener.status.clone()
            },
            shortcuts: Vec::new(),
        },
        capture,
        transcription: TranscriptionReport {
            provider: serde_json::to_value(selection.provider)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
            language: selection.language.clone(),
        },
        features: FeatureReport {
            release_microphone_while_idle: settings.release_microphone_while_idle,
            history_retention: serde_json::to_value(settings.history_retention)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
        },
        permissions: PermissionReport {
            ready: setup.ready(),
            microphone: permission(setup.microphone),
            accessibility: permission(setup.accessibility),
        },
        window: WindowReport {
            open,
            pane: open.then(|| current_pane(window)).flatten(),
        },
        update: control::update_label(&published.update_status).into(),
        labs: crate::labs::active_names(settings),
        capabilities: crate::rune_ui::capabilities(),
    })))
}
