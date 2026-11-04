//! The macOS window's host: the runtime thread `macos_rune` starts in this
//! process, which owns model preparation and publishes the listener, model,
//! update, and settings state, plus the OS-only work the panes ask for
//! (permissions, the login item, Hex Microphone).
//!
//! Shortcut capture stays in the window: an AppKit local event
//! monitor records the chord, because a modifier-only binding such as Option
//! or Fn never reaches Rune as a key event. The listener's event tap is
//! suspended meanwhile so the current binding can be recorded again.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use block2::RcBlock;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};

use super::platform::{
    HudDemo, HudLab, NativeHost, PlatformRequest, PlatformState, poll_voice_filter,
    voice_filter_request,
};
use super::practice::PracticeFeed;
use super::settings::Chord;
use super::{HostState, ServiceView};
use crate::app_settings::{HotkeyBinding, HotkeyKey, HotkeyModifiers, modifiers_from_flags};
use crate::desktop::host::DesktopAction;
use crate::desktop::i18n::tr;
use crate::desktop::login_item::{LoginItemFailure, LoginItemRequest, LoginItemWorker};
use crate::desktop::onboarding::{
    Input, PermissionAction, PermissionKind, PermissionState, StalePermissionTracker,
};
use crate::macos_rune::RuntimeHandle;
pub(super) use crate::macos_rune::{Request, ShortcutTarget};

/// The window saves a recorded shortcut; the listener follows `settings.json`.
pub(super) const HOST_SAVES_SHORTCUTS: bool = false;
pub(super) const RECORDS_KEY_EVENTS: bool = false;
pub(super) const TERMINAL_PASTE: bool = false;
/// Quit Hex lives in the menu bar and the app menu (⌘Q).
pub(super) const QUIT_IN_WINDOW: bool = false;

/// How often the native host re-reads permissions and driver state.
const PLATFORM_REFRESH: Duration = Duration::from_secs(1);
const ESCAPE_KEY_CODE: u16 = 53;

/// The runtime is found through its own module; nothing is handed over.
#[derive(Default)]
pub(crate) struct Connect {
    _nothing: (),
}

pub(super) struct Backend {
    runtime: Option<RuntimeHandle>,
    wake: Arc<dyn Fn() + Send + Sync>,
    chords: Option<ChordWatch>,
}

impl Backend {
    pub(super) fn connect(_: Connect, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let runtime = crate::macos_rune::runtime();
        if let Some(runtime) = &runtime {
            runtime.subscribe(wake.clone());
        }
        Self {
            runtime,
            wake,
            chords: None,
        }
    }

    /// Runs on the main thread (Rune's handlers), where the event monitor
    /// lives.
    pub(super) fn send(&mut self, request: Request) -> Result<()> {
        match request {
            Request::CaptureShortcutFor(_) => {
                self.chords = Some(ChordWatch::start(self.wake.clone())?);
                Ok(())
            }
            Request::CancelShortcut => {
                self.chords = None;
                Ok(())
            }
            Request::Desktop(
                action @ (DesktopAction::CheckForUpdates
                | DesktopAction::OpenUpdate
                | DesktopAction::RestartIntoUpdate
                | DesktopAction::ConfirmUpdateSwitch
                | DesktopAction::WindowFocused),
            ) => crate::macos_rune::update_action(&action),
            request => match &self.runtime {
                Some(runtime) => runtime.send(request),
                None => Err(eyre!("Hex's runtime is not running")),
            },
        }
    }

    /// Saves `settings.json`, then asks the runtime to reload it. The
    /// listener follows the saved file itself.
    pub(super) fn save(&mut self, settings: &crate::app_settings::AppSettings) -> Result<()> {
        settings.save()?;
        self.send(Request::ReloadSettings)
    }

    pub(super) fn quit(&mut self) {
        crate::macos_rune::quit();
    }

    /// Rune hides the window (`keep_running`); the window's own close hook
    /// restores the Dock policy (`macos_rune::window_closed`).
    pub(super) fn closed(&mut self) {}

    /// The runtime publishes into shared memory; nothing to pull.
    pub(super) fn refresh(&mut self) {}

    pub(super) fn error(&self) -> Option<String> {
        self.runtime
            .is_none()
            .then(|| "Hex's runtime is not running".into())
    }

    pub(super) fn state(&self) -> Option<HostState> {
        let published = self.runtime.as_ref()?.published()?;
        Some(HostState {
            view: ServiceView {
                listener: published.listener.clone(),
                operation_error: None,
                transcription: published.transcription.clone(),
                update_status: published.update_status.clone(),
                update_notes: published.update_notes.clone(),
                editing: false,
            },
            settings: published.settings.clone(),
        })
    }

