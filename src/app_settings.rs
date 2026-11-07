use std::fs;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use color_eyre::eyre::{Result, eyre};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use serde::{Deserialize, Serialize};

use crate::dictation::vocabulary::{VocabularyEntry, serialize_saved};
use crate::transcription::TranscriptionSelection;

mod dictation_processing;
mod hotkeys;
mod labs;
mod persistence;
mod shortcut_repair;

pub use dictation_processing::{
    DictationMode, DictationPostProcessing, DictationProcessingSettings,
};
pub use hotkeys::*;
pub use labs::*;

static RECORDING_DUCK_VOLUME: AtomicU32 = AtomicU32::new(0.2_f32.to_bits());
// Until settings load, "While dictating" is its default, not Mute: the
// hotkey pre-roll depends on it.
static RECORDING_AUDIO_BEHAVIOR: AtomicU8 =
    AtomicU8::new(RecordingAudioBehavior::DoNothing.encoded());
static DOUBLE_TAP_LOCK: AtomicBool = AtomicBool::new(true);
static DOUBLE_TAP_ONLY: AtomicBool = AtomicBool::new(false);
static SHORTCUT_TOGGLE: AtomicBool = AtomicBool::new(false);
static PAUSE_BROWSER_MEDIA: AtomicBool = AtomicBool::new(false);
static MUMBLE_MICROPHONE_ENABLED: AtomicBool = AtomicBool::new(false);
static MICROPHONE_POLICY: AtomicU8 = AtomicU8::new(0);
static CUSTOM_TRANSFORMATIONS_ENABLED: AtomicBool = AtomicBool::new(false);
static INDICATOR_POSITION: AtomicU8 = AtomicU8::new(0);
static HOTKEYS: OnceLock<RwLock<AppliedHotkeys>> = OnceLock::new();
static HOTKEY_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
/// Distinct names for the tests' settings folders.
static SETTINGS_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPTION_SELECTION: OnceLock<RwLock<RuntimeTranscriptionSelection>> = OnceLock::new();
static MICROPHONE_SELECTION: OnceLock<RwLock<RuntimeMicrophoneSelection>> = OnceLock::new();

#[derive(Default)]
struct RuntimeTranscriptionSelection {
    revision: u64,
    selection: TranscriptionSelection,
}

#[derive(Default)]
struct RuntimeMicrophoneSelection {
    revision: u64,
    device: Option<String>,
}

/// Experimental opt-in features hidden by default.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ExperimentalSettings {
    pub mumble_microphone: bool,
    pub mumble_microphone_mode: crate::audio::voice_filter::VoiceFilterMode,
    pub mumble_microphone_comfort_noise: bool,
    pub mumble_microphone_strength: f32,
}

impl Default for ExperimentalSettings {
    fn default() -> Self {
        Self {
            mumble_microphone: false,
            mumble_microphone_mode: crate::audio::voice_filter::VoiceFilterMode::Muffled,
            mumble_microphone_comfort_noise: true,
            mumble_microphone_strength: 1.0,
        }
    }
}

/// Whether the Voice Filter runs: the listener streams the Mumble Microphone
/// and Mumble watches its driver. Its Labs toggle (`experimental.mumble_microphone`,
/// or `MUMBLE_EXPERIMENTAL_MIC`) or an installed driver turns it on, so a driver
/// apps already use keeps receiving audio.
pub fn voice_filter_enabled(setting: bool, driver_installed: bool) -> bool {
    setting || env_mumble_experimental_mic() || driver_installed
}

pub fn env_mumble_experimental_mic() -> bool {
    std::env::var("MUMBLE_EXPERIMENTAL_MIC")
        .ok()
        .is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes"))
}

/// Where Linux and Windows run the speech model (#856): Auto picks a GPU
/// unless it failed before under its current driver, then the CPU.
/// Where the recording HUD appears while dictating.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum IndicatorPosition {
    #[default]
    Top = 0,
    Bottom = 1,
    Hidden = 2,
}

/// Which release-service channel a direct install updates from
/// (`crate::updater`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Dev,
}

impl UpdateChannel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::Dev];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Dev => "dev",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Dev => "Dev",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// `None` follows the build: the channel it was published on (stamped at
    /// build time), else `dev` for a dev version, else its install's channel.
    /// Another channel than the build's is a switch the user confirms.
    pub channel: Option<UpdateChannel>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingAudioBehavior {
    Mute,
    Duck,
    PauseMedia,
    #[default]
    DoNothing,
}

pub use crate::dictation::text_replacements::TextReplacement;

impl RecordingAudioBehavior {
    pub const ALL: [Self; 4] = [Self::Mute, Self::PauseMedia, Self::Duck, Self::DoNothing];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mute => "Mute",
            Self::Duck => "Lower audio",
            Self::PauseMedia => "Pause media",
            Self::DoNothing => "Do nothing",
        }
    }

    const fn encoded(self) -> u8 {
        match self {
            Self::Mute => 0,
            Self::Duck => 3,
            Self::PauseMedia => 1,
            Self::DoNothing => 2,
        }
    }

    fn decode(value: u8) -> Self {
        match value {
            3 => Self::Duck,
            1 => Self::PauseMedia,
            2 => Self::DoNothing,
            _ => Self::Mute,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppSettings {
    /// Retired spoken-commands opt-in, kept untouched in `settings.json`.
    #[serde(
        default,
        rename = "commands_enabled",
        deserialize_with = "deserialize_retired_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub retired_commands_enabled: Option<serde_json::Value>,
    pub release_microphone_while_idle: bool,
    pub sound_effects: bool,
    pub sound_effect_volume: f32,
    /// Which feedback sounds play (`crate::feedback`).
    pub sound_theme: crate::feedback::SoundSet,
    pub microphone: Option<String>,
    /// System default records from the built-in microphone, else another
    /// real input, rather than a Bluetooth default.
    pub avoid_bluetooth_microphone: bool,
    pub recording_audio_behavior: RecordingAudioBehavior,
    /// Fraction of the original output volume while ducking.
    pub recording_duck_volume: f32,
    /// With Pause media on macOS, also pause a browser that is audibly playing.
    pub pause_browser_media: bool,
    pub double_tap_lock: bool,
    pub double_tap_only: bool,
    pub shortcut_mode: crate::dictation::ShortcutMode,
    pub dictation_hotkey: HotkeyBinding,
    /// Retired Voice Action shortcut, kept untouched in `settings.json`.
    #[serde(
        default,
        rename = "edit_hotkey",
        deserialize_with = "deserialize_retired_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub retired_edit_hotkey: Option<serde_json::Value>,
    pub paste_last_hotkey: Option<HotkeyBinding>,
    /// Toggle mode's cancel shortcut: discards the recording. Mumble takes it
    /// only while a toggled recording runs; Hold mode has no cancel.
    pub cancel_hotkey: HotkeyBinding,
    /// Dictate without processing: records like the dictation shortcut and
    /// pastes the transcript as heard. Unbound by default.
    pub raw_dictation_hotkey: Option<HotkeyBinding>,
    /// Labs › Second dictation shortcut: another shortcut that dictates
    /// exactly as `dictation_hotkey` does (macOS). Unbound by default and
    /// ignored while its Labs toggle is off.
    pub second_dictation_hotkey: Option<HotkeyBinding>,
    pub show_dock_icon: bool,
    /// The transcription service (Claude or Codex) and language.
    pub transcription: TranscriptionSelection,
    pub dictation_processing: DictationProcessingSettings,
    /// Retired Voice Action settings, kept untouched in `settings.json`.
    #[serde(
        default,
        rename = "voice_action",
        deserialize_with = "deserialize_retired_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub retired_voice_action: Option<serde_json::Value>,
    pub history_retention: crate::history::HistoryRetention,
    #[serde(default)]
    pub experimental: ExperimentalSettings,
    /// The Labs features' own switches (`crate::labs`).
    pub labs: LabsSettings,
    /// Start listening when Mumble opens (Windows and Linux, where the
    /// listener can be off).
    pub enable_on_launch: bool,
    /// Paste with Ctrl-Shift-V for terminals (Linux and Windows).
    pub paste_with_shift: bool,
    /// Mumble keeps its own GNOME custom keyboard shortcuts, on GNOME without a
    /// shortcuts portal (Linux, #781).
    pub gnome_keyboard_shortcuts: bool,
    /// Let other apps see the dictation shortcut's key presses too
    /// (Windows), so a push-to-talk key can drive both.
    pub share_dictation_shortcut: bool,
    /// A mouse-button shortcut went back to its default because Labs ›
    /// Mouse shortcuts is off; the window says so once, then clears it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mouse_shortcut_notice: bool,
    /// The interface language code; `None` follows the system language.
    pub ui_language: Option<String>,
    pub indicator_position: IndicatorPosition,
    pub updates: UpdateSettings,
    /// The shared vocabulary (`crate::dictation::vocabulary`). Left out of the file
    /// while empty, so settings without one stay as they were.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_saved"
    )]
    pub vocabulary: Vec<VocabularyEntry>,
    /// One-time settings changes already applied. A file that predates a
    /// change lacks its ID, so the change runs once on the next load; new
    /// settings start with every ID because they need none.
    #[serde(default)]
    pub migrations: Vec<String>,
    #[serde(skip_serializing)]
    pub text_replacements: Vec<TextReplacement>,
}

