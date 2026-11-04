//! What the current OS's runtime can do, the OS-only state the panes show,
//! and the requests they make of it.
//!
//! The panes never call macOS or Windows APIs. Permissions, the login item,
//! and the Mumble Microphone driver reach them as plain data
//! in [`PlatformState`], and every action they take goes out as a
//! [`PlatformRequest`] through [`PlatformHost`]. Linux scenes and tests feed
//! preview data and record the requests, so macOS and Windows surfaces render
//! and are tested on Linux; a native host fills the same data on its own OS.

use crate::audio::mumble_microphone::{
    DemoPlaybackTrack, DemoSnapshot, DemoSourceKind, MumbleMicrophoneStatus, TunedFilterConfig,
};
use crate::desktop::login_item::{LoginItemResult, LoginItemStatus};
use crate::desktop::onboarding::{
    Input, PermissionAction, PermissionKind, PermissionState, SetupStatus,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Os {
    Linux,
    Macos,
    Windows,
}

/// What the runtime can do. A missing capability omits its pane or row; the
/// window never shows an imitation of a feature the runtime cannot perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Capabilities {
    pub(super) os: Os,
    /// Privacy permissions gate dictation (macOS).
    pub(super) permissions: bool,
    pub(super) launch_at_login: bool,
    pub(super) dock_icon: bool,
    /// The listener can be off: the status card and Enable on launch.
    pub(super) listener_control: bool,
    /// The Terminal paste row in General (Linux; on Windows it is in Labs).
    pub(super) terminal_paste: bool,
    /// The dictation shortcut can pass through to other apps (Windows).
    pub(super) share_shortcut: bool,
    /// Mumble Microphone is a driver to install (macOS). Elsewhere with the
    /// Voice Filter, Mumble creates it while the filter is on (a PipeWire
    /// virtual source on Linux).
    pub(super) mumble_microphone: bool,
    /// The window sees dictation events, so the tour can end with a
    /// practice dictation into its own field.
    pub(super) practice_dictation: bool,
    /// The Voice Filter pane: Mumble Microphone, its tuning, and the try-it
    /// demo (macOS and Linux, in Labs).
    pub(super) fun: bool,
    /// Modifier-only shortcuts can require the left or right key (macOS,
    /// Windows; X11 grabs and the Wayland portal see no sides).
    pub(super) modifier_sides: bool,
}

impl Capabilities {
    pub(super) const LINUX: Self = Self {
        os: Os::Linux,
        permissions: false,
        launch_at_login: true,
        dock_icon: false,
        listener_control: true,
        terminal_paste: true,
        share_shortcut: false,
        mumble_microphone: false,
        practice_dictation: true,
        fun: true,
        modifier_sides: false,
    };

    pub(super) const MACOS: Self = Self {
        os: Os::Macos,
        permissions: true,
        launch_at_login: true,
        dock_icon: true,
        listener_control: false,
        terminal_paste: false,
        share_shortcut: false,
        mumble_microphone: true,
        practice_dictation: true,
        fun: true,
        modifier_sides: true,
    };

    pub(super) const WINDOWS: Self = Self {
        os: Os::Windows,
        permissions: false,
        launch_at_login: true,
        dock_icon: false,
        listener_control: true,
        terminal_paste: false,
        share_shortcut: true,
        mumble_microphone: false,
        practice_dictation: true,
        fun: false,
        modifier_sides: true,
    };

    /// This OS's capabilities, with the rows its window host decides.
    pub(super) fn current() -> Self {
        let os = if cfg!(target_os = "macos") {
            Self::MACOS
        } else if cfg!(target_os = "windows") {
            Self::WINDOWS
        } else {
            Self::LINUX
        };
        Self {
            terminal_paste: super::host::TERMINAL_PASTE,
            ..os
        }
    }
}