    pub(super) fn take_chord(&mut self) -> Option<Chord> {
        let chord = self.chords.as_ref()?.results.try_recv().ok();
        if matches!(chord, Some(Chord::Cancelled)) {
            self.chords = None;
        }
        chord
    }
}

/// Records chords from the window's keyboard events while Settings listens,
/// with the listener's event tap suspended. Removing the monitor ends it.
struct ChordWatch {
    results: Receiver<Chord>,
    monitor: Retained<AnyObject>,
}

impl ChordWatch {
    fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self> {
        crate::app_settings::set_hotkey_capture_active(true);
        let (sender, results) = mpsc::channel();
        // Labs › Sided modifier chords is read once, as recording starts.
        let capture = RefCell::new(ChordCapture::with_sides(
            crate::labs::sided_modifier_chords(),
        ));
        let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a live event for the handler's duration.
            let event_ref = unsafe { event.as_ref() };
            let flags = event_ref.modifierFlags().bits() as u64;
            let chord = match event_ref.r#type() {
                NSEventType::KeyDown => {
                    let characters = event_ref
                        .charactersIgnoringModifiers()
                        .map(|characters| characters.to_string());
                    capture.borrow_mut().key_down(
                        event_ref.keyCode(),
                        event_ref.isARepeat(),
                        characters.as_deref(),
                        flags,
                    )
                }
                NSEventType::FlagsChanged => capture.borrow_mut().flags_changed(flags),
                // Mouse buttons record only with Labs › Mouse shortcuts on;
                // otherwise the click reaches the window as usual.
                NSEventType::LeftMouseDown
                | NSEventType::RightMouseDown
                | NSEventType::OtherMouseDown
                    if crate::labs::mouse_shortcuts() =>
                {
                    capture
                        .borrow_mut()
                        .mouse_down(event_ref.buttonNumber(), flags)
                }
                // Key releases still reach the window so its key state
                // stays balanced.
                _ => return event.as_ptr(),
            };
            if let Some(chord) = chord
                && sender.send(chord).is_ok()
            {
                wake();
            }
            std::ptr::null_mut()
        });
        // SAFETY: the block lives as long as AppKit retains it; the monitor
        // is removed on drop, on the main thread that installed it.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::KeyDown
                    | NSEventMask::FlagsChanged
                    | NSEventMask::LeftMouseDown
                    | NSEventMask::RightMouseDown
                    | NSEventMask::OtherMouseDown,
                &block,
            )
        };
        match monitor {
            Some(monitor) => Ok(Self { results, monitor }),
            None => {
                crate::app_settings::set_hotkey_capture_active(false);
                Err(eyre!("could not watch the keyboard to record a shortcut"))
            }
        }
    }
}

impl Drop for ChordWatch {
    fn drop(&mut self) {
        // SAFETY: the monitor came from addLocalMonitor on this thread.
        unsafe { NSEvent::removeMonitor(&self.monitor) };
        crate::app_settings::set_hotkey_capture_active(false);
    }
}

/// Turns key and modifier events into a chord: a key with its held
/// modifiers, or the most modifier keys held at once when they are all
/// released without a key, with the sides `recorded_sides` keeps (or,
/// with Labs › Sided modifier chords on, the side each was pressed on).
#[derive(Default)]
struct ChordCapture {
    peak: HotkeyModifiers,
    /// A key went down with these modifiers; their release records nothing.
    keyed: bool,
    /// A modifier-only chord keeps every modifier's side, such as
    /// Right ⌘ + Right ⌥, instead of widening each to either side.
    sided: bool,
}

impl ChordCapture {
    fn with_sides(sided: bool) -> Self {
        Self {
            sided,
            ..Self::default()
        }
    }

    fn mouse_down(&mut self, button: isize, flags: u64) -> Option<Chord> {
        let key = u16::try_from(button).ok().and_then(HotkeyKey::mouse)?;
        let mut modifiers = modifiers_from_flags(flags).without_side_constraints();
        modifiers.function = false;
        self.keyed = !modifiers.is_empty();
        self.peak = HotkeyModifiers::default();
        Some(Chord::Captured(Ok(HotkeyBinding {
            modifiers,
            key: Some(key),
        })))
    }

    fn key_down(
        &mut self,
        code: u16,
        repeat: bool,
        characters: Option<&str>,
        flags: u64,
    ) -> Option<Chord> {
        if repeat {
            return None;
        }
        if code == ESCAPE_KEY_CODE {
            return Some(Chord::Cancelled);
        }
        let mut modifiers = modifiers_from_flags(flags).without_side_constraints();
        // Arrow and function keys report Fn; a chord never binds it.
        modifiers.function = false;
        // Only modifiers held with the key are spent by it.
        self.keyed = !modifiers.is_empty();
        self.peak = HotkeyModifiers::default();
        let key = match hotkey_key(code, characters) {
            Ok(key) => key,
            Err(message) => return Some(Chord::Captured(Err(message))),
        };
        if modifiers.is_empty() && !is_function_key(&key.label) && !is_navigation_key(code) {
            return Some(Chord::Captured(Err(tr("Add a modifier"))));
        }
        Some(Chord::Captured(Ok(HotkeyBinding {
            modifiers,
            key: Some(key),
        })))
    }