/// Settings saved by an `anomaly.mumble` build. Up to 2.3.1 Hex was
/// `com.kitlangton.hex2` and used the same folder, so a file without this
/// ID may carry onboarding finished under grants macOS no longer gives the
/// new identity; the macOS launch checks setup once
/// (`onboarding::recheck_after_identity_change`).
const ANOMALY_IDENTITY_MIGRATION: &str = "anomaly-mumble-identity";
const MIGRATIONS: &[&str] = &[ANOMALY_IDENTITY_MIGRATION];

fn deserialize_retired_value<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            retired_commands_enabled: None,
            release_microphone_while_idle: false,
            sound_effects: true,
            sound_effect_volume: 0.5,
            sound_theme: crate::feedback::SoundSet::default(),
            microphone: None,
            avoid_bluetooth_microphone: true,
            recording_audio_behavior: RecordingAudioBehavior::DoNothing,
            recording_duck_volume: 0.2,
            pause_browser_media: false,
            double_tap_lock: true,
            double_tap_only: false,
            shortcut_mode: crate::dictation::ShortcutMode::Hold,
            dictation_hotkey: HotkeyBinding::default(),
            retired_edit_hotkey: None,
            paste_last_hotkey: Some(HotkeyBinding::paste_last_default()),
            cancel_hotkey: HotkeyBinding::cancel_default(),
            raw_dictation_hotkey: None,
            second_dictation_hotkey: None,
            show_dock_icon: true,
            transcription: TranscriptionSelection::default(),
            dictation_processing: DictationProcessingSettings::default(),
            retired_voice_action: None,
            history_retention: crate::history::HistoryRetention::default(),
            experimental: ExperimentalSettings::default(),
            labs: LabsSettings::default(),
            enable_on_launch: true,
            paste_with_shift: false,
            gnome_keyboard_shortcuts: false,
            share_dictation_shortcut: false,
            mouse_shortcut_notice: false,
            ui_language: None,
            indicator_position: IndicatorPosition::default(),
            updates: UpdateSettings::default(),
            vocabulary: Vec::new(),
            migrations: MIGRATIONS.iter().map(|id| id.to_string()).collect(),
            text_replacements: Vec::new(),
        }
    }
}

impl AppSettings {
    fn normalize_double_tap_settings(&mut self) {
        if !self.double_tap_lock || self.dictation_hotkey.key.is_none() {
            self.double_tap_only = false;
        }
    }

    pub fn load() -> Result<Self> {
        let path = path()?;
        let settings = Self::load_from(&path)?;
        settings.apply_runtime();
        Ok(settings)
    }