/// The OS-only state the panes show; the host keeps it current.
#[derive(Clone, Debug)]
pub(super) struct PlatformState {
    pub(super) caps: Capabilities,
    /// Microphone and Accessibility (macOS). Every permission is ready where
    /// the OS has none.
    pub(super) permissions: SetupStatus,
    /// Permissions whose Allow was pressed since they were last granted.
    pub(super) permissions_asked: Vec<PermissionKind>,
    /// Permissions that stayed untrusted after returning from System Settings,
    /// so their row offers `"Still grey? Fix it"`.
    pub(super) permissions_stale: Vec<PermissionKind>,
    /// Untrusted permissions whose row is visible right now, so the native
    /// host clears retired Mumble bundle IDs' stale TCC rows once in the
    /// background.
    pub(super) permissions_shown: Vec<PermissionKind>,
    /// Screen & System Audio Recording, for Labs › Also pause the browser.
    pub(super) system_audio: bool,
    /// `None` while the login item worker checks, or when it is unavailable.
    pub(super) login_item: Option<LoginItemStatus>,
    pub(super) login_item_checking: bool,
    pub(super) login_item_error: Option<String>,
    /// The last Launch at login click while macOS registers or unregisters
    /// the login item, so the switch moves at once. Memory only: macOS's
    /// answer replaces it.
    pub(super) login_item_requested: Option<bool>,
    pub(super) mumble_microphone: Option<MumbleMicrophoneStatus>,
    /// The tour's relaunch failed; the person must reopen Mumble.
    pub(super) relaunch_failed: bool,
    /// The desktop's paste permission where Mumble pastes through the
    /// RemoteDesktop portal (GNOME and KDE on Wayland); `None` elsewhere.
    pub(super) paste: Option<super::paste_access::PasteAccess>,
    /// A Wayland desktop without data-control (GNOME): a paste can't put
    /// the clipboard back unless Labs › Restore clipboard on GNOME is on.
    pub(super) clipboard_unrestorable: bool,
    /// A scene's state: [`Self::setup_status`] reports `permissions` on
    /// every OS rather than asking this computer.
    preview: bool,
    /// macOS 27 lists Accessibility as "Device Control and Data Access"
    /// (`permission_guide::list_name`); the permission rows use that name.
    pub(super) accessibility_renamed: bool,
    /// The Mumble Microphone's live filter tuning
    /// (`mumble_microphone::current_tuned_config`).
    pub(super) tuning: TunedFilterConfig,
    /// The Voice Filter pane's try-it demo (`mumble_microphone::demo_snapshot`); `None`
    /// until the host has one. Refresh it every frame while it records or
    /// plays (`DemoSnapshot::is_recording`, `playing`).
    pub(super) demo: Option<DemoSnapshot>,
    /// What HUD Lab shows on the dictation indicator. The pane edits it and
    /// sends `PlatformRequest::ConfigureHud`; the host need not write it.
    pub(super) hud: HudLab,
    /// The dictation event log the Activity pane reads (`live.ndjson`);
    /// `None` where the host keeps none.
    pub(super) activity_log: Option<std::path::PathBuf>,
}

/// The indicator state HUD Lab demonstrates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum HudDemo {
    #[default]
    Recording,
    Transcribing,
    Processing,
}

/// Everything HUD Lab drives: Dot scope's tuning and the demonstrated
/// state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct HudLab {
    pub(super) tuning: crate::rune_hud::dot_scope::Tuning,
    pub(super) demo: HudDemo,
}

/// The Voice Filter pane's demo controls (`mumble_microphone`'s demo functions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DemoRequest {
    /// `start_demo_recording` (five seconds of the microphone).
    Record,
    StopRecording,
    SelectSource(DemoSourceKind),
    Play(DemoPlaybackTrack),
    StopPlayback,
}

impl PlatformState {
    pub(super) fn new(caps: Capabilities) -> Self {
        Self {
            caps,
            permissions: crate::desktop::onboarding::status(),
            permissions_asked: Vec::new(),
            permissions_stale: Vec::new(),
            permissions_shown: Vec::new(),
            system_audio: false,
            login_item: None,
            login_item_checking: false,
            login_item_error: None,
            login_item_requested: None,
            mumble_microphone: None,
            relaunch_failed: false,
            paste: None,
            clipboard_unrestorable: false,
            preview: false,
            accessibility_renamed: false,
            tuning: TunedFilterConfig::default(),
            demo: None,
            hud: HudLab::default(),
            activity_log: None,
        }
    }

    /// The permission state; every permission reads ready where the OS has
    /// none, or in a preview.
    pub(super) fn setup_status(&self) -> SetupStatus {
        if !self.caps.permissions && !self.preview {
            return crate::desktop::onboarding::status();
        }
        self.permissions
    }

    /// Adopts what the window host's login item reported: `None` where the
    /// host has no such row (it stays unavailable), an error beside the last
    /// known state.
    pub(super) fn set_login_item(&mut self, result: Option<Result<bool, String>>) {
        match result {
            None => {}
            Some(Ok(enabled)) => {
                self.login_item = Some(if enabled {
                    LoginItemStatus::Enabled
                } else {
                    LoginItemStatus::Disabled
                });
                self.login_item_error = None;
            }
            Some(Err(error)) => {
                self.login_item.get_or_insert(LoginItemStatus::Disabled);
                self.login_item_error = Some(error);
            }
        }
    }