    /// While modifiers are down, reports the ones their release would record.
    fn flags_changed(&mut self, flags: u64) -> Option<Chord> {
        let held = modifiers_from_flags(flags);
        let current = if self.sided {
            held
        } else {
            held.recorded_sides()
        };
        if !current.is_empty() {
            if self.keyed {
                return None;
            }
            if current.count() >= self.peak.count() {
                self.peak = current;
            }
            return Some(Chord::Held(self.peak));
        }
        let keyed = std::mem::take(&mut self.keyed);
        let peak = std::mem::take(&mut self.peak);
        if keyed || peak.is_empty() {
            return None;
        }
        Some(Chord::Captured(Ok(HotkeyBinding {
            modifiers: peak,
            key: None,
        })))
    }
}

fn is_function_key(label: &str) -> bool {
    label
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=20).contains(&number))
}

/// Home, End, Page Up, Page Down, and Help (Insert on PC keyboards): keys
/// that type nothing, so, like a function key, one may be a shortcut alone,
/// as it could before (#1138).
fn is_navigation_key(code: u16) -> bool {
    matches!(code, 115 | 119 | 116 | 121 | 114)
}

/// The key a virtual key code records, and its label.
fn hotkey_key(code: u16, characters: Option<&str>) -> Result<HotkeyKey, &'static str> {
    let special = match code {
        49 => Some("Space"),
        48 => Some("Tab"),
        36 => Some("Return"),
        51 => Some(tr("Delete")),
        126 => Some("Up"),
        125 => Some("Down"),
        123 => Some(tr("Left")),
        124 => Some(tr("Right")),
        116 => Some("Page Up"),
        121 => Some("Page Down"),
        115 => Some("Home"),
        119 => Some("End"),
        117 => Some("Forward Delete"),
        114 => Some("Help"),
        122 => Some("F1"),
        120 => Some("F2"),
        99 => Some("F3"),
        118 => Some("F4"),
        96 => Some("F5"),
        97 => Some("F6"),
        98 => Some("F7"),
        100 => Some("F8"),
        101 => Some("F9"),
        109 => Some("F10"),
        103 => Some("F11"),
        111 => Some("F12"),
        105 => Some("F13"),
        107 => Some("F14"),
        113 => Some("F15"),
        106 => Some("F16"),
        64 => Some("F17"),
        79 => Some("F18"),
        80 => Some("F19"),
        90 => Some("F20"),
        _ => None,
    };
    if let Some(label) = special {
        return Ok(HotkeyKey {
            code,
            label: label.into(),
        });
    }
    let mut chars = characters.unwrap_or_default().chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) if !character.is_control() => Ok(HotkeyKey {
            code,
            label: character.to_uppercase().collect(),
        }),
        _ => Err(tr("Unsupported key")),
    }
}

/// The key a typed name records, with the label recording it would show.
pub(super) fn hotkey_key_named(name: &str) -> Result<HotkeyKey, &'static str> {
    let normalized: String = name
        .chars()
        .filter(|character| !matches!(character, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect();
    if let Some(number) = normalized.strip_prefix("mouse") {
        return number
            .parse::<u16>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(HotkeyKey::mouse)
            .ok_or(tr("Unsupported key"));
    }
    let code = match normalized.as_str() {
        "space" => Some(49),
        "tab" => Some(48),
        "return" | "enter" => Some(36),
        "delete" | "backspace" => Some(51),
        "up" => Some(126),
        "down" => Some(125),
        "left" => Some(123),
        "right" => Some(124),
        "pageup" => Some(116),
        "pagedown" => Some(121),
        "home" => Some(115),
        "end" => Some(119),
        "forwarddelete" => Some(117),
        "help" => Some(114),
        function => function
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .and_then(|number| {
                const CODES: [u16; 20] = [
                    122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64,
                    79, 80, 90,
                ];
                CODES.get(usize::from(number).checked_sub(1)?).copied()
            }),
    };
    if let Some(code) = code {
        return hotkey_key(code, None);
    }
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => {
            let character = character.to_lowercase().next().unwrap_or(character);
            let code =
                crate::keyboard::key_code_for(character).map_err(|_| tr("Unsupported key"))?;
            hotkey_key(code, Some(&character.to_string()))
        }
        _ => Err(tr("Unsupported key")),
    }
}