    /// The saved `labs` switches, read without loading, normalizing, or
    /// writing anything else.
    pub fn saved_labs() -> LabsSettings {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            labs: LabsSettings,
        }
        path()
            .ok()
            .and_then(|path| fs::read(path).ok())
            .and_then(|data| serde_json::from_slice::<File>(&data).ok())
            .map(|file| file.labs)
            .unwrap_or_default()
    }

    /// The saved `updates.channel`, read without loading, normalizing, or
    /// writing anything else, so the updater thread never races a save.
    pub fn saved_update_channel() -> Option<UpdateChannel> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            updates: UpdateSettings,
        }
        let data = fs::read(path().ok()?).ok()?;
        serde_json::from_slice::<File>(&data).ok()?.updates.channel
    }

    /// The saved settings as `load` would see them, for a reader that must
    /// never change the file: no migration is written back.
    pub fn read() -> Result<Self> {
        Ok(Self::read_from(&path()?)?.0)
    }

    pub fn save(&self) -> Result<()> {
        let path = path()?;
        self.write_to(&path)?;
        self.apply_runtime();
        Ok(())
    }

    fn validate_recording_duck_volume(&self) -> Result<()> {
        if !self.recording_duck_volume.is_finite()
            || !(0.0..=1.0).contains(&self.recording_duck_volume)
        {
            return Err(eyre!("recording_duck_volume must be between 0 and 1"));
        }
        Ok(())
    }

    fn apply_runtime(&self) {
        // Whatever an editor holds, a managed lock keeps its switches off.
        let policy = crate::policy::Policy::current();
        if policy.any() {
            let mut managed = self.clone();
            policy.enforce(&mut managed);
            if managed != *self {
                return managed.apply_runtime();
            }
        }
        MICROPHONE_POLICY.store(
            u8::from(self.release_microphone_while_idle),
            Ordering::Release,
        );
        crate::audio::set_avoid_bluetooth_microphone(self.avoid_bluetooth_microphone);
        CUSTOM_TRANSFORMATIONS_ENABLED.store(
            transformations_used(&self.dictation_processing)
                && !policy.locks(crate::policy::Lock::Transforms),
            Ordering::Release,
        );
        crate::feedback::set_enabled(self.sound_effects);
        crate::feedback::set_volume(self.sound_effect_volume.clamp(0.0, 1.0));
        crate::feedback::set_sound_set(self.sound_theme);
        crate::desktop::i18n::apply(self.ui_language.as_deref());
        INDICATOR_POSITION.store(self.indicator_position as u8, Ordering::Relaxed);
        RECORDING_DUCK_VOLUME.store(self.recording_duck_volume.to_bits(), Ordering::Relaxed);
        RECORDING_AUDIO_BEHAVIOR.store(self.recording_audio_behavior.encoded(), Ordering::Relaxed);
        PAUSE_BROWSER_MEDIA.store(self.pause_browser_media, Ordering::Relaxed);
        DOUBLE_TAP_LOCK.store(self.double_tap_lock, Ordering::Relaxed);
        DOUBLE_TAP_ONLY.store(
            self.double_tap_lock && self.double_tap_only && self.dictation_hotkey.key.is_some(),
            Ordering::Relaxed,
        );
        SHORTCUT_TOGGLE.store(
            self.shortcut_mode == crate::dictation::ShortcutMode::Toggle,
            Ordering::Relaxed,
        );
        crate::labs::apply(&self.labs);
        MUMBLE_MICROPHONE_ENABLED.store(self.experimental.mumble_microphone, Ordering::Release);
        crate::audio::mumble_microphone::sync_from_settings(
            self.experimental.mumble_microphone_mode,
            self.experimental.mumble_microphone_comfort_noise,
            self.experimental.mumble_microphone_strength,
        );
        *HOTKEYS
            .get_or_init(Default::default)
            .write()
            .unwrap_or_else(|error| error.into_inner()) = AppliedHotkeys {
            runtime: self.runtime_hotkeys(),
            settings: Some(Arc::new(self.clone())),
        };
        set_transcription_selection(&self.transcription);
        set_microphone_selection(self.microphone.as_deref());
        #[cfg(target_os = "macos")]
        crate::dictation::config::update_dictation_profiles(&self.dictation_processing);
        crate::dictation::vocabulary::apply(&self.vocabulary);
    }

    /// Mode activations saved while Finder showed filename extensions carry a
    /// `.app` suffix that never matched the foreground application name.
    #[cfg(not(target_os = "macos"))]
    fn normalize_mode_application_names(&mut self) {}

    #[cfg(target_os = "macos")]
    fn normalize_mode_application_names(&mut self) {
        let modes = std::iter::once(&mut self.dictation_processing.default_mode)
            .chain(self.dictation_processing.modes.iter_mut());
        for mode in modes {
            let mut mode_migrated = false;
            for application in &mut mode.applications {
                let normalized =
                    crate::dictation::context::strip_bundle_extension(application).to_owned();
                if normalized != *application {
                    *application = normalized;
                    mode_migrated = true;
                }
            }
            if mode_migrated {
                mode.applications.sort_by_key(|name| name.to_lowercase());
                mode.applications.dedup();
            }
        }
    }

    fn migrate_legacy_replacements(&mut self) {
        if self.text_replacements.is_empty() {
            return;
        }
        if self
            .dictation_processing
            .default_mode
            .replacements
            .is_empty()
        {
            self.dictation_processing.default_mode.replacements = self.text_replacements.clone();
        }
        for mode in &mut self.dictation_processing.modes {
            if mode.replacements.is_empty() {
                mode.replacements = self.text_replacements.clone();
            }
        }
        self.text_replacements.clear();
    }

    #[cfg(target_os = "macos")]
    fn has_migration(&self, id: &str) -> bool {
        self.migrations.iter().any(|applied| applied == id)
    }

    /// Whether these settings come from before the `anomaly.mumble` identity.
    #[cfg(target_os = "macos")]
    pub(crate) fn predates_anomaly_identity(&self) -> bool {
        !self.has_migration(ANOMALY_IDENTITY_MIGRATION)
    }

    /// Records the identity check in the file at `path`, changing nothing else.
    #[cfg(target_os = "macos")]
    pub(crate) fn record_anomaly_identity(&mut self, path: &std::path::Path) -> Result<()> {
        if self.predates_anomaly_identity() {
            self.migrations.push(ANOMALY_IDENTITY_MIGRATION.into());
            self.write_to(path)?;
        }
        Ok(())
    }

    pub fn runtime_hotkeys(&self) -> RuntimeHotkeys {
        RuntimeHotkeys {
            dictation: self.dictation_hotkey.runtime(),
            paste_last: self.paste_last_hotkey.as_ref().map(HotkeyBinding::runtime),
            raw_dictation: self
                .raw_dictation_hotkey
                .as_ref()
                .map(HotkeyBinding::runtime),
            second_dictation: self
                .active_second_dictation_hotkey()
                .map(HotkeyBinding::runtime),
            cancel: self.active_cancel_hotkey().map(HotkeyBinding::runtime),
        }
    }

    /// The cancel shortcut while it takes effect: in Toggle mode only.
    pub fn active_cancel_hotkey(&self) -> Option<&HotkeyBinding> {
        (self.shortcut_mode == crate::dictation::ShortcutMode::Toggle)
            .then_some(&self.cancel_hotkey)
    }

    /// Labs › Second dictation shortcut's binding while it takes effect:
    /// macOS, its toggle on, bound, and sharing no key with another active
    /// shortcut. Otherwise Mumble matches, suppresses, and checks nothing for
    /// it.
    pub fn active_second_dictation_hotkey(&self) -> Option<&HotkeyBinding> {
        if !cfg!(target_os = "macos") || !self.labs.second_dictation_shortcut {
            return None;
        }
        self.second_dictation_hotkey
            .as_ref()
            .filter(|binding| !self.hotkey_in_use(HotkeySlot::SecondDictation, binding))
    }

    /// A shortcut saved as a bare left click swallowed every click, so its
    /// owner couldn't change it in Settings. Such a shortcut goes back to its
    /// default when settings load.
    fn repair_bare_primary_clicks(&mut self) {
        if self.dictation_hotkey.is_bare_primary_click() {
            tracing::warn!("replaced a left-click dictation shortcut with the default");
            self.dictation_hotkey = HotkeyBinding::default();
        }
        if self
            .paste_last_hotkey
            .as_ref()
            .is_some_and(HotkeyBinding::is_bare_primary_click)
        {
            tracing::warn!("replaced a left-click paste-last shortcut with the default");
            self.paste_last_hotkey = Some(HotkeyBinding::paste_last_default());
        }
        if self.cancel_hotkey.is_bare_primary_click() {
            tracing::warn!("replaced a left-click cancel shortcut with the default");
            self.cancel_hotkey = HotkeyBinding::cancel_default();
        }
        if self
            .raw_dictation_hotkey
            .as_ref()
            .is_some_and(HotkeyBinding::is_bare_primary_click)
        {
            tracing::warn!("cleared a left-click raw dictation shortcut");
            self.raw_dictation_hotkey = None;
        }
        if self
            .second_dictation_hotkey
            .as_ref()
            .is_some_and(HotkeyBinding::is_bare_primary_click)
        {
            tracing::warn!("cleared a left-click second dictation shortcut");
            self.second_dictation_hotkey = None;
        }
    }
}

pub fn hotkey_conflicts(
    candidate: &HotkeyBinding,
    others: impl IntoIterator<Item = HotkeyBinding>,
) -> bool {
    others
        .into_iter()
        .any(|binding| candidate.overlaps(&binding))
}

/// A shortcut Settings and `mumble settings` record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeySlot {
    Dictation,
    PasteLast,
    RawDictation,
    /// Labs › Second dictation shortcut (macOS).
    SecondDictation,
    /// Toggle mode's cancel shortcut.
    Cancel,
}

impl AppSettings {
    /// Whether `binding` overlaps a shortcut `slot` must not share: the other
    /// active shortcuts.
    pub fn hotkey_in_use(&self, slot: HotkeySlot, binding: &HotkeyBinding) -> bool {
        let mut others = Vec::new();
        if slot != HotkeySlot::Dictation {
            others.push(self.dictation_hotkey.clone());
        }
        if slot != HotkeySlot::PasteLast
            && let Some(paste) = &self.paste_last_hotkey
        {
            others.push(paste.clone());
        }
        if slot != HotkeySlot::RawDictation
            && let Some(raw) = &self.raw_dictation_hotkey
        {
            others.push(raw.clone());
        }
        if slot != HotkeySlot::Cancel {
            others.push(self.cancel_hotkey.clone());
        }
        if slot != HotkeySlot::SecondDictation
            && cfg!(target_os = "macos")
            && self.labs.second_dictation_shortcut
            && let Some(second) = &self.second_dictation_hotkey
        {
            others.push(second.clone());
        }
        hotkey_conflicts(binding, others)
    }
}

fn set_microphone_selection(device: Option<&str>) {
    let device = device
        .map(str::trim)
        .filter(|device| !device.is_empty())
        .map(str::to_string);
    let state = MICROPHONE_SELECTION.get_or_init(Default::default);
    let mut state = state.write().unwrap_or_else(|error| error.into_inner());
    if state.device != device {
        state.revision = state.revision.wrapping_add(1);
        state.device = device;
    }
}

pub fn microphone_selection() -> (u64, Option<String>) {
    let state = MICROPHONE_SELECTION.get_or_init(Default::default);
    let state = state.read().unwrap_or_else(|error| error.into_inner());
    (state.revision, state.device.clone())
}

fn set_transcription_selection(selection: &TranscriptionSelection) {
    let state = TRANSCRIPTION_SELECTION.get_or_init(Default::default);
    let mut state = state.write().unwrap_or_else(|error| error.into_inner());
    if state.selection != *selection {
        state.revision = state.revision.wrapping_add(1);
        state.selection = selection.clone();
    }
}

pub fn transcription_selection() -> (u64, TranscriptionSelection) {
    let state = TRANSCRIPTION_SELECTION.get_or_init(Default::default);
    let state = state.read().unwrap_or_else(|error| error.into_inner());
    (state.revision, state.selection.clone())
}

pub fn recording_duck_volume() -> f32 {
    f32::from_bits(RECORDING_DUCK_VOLUME.load(Ordering::Relaxed))
}

pub fn recording_audio_behavior() -> RecordingAudioBehavior {
    RecordingAudioBehavior::decode(RECORDING_AUDIO_BEHAVIOR.load(Ordering::Relaxed))
}