    /// Whether the Launch at login switch shows on: the pending click, else
    /// what macOS last reported.
    pub(super) fn login_item_on(&self) -> bool {
        self.login_item_requested
            .unwrap_or(self.login_item == Some(LoginItemStatus::Enabled))
    }

    /// The switch was pressed: it shows `enabled` while the host asks macOS.
    pub(super) fn request_login_item(&mut self, enabled: bool) {
        self.login_item_requested = Some(enabled);
        self.login_item_error = None;
    }

    /// Adopts the login item worker's answer. `settled` is false while a
    /// later click still waits behind it: the switch keeps that click and
    /// only the reported status is remembered. Once settled, the switch
    /// shows macOS's state, springing back beside the error if the request
    /// failed or macOS kept the other state.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn login_item_answered(&mut self, answer: LoginItemResult, settled: bool) {
        let current = settled || self.login_item_requested.is_none();
        let error = match answer {
            Ok(status) => {
                self.login_item = Some(status);
                None
            }
            Err(failure) => {
                if failure.status.is_some() {
                    self.login_item = failure.status;
                }
                Some(failure.message)
            }
        };
        if current {
            self.login_item_requested = None;
            self.login_item_error = error;
        }
    }

    pub(super) fn asked(&self, kind: PermissionKind) -> bool {
        self.permissions_asked.contains(&kind)
    }

    pub(super) fn note_asked(&mut self, kind: PermissionKind) {
        if !self.asked(kind) {
            self.permissions_asked.push(kind);
        }
    }

    pub(super) fn stale(&self, kind: PermissionKind) -> bool {
        self.permissions_stale.contains(&kind)
    }

    pub(super) fn note_stale(&mut self, kind: PermissionKind) {
        self.note_asked(kind);
        if !self.stale(kind) {
            self.permissions_stale.push(kind);
        }
    }

    pub(super) fn clear_stale(&mut self, kind: PermissionKind) {
        self.permissions_stale.retain(|entry| *entry != kind);
    }

    pub(super) fn control_for(
        &self,
        kind: PermissionKind,
        permitted: bool,
    ) -> super::lists::PermissionControl {
        use super::lists::PermissionControl;
        if permitted {
            PermissionControl::Allowed
        } else if self.stale(kind) {
            PermissionControl::Stale
        } else if self.asked(kind) {
            PermissionControl::Waiting
        } else {
            PermissionControl::Allow
        }
    }

    /// Deterministic macOS state for scenes: every permission granted, the
    /// login item off, and the driver installed.
    pub(super) fn preview(caps: Capabilities) -> Self {
        let mut state = Self::new(caps);
        state.preview = true;
        let ready = PermissionState::Ready;
        state.permissions = SetupStatus {
            microphone: ready,
            accessibility: ready,
        };
        state.system_audio = true;
        state.login_item = caps.launch_at_login.then_some(LoginItemStatus::Disabled);
        if caps.fun {
            // Fixed, so the scenes' goldens don't change with the crate version.
            let version = "0.0.0".to_string();
            let driver = caps.mumble_microphone;
            state.mumble_microphone = Some(MumbleMicrophoneStatus {
                driver_bundle_installed: driver,
                driver_loaded: true,
                installed_version: driver.then(|| version.clone()),
                bundled_version: version,
                sink_streaming: true,
                mic_in_use: driver,
                active_apps: if driver {
                    vec!["Slack".into()]
                } else {
                    Vec::new()
                },
                action_state: crate::audio::mumble_microphone::DriverActionState::Idle,
            });
            state.demo = Some(super::fun::preview_demo(false, None));
        }
        state
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PlatformRequest {
    /// Request the microphone or open the System Settings guide.
    Permission(PermissionAction),
    /// Screen & System Audio Recording, for Labs › Also pause the browser.
    SystemAudio,
    SetLaunchAtLogin(bool),
    OpenLoginItemSettings,
    InstallMumbleMicrophone,
    UninstallMumbleMicrophone,
    /// Quit and reopen Mumble so macOS applies new permissions.
    Relaunch,
    /// Apply this tuning to the live Mumble Microphone filter
    /// (`mumble_microphone::update_tuned_config`). The pane has already saved
    /// the mode, comfort noise, and strength in `settings.json`.
    SetTuning(TunedFilterConfig),
    MumbleMicrophoneDemo(DemoRequest),
    /// Show this on the dictation indicator: `DictationIndicatorEvent::
    /// ConfigureDotScope` with the tuning, then `Reset` and the
    /// demonstrated state.
    ConfigureHud(HudLab),
    /// Only the tuning, live on whatever the indicator shows (a dictation
    /// or the demo): `ConfigureDotScope`, no `Reset`.
    TuneHud(HudLab),
    /// Leaving HUD Lab: `Reset` hides its demo.
    EndHudDemo,
    /// Save Dot scope's tuning as `dot-scope.json`, which the HUD loads at
    /// launch.
    SaveDotScope(crate::rune_hud::dot_scope::Tuning),
}

/// Performs a Voice Filter request on the Mumble Microphone in this process.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn voice_filter_request(request: PlatformRequest) {
    use crate::audio::mumble_microphone as mic;
    match request {
        PlatformRequest::InstallMumbleMicrophone => mic::install_driver_async(),
        PlatformRequest::UninstallMumbleMicrophone => mic::uninstall_driver_async(),
        PlatformRequest::SetTuning(tuning) => mic::update_tuned_config(|current| *current = tuning),
        PlatformRequest::MumbleMicrophoneDemo(request) => match request {
            DemoRequest::Record => mic::start_demo_recording(),
            DemoRequest::StopRecording => mic::stop_demo_recording(),
            DemoRequest::SelectSource(source) => mic::select_demo_source(source),
            DemoRequest::Play(track) => mic::play_demo_track(track),
            DemoRequest::StopPlayback => mic::stop_demo_playback(),
        },
        _ => {}
    }
}