/// macOS accepts every binding the capture can produce, except that Paste
/// Last fires only on a key press (`suppression::paste_action`), and the
/// capture shortcut, pressed once to start and again to finish, and the
/// send-to list's need a key.
pub(super) fn validate_hotkey(
    target: ShortcutTarget,
    binding: &HotkeyBinding,
) -> Result<(), &'static str> {
    if matches!(target, ShortcutTarget::PasteLast | ShortcutTarget::Cancel) && binding.key.is_none()
    {
        return Err(tr("Add a key"));
    }
    Ok(())
}

/// The listener's retained history, shared with the window.
pub(super) fn history(
    _: &Connect,
    retention: crate::history::HistoryRetention,
) -> Option<crate::history::History> {
    match crate::macos_rune::runtime() {
        Some(runtime) => runtime.history(),
        None => crate::history::History::open_default(retention)
            .inspect_err(|error| tracing::warn!(%error, "could not open retained history"))
            .ok(),
    }
}

/// The listener knows the frontmost application.
pub(super) fn focused_application_known() -> bool {
    true
}

/// The native host reports the login item (`PlatformState::login_item`).
pub(super) fn launch_at_login() -> Option<Result<bool, String>> {
    None
}

pub(super) fn set_launch_at_login(_: bool) -> Result<bool, String> {
    Err("Launch at login goes through the macOS host".into())
}

pub(super) fn native_host(wake: Arc<dyn Fn() + Send + Sync>) -> Box<dyn NativeHost> {
    Box::new(MacHost::new(wake))
}

fn stale_slot(kind: PermissionKind) -> Option<usize> {
    match kind {
        PermissionKind::Accessibility => Some(0),
        PermissionKind::SystemAudio => Some(1),
        PermissionKind::Microphone => None,
    }
}

/// Performs the panes' OS-only requests and refreshes their state once a
/// second (permission checks and the driver status are not free per frame).
struct MacHost {
    wake: Arc<dyn Fn() + Send + Sync>,
    login: Option<LoginItemWorker>,
    login_checking: bool,
    refresh_at: Instant,
    practice: Option<PracticeFeed>,
    /// Tracks System Settings return timing for Accessibility and System
    /// Audio.
    stale: [StalePermissionTracker; 2],
    /// In-flight `"Still grey? Fix it"` resets, indexed by [`stale_slot`].
    resetting: [bool; 2],
    reset_tx: mpsc::Sender<PermissionKind>,
    reset_rx: Receiver<PermissionKind>,
}

impl MacHost {
    fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let login = LoginItemWorker::new()
            .inspect_err(|error| tracing::warn!(%error, "login item worker is unavailable"))
            .ok();
        let (reset_tx, reset_rx) = mpsc::channel();
        let mut host = Self {
            wake,
            login,
            login_checking: false,
            refresh_at: Instant::now(),
            practice: None,
            stale: Default::default(),
            resetting: [false; 2],
            reset_tx,
            reset_rx,
        };
        let _ = host.request_login(LoginItemRequest::Status);
        host
    }

    fn note_permission_asked(&mut self, kind: PermissionKind) {
        if let Some(slot) = stale_slot(kind) {
            self.stale[slot].note_asked(Instant::now());
        }
    }

    fn finish_permission_reset(&mut self, kind: PermissionKind) {
        if let Some(slot) = stale_slot(kind) {
            self.resetting[slot] = false;
            self.stale[slot].note_asked(Instant::now());
        }
        match kind {
            PermissionKind::Accessibility => {
                if crate::desktop::onboarding::status().accessibility != PermissionState::Ready {
                    let _ = crate::desktop::permission_guide::prompt_accessibility_trust();
                    crate::desktop::permission_guide::show(PermissionKind::Accessibility);
                }
            }
            PermissionKind::SystemAudio => {
                crate::audio::system_audio::reset_request();
                if !crate::audio::system_audio::ready()
                    && !crate::audio::system_audio::request_access()
                {
                    crate::desktop::permission_guide::show(PermissionKind::SystemAudio);
                }
            }
            PermissionKind::Microphone => {}
        }
        self.refresh_at = Instant::now();
    }

    fn request_login(&mut self, request: LoginItemRequest) -> Result<(), String> {
        let login = self
            .login
            .as_mut()
            .ok_or("Launch at login is unavailable")?;
        login.request(request).inspect_err(|error| {
            tracing::warn!(%error, "could not ask for the login item");
        })?;
        self.login_checking = true;
        Ok(())
    }

    fn runtime(&self) -> Result<RuntimeHandle, String> {
        crate::macos_rune::runtime().ok_or_else(|| "Hex's runtime is not running".into())
    }
}