pub fn pause_browser_media() -> bool {
    PAUSE_BROWSER_MEDIA.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MicrophonePolicy {
    pub release_while_idle: bool,
}

pub fn microphone_policy() -> MicrophonePolicy {
    let policy = MICROPHONE_POLICY.load(Ordering::Acquire);
    MicrophonePolicy {
        release_while_idle: policy != 0,
    }
}

/// Whether the global mode or any mode runs a transformation.
pub fn transformations_used(processing: &DictationProcessingSettings) -> bool {
    !processing.default_mode.transformations.is_empty()
        || processing
            .modes
            .iter()
            .any(|mode| !mode.transformations.is_empty())
}

/// Where the recording HUD appears, as last saved.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn indicator_position() -> IndicatorPosition {
    match INDICATOR_POSITION.load(Ordering::Relaxed) {
        1 => IndicatorPosition::Bottom,
        2 => IndicatorPosition::Hidden,
        _ => IndicatorPosition::Top,
    }
}

pub fn custom_transformations_enabled() -> bool {
    CUSTOM_TRANSFORMATIONS_ENABLED.load(Ordering::Acquire)
}

pub fn double_tap_lock() -> bool {
    DOUBLE_TAP_LOCK.load(Ordering::Relaxed)
}

pub fn double_tap_only() -> bool {
    DOUBLE_TAP_ONLY.load(Ordering::Relaxed)
}

pub fn shortcut_toggle() -> bool {
    SHORTCUT_TOGGLE.load(Ordering::Relaxed)
}

/// [`voice_filter_enabled`] for the running listener.
pub fn mumble_microphone_enabled() -> bool {
    crate::labs::Feature::VoiceFilter.shipped()
        && crate::labs::Feature::VoiceFilter.managed().is_none()
        && voice_filter_enabled(
            MUMBLE_MICROPHONE_ENABLED.load(Ordering::Acquire),
            crate::audio::mumble_microphone::driver_bundle_installed(),
        )
}

/// The shortcuts the input layers match, and the settings they came from,
/// so a keyboard layout change can resolve their letter keys again.
#[derive(Default)]
struct AppliedHotkeys {
    runtime: RuntimeHotkeys,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    settings: Option<Arc<AppSettings>>,
}

impl AppliedHotkeys {
    /// Resolves the shortcuts again from the settings they came from, on the
    /// layout published now, unless newer settings were applied meanwhile.
    #[cfg(target_os = "macos")]
    fn resolve_again(
        applied: &RwLock<Self>,
        resolve: impl FnOnce(Option<&AppSettings>) -> RuntimeHotkeys,
    ) {
        let settings = applied
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .settings
            .clone();
        let runtime = resolve(settings.as_deref());
        let mut applied = applied.write().unwrap_or_else(|error| error.into_inner());
        // Settings applied while this resolved carry the new layout already.
        let same = match (&applied.settings, &settings) {
            (Some(current), Some(resolved)) => Arc::ptr_eq(current, resolved),
            (None, None) => true,
            _ => false,
        };
        if same {
            applied.runtime = runtime;
        }
    }
}

/// Resolves the shortcuts' letter keys again after the main thread published
/// a new keyboard layout, from the settings last applied, as a settings
/// reload does. The listener rebinds each shortcut once it is idle.
#[cfg(target_os = "macos")]
pub fn refresh_layout_hotkeys() {
    AppliedHotkeys::resolve_again(HOTKEYS.get_or_init(Default::default), |settings| {
        settings.map_or_else(RuntimeHotkeys::default, AppSettings::runtime_hotkeys)
    });
}

pub fn runtime_hotkeys() -> RuntimeHotkeys {
    HOTKEYS
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .runtime
}

pub fn dictation_hotkey() -> RuntimeHotkey {
    runtime_hotkeys().dictation
}

pub fn hotkey_capture_active() -> bool {
    HOTKEY_CAPTURE_ACTIVE.load(Ordering::Acquire)
}

pub fn set_hotkey_capture_active(active: bool) {
    HOTKEY_CAPTURE_ACTIVE.store(active, Ordering::Release);
}

/// The Dock policy while no Settings window is open. An open window always
/// shows the icon: hiding it deactivates Mumble asynchronously, which would drop
/// that window behind other applications.
pub fn dock_icon_visible(show_dock_icon: bool, status_item_available: bool) -> bool {
    show_dock_icon || !status_item_available
}

#[cfg(target_os = "macos")]
pub fn set_dock_icon_visible(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("Dock icon visibility must be changed on the main thread");
        return;
    };
    let application = NSApplication::sharedApplication(mtm);
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if application.activationPolicy() != policy && !application.setActivationPolicy(policy) {
        tracing::warn!(visible, "could not change Dock icon visibility");
    }
}

#[cfg(target_os = "macos")]
pub fn hide_application() {
    let Some(marker) = MainThreadMarker::new() else {
        tracing::warn!("cannot hide Mumble outside the main thread");
        return;
    };
    NSApplication::sharedApplication(marker).hide(None);
}

pub(crate) fn path() -> Result<PathBuf> {
    Ok(crate::app_paths::support_dir()?.join("settings.json"))
}