/// The Voice Filter pane's state from the Mumble Microphone in this process:
/// the demo every frame while it records or plays, and, when `refresh`, the
/// tuning and Mumble Microphone's status. Nothing decodes or renders while the
/// Voice Filter is off.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn poll_voice_filter(state: &mut PlatformState, refresh: bool) -> bool {
    use crate::audio::mumble_microphone as mic;
    let mut changed = false;
    let on = crate::app_settings::mumble_microphone_enabled();
    if on
        && (state.demo.is_none()
            || mic::demo_is_busy()
            || mic::demo_recompute_is_running()
            || state
                .demo
                .as_ref()
                .is_some_and(|demo| demo.is_recording || demo.playing.is_some()))
    {
        state.demo = Some(mic::demo_snapshot());
        changed = true;
    }
    if refresh {
        let tuning = mic::current_tuned_config();
        if state.tuning != tuning {
            state.tuning = tuning;
            changed = true;
        }
        let status = on.then(mic::status);
        if state.mumble_microphone != status {
            state.mumble_microphone = status;
            changed = true;
        }
    }
    changed
}

/// The host that performs [`PlatformRequest`]s and refreshes
/// [`PlatformState`] on its own OS.
// Only macOS has a native host.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) trait NativeHost {
    fn send(&mut self, request: PlatformRequest) -> Result<(), String>;
    /// Copies fresh state in; true when anything shown changed.
    fn poll(&mut self, state: &mut PlatformState) -> bool;
    /// What the tour hears from the runtime: practice dictation outcomes
    /// while `practicing`, and a shortcut listener that failed to start.
    fn tour_inputs(&mut self, practicing: bool) -> Vec<Input>;
}

pub(super) enum PlatformHost {
    /// Linux and Windows: their runtimes own everything but the practice
    /// dictation, which the window reads from the listener's event log. On
    /// Linux the dictation host runs in this process, so the Voice Filter is
    /// asked of it directly.
    Local(super::practice::LocalPractice),
    /// Scenes and tests: requests are recorded, never performed.
    Fixture(Vec<PlatformRequest>),
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Native(Box<dyn NativeHost>),
}

impl PlatformHost {
    pub(super) fn send(&mut self, request: PlatformRequest) -> Result<(), String> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Local(_) => {
                voice_filter_request(request);
                Ok(())
            }
            // Windows: the one permission is the microphone switch in Settings.
            #[cfg(target_os = "windows")]
            Self::Local(_) => {
                if request
                    == PlatformRequest::Permission(PermissionAction::ShowGuide(
                        PermissionKind::Microphone,
                    ))
                {
                    crate::desktop::opencode_catalog::open_url(
                        crate::desktop::onboarding::WINDOWS_MICROPHONE_SETTINGS,
                    )
                    .map_err(|error| format!("{error:#}"))?;
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Self::Local(_) => Ok(()),
            Self::Fixture(sent) => {
                sent.push(request);
                Ok(())
            }
            Self::Native(host) => host.send(request),
        }
    }

    pub(super) fn poll(&mut self, state: &mut PlatformState) -> bool {
        match self {
            Self::Native(host) => host.poll(state),
            // Mumble Microphone's status is a few atomics on Linux, so it
            // refreshes every frame.
            #[cfg(target_os = "linux")]
            Self::Local(_) => poll_voice_filter(state, true),
            _ => false,
        }
    }