impl NativeHost for MacHost {
    fn send(&mut self, request: PlatformRequest) -> Result<(), String> {
        match request {
            PlatformRequest::Permission(PermissionAction::RequestMicrophone) => {
                crate::desktop::onboarding::request_microphone();
            }
            PlatformRequest::Permission(PermissionAction::ShowGuide(kind)) => {
                self.note_permission_asked(kind);
                crate::desktop::permission_guide::show(kind);
            }
            PlatformRequest::Permission(PermissionAction::ResetAndPrompt(kind)) => {
                if let Some(slot) = stale_slot(kind)
                    && !self.resetting[slot]
                {
                    self.resetting[slot] = true;
                    self.stale[slot].note_asked(Instant::now());
                    let tx = self.reset_tx.clone();
                    let wake = self.wake.clone();
                    crate::desktop::permission_guide::reset_and_reopen_async(kind, move |done| {
                        let _ = tx.send(done);
                        wake();
                    });
                }
            }
            PlatformRequest::Permission(PermissionAction::Restart) => {
                if !crate::desktop::onboarding::spawn_relaunch() {
                    return Err("could not restart Hex".into());
                }
                crate::macos_rune::quit();
            }
            PlatformRequest::SystemAudio => {
                self.note_permission_asked(PermissionKind::SystemAudio);
                crate::desktop::permission_guide::clear_legacy_entries_async(
                    PermissionKind::SystemAudio,
                );
                // macOS prompts once; after that the switch is in System
                // Settings, with the guide beside it.
                if !crate::audio::system_audio::request_access() {
                    crate::desktop::permission_guide::show(PermissionKind::SystemAudio);
                }
            }
            PlatformRequest::SetLaunchAtLogin(enabled) => {
                self.request_login(LoginItemRequest::SetEnabled(enabled))?;
            }
            PlatformRequest::OpenLoginItemSettings => {
                let _ = self.request_login(LoginItemRequest::OpenSettings);
            }
            PlatformRequest::InstallHexMicrophone
            | PlatformRequest::UninstallHexMicrophone
            | PlatformRequest::SetTuning(_)
            | PlatformRequest::HexMicrophoneDemo(_) => voice_filter_request(request),
            PlatformRequest::Relaunch => {
                if let Err(error) = crate::desktop::onboarding::record_relaunch(
                    crate::desktop::onboarding::unix_ms(),
                ) {
                    tracing::warn!(%error, "could not record the onboarding relaunch");
                }
                if !crate::desktop::onboarding::spawn_relaunch() {
                    return Err("could not relaunch Hex".into());
                }
                crate::macos_rune::quit();
            }
            PlatformRequest::ConfigureHud(lab) => configure_hud(&self.runtime()?, lab),
            PlatformRequest::TuneHud(lab) => tune_hud(&self.runtime()?, lab),
            PlatformRequest::EndHudDemo => self
                .runtime()?
                .indicator(crate::dictation::indicator::DictationIndicatorEvent::Reset),
            PlatformRequest::SaveDotScope(tuning) => {
                tuning.save().map_err(|error| format!("{error:#}"))?;
            }
        }
        // Show the result of an action at once, not on the next refresh.
        self.refresh_at = Instant::now();
        Ok(())
    }

