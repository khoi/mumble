//! Fixtures for snapshots and tests: the named starting states `--scene`
//! opens.

use super::*;

/// A named starting state for `--scene`.
pub(super) type Scene = (&'static str, fn(&mut Ui));

pub(super) const SCENES: &[Scene] = &[
    // The running service's real state, for headless snapshots and scripts
    // (`--snap`): waits for its first snapshot instead of drawing
    // "Connecting…" at virtual time zero.
    ("live", |ui| {
        // Reads the saved settings and the running Mumble; writes neither.
        let settings = AppSettings::read().unwrap_or_default();
        crate::desktop::i18n::apply(settings.ui_language.as_deref());
        *ui = Ui::with(settings, Service::Fixture(Vec::new()));
        let mut backend = Backend::connect(Default::default(), Arc::new(|| {}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while backend.state().is_none() && backend.error().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
            backend.refresh();
        }
        ui.service = Service::Live(Box::new(backend));
        poll(ui, 0.0, &mut Tx::default());
    }),
    ("portal", |ui| *ui = portal_fixture()),
    ("portal-mouse", |ui| {
        *ui = portal_fixture();
        ui.settings.dictation_hotkey = HotkeyBinding {
            modifiers: Default::default(),
            key: crate::app_settings::HotkeyKey::mouse(3),
        };
        if let Some(view) = &mut ui.view {
            view.listener.shortcuts =
                DesktopShortcuts::Portal(vec![crate::desktop::host::DesktopGrantedShortcut {
                    name: "Dictation".into(),
                    trigger: None,
                    unconfirmed: false,
                }]);
        }
    }),
    ("commands", |ui| *ui = fixture(DesktopShortcuts::Commands)),
    ("disabled", |ui| {
        *ui = portal_fixture();
        if let Some(view) = &mut ui.view {
            view.listener.running = false;
            view.listener.status = "Disabled".into();
        }
    }),
    ("capturing", |ui| {
        *ui = portal_fixture();
        ui.capture = Capture::Listening {
            target: ShortcutTarget::Dictation,
            message: None,
            held: Default::default(),
        };
    }),
    ("chinese", |ui| {
        *ui = portal_fixture();
        ui.settings.ui_language = Some("zh".into());
        crate::desktop::i18n::apply(Some("zh"));
    }),
    ("menus", |ui| {
        *ui = portal_fixture();
        ui.open = Some(Menu::Microphone);
        ui.microphones = Ok(crate::audio::InputDevices {
            names: vec!["Built-in Audio Analog Stereo".into(), "Shure MV7".into()],
            system_default: Some("Shure MV7".into()),
            default_is_mumble_microphone: false,
            ..Default::default()
        });
    }),
    ("history", |ui| {
        *ui = portal_fixture();
        ui.history = history::HistoryPane::fixture();
        ui.pane = Pane::History;
    }),
    ("history-menu", |ui| {
        *ui = portal_fixture();
        ui.history = history::HistoryPane::fixture();
        ui.pane = Pane::History;
        ui.open = Some(Menu::HistoryRetention);
    }),
    ("history-clear", |ui| {
        *ui = portal_fixture();
        ui.history = history::HistoryPane::fixture();
        ui.history.clear_armed = true;
        ui.pane = Pane::History;
    }),
    // Labs › Dictation stats: the Stats button with its dialog open.
    ("history-stats", |ui| {
        *ui = portal_fixture();
        ui.history = history::HistoryPane::fixture();
        ui.history.stats = Some(history::HistoryPane::fixture_stats());
        ui.history.stats_open = true;
        ui.settings.labs.history_stats = true;
        crate::labs::apply(&ui.settings.labs);
        ui.pane = Pane::History;
    }),
    ("history-empty", |ui| {
        *ui = portal_fixture();
        ui.history = history::HistoryPane::fixture_empty();
        ui.pane = Pane::History;
    }),
    ("output", |ui| {
        *ui = portal_fixture();
        output::fixture(ui);
        ui.pane = Pane::Output;
    }),
    ("output-open", |ui| {
        *ui = portal_fixture();
        output::fixture(ui);
        ui.output.expanded = Some(0);
        ui.output.prompt_copied = true;
        ui.pane = Pane::Output;
    }),
    ("output-trust", |ui| {
        *ui = portal_fixture();
        output::trust_fixture(ui);
    }),
    ("output-empty", |ui| {
        *ui = portal_fixture();
        ui.pane = Pane::Output;
    }),
    ("modes", |ui| {
        *ui = portal_fixture();
        modes::fixture(ui);
        ui.pane = Pane::Modes;
    }),
    ("modes-opencode-not-running", |ui| {
        *ui = portal_fixture();
        modes::fixture(ui);
        ui.models = models::Models::fixture_not_running();
        ui.settings.dictation_processing.modes[0]
            .post_processing
            .enabled = false;
        ui.pane = Pane::Modes;
    }),
    ("modes-global", |ui| {
        *ui = portal_fixture();
        modes::fixture(ui);
        ui.modes.select(modes::Selection::Default);
        ui.pane = Pane::Modes;
    }),
    ("modes-picker", |ui| {
        *ui = portal_fixture();
        modes::fixture(ui);
        ui.modes.application_picker_open = true;
        ui.pane = Pane::Modes;
    }),
    ("modes-transformation-menu", |ui| {
        *ui = portal_fixture();
        modes::fixture(ui);
        ui.open = Some(Menu::Transformations);
        ui.pane = Pane::Modes;
    }),
    ("shortcut-mode", shortcut_mode::scene_modifier),
    ("shortcut-mode-key", shortcut_mode::scene_key),
    ("shortcut-mode-chord", shortcut_mode::scene_chord),
    ("shortcut-mode-motion", shortcut_mode::scene_motion),
    // The Shortcut row with a modifier-only shortcut, Left Option, beside
    // its side picker (macOS), in each face of the recorder.
    ("shortcut-row", |ui| shortcut_row(ui, Capture::Idle)),
    ("shortcut-row-right", |ui| {
        shortcut_row(ui, Capture::Idle);
        ui.settings.dictation_hotkey.modifiers.option = Some(ModifierSide::Right);
    }),
    // Two sided modifiers have no side picker, so their keycaps name the side.
    ("shortcut-row-sided-chord", |ui| {
        shortcut_row(ui, Capture::Idle);
        ui.settings.dictation_hotkey.modifiers.command = Some(ModifierSide::Right);
    }),
    ("shortcut-row-recording", |ui| {
        shortcut_row(ui, listening(None, HotkeyModifiers::default()));
    }),
    ("shortcut-row-held", |ui| {
        shortcut_row(ui, listening(None, HotkeyModifiers::option_command()));
    }),
    ("shortcut-row-rejected", |ui| {
        let message = crate::desktop::i18n::tr("Add a modifier");
        shortcut_row(ui, listening(Some(message), HotkeyModifiers::default()));
    }),
    ("shortcut-row-saved", |ui| {
        shortcut_row(
            ui,
            Capture::Saved {
                target: ShortcutTarget::Dictation,
                until: None,
            },
        );
    }),
    // The first run on Linux (Windows is the same window): the demo, the
    // practice game, and Mumble is ready. Transcription is cloud-only, so there
    // is no model download step.
    ("onboarding", |ui| {
        first_run(ui, FirstRun::Ready, Page::Demo)
    }),
    ("onboarding-demo", |ui| {
        first_run(ui, FirstRun::Ready, Page::Demo)
    }),
    // GNOME and KDE on Wayland: why the desktop asks before the first paste.
    ("onboarding-paste", |ui| {
        first_run(ui, FirstRun::PasteAccess, Page::Permissions)
    }),
    ("onboarding-try", |ui| {
        first_run(ui, FirstRun::Ready, Page::Try)
    }),
    ("onboarding-ready", |ui| {
        first_run(ui, FirstRun::Ready, Page::Ready)
    }),
    // Windows with its microphone privacy switch off.
    ("windows-onboarding-microphone", |ui| {
        *ui = fixture(DesktopShortcuts::Registered);
        ui.platform = platform::PlatformState::preview(platform::Capabilities::WINDOWS);
        ui.host = platform::PlatformHost::Fixture(Vec::new());
        ui.platform.permissions.microphone =
            crate::desktop::onboarding::PermissionState::NeedsSettings;
        onboarding::fixture(ui, crate::desktop::onboarding::Step::Setup);
        onboarding_kit::live::fixture(ui, Page::Permissions);
    }),
    // macOS and Windows surfaces, from preview data (see `platform`).
    ("mac-settings", |ui| *ui = mac_fixture()),
    // The microphone row following a recorded voice (`--video`), and with
    // the microphone released while idle, when no audio arrives.
    ("mac-settings-mic-level", |ui| {
        *ui = mac_fixture();
        ui.mic_level = general::MicLevel::playing(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vocabulary/audio/term-opencode-2.wav"
        )));
    }),
    ("mac-settings-mic-released", |ui| {
        *ui = mac_fixture();
        ui.settings.release_microphone_while_idle = true;
        ui.mic_level = general::MicLevel::closed();
    }),
    ("mac-settings-ducking", |ui| {
        *ui = mac_fixture();
        ui.settings.recording_audio_behavior = crate::app_settings::RecordingAudioBehavior::Duck;
    }),
    // The sound sets sheet at rest, then Wood's dictation recording.
    ("sound-sets", |ui| {
        *ui = mac_fixture();
        sound_sets::fixture(ui, None);
    }),
    ("sound-sets-playing", |ui| {
        *ui = mac_fixture();
        sound_sets::fixture(
            ui,
            Some((
                crate::feedback::SoundSet::Wood,
                crate::feedback::Audition::Dictation,
                900.0,
            )),
        );
    }),
    ("mac-microphone-missing", |ui| {
        *ui = mac_fixture();
        ui.settings.microphone = Some("Shure MV7".into());
        ui.open = Some(Menu::Microphone);
    }),
    ("mac-microphone-lid-closed", |ui| {
        *ui = mac_fixture();
        if let Ok(devices) = &mut ui.microphones {
            devices.system_default = Some("Studio Display Microphone".into());
            devices.off_with_lid_closed = Some("MacBook Pro Microphone".into());
            devices.default_is_off_with_lid_closed = true;
        }
        ui.open = Some(Menu::Microphone);
    }),
    // System default passes over the Bluetooth default for the built-in mic.
    ("mac-microphone-bluetooth", |ui| {
        *ui = mac_fixture();
        if let Ok(devices) = &mut ui.microphones {
            devices.system_default = Some("MacBook Pro Microphone".into());
            devices.bluetooth_default_passed_over = Some("Kit Pods".into());
        }
        ui.advanced = vec![Pane::General];
    }),
    ("mac-settings-vocabulary", |ui| {
        *ui = mac_fixture();
        vocabulary::fixture(ui, true);
    }),
    ("mac-settings-vocabulary-empty", |ui| {
        *ui = mac_fixture();
        vocabulary::fixture(ui, false);
    }),
    ("mac-settings-recording", |ui| {
        *ui = mac_fixture();
        let held = crate::app_settings::HotkeyModifiers {
            option: Some(crate::app_settings::ModifierSide::Either),
            command: Some(crate::app_settings::ModifierSide::Either),
            ..Default::default()
        };
        ui.capture = Capture::Listening {
            target: ShortcutTarget::Dictation,
            message: None,
            held,
        };
    }),
    ("mac-settings-permissions", |ui| {
        platform_settings::mac_permissions_fixture(ui, false)
    }),
    ("mac-settings-permissions-stale", |ui| {
        platform_settings::mac_permissions_fixture(ui, true)
    }),
    ("mac-settings-pause-browser", |ui| {
        *ui = mac_fixture();
        ui.settings.recording_audio_behavior =
            crate::app_settings::RecordingAudioBehavior::PauseMedia;
        ui.settings.pause_browser_media = true;
        ui.platform.system_audio = false;
    }),
    ("mac-settings-advanced", |ui| {
        *ui = mac_fixture();
        ui.advanced = vec![Pane::General];
    }),
    ("mac-settings-dev-channel", |ui| {
        *ui = mac_fixture();
        ui.settings.updates.channel = Some(crate::app_settings::UpdateChannel::Dev);
    }),
    // A local build: the badge, the quiet status line, no channel row.
    ("mac-local-build", |ui| {
        *ui = mac_fixture();
        ui.build_preview = Some(BuildKind::Local);
        if let Some(view) = &mut ui.view {
            view.update_status = DesktopUpdateStatus::SourceBuild;
        }
    }),
    ("mac-test-build", |ui| {
        *ui = mac_fixture();
        ui.build_preview = Some(BuildKind::Test);
    }),
    ("mac-update-ready", |ui| {
        *ui = mac_fixture();
        if let Some(view) = &mut ui.view {
            view.update_status = DesktopUpdateStatus::ReadyToRestart;
        }
    }),
    ("mac-update-checking", |ui| {
        *ui = mac_fixture();
        if let Some(view) = &mut ui.view {
            view.update_status = DesktopUpdateStatus::Checking;
        }
    }),
    ("mac-update-downloading", |ui| {
        *ui = mac_fixture();
        if let Some(view) = &mut ui.view {
            view.update_status = DesktopUpdateStatus::Downloading;
        }
    }),
    ("windows-settings", |ui| {
        *ui = fixture(DesktopShortcuts::Registered);
        ui.platform = platform::PlatformState::preview(platform::Capabilities::WINDOWS);
        ui.host = platform::PlatformHost::Fixture(Vec::new());
        ui.settings.paste_with_shift = false;
    }),
    ("mac-voice-filter", |ui| {
        *ui = mac_fixture();
        ui.pane = Pane::VoiceFilter;
    }),
    ("mac-voice-filter-muted", |ui| {
        *ui = mac_fixture();
        ui.platform.tuning.mode = crate::audio::voice_filter::VoiceFilterMode::Muted;
        ui.platform.demo = Some(fun::preview_demo(
            false,
            Some(crate::audio::mumble_microphone::DemoPlaybackTrack::CallHears),
        ));
        ui.pane = Pane::VoiceFilter;
    }),
    ("mac-voice-filter-driver-missing", |ui| {
        *ui = mac_fixture();
        if let Some(driver) = &mut ui.platform.mumble_microphone {
            driver.driver_bundle_installed = false;
            driver.driver_loaded = false;
            driver.installed_version = None;
            driver.active_apps.clear();
        }
        ui.platform.demo = Some(fun::preview_demo(true, None));
        ui.pane = Pane::VoiceFilter;
    }),
    ("voice-filter", |ui| {
        *ui = fixture(DesktopShortcuts::Commands);
        ui.platform = platform::PlatformState::preview(platform::Capabilities::LINUX);
        ui.host = platform::PlatformHost::Fixture(Vec::new());
        ui.settings.experimental.mumble_microphone = true;
        ui.pane = Pane::VoiceFilter;
    }),
    ("voice-filter-unavailable", |ui| {
        *ui = fixture(DesktopShortcuts::Commands);
        ui.platform = platform::PlatformState::preview(platform::Capabilities::LINUX);
        ui.host = platform::PlatformHost::Fixture(Vec::new());
        if let Some(source) = &mut ui.platform.mumble_microphone {
            source.driver_loaded = false;
            source.action_state = crate::audio::mumble_microphone::DriverActionState::Failed(
                "pw-cat was not found; Mumble Microphone needs PipeWire".into(),
            );
        }
        ui.settings.experimental.mumble_microphone = true;
        ui.pane = Pane::VoiceFilter;
    }),
    ("mac-hud-lab", |ui| {
        *ui = mac_fixture();
        ui.pane = Pane::HudLab;
    }),
    ("mac-labs", labs_with_some_on),
    // Where mumble://labs/live-text lands: its group's chip.
    ("mac-labs-link", |ui| {
        *ui = mac_fixture();
        ui.labs_open = true;
        ui.lab_store.group = Some(crate::labs::Feature::LiveText.group());
    }),
    ("mac-labs-on", |ui| {
        labs_with_some_on(ui);
        ui.lab_store.tab = labs::Tab::On;
    }),
    ("mac-labs-category", |ui| {
        labs_with_some_on(ui);
        ui.lab_store.group = Some(crate::labs::Group::Shortcuts);
    }),
    ("mac-labs-search", |ui| {
        labs_with_some_on(ui);
        ui.lab_store.search = rune::TextEdit::new("paste".to_string());
    }),
    // Labs on Windows and Linux: only their OS's labs, groups holding a
    // lab that's on open.
    ("windows-labs", |ui| {
        *ui = fixture(DesktopShortcuts::Registered);
        ui.platform = platform::PlatformState::preview(platform::Capabilities::WINDOWS);
        ui.host = platform::PlatformHost::Fixture(Vec::new());
        ui.settings.paste_with_shift = true;
        ui.labs_open = true;
    }),
    ("linux-labs", |ui| {
        *ui = portal_fixture();
        ui.platform.caps = platform::Capabilities::LINUX;
        ui.settings.labs.section_key_alone = true;
        ui.labs_open = true;
    }),
    ("mac-activity", |ui| {
        *ui = mac_fixture();
        ui.activity = activity::ActivityPane::fixture();
        ui.pane = Pane::Activity;
    }),
    // Toggle's cancel shortcut under the shortcut mode, on its default
    // (Escape), and on Wayland, where the desktop binds it.
    ("toggle-cancel", |ui| {
        *ui = mac_fixture();
        ui.settings.shortcut_mode = crate::dictation::ShortcutMode::Toggle;
    }),
    ("portal-toggle-cancel", |ui| {
        *ui = portal_fixture();
        ui.settings.shortcut_mode = crate::dictation::ShortcutMode::Toggle;
    }),
    ("mac-settings-sides", |ui| {
        *ui = mac_fixture();
        ui.advanced = vec![Pane::General];
        ui.settings.dictation_hotkey = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(crate::app_settings::ModifierSide::Left),
                ..HotkeyModifiers::default()
            },
            key: None,
        };
    }),
    // The first run on macOS: each permission state, the practice game, and
    // Mumble is ready.
    ("mac-onboarding-permissions", |ui| {
        mac_first_run(ui, onboarding::MacStep::Setup, Page::Permissions)
    }),
    ("mac-onboarding-denied", |ui| {
        mac_first_run(ui, onboarding::MacStep::SetupDenied, Page::Permissions)
    }),
    ("mac-onboarding-stale", |ui| {
        mac_first_run(ui, onboarding::MacStep::SetupStale, Page::Permissions)
    }),
    ("mac-onboarding-restart", |ui| {
        mac_first_run(ui, onboarding::MacStep::SetupRestart, Page::Permissions)
    }),
    ("mac-onboarding-try", |ui| {
        mac_first_run(ui, onboarding::MacStep::TryIt, Page::Try)
    }),
    ("mac-onboarding-reopen", |ui| {
        mac_first_run(ui, onboarding::MacStep::TryItReopen, Page::Try)
    }),
    ("mac-onboarding-ready", |ui| {
        mac_first_run(ui, onboarding::MacStep::Done, Page::Ready)
    }),
];