    pub(super) fn tour_inputs(&mut self, practicing: bool) -> Vec<Input> {
        match self {
            Self::Native(host) => host.tour_inputs(practicing),
            Self::Local(practice) => practice.tour_inputs(practicing),
            Self::Fixture(_) => Vec::new(),
        }
    }

    pub(super) fn is_fixture(&self) -> bool {
        matches!(self, Self::Fixture(_))
    }
}

/// Why a capability is on for some OSes and off for others. Every
/// difference between [`Capabilities::LINUX`], [`Capabilities::MACOS`], and
/// [`Capabilities::WINDOWS`] names one in [`DIFFERENCES`], so a Mac-only
/// feature is a recorded, reviewed choice rather than drift.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
enum Difference {
    /// Belongs to one OS's own pattern (the Dock, TCC).
    OsPattern(&'static str),
    /// Can't be done there as policy stands.
    Impossible(&'static str),
    /// Can be done there; nobody has built the adapter yet.
    Pending(&'static str),
}

#[cfg(test)]
impl Difference {
    fn why(self) -> &'static str {
        match self {
            Self::OsPattern(why) | Self::Impossible(why) | Self::Pending(why) => why,
        }
    }
}

#[cfg(test)]
const DIFFERENCES: &[(&str, Difference)] = &[
    (
        "permissions",
        Difference::OsPattern("only macOS gates dictation on privacy permissions (TCC)"),
    ),
    (
        "dock_icon",
        Difference::OsPattern("the Dock; Windows and Linux have no dock policy"),
    ),
    (
        "listener_control",
        Difference::OsPattern("macOS listens once setup is ready; its menu bar is the control"),
    ),
    (
        "terminal_paste",
        Difference::OsPattern("macOS terminals take ⌘V; Windows keeps the row in Labs"),
    ),
    (
        "share_shortcut",
        Difference::Pending("a macOS event tap could pass the shortcut through too"),
    ),
    (
        "mumble_microphone",
        Difference::OsPattern(
            "only macOS installs a driver; Linux creates a PipeWire source while the filter is on",
        ),
    ),
    (
        "fun",
        Difference::Impossible("a Windows virtual microphone needs a signed kernel driver"),
    ),
    (
        "modifier_sides",
        Difference::Impossible("X11 grabs and the Wayland portal see no sides"),
    ),
];

impl Capabilities {
    /// Every capability by name. Destructuring makes a new capability a
    /// compile error here until it is named.
    pub(super) fn fields(self) -> [(&'static str, bool); 10] {
        let Self {
            os: _,
            permissions,
            launch_at_login,
            dock_icon,
            listener_control,
            terminal_paste,
            share_shortcut,
            mumble_microphone,
            practice_dictation,
            fun,
            modifier_sides,
        } = self;
        [
            ("permissions", permissions),
            ("launch_at_login", launch_at_login),
            ("dock_icon", dock_icon),
            ("listener_control", listener_control),
            ("terminal_paste", terminal_paste),
            ("share_shortcut", share_shortcut),
            ("mumble_microphone", mumble_microphone),
            ("practice_dictation", practice_dictation),
            ("fun", fun),
            ("modifier_sides", modifier_sides),
        ]
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn capabilities_differ_across_oses_only_for_a_recorded_reason() {
        let [linux, macos, windows] = [
            Capabilities::LINUX,
            Capabilities::MACOS,
            Capabilities::WINDOWS,
        ]
        .map(Capabilities::fields);
        let reason = |name: &str| {
            DIFFERENCES
                .iter()
                .find(|(field, _)| *field == name)
                .map(|(_, why)| why.why())
        };
        for index in 0..linux.len() {
            let (name, on_linux) = linux[index];
            let (on_macos, on_windows) = (macos[index].1, windows[index].1);
            let differs = !(on_linux == on_macos && on_macos == on_windows);
            match (differs, reason(name)) {
                (true, None) => panic!(
                    "`{name}` is linux {on_linux}, macOS {on_macos}, windows {on_windows}; \
                     port it or record why in DIFFERENCES"
                ),
                (false, Some(why)) => panic!(
                    "`{name}` is the same on every OS now; drop its DIFFERENCES entry ({why})"
                ),
                _ => {}
            }
        }
        for (name, _) in DIFFERENCES {
            assert!(
                linux.iter().any(|(field, _)| field == name),
                "DIFFERENCES names `{name}`, which is no capability"
            );
        }
    }
}