    fn poll(&mut self, state: &mut PlatformState) -> bool {
        let mut changed = false;
        for &kind in &state.permissions_shown {
            crate::desktop::permission_guide::clear_legacy_entries_async(kind);
        }
        while let Ok(kind) = self.reset_rx.try_recv() {
            self.finish_permission_reset(kind);
            state.clear_stale(kind);
            changed = true;
        }
        // The playground's HUD stories change the tuning HUD Lab shows.
        let shared = crate::rune_hud::dot_scope::lab();
        if shared != state.hud.tuning {
            state.hud.tuning = shared;
            changed = true;
        }
        if let Some(login) = &mut self.login {
            match login.poll() {
                Ok(Some(answer)) => {
                    // A later click may already be running behind this answer.
                    let settled = login.idle();
                    self.login_checking = !settled;
                    state.login_item_answered(answer, settled);
                    changed = true;
                }
                Ok(None) => {}
                Err(message) => {
                    self.login = None;
                    self.login_checking = false;
                    let failure = LoginItemFailure {
                        status: None,
                        message,
                    };
                    state.login_item_answered(Err(failure), true);
                    changed = true;
                }
            }
        }
        let refresh = Instant::now() >= self.refresh_at;
        changed |= poll_voice_filter(state, refresh);
        if Instant::now() < self.refresh_at {
            return changed;
        }
        self.refresh_at = Instant::now() + PLATFORM_REFRESH;
        changed |= update(
            &mut state.accessibility_renamed,
            crate::desktop::permission_guide::list_name(PermissionKind::Accessibility)
                != "Accessibility",
        );
        changed |= update(
            &mut state.activity_log,
            crate::app_paths::logs_dir()
                .ok()
                .map(|logs| logs.join("live.ndjson")),
        );
        let permissions = crate::desktop::onboarding::status();
        let system_audio = crate::audio::system_audio::ready();
        changed |= update(&mut state.permissions, permissions);
        changed |= update(&mut state.system_audio, system_audio);
        let asked_len = state.permissions_asked.len();
        state.permissions_asked.retain(|kind| match kind {
            PermissionKind::Microphone => permissions.microphone != PermissionState::Ready,
            PermissionKind::Accessibility => permissions.accessibility != PermissionState::Ready,
            PermissionKind::SystemAudio => !system_audio,
        });
        changed |= state.permissions_asked.len() != asked_len;
        let now = Instant::now();
        let focus = self
            .stale
            .iter()
            .any(StalePermissionTracker::is_waiting)
            .then(crate::desktop::permission_guide::settings_focus)
            .unwrap_or_default();
        let mut next_stale = Vec::new();
        for (slot, (kind, untrusted)) in [
            (
                PermissionKind::Accessibility,
                matches!(
                    permissions.accessibility,
                    PermissionState::NeedsRequest | PermissionState::NeedsSettings
                ),
            ),
            (PermissionKind::SystemAudio, !system_audio),
        ]
        .into_iter()
        .enumerate()
        {
            if !self.resetting[slot] && self.stale[slot].poll(now, untrusted, focus) {
                next_stale.push(kind);
            }
        }
        changed |= update(&mut state.permissions_stale, next_stale);
        changed |= update(&mut state.login_item_checking, self.login_checking);
        changed
    }

    fn tour_inputs(&mut self, practicing: bool) -> Vec<Input> {
        let mut inputs = Vec::new();
        if crate::desktop::onboarding::take_listener_failure() {
            inputs.push(Input::ListenerFailed {
                relaunch_allowed: crate::desktop::onboarding::relaunch_allowed(
                    crate::desktop::onboarding::unix_ms(),
                ),
            });
        }
        if !practicing {
            self.practice = None;
            return inputs;
        }
        if self.practice.is_none() {
            self.practice =
                crate::macos_rune::runtime().map(|runtime| PracticeFeed::new(runtime.event_path()));
        }
        if let Some(practice) = &mut self.practice {
            inputs.extend(practice.observations().into_iter().map(Input::Dictation));
        }
        inputs
    }
}

/// Shows HUD Lab's tuning and demonstrated state on the dictation HUD.
fn configure_hud(runtime: &RuntimeHandle, lab: HudLab) {
    use crate::dictation::indicator::DictationIndicatorEvent as Event;
    tune_hud(runtime, lab);
    runtime.indicator(Event::Reset);
    match lab.demo {
        HudDemo::Recording => {
            runtime.indicator(Event::Started);
            runtime.indicator(Event::DemoVoice);
        }
        HudDemo::Transcribing | HudDemo::Processing => {
            runtime.indicator(Event::Submitted { job_id: 0 });
            runtime.indicator(if lab.demo == HudDemo::Transcribing {
                Event::Transcribing { job_id: 0 }
            } else {
                Event::Processing { job_id: 0 }
            });
        }
    }
}

/// Applies HUD Lab's tuning to whatever the HUD shows.
fn tune_hud(runtime: &RuntimeHandle, lab: HudLab) {
    use crate::dictation::indicator::DictationIndicatorEvent as Event;
    runtime.indicator(Event::ConfigureDotScope(lab.tuning));
}