/// The tour state a Linux first-run scene starts from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstRun {
    /// Setup is ready.
    Ready,
    /// Setup is ready, and the desktop asks before Mumble's first paste (GNOME
    /// and KDE on Wayland).
    PasteAccess,
}

fn first_run(ui: &mut Ui, state: FirstRun, page: Page) {
    *ui = portal_fixture();
    onboarding::fixture(ui, crate::desktop::onboarding::Step::Setup);
    if state == FirstRun::PasteAccess {
        ui.platform.paste = Some(paste_access::PasteAccess::Ask);
    }
    onboarding::poll(ui);
    onboarding_kit::live::fixture(ui, page);
}

fn mac_first_run(ui: &mut Ui, step: onboarding::MacStep, page: Page) {
    *ui = mac_fixture();
    onboarding::mac_fixture(ui, step);
    onboarding_kit::live::fixture(ui, page);
}

pub(super) fn mac_fixture() -> Ui {
    let mut ui = fixture(DesktopShortcuts::Registered);
    ui.platform = platform::PlatformState::preview(platform::Capabilities::MACOS);
    ui.host = platform::PlatformHost::Fixture(Vec::new());
    ui.settings.dictation_hotkey = AppSettings::default().dictation_hotkey;
    ui.settings.paste_with_shift = false;
    ui.settings.experimental.mumble_microphone = true;
    ui.microphones = Ok(crate::audio::InputDevices {
        names: vec![
            "Kit Pods".into(),
            "MacBook Pro Microphone".into(),
            "Studio Display Microphone".into(),
        ],
        system_default: Some("Kit Pods".into()),
        default_is_mumble_microphone: false,
        ..Default::default()
    });
    ui
}