/// Whether settings were ever saved; a first run has none and onboards.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn exists() -> bool {
    path().is_ok_and(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GNOME's window menu and KDE's KRunner take Alt+Space, so new setups
    /// there dictate with Ctrl+Alt+Space; it shares no key with Paste last.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnome_dictates_with_a_key_gnome_leaves_free() {
        assert_eq!(
            HotkeyBinding::linux_dictation_default(false).keycaps(),
            ["Alt", "Space"]
        );
        let gnome = HotkeyBinding::linux_dictation_default(true);
        assert_eq!(gnome.keycaps(), ["Ctrl", "Alt", "Space"]);
        assert_ne!(gnome, HotkeyBinding::paste_last_default());
    }

    #[test]
    fn loading_folds_saved_recognition_hints_into_the_vocabulary_once() {
        let dir = TestSettingsDir::new("folded-hints");
        let mut file = serde_json::to_value(AppSettings {
            vocabulary: vec![VocabularyEntry {
                term: "Rune".into(),
                sounds_like: vec!["rune's".into()],
            }],
            sound_effect_volume: 0.25,
            ..AppSettings::default()
        })
        .unwrap();
        // Recognition hints saved by an older build (on the selected model and
        // the recent models) fold into the vocabulary once.
        file["transcription"]["recognition_hints"] = "Mumble, rune\nKaffar".into();
        file["transcription_recents"] = serde_json::json!([
            {"language": "de", "recognition_hints": "Old; Mumble"},
            {"language": "en", "recognition_hints": "Rodolfo"},
        ]);
        fs::write(&dir.path, serde_json::to_vec(&file).unwrap()).unwrap();
        let loaded = AppSettings::load_from(&dir.path).unwrap();
        let terms: Vec<_> = loaded
            .vocabulary
            .iter()
            .map(|entry| entry.term.as_str())
            .collect();
        assert_eq!(terms, ["Rune", "Mumble", "Kaffar", "Rodolfo", "Old"]);
        assert_eq!(loaded.vocabulary[0].sounds_like, ["rune's"]);
        assert_eq!(loaded.sound_effect_volume, 0.25);
        let written = fs::read_to_string(&dir.path).unwrap();
        assert!(!written.contains("recognition_hints"));
        assert_eq!(
            AppSettings::load_from(&dir.path).unwrap().vocabulary,
            loaded.vocabulary
        );
    }

    /// Labs › Second dictation shortcut (issue 627): its binding takes
    /// effect only on macOS with its toggle on, never sharing a key with
    /// another shortcut, and the other shortcuts' checks count it.
    #[test]
    fn the_second_dictation_shortcut_needs_its_lab_and_its_own_keys() {
        let option = HotkeyBinding {
            modifiers: HotkeyModifiers::option(),
            key: None,
        };
        let mut settings = AppSettings {
            second_dictation_hotkey: Some(option.clone()),
            ..AppSettings::default()
        };
        assert!(!settings.labs.second_dictation_shortcut);
        assert_eq!(settings.runtime_hotkeys().second_dictation, None);
        assert!(!settings.hotkey_in_use(HotkeySlot::RawDictation, &option));

        settings.labs.second_dictation_shortcut = true;
        let active = settings.runtime_hotkeys().second_dictation;
        if cfg!(target_os = "macos") {
            assert_eq!(active, Some(option.runtime()));
            assert!(settings.hotkey_in_use(HotkeySlot::RawDictation, &option));
            assert!(settings.hotkey_in_use(HotkeySlot::Dictation, &option));
        } else {
            assert_eq!(active, None);
        }

        // The dictation shortcut's own keys are refused, and a saved overlap
        // is dropped rather than matched twice.
        let dictation = settings.dictation_hotkey.clone();
        assert!(settings.hotkey_in_use(HotkeySlot::SecondDictation, &dictation));
        settings.second_dictation_hotkey = Some(dictation);
        assert_eq!(settings.runtime_hotkeys().second_dictation, None);
    }

    #[test]
    fn ducking_settings_are_opt_in_bounded_and_round_trip() {
        let mut settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            settings.recording_audio_behavior,
            RecordingAudioBehavior::DoNothing
        );
        assert_eq!(settings.recording_duck_volume, 0.2);
        settings.recording_audio_behavior = RecordingAudioBehavior::Duck;
        settings.recording_duck_volume = 0.37;
        let decoded: AppSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            decoded.recording_audio_behavior,
            RecordingAudioBehavior::Duck
        );
        assert_eq!(decoded.recording_duck_volume, 0.37);
        for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            settings.recording_duck_volume = invalid;
            assert!(settings.validate_recording_duck_volume().is_err());
        }
        for value in [0.0, 0.2, 1.0] {
            settings.recording_duck_volume = value;
            assert!(settings.validate_recording_duck_volume().is_ok());
        }
    }

    #[test]
    fn mouse_names_have_one_portable_representation() {
        for number in 1..=32 {
            let label = format!("Mouse {number}");
            let key = HotkeyKey::named(&label);
            assert!(key.is_mouse());
            assert_eq!(key.name(), format!("mouse{number}"));
            assert_eq!(key, HotkeyKey::mouse(number - 1).unwrap());
            assert_eq!(HotkeyKey::named(&key.name()), key);
        }
        for invalid in ["mouse0", "mouse33", "mouse", "mouse-1"] {
            assert_eq!(HotkeyKey::mouse_number(invalid), None);
        }
    }

    #[test]
    fn a_saved_bare_left_click_shortcut_returns_to_the_default_on_load() {
        let left_click = HotkeyBinding {
            modifiers: HotkeyModifiers::default(),
            key: HotkeyKey::mouse(0),
        };
        assert!(left_click.is_bare_primary_click());
        let option_click = HotkeyBinding {
            modifiers: HotkeyModifiers::option(),
            key: HotkeyKey::mouse(0),
        };
        assert!(!option_click.is_bare_primary_click());
        let back_button = HotkeyBinding {
            modifiers: HotkeyModifiers::default(),
            key: HotkeyKey::mouse(3),
        };
        assert!(!back_button.is_bare_primary_click());

        let dir = TestSettingsDir::new("left-click-shortcut");
        let settings = AppSettings {
            dictation_hotkey: left_click,
            ..AppSettings::default()
        };
        fs::write(&dir.path, serde_json::to_vec(&settings).unwrap()).unwrap();
        let loaded = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(loaded.dictation_hotkey, HotkeyBinding::default());

        let mut settings = AppSettings {
            dictation_hotkey: back_button.clone(),
            ..AppSettings::default()
        };
        // macOS keeps a mouse shortcut only with Labs › Mouse shortcuts on.
        settings.labs.mouse_shortcuts = true;
        fs::write(&dir.path, serde_json::to_vec(&settings).unwrap()).unwrap();
        let loaded = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(loaded.dictation_hotkey, back_button);
    }

    #[test]
    fn the_setting_or_an_installed_driver_turns_the_voice_filter_on_and_labs_never_does() {
        if env_mumble_experimental_mic() {
            return;
        }
        assert!(!voice_filter_enabled(false, false));
        assert!(voice_filter_enabled(true, false));
        assert!(voice_filter_enabled(false, true));
    }

    #[test]
    fn mouse_shortcuts_go_back_to_their_default_while_their_labs_toggle_is_off() {
        let mouse = HotkeyBinding {
            modifiers: HotkeyModifiers::default(),
            key: HotkeyKey::mouse(3),
        };
        let mut settings = AppSettings {
            dictation_hotkey: mouse.clone(),
            ..Default::default()
        };
        settings.labs.mouse_shortcuts = true;
        settings.repair_mouse_shortcuts();
        assert_eq!(settings.dictation_hotkey, mouse);
        assert!(!settings.mouse_shortcut_notice);
        settings.labs.mouse_shortcuts = false;
        settings.repair_mouse_shortcuts();
        assert_eq!(settings.dictation_hotkey, HotkeyBinding::default());
        assert!(settings.mouse_shortcut_notice);
    }

    #[test]
    fn missing_fields_receive_defaults() {
        let settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.retired_commands_enabled, None);
        assert!(!settings.release_microphone_while_idle);
        assert!(settings.sound_effects);
        assert_eq!(settings.sound_effect_volume, 0.5);
        assert_eq!(settings.microphone, None);
        assert_eq!(
            settings.recording_audio_behavior,
            RecordingAudioBehavior::DoNothing
        );
        assert!(!settings.pause_browser_media);
        assert!(settings.double_tap_lock);
        assert!(!settings.double_tap_only);
        assert_eq!(settings.dictation_hotkey, HotkeyBinding::default());
        assert_eq!(
            settings.paste_last_hotkey,
            Some(HotkeyBinding::paste_last_default())
        );
        assert!(settings.show_dock_icon);
        assert_eq!(settings.transcription, AppSettings::default().transcription);
        assert!(
            !settings
                .dictation_processing
                .default_mode
                .post_processing
                .enabled
        );
        assert!(settings.text_replacements.is_empty());
    }

    #[test]
    fn vocabulary_round_trips_and_stays_out_of_files_without_one() {
        // A file from before the vocabulary loads with an empty one and is
        // not rewritten to gain the field.
        let dir = TestSettingsDir::new("vocabulary");
        let older = r#"{"sound_effects": false}"#;
        fs::write(&dir.path, older).unwrap();
        let settings = AppSettings::load_from(&dir.path).unwrap();
        assert!(settings.vocabulary.is_empty());
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&dir.path).unwrap()).unwrap();
        assert!(written.get("vocabulary").is_none());
        assert!(
            serde_json::to_value(AppSettings::default())
                .unwrap()
                .get("vocabulary")
                .is_none()
        );

        let filled = AppSettings {
            vocabulary: vec![
                VocabularyEntry {
                    term: "Cappuccino".into(),
                    sounds_like: vec!["capuchin".into(), "cap a chino".into()],
                },
                VocabularyEntry {
                    term: "Rune".into(),
                    sounds_like: vec![],
                },
            ],
            ..settings
        };
        filled.write_to(&dir.path).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&dir.path).unwrap()).unwrap();
        assert_eq!(
            json["vocabulary"],
            serde_json::json!([
                {"term": "Cappuccino", "sounds_like": ["capuchin", "cap a chino"]},
                "Rune",
            ])
        );
        let reloaded = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(reloaded.vocabulary, filled.vocabulary);
    }

    struct TestSettingsDir {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestSettingsDir {
        fn new(prefix: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "mumble-{prefix}-{}-{}",
                std::process::id(),
                SETTINGS_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&directory).unwrap();
            let path = directory.join("settings.json");
            Self { directory, path }
        }
    }

    impl Drop for TestSettingsDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    /// #744: a value this build can't read costs only that setting, and the
    /// file (perhaps a newer build's) is left as it was.
    #[test]
    fn an_unreadable_setting_keeps_its_default_and_the_rest_loads() {
        let dir = TestSettingsDir::new("lenient");
        let file = r#"{
            "sound_effects": false,
            "history_retention": null,
            "show_dock_icon": "sometimes",
            "experimental": {"mumble_microphone_comfort_noise": false, "mumble_microphone_mode": "quarterly"},
            "recording_duck_volume": 7.0
        }"#;
        fs::write(&dir.path, file).unwrap();
        let settings = AppSettings::load_from(&dir.path).unwrap();
        let defaults = AppSettings::default();
        assert!(!settings.sound_effects);
        assert!(!settings.experimental.mumble_microphone_comfort_noise);
        assert_eq!(settings.history_retention, defaults.history_retention);
        assert_eq!(settings.show_dock_icon, defaults.show_dock_icon);
        assert_eq!(
            settings.experimental.mumble_microphone_mode,
            defaults.experimental.mumble_microphone_mode
        );
        assert_eq!(
            settings.recording_duck_volume,
            defaults.recording_duck_volume
        );
        assert_eq!(
            fs::read_to_string(&dir.path).unwrap(),
            file,
            "not rewritten"
        );

        // Even malformed JSON defaults in memory and stays in place.
        fs::write(&dir.path, "{ not json").unwrap();
        assert_eq!(AppSettings::load_from(&dir.path).unwrap(), defaults);
        assert_eq!(fs::read_to_string(&dir.path).unwrap(), "{ not json");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn new_settings_require_onboarding_across_reloads() {
        use crate::desktop::onboarding::{completion_recorded_at, record_completion_at};
        use std::os::unix::fs::PermissionsExt;

        let dir = TestSettingsDir::new("new-onboarding");
        assert!(!completion_recorded_at(&dir.directory));

        let mut settings = AppSettings::load_from(&dir.path).unwrap();
        assert!(!dir.path.exists());
        assert_eq!(settings.sound_effects, AppSettings::default().sound_effects);
        settings.sound_effects = false;
        settings.double_tap_lock = false;
        settings.write_to(&dir.path).unwrap();
        assert!(dir.path.is_file());
        assert!(!dir.directory.join("models").exists());
        assert!(!completion_recorded_at(&dir.directory));
        assert!(!dir.directory.join("onboarding-complete").exists());
        assert_eq!(
            fs::metadata(dir.directory.join("onboarding-pending"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
        );
        drop(settings);

        let reloaded = AppSettings::load_from(&dir.path).unwrap();
        assert!(!reloaded.sound_effects);
        assert!(!reloaded.double_tap_lock);
        reloaded.write_to(&dir.path).unwrap();
        assert!(!completion_recorded_at(&dir.directory));
        assert!(!dir.directory.join("onboarding-complete").exists());

        record_completion_at(&dir.directory).unwrap();
        assert!(completion_recorded_at(&dir.directory));
        drop(reloaded);
        AppSettings::load_from(&dir.path).unwrap();
        assert!(completion_recorded_at(&dir.directory));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn loading_strips_finder_bundle_extensions_from_mode_applications() {
        let dir = TestSettingsDir::new("mode-applications");
        fs::write(
            &dir.path,
            br#"{"dictation_processing":{"modes":[{"name":"Code","applications":["Zed.app","Ghostty.app","Zed"]}]}}"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(
            settings.dictation_processing.modes[0].applications,
            vec!["Ghostty".to_string(), "Zed".to_string()]
        );
        let persisted: AppSettings = serde_json::from_slice(&fs::read(&dir.path).unwrap()).unwrap();
        assert_eq!(
            persisted.dictation_processing.modes[0].applications,
            vec!["Ghostty".to_string(), "Zed".to_string()]
        );
    }

    #[test]
    fn loading_persists_normalizations_only_when_they_change_settings() {
        let dir = TestSettingsDir::new("normalized-settings");
        fs::write(
            &dir.path,
            br#"{"text_replacements":[{"matched_phrase":"open code","output":"OpenCode"}]}"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&dir.path).unwrap();
        assert!(settings.text_replacements.is_empty());
        let persisted = fs::read_to_string(&dir.path).unwrap();
        assert!(!persisted.contains("text_replacements"));
        assert!(persisted.contains("OpenCode"));

        let written = fs::metadata(&dir.path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let reloaded = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(
            reloaded.dictation_processing.default_mode.replacements,
            settings.dictation_processing.default_mode.replacements
        );
        assert_eq!(
            fs::metadata(&dir.path).unwrap().modified().unwrap(),
            written
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rewriting_legacy_rust_settings_preserves_onboarding_migration() {
        let dir = TestSettingsDir::new("legacy-onboarding");
        fs::write(&dir.path, b"{}").unwrap();

        let settings = AppSettings::load_from(&dir.path).unwrap();
        settings.write_to(&dir.path).unwrap();
        assert!(!dir.directory.join("onboarding-pending").exists());
        assert!(crate::desktop::onboarding::completion_recorded_at(
            &dir.directory
        ));
        assert!(dir.directory.join("onboarding-complete").is_file());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn first_settings_write_requires_the_pending_marker() {
        let dir = TestSettingsDir::new("pending-onboarding");
        fs::create_dir(dir.directory.join("onboarding-pending")).unwrap();

        assert!(AppSettings::default().write_to(&dir.path).is_err());
        assert!(!dir.path.exists());
    }

    #[test]
    fn legacy_sleep_prevention_setting_is_ignored() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"prevent_system_sleep":false}"#).unwrap();
        let serialized = serde_json::to_value(settings).unwrap();

        assert!(serialized.get("prevent_system_sleep").is_none());
    }

    #[test]
    fn an_unset_microphone_is_system_default_and_a_saved_one_is_kept() {
        let automatic: AppSettings = serde_json::from_str(r#"{"microphone":null}"#).unwrap();
        assert_eq!(automatic.microphone, None);
        let absent: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.microphone, None);
        let saved: AppSettings =
            serde_json::from_str(r#"{"microphone":"Studio Display Microphone"}"#).unwrap();
        assert_eq!(
            saved.microphone.as_deref(),
            Some("Studio Display Microphone")
        );
    }

    #[test]
    fn retired_commands_settings_load_cleanly_and_survive_saves() {
        let directory = std::env::temp_dir().join(format!(
            "mumble-retired-commands-{}-{}",
            std::process::id(),
            SETTINGS_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");

        let mut initial = serde_json::to_value(AppSettings::default()).unwrap();
        assert!(initial.get("commands_enabled").is_none());
        initial["commands_enabled"] = serde_json::json!(true);
        initial["release_microphone_while_idle"] = serde_json::json!(true);
        let raw = serde_json::to_vec_pretty(&initial).unwrap();
        fs::write(&path, &raw).unwrap();

        let mut loaded = AppSettings::load_from(&path).unwrap();
        assert!(loaded.release_microphone_while_idle);
        assert_eq!(
            fs::read(&path).unwrap(),
            raw,
            "loading already-normalized settings does not rewrite retired keys away"
        );

        loaded.sound_effect_volume = 0.75;
        loaded.write_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["commands_enabled"], serde_json::json!(true));
        assert_eq!(
            saved["release_microphone_while_idle"],
            serde_json::json!(true)
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn retired_voice_action_settings_load_cleanly_and_survive_saves() {
        let directory = std::env::temp_dir().join(format!(
            "mumble-retired-voice-action-{}-{}",
            std::process::id(),
            SETTINGS_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");

        let mut initial = serde_json::to_value(AppSettings::default()).unwrap();
        initial["edit_hotkey"] = serde_json::json!({
            "modifiers": {"option": "either", "command": "either"},
            "key": null
        });
        initial["voice_action"] = serde_json::json!({
            "enabled": true,
            "model": "anthropic/claude-sonnet-4-5",
            "variant": "fast",
            "deadline_seconds": 45
        });
        let raw = serde_json::to_vec_pretty(&initial).unwrap();
        fs::write(&path, &raw).unwrap();

        let mut loaded = AppSettings::load_from(&path).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            raw,
            "loading already-normalized settings does not rewrite retired keys away"
        );

        loaded.sound_effect_volume = 0.75;
        loaded.write_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["edit_hotkey"], initial["edit_hotkey"]);
        assert_eq!(saved["voice_action"], initial["voice_action"]);

        let null_edit: AppSettings = serde_json::from_str(r#"{"edit_hotkey":null}"#).unwrap();
        assert_eq!(
            serde_json::to_value(null_edit).unwrap()["edit_hotkey"],
            serde_json::Value::Null
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn transcription_selection_round_trips() {
        use crate::transcription::CloudProvider;
        let settings = AppSettings {
            transcription: TranscriptionSelection {
                provider: CloudProvider::Codex,
                language: "zh".into(),
            },
            ..AppSettings::default()
        };

        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.transcription, settings.transcription);
        assert!(encoded.contains(r#""provider":"codex""#));
    }

    #[test]
    fn a_settings_file_naming_a_local_model_loads_with_the_default_provider() {
        // Files from before the cloud-only move named a local model and
        // language; the removed field is ignored and the provider defaults.
        let dir = TestSettingsDir::new("legacy-model");
        fs::write(
            &dir.path,
            r#"{"transcription":{"model":"parakeet_v3","language":"de"},"transcription_device":"gpu"}"#,
        )
        .unwrap();
        let settings = AppSettings::load_from(&dir.path).unwrap();
        assert_eq!(
            settings.transcription,
            TranscriptionSelection {
                provider: crate::transcription::CloudProvider::default(),
                language: "de".into(),
            }
        );
    }

    #[test]
    fn legacy_global_replacements_migrate_into_every_mode() {
        let mut settings: AppSettings = serde_json::from_str(
            r#"{"text_replacements":[{"matched_phrase":"open code","output":"OpenCode"}]}"#,
        )
        .unwrap();
        settings.dictation_processing.modes.push(DictationMode {
            name: "Messages".into(),
            ..DictationMode::default()
        });
        settings.migrate_legacy_replacements();

        assert_eq!(
            settings.dictation_processing.default_mode.replacements,
            [TextReplacement {
                matched_phrase: "open code".into(),
                output: "OpenCode".into(),
            }]
        );
        assert_eq!(
            settings.dictation_processing.modes[0].replacements,
            settings.dictation_processing.default_mode.replacements
        );
        assert!(settings.text_replacements.is_empty());
        assert!(
            !serde_json::to_string(&settings)
                .unwrap()
                .contains("text_replacements")
        );
    }

    #[test]
    fn hotkey_runtime_preserves_modifier_sides_and_key_code() {
        let binding = HotkeyBinding {
            modifiers: HotkeyModifiers {
                control: Some(ModifierSide::Left),
                option: None,
                shift: Some(ModifierSide::Right),
                command: None,
                function: true,
            },
            key: Some(HotkeyKey {
                code: 49,
                label: "Space".into(),
            }),
        };
        let runtime = binding.runtime();

        assert_eq!(runtime.modifiers, binding.modifiers);
        assert_eq!(runtime.key_code, Some(49));
    }

    #[test]
    fn shortcut_mode_defaults_to_hold_and_round_trips() {
        let legacy: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.shortcut_mode, crate::dictation::ShortcutMode::Hold);
        let settings = AppSettings {
            shortcut_mode: crate::dictation::ShortcutMode::Toggle,
            ..Default::default()
        };
        let decoded: AppSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            decoded.shortcut_mode,
            crate::dictation::ShortcutMode::Toggle
        );
    }

    #[test]
    fn modifier_sides_round_trip_and_legacy_booleans_decode_as_either() {
        let settings = AppSettings {
            dictation_hotkey: HotkeyBinding {
                modifiers: HotkeyModifiers {
                    option: Some(ModifierSide::Right),
                    command: Some(ModifierSide::Left),
                    ..Default::default()
                },
                key: Some(HotkeyKey {
                    code: 0,
                    label: "A".into(),
                }),
            },
            paste_last_hotkey: None,
            double_tap_only: true,
            ..Default::default()
        };

        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.dictation_hotkey, settings.dictation_hotkey);
        assert_eq!(decoded.paste_last_hotkey, None);
        assert!(decoded.double_tap_only);
        // Settings saved before dictation streamed still load.
        assert!(
            serde_json::from_str::<AppSettings>(r#"{"incremental_transcription":false}"#).is_ok()
        );

        let legacy: HotkeyModifiers = serde_json::from_str(
            r#"{"control":false,"option":true,"shift":false,"command":false,"function":false}"#,
        )
        .unwrap();
        assert_eq!(legacy.option, Some(ModifierSide::Either));
    }

    #[test]
    fn side_constraints_can_be_removed_from_chord_modifiers() {
        let modifiers = HotkeyModifiers {
            option: Some(ModifierSide::Left),
            command: Some(ModifierSide::Right),
            function: true,
            ..Default::default()
        };

        assert_eq!(
            modifiers.without_side_constraints(),
            HotkeyModifiers {
                option: Some(ModifierSide::Either),
                command: Some(ModifierSide::Either),
                function: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn side_aware_matching_and_conflicts_share_the_same_algebra() {
        let left = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Left),
                ..Default::default()
            },
            key: Some(HotkeyKey {
                code: 0,
                label: "A".into(),
            }),
        };
        let right = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Right),
                ..Default::default()
            },
            key: left.key.clone(),
        };
        let either = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Either),
                ..Default::default()
            },
            key: left.key.clone(),
        };

        assert!(!left.overlaps(&right));
        assert!(left.overlaps(&either));
        assert!(
            left.runtime()
                .matches_key_press(0, OPTION_KEY_MASK | LEFT_OPTION_MASK,)
        );
        assert!(
            !left
                .runtime()
                .matches_key_press(0, OPTION_KEY_MASK | RIGHT_OPTION_MASK,)
        );
        assert!(
            either
                .runtime()
                .matches_key_press(0, OPTION_KEY_MASK | RIGHT_OPTION_MASK,)
        );
    }

    #[test]
    fn runtime_side_constraints_require_the_requested_device_flag() {
        for (modifier, general, left, right) in [
            (
                "control",
                CONTROL_KEY_MASK,
                LEFT_CONTROL_MASK,
                RIGHT_CONTROL_MASK,
            ),
            (
                "option",
                OPTION_KEY_MASK,
                LEFT_OPTION_MASK,
                RIGHT_OPTION_MASK,
            ),
            ("shift", SHIFT_KEY_MASK, LEFT_SHIFT_MASK, RIGHT_SHIFT_MASK),
            (
                "command",
                COMMAND_KEY_MASK,
                LEFT_COMMAND_MASK,
                RIGHT_COMMAND_MASK,
            ),
        ] {
            for side in ["left", "right", "either"] {
                let binding: HotkeyBinding = serde_json::from_value(serde_json::json!({
                    "modifiers": { (modifier): side },
                    "key": { "code": 0, "label": "A" },
                }))
                .unwrap();
                let binding = binding.runtime();
                // Both sides held hold a sided binding's key, but only an
                // either-side binding matches them exactly.
                for (device_flags, held, exact) in [
                    (0, side == "either", side == "either"),
                    (left, side != "right", side != "right"),
                    (right, side != "left", side != "left"),
                    (left | right, true, side == "either"),
                ] {
                    let flags = general | device_flags;
                    assert_eq!(
                        binding.required_modifiers_down(flags),
                        held,
                        "{side:?}, flags={flags:#x}"
                    );
                    assert_eq!(
                        binding.matches_key_press(0, flags),
                        exact,
                        "{side:?}, flags={flags:#x}"
                    );
                    assert!(!binding.exact_modifiers(flags | FUNCTION_KEY_MASK));
                    assert_eq!(
                        binding.required_modifiers_down(flags | FUNCTION_KEY_MASK),
                        held
                    );
                }
            }
        }
    }

    #[test]
    fn default_paste_hotkeys_follow_build_capabilities() {
        for hotkeys in [
            RuntimeHotkeys::default(),
            AppSettings::default().runtime_hotkeys(),
        ] {
            assert_eq!(
                hotkeys.paste_last,
                Some(HotkeyBinding::paste_last_default().runtime())
            );
        }
    }

    /// Launched on Dvorak, then switched to U.S. (t1304): letter shortcuts,
    /// saved or default, follow the layout the main thread published; other
    /// keys keep their recorded code.
    #[cfg(target_os = "macos")]
    #[test]
    fn letter_shortcuts_resolve_again_on_a_new_layout() {
        use super::hotkeys::test_layout;
        let resolve = |settings: Option<&AppSettings>| {
            settings.map_or_else(RuntimeHotkeys::default, AppSettings::runtime_hotkeys)
        };
        test_layout::set([('v', 47)]);
        let settings = AppSettings {
            raw_dictation_hotkey: Some(HotkeyBinding {
                modifiers: HotkeyModifiers::option(),
                key: Some(HotkeyKey {
                    code: 96,
                    label: "F5".into(),
                }),
            }),
            ..AppSettings::default()
        };
        let saved_on_dvorak = settings.paste_last_hotkey.clone().unwrap();
        assert_eq!(saved_on_dvorak.key.as_ref().unwrap().code, 47);
        let applied = RwLock::new(AppliedHotkeys {
            runtime: settings.runtime_hotkeys(),
            settings: Some(Arc::new(settings)),
        });
        let key = |applied: &RwLock<AppliedHotkeys>| {
            let runtime = applied.read().unwrap().runtime;
            [runtime.paste_last, runtime.raw_dictation]
                .map(|hotkey| hotkey.and_then(|hotkey| hotkey.key_code))
        };
        assert_eq!(key(&applied), [Some(47), Some(96)]);

        test_layout::set([('v', 9)]);
        AppliedHotkeys::resolve_again(&applied, resolve);
        assert_eq!(key(&applied), [Some(9), Some(96)]);

        // A layout without the letter keeps the recorded key.
        test_layout::set([('x', 7)]);
        assert_eq!(saved_on_dvorak.runtime().key_code, Some(47));

        // Settings applied while it resolved win: they saw the new layout.
        test_layout::set([('v', 47)]);
        let newer = AppSettings {
            paste_last_hotkey: None,
            ..AppSettings::default()
        };
        AppliedHotkeys::resolve_again(&applied, |settings| {
            let resolved = resolve(settings);
            *applied.write().unwrap() = AppliedHotkeys {
                runtime: newer.runtime_hotkeys(),
                settings: Some(Arc::new(newer.clone())),
            };
            resolved
        });
        assert_eq!(key(&applied)[0], None);
    }

    #[test]
    fn side_selection_changes_one_modifier_chords_only() {
        let mut standalone = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Left),
                ..Default::default()
            },
            key: None,
        };
        assert_eq!(standalone.standalone_side(), Some(ModifierSide::Left));
        standalone.set_standalone_side(ModifierSide::Right);
        assert_eq!(standalone.modifiers.option, Some(ModifierSide::Right));
        standalone.set_standalone_side(ModifierSide::Both);
        assert_eq!(standalone.modifiers.option, Some(ModifierSide::Right));

        for modifiers in [
            HotkeyModifiers {
                option: Some(ModifierSide::Left),
                command: Some(ModifierSide::Left),
                ..Default::default()
            },
            HotkeyModifiers {
                option: Some(ModifierSide::Both),
                ..Default::default()
            },
        ] {
            let chord = HotkeyBinding {
                modifiers,
                key: None,
            };
            assert_eq!(chord.standalone_side(), None, "{modifiers:?}");
        }

        let mut chord = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Left),
                ..Default::default()
            },
            key: Some(HotkeyKey {
                code: 0,
                label: "A".into(),
            }),
        };
        assert_eq!(chord.standalone_side(), None);
        chord.set_standalone_side(ModifierSide::Right);
        assert_eq!(chord.modifiers.option, Some(ModifierSide::Left));
    }

    #[test]
    fn shortcuts_store_as_chords_and_old_files_convert_once() {
        let both_options = HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(ModifierSide::Both),
                ..Default::default()
            },
            key: None,
        };
        assert_eq!(
            serde_json::to_value(&both_options).unwrap(),
            serde_json::json!({ "keys": [
                { "modifier": "option", "side": "left" },
                { "modifier": "option", "side": "right" },
            ]})
        );
        let chord: HotkeyBinding = serde_json::from_value(serde_json::json!({ "keys": [
            { "code": 49, "label": "Space" },
            { "modifier": "option", "side": "right" },
            { "modifier": "option", "side": "left" },
        ]}))
        .unwrap();
        assert_eq!(
            chord.modifiers.option,
            Some(ModifierSide::Both),
            "any order"
        );
        assert_eq!(chord.key.unwrap().code, 49);
        for refused in [
            serde_json::json!({ "keys": [
                { "modifier": "option", "side": "left" },
                { "modifier": "option", "side": "left" },
            ]}),
            serde_json::json!({ "keys": [
                { "modifier": "option", "side": "any" },
                { "modifier": "option", "side": "right" },
            ]}),
            serde_json::json!({ "keys": [
                { "code": 49, "label": "Space" },
                { "code": 0, "label": "A" },
            ]}),
            serde_json::json!({ "keys": [{ "modifier": "hyper" }] }),
        ] {
            assert!(
                serde_json::from_value::<HotkeyBinding>(refused.clone()).is_err(),
                "{refused}"
            );
        }

        // Every shortcut in a file from before chords is the same chord after.
        let legacy = serde_json::json!({
            "dictation_hotkey": { "modifiers": { "option": "right" }, "key": null },
            "edit_hotkey": { "modifiers": { "option": true, "command": true, "control": false }, "key": null },
            "paste_last_hotkey": {
                "modifiers": { "option": "either", "shift": "either" },
                "key": { "code": 9, "label": "V" }
            },
        });
        let settings: AppSettings = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            settings.dictation_hotkey.modifiers,
            HotkeyModifiers {
                option: Some(ModifierSide::Right),
                ..Default::default()
            }
        );
        let stored = serde_json::to_value(&settings).unwrap();
        assert_eq!(stored["edit_hotkey"], legacy["edit_hotkey"]);
        assert_eq!(
            stored["dictation_hotkey"],
            serde_json::json!({ "keys": [{ "modifier": "option", "side": "right" }] })
        );
        assert_eq!(
            stored["paste_last_hotkey"],
            serde_json::json!({ "keys": [
                { "modifier": "option", "side": "any" },
                { "modifier": "shift", "side": "any" },
                { "code": 9, "label": "V" },
            ]})
        );
        let reloaded: AppSettings = serde_json::from_value(stored).unwrap();
        assert_eq!(reloaded.dictation_hotkey, settings.dictation_hotkey);
        assert_eq!(reloaded.retired_edit_hotkey, settings.retired_edit_hotkey);
        assert_eq!(reloaded.paste_last_hotkey, settings.paste_last_hotkey);
        for binding in [
            HotkeyBinding::default(),
            HotkeyBinding::paste_last_default(),
        ] {
            let json = serde_json::to_value(&binding).unwrap();
            assert_eq!(
                serde_json::from_value::<HotkeyBinding>(json).unwrap(),
                binding
            );
        }
    }

    #[test]
    fn both_sides_conflict_only_with_either_side_and_match_only_together() {
        let option = |side| HotkeyBinding {
            modifiers: HotkeyModifiers {
                option: Some(side),
                ..Default::default()
            },
            key: None,
        };
        let both = option(ModifierSide::Both);
        assert!(both.overlaps(&option(ModifierSide::Either)));
        assert!(!both.overlaps(&option(ModifierSide::Left)));
        assert!(!both.overlaps(&option(ModifierSide::Right)));
        assert_eq!(both.modifiers.count(), 2);
        let both = both.runtime();
        let general = OPTION_KEY_MASK;
        assert!(both.exact_modifiers(general | LEFT_OPTION_MASK | RIGHT_OPTION_MASK));
        assert!(!both.exact_modifiers(general | LEFT_OPTION_MASK));
        assert!(!both.exact_modifiers(general | RIGHT_OPTION_MASK));
        assert!(!both.exact_modifiers(general), "input naming no side");
        assert!(
            !both
                .exact_modifiers(general | LEFT_OPTION_MASK | RIGHT_OPTION_MASK | COMMAND_KEY_MASK)
        );
        assert!(both.required_modifiers_down(
            general | LEFT_OPTION_MASK | RIGHT_OPTION_MASK | COMMAND_KEY_MASK
        ));
        assert_eq!(
            both.both_sides_masks().collect::<Vec<_>>(),
            [(LEFT_OPTION_MASK, RIGHT_OPTION_MASK)]
        );
        assert_eq!(
            option(ModifierSide::Either)
                .runtime()
                .both_sides_masks()
                .count(),
            0
        );
    }

    #[test]
    fn recorded_sides_keep_one_modifiers_side_and_both_sides() {
        let lone = HotkeyModifiers {
            option: Some(ModifierSide::Right),
            ..Default::default()
        };
        assert_eq!(lone.recorded_sides(), lone);
        let pair = HotkeyModifiers {
            option: Some(ModifierSide::Left),
            command: Some(ModifierSide::Right),
            ..Default::default()
        };
        assert_eq!(pair.recorded_sides(), HotkeyModifiers::option_command());
        let both = HotkeyModifiers {
            option: Some(ModifierSide::Both),
            shift: Some(ModifierSide::Left),
            ..Default::default()
        };
        assert_eq!(
            both.recorded_sides(),
            HotkeyModifiers {
                option: Some(ModifierSide::Both),
                shift: Some(ModifierSide::Either),
                ..Default::default()
            }
        );
    }
}