fn update<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        false
    } else {
        *slot = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::ModifierSide;

    #[test]
    fn paste_last_needs_a_key() {
        let modifiers_only = HotkeyBinding {
            modifiers: crate::app_settings::HotkeyModifiers::option_command(),
            key: None,
        };
        assert_eq!(
            validate_hotkey(ShortcutTarget::PasteLast, &modifiers_only),
            Err(tr("Add a key"))
        );
        assert_eq!(
            validate_hotkey(ShortcutTarget::Dictation, &modifiers_only),
            Ok(())
        );
        let chord = HotkeyBinding::paste_last_default();
        assert_eq!(validate_hotkey(ShortcutTarget::PasteLast, &chord), Ok(()));
    }

    const LEFT_OPTION: u64 = 0x0008_0020;
    const RIGHT_OPTION: u64 = 0x0008_0040;
    const LEFT_COMMAND: u64 = 0x0010_0008;
    const FUNCTION: u64 = 0x0080_0000;

    fn captured(chord: Option<Chord>) -> Result<HotkeyBinding, &'static str> {
        match chord {
            Some(Chord::Captured(result)) => result,
            Some(Chord::Held(_)) => panic!("still held"),
            Some(Chord::Cancelled) => panic!("cancelled"),
            None => panic!("nothing recorded"),
        }
    }

    fn held(chord: Option<Chord>) -> Vec<String> {
        match chord {
            Some(Chord::Held(modifiers)) => HotkeyBinding {
                modifiers,
                key: None,
            }
            .keycaps(),
            _ => panic!("not holding"),
        }
    }

    #[test]
    fn mouse_buttons_record_with_or_without_modifiers() {
        for button in 0..32 {
            for flags in [0, LEFT_OPTION, RIGHT_OPTION | LEFT_COMMAND] {
                let mut capture = ChordCapture::default();
                capture.flags_changed(flags);
                let binding = captured(capture.mouse_down(button, flags)).unwrap();
                assert_eq!(binding.key, HotkeyKey::mouse(button as u16));
                assert_eq!(
                    binding.modifiers,
                    modifiers_from_flags(flags).without_side_constraints()
                );
                assert!(capture.flags_changed(0).is_none());
                let text = crate::control::settings::format_hotkey(&binding);
                let parsed = crate::control::settings::parse_hotkey(&text, |name| {
                    hotkey_key_named(name).map_err(str::to_owned)
                });
                // A bare left click is recorded, then refused (issue 411).
                if binding.is_bare_primary_click() {
                    assert!(parsed.is_err());
                } else {
                    assert_eq!(parsed.unwrap(), binding);
                }
            }
        }
        let mut capture = ChordCapture::default();
        assert!(capture.mouse_down(-1, 0).is_none());
        assert!(capture.mouse_down(32, 0).is_none());
        assert!(matches!(
            capture.key_down(ESCAPE_KEY_CODE, false, None, 0),
            Some(Chord::Cancelled)
        ));
    }

    #[test]
    fn mouse_bindings_persist_and_conflict_only_with_the_same_trigger() {
        let binding = captured(ChordCapture::default().mouse_down(3, 0)).unwrap();
        let restored: HotkeyBinding =
            serde_json::from_str(&serde_json::to_string(&binding).unwrap()).unwrap();
        assert_eq!(restored, binding);
        assert_eq!(restored.keycaps(), ["Mouse 4"]);
        assert!(binding.overlaps(&restored));
        assert!(!binding.overlaps(&captured(ChordCapture::default().mouse_down(4, 0)).unwrap()));
        assert!(!binding.overlaps(&HotkeyBinding::default()));
        for name in ["mouse0", "mouse33", "mouse"] {
            assert!(hotkey_key_named(name).is_err());
        }
        assert_eq!(
            hotkey_key_named("mouse-1").unwrap(),
            HotkeyKey::mouse(0).unwrap()
        );
    }

    #[test]
    fn releasing_modifiers_records_the_most_held_with_their_side() {
        let mut capture = ChordCapture::default();
        assert_eq!(held(capture.flags_changed(LEFT_OPTION)), ["L⌥"]);
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION | LEFT_COMMAND)),
            ["⌥", "⌘"],
            "two modifiers record from either side"
        );
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION)),
            ["⌥", "⌘"],
            "letting one go still shows what releasing records"
        );
        let binding = captured(capture.flags_changed(0)).unwrap();
        assert_eq!(binding.key, None);
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Either));
        assert_eq!(binding.modifiers.command, Some(ModifierSide::Either));
        let binding = {
            assert_eq!(held(capture.flags_changed(RIGHT_OPTION)), ["R⌥"]);
            captured(capture.flags_changed(0)).unwrap()
        };
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Right));
        assert_eq!(held(capture.flags_changed(FUNCTION)), ["fn"]);
        assert!(
            captured(capture.flags_changed(0))
                .unwrap()
                .modifiers
                .function
        );
    }

    const RIGHT_COMMAND: u64 = 0x0010_0010;

    #[test]
    fn with_sided_chords_each_modifier_keeps_its_side() {
        let mut capture = ChordCapture::with_sides(true);
        assert_eq!(held(capture.flags_changed(RIGHT_COMMAND)), ["R⌘"]);
        assert_eq!(
            held(capture.flags_changed(RIGHT_COMMAND | RIGHT_OPTION)),
            ["R⌥", "R⌘"]
        );
        let binding = captured(capture.flags_changed(0)).unwrap();
        assert_eq!(binding.key, None);
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Right));
        assert_eq!(binding.modifiers.command, Some(ModifierSide::Right));
        assert_eq!(binding.modifiers.count(), 2);
        assert_eq!(binding.standalone_side(), None, "no side picker");
        assert_eq!(binding.keycaps(), ["R⌥", "R⌘"]);
        let runtime = binding.runtime();
        assert!(runtime.exact_modifiers(RIGHT_COMMAND | RIGHT_OPTION));
        assert!(!runtime.exact_modifiers(LEFT_COMMAND | LEFT_OPTION));
        assert!(!runtime.exact_modifiers(LEFT_COMMAND | RIGHT_OPTION));
        let restored: HotkeyBinding =
            serde_json::from_str(&serde_json::to_string(&binding).unwrap()).unwrap();
        assert_eq!(restored, binding);
        // It still conflicts with ⌥⌘ from either side, not with the left keys.
        let either = HotkeyBinding {
            modifiers: HotkeyModifiers::option_command(),
            key: None,
        };
        assert!(binding.overlaps(&either));
        assert!(!binding.overlaps(&HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Left),
                command: Some(ModifierSide::Left),
                ..Default::default()
            },
            key: None,
        }));
    }

    #[test]
    fn without_sided_chords_two_modifiers_record_from_either_side() {
        let mut capture = ChordCapture::with_sides(false);
        capture.flags_changed(RIGHT_COMMAND);
        assert_eq!(
            held(capture.flags_changed(RIGHT_COMMAND | RIGHT_OPTION)),
            ["⌥", "⌘"]
        );
        let binding = captured(capture.flags_changed(0)).unwrap();
        assert_eq!(binding.modifiers, HotkeyModifiers::option_command());
        assert!(
            binding
                .runtime()
                .exact_modifiers(LEFT_COMMAND | LEFT_OPTION)
        );
    }

    #[test]
    fn both_options_record_as_one_chord_of_two_keys() {
        let mut capture = ChordCapture::default();
        assert_eq!(held(capture.flags_changed(RIGHT_OPTION)), ["R⌥"]);
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION | RIGHT_OPTION)),
            ["L⌥", "R⌥"]
        );
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION)),
            ["L⌥", "R⌥"],
            "the peak stays"
        );
        let binding = captured(capture.flags_changed(0)).unwrap();
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Both));
        assert_eq!(binding.modifiers.count(), 2);
        assert_eq!(binding.standalone_side(), None, "no side picker");
        assert_eq!(
            crate::control::settings::format_hotkey(&binding),
            "left-option+right-option"
        );
        assert!(!binding.overlaps(&HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Right),
                ..Default::default()
            },
            key: None,
        }));
        assert!(
            binding.overlaps(&HotkeyBinding::default()),
            "⌥ takes both too"
        );
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION | RIGHT_OPTION | LEFT_COMMAND)),
            ["L⌥", "R⌥", "⌘"],
            "an extra key shows while held"
        );
    }

    #[test]
    fn a_key_records_its_modifiers_on_either_side_and_their_release_nothing() {
        let mut capture = ChordCapture::default();
        assert_eq!(held(capture.flags_changed(LEFT_OPTION)), ["L⌥"]);
        let binding = captured(capture.key_down(9, false, Some("v"), LEFT_OPTION)).unwrap();
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Either));
        assert_eq!(
            binding.key,
            Some(HotkeyKey {
                code: 9,
                label: "V".into()
            })
        );
        assert!(capture.flags_changed(0).is_none());
        assert!(capture.key_down(9, true, Some("v"), LEFT_OPTION).is_none());
    }

    #[test]
    fn keys_without_modifiers_need_one_except_function_keys() {
        let mut capture = ChordCapture::default();
        assert_eq!(
            captured(capture.key_down(0, false, Some("a"), 0)),
            Err(tr("Add a modifier"))
        );
        assert_eq!(
            held(capture.flags_changed(LEFT_OPTION)),
            ["L⌥"],
            "a rejected bare key spends no modifiers"
        );
        assert!(captured(capture.flags_changed(0)).is_ok());
        let f12 = captured(capture.key_down(111, false, None, FUNCTION)).unwrap();
        assert!(f12.modifiers.is_empty());
        assert_eq!(f12.key.unwrap().label, "F12");
        // Issue 1138: End, and the other keys that type nothing, record alone.
        for (code, label) in [
            (119, "End"),
            (115, "Home"),
            (116, "Page Up"),
            (121, "Page Down"),
            (114, "Help"),
        ] {
            let binding = captured(capture.key_down(code, false, None, FUNCTION)).unwrap();
            assert!(binding.modifiers.is_empty());
            assert_eq!(binding.key.as_ref().unwrap().label, label);
            assert!(binding.runtime().matches_key_press(code, FUNCTION));
        }
        assert_eq!(
            captured(capture.key_down(123, false, None, FUNCTION)),
            Err(tr("Add a modifier")),
            "arrows still need one"
        );
        assert!(matches!(
            capture.key_down(ESCAPE_KEY_CODE, false, None, 0),
            Some(Chord::Cancelled)
        ));
    }
}