fn shortcut_row(ui: &mut Ui, capture: Capture) {
    *ui = mac_fixture();
    ui.settings.dictation_hotkey = HotkeyBinding {
        modifiers: HotkeyModifiers {
            option: Some(ModifierSide::Left),
            ..HotkeyModifiers::default()
        },
        key: None,
    };
    ui.capture = capture;
}

fn listening(message: Option<&'static str>, held: HotkeyModifiers) -> Capture {
    Capture::Listening {
        target: ShortcutTarget::Dictation,
        message,
        held,
    }
}

pub(super) fn portal_fixture() -> Ui {
    fixture(DesktopShortcuts::Portal(vec![
        DesktopGrantedShortcut {
            name: "Dictation".into(),
            trigger: Some("F12".into()),
            unconfirmed: false,
        },
        DesktopGrantedShortcut {
            name: "Paste last dictation".into(),
            trigger: Some("Ctrl+Alt+V".into()),
            unconfirmed: false,
        },
    ]))
}

/// Deterministic settings and service state matching a typical Wayland
/// desktop with the shortcuts portal.
pub(super) fn fixture(shortcuts: DesktopShortcuts) -> Ui {
    crate::desktop::i18n::apply(Some("en"));
    let settings = AppSettings {
        dictation_hotkey: HotkeyBinding {
            modifiers: HotkeyModifiers::default(),
            key: Some(HotkeyKey::named("f12")),
        },
        transcription: crate::transcription::TranscriptionSelection::default(),
        paste_with_shift: true,
        ..AppSettings::default()
    };
    let mut ui = Ui::with(settings, Service::Fixture(Vec::new()));
    ui.models = models::Models::fixture();
    ui.view = Some(ServiceView {
        listener: DesktopListenerSnapshot {
            running: true,
            status: "Enabled".into(),
            warning: None,
            shortcuts,
            shortcuts_configurable: false,
        },
        operation_error: None,
        update_status: DesktopUpdateStatus::Current,
        update_notes: None,
        update_blocked: None,
        editing: false,
    });
    ui
}

/// Labs with a few labs on: every store scene's fixture, so a lab reads
/// the same in each.
fn labs_with_some_on(ui: &mut Ui) {
    *ui = mac_fixture();
    for feature in [
        crate::labs::Feature::LiveText,
        crate::labs::Feature::SmartSpacing,
        crate::labs::Feature::TapOrHold,
    ] {
        feature.set(&mut ui.settings, true);
    }
    ui.labs_open = true;
}
