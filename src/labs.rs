//! Labs: features we're unsure about, especially ones people asked for,
//! each behind its own switch, off by default. There is no master switch:
//! Settings › Labs lists every feature as a row with a toggle. A feature
//! with its own pane (Voice Filter, Playground) joins the sidebar as a
//! normal item while on and is hidden while off; a behaviour-only feature is
//! just its toggle.
//!
//! Each feature has an audience (`Feature::audience`): `Everyone` ships in
//! every build; `Dev` exists only in dev builds (`dev_build::is_dev_build`:
//! debug, local, test, and dev-channel builds). A stable build never lists,
//! accepts, or runs a Dev feature, and keeps its saved setting untouched.
//!
//! Gate a feature where its work starts (the worker, thread, or capture
//! that does it), not only where its UI shows.

use std::sync::atomic::{AtomicU64, Ordering};

use clap::Subcommand;
use color_eyre::Result;
use color_eyre::eyre::eyre;

use crate::app_settings::{AppSettings, LabsSettings};

/// Where people send feedback on Labs features.
pub const FEEDBACK_URL: &str = "https://github.com/anomalyco/hex/issues/new";

/// Who a Labs feature is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Audience {
    /// Ships in every build.
    Everyone,
    /// Only dev builds have it.
    Dev,
}

impl Audience {
    /// Whether a build of this kind has features for it.
    pub const fn reaches(self, dev_build: bool) -> bool {
        match self {
            Self::Everyone => true,
            Self::Dev => dev_build,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Everyone => "everyone",
            Self::Dev => "dev",
        }
    }
}

/// One Labs feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feature {
    VoiceFilter,
    Playground,
    LiveText,
    TerminalPaste,
    MouseShortcuts,
    PauseBrowser,
    ShareShortcut,
    RestoreShortcutDefault,
    KeepOnClipboard,
    CopyOnly,
    SpokenFormatting,
    FillerWords,
    SecondDictationShortcut,
    SmartSpacing,
    SpokenSymbols,
    SidedChords,
    TapOrHold,
    StandaloneSpecialKeys,
    HistoryStats,
    RecentDictations,
    OutputFixes,
    SectionKeyAlone,
    HideMenuBarIcon,
    GnomeClipboardRestore,
}

impl Feature {
    pub const ALL: [Self; 24] = [
        Self::VoiceFilter,
        Self::Playground,
        Self::LiveText,
        Self::TerminalPaste,
        Self::MouseShortcuts,
        Self::PauseBrowser,
        Self::ShareShortcut,
        Self::RestoreShortcutDefault,
        Self::KeepOnClipboard,
        Self::CopyOnly,
        Self::SpokenFormatting,
        Self::FillerWords,
        Self::SecondDictationShortcut,
        Self::SmartSpacing,
        Self::SpokenSymbols,
        Self::SidedChords,
        Self::TapOrHold,
        Self::StandaloneSpecialKeys,
        Self::HistoryStats,
        Self::RecentDictations,
        Self::OutputFixes,
        Self::SectionKeyAlone,
        Self::HideMenuBarIcon,
        Self::GnomeClipboardRestore,
    ];

    /// The features this OS and build have.
    pub fn available() -> impl Iterator<Item = Self> {
        Self::available_where(Self::shipped)
    }

    /// The features on this OS that `reaches` admits.
    fn available_where(reaches: impl Fn(Self) -> bool) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |feature| feature.on_this_os() && reaches(*feature))
    }

    /// Whether this build has the feature: its audience reaches a dev or
    /// stable build.
    pub fn shipped(self) -> bool {
        self.audience().reaches(crate::dev_build::is_dev_build())
    }

    /// Who the feature is for. Every feature picks one.
    pub const fn audience(self) -> Audience {
        match self {
            Self::VoiceFilter
            | Self::Playground
            | Self::LiveText
            | Self::TerminalPaste
            | Self::MouseShortcuts
            | Self::PauseBrowser
            | Self::ShareShortcut
            | Self::RestoreShortcutDefault
            | Self::KeepOnClipboard
            | Self::CopyOnly
            | Self::SpokenFormatting
            | Self::FillerWords
            | Self::SecondDictationShortcut
            | Self::SmartSpacing
            | Self::SpokenSymbols
            | Self::SidedChords
            | Self::TapOrHold
            | Self::StandaloneSpecialKeys
            | Self::HistoryStats
            | Self::RecentDictations
            | Self::OutputFixes
            | Self::SectionKeyAlone
            | Self::HideMenuBarIcon
            | Self::GnomeClipboardRestore => Audience::Everyone,
        }
    }

    /// The feature `hex settings` key `key` switches.
    pub fn for_settings_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|feature| feature.settings_key() == key)
    }

    fn on_this_os(self) -> bool {
        match self {
            Self::VoiceFilter => cfg!(any(target_os = "macos", target_os = "linux")),
            Self::TerminalPaste | Self::ShareShortcut => cfg!(target_os = "windows"),
            Self::SectionKeyAlone | Self::GnomeClipboardRestore => cfg!(target_os = "linux"),
            Self::StandaloneSpecialKeys => {
                cfg!(any(target_os = "linux", target_os = "windows"))
            }
            Self::MouseShortcuts
            | Self::PauseBrowser
            | Self::KeepOnClipboard
            | Self::CopyOnly
            | Self::SecondDictationShortcut
            | Self::SmartSpacing
            | Self::SidedChords
            | Self::RecentDictations
            | Self::HideMenuBarIcon => cfg!(target_os = "macos"),
            Self::TapOrHold => cfg!(any(target_os = "macos", target_os = "linux")),
            Self::Playground
            | Self::LiveText
            | Self::RestoreShortcutDefault
            | Self::SpokenFormatting
            | Self::SpokenSymbols
            | Self::HistoryStats
            | Self::OutputFixes
            | Self::FillerWords => true,
        }
    }

    /// The name `hex labs on|off` takes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::VoiceFilter => "voice-filter",
            Self::Playground => "playground",
            Self::LiveText => "live-text",
            Self::TerminalPaste => "terminal-paste",
            Self::MouseShortcuts => "mouse-shortcuts",
            Self::PauseBrowser => "pause-browser",
            Self::ShareShortcut => "share-shortcut",
            Self::RestoreShortcutDefault => "restore-shortcut-default",
            Self::KeepOnClipboard => "keep-on-clipboard",
            Self::CopyOnly => "copy-only",
            Self::SpokenFormatting => "spoken-formatting",
            Self::FillerWords => "filler-words",
            Self::SecondDictationShortcut => "second-dictation-shortcut",
            Self::SmartSpacing => "smart-spacing",
            Self::SpokenSymbols => "spoken-symbols",
            Self::SidedChords => "sided-modifier-chords",
            Self::TapOrHold => "tap-or-hold",
            Self::StandaloneSpecialKeys => "special-keys",
            Self::HistoryStats => "history-stats",
            Self::RecentDictations => "recent-dictations",
            Self::OutputFixes => "output-fixes",
            Self::SectionKeyAlone => "section-key",
            Self::HideMenuBarIcon => "hide-menu-bar-icon",
            Self::GnomeClipboardRestore => "gnome-clipboard-restore",
        }
    }

    /// Its `hex settings` key.
    pub const fn settings_key(self) -> &'static str {
        match self {
            Self::VoiceFilter => "experimental.hex_microphone",
            Self::Playground => "labs.playground",
            Self::LiveText => "labs.live_text",
            Self::TerminalPaste => "paste_with_shift",
            Self::MouseShortcuts => "labs.mouse_shortcuts",
            Self::PauseBrowser => "pause_browser_media",
            Self::ShareShortcut => "share_dictation_shortcut",
            Self::RestoreShortcutDefault => "labs.restore_shortcut_default",
            Self::KeepOnClipboard => "labs.keep_on_clipboard",
            Self::CopyOnly => "labs.copy_only",
            Self::SpokenFormatting => "labs.spoken_formatting",
            Self::FillerWords => "labs.filler_words",
            Self::SecondDictationShortcut => "labs.second_dictation_shortcut",
            Self::SmartSpacing => "labs.smart_spacing",
            Self::SpokenSymbols => "labs.spoken_symbols",
            Self::SidedChords => "labs.sided_modifier_chords",
            Self::TapOrHold => "labs.tap_or_hold",
            Self::StandaloneSpecialKeys => "labs.standalone_special_keys",
            Self::HistoryStats => "labs.history_stats",
            Self::RecentDictations => "labs.recent_dictations",
            Self::OutputFixes => "labs.output_fixes",
            Self::SectionKeyAlone => "labs.section_key_alone",
            Self::HideMenuBarIcon => "labs.hide_menu_bar_icon",
            Self::GnomeClipboardRestore => "labs.gnome_clipboard_restore",
        }
    }

    /// The English one-line description the CLI prints; the window shows
    /// its translated row.
    pub const fn description(self) -> &'static str {
        match self {
            Self::VoiceFilter => {
                "Hex Microphone virtual input and real-time voice filters (its own pane)"
            }
            Self::Playground => "Scene and story browser in the window",
            Self::LiveText => "Show provisional words beside the HUD while dictating",
            Self::TerminalPaste => "Paste with Ctrl+Shift+V, for apps that ignore Ctrl+V",
            Self::MouseShortcuts => "Record mouse buttons as shortcuts",
            Self::PauseBrowser => "Also pause the browser while dictating, with Pause media",
            Self::ShareShortcut => "Let apps such as Discord see the dictation shortcut too",
            Self::RestoreShortcutDefault => {
                "Offer Restore default next to Paste last dictation's shortcut"
            }
            Self::KeepOnClipboard => {
                "Leave the dictation on the clipboard after pasting instead of restoring what was there"
            }
            Self::CopyOnly => "Put the dictation on the clipboard instead of pasting it",
            Self::SpokenFormatting => {
                "Turn spoken \"new line\", \"new paragraph\", and \"open/close quote\" into line breaks and quotation marks"
            }
            Self::FillerWords => {
                "Remove filler words such as um and uh from dictations, without an AI model"
            }
            Self::SecondDictationShortcut => {
                "Another shortcut that dictates like the main one (second_dictation_hotkey), such as a key on an external keyboard"
            }
            Self::SmartSpacing => "Add a space before a dictation pasted right after a word",
            Self::SpokenSymbols => {
                "Turn spoken \"comma\", \"colon\", \"hashtag\", \"underscore\", and similar words into symbols joined to their words"
            }
            Self::SidedChords => {
                "Record which side each modifier of a shortcut is on, such as Right Command + Right Option"
            }
            Self::TapOrHold => {
                "In Toggle mode, hold the shortcut to dictate until you let go; a short tap still starts and stops"
            }
            Self::StandaloneSpecialKeys if cfg!(target_os = "windows") => {
                "Use Insert, Home, End, Page Up, or Page Down alone as a shortcut"
            }
            Self::StandaloneSpecialKeys => {
                "Use Menu, Pause, or Scroll Lock alone as a shortcut, such as Caps Lock remapped to Menu"
            }
            Self::HistoryStats => {
                "Show a summary of your retained dictations at the top of History, with hex history stats"
            }
            Self::RecentDictations => {
                "Copy one of your last five dictations from the menu bar, under Paste Last Dictation"
            }
            Self::OutputFixes => {
                "Offer Vocabulary and Fix this beside Output's Try result, to correct a wrong result"
            }
            Self::SectionKeyAlone => {
                "Use the § key, left of 1 on ISO keyboards, alone as a shortcut; Hex takes § from every app while it's bound"
            }
            Self::HideMenuBarIcon => {
                "Hide Hex's menu bar icon; shortcuts keep working, and opening Hex again shows Settings"
            }
            Self::GnomeClipboardRestore => {
                "Put the previous clipboard back after a paste on GNOME Wayland, through the XWayland clipboard while XWayland is running"
            }
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| feature.name() == name)
    }

    /// Whether `settings` turn it on.
    pub fn on(self, settings: &AppSettings) -> bool {
        match self {
            Self::VoiceFilter => settings.experimental.hex_microphone,
            Self::Playground => settings.labs.playground,
            Self::LiveText => settings.labs.live_text,
            Self::TerminalPaste => settings.paste_with_shift,
            Self::MouseShortcuts => settings.labs.mouse_shortcuts,
            Self::PauseBrowser => settings.pause_browser_media,
            Self::ShareShortcut => settings.share_dictation_shortcut,
            Self::RestoreShortcutDefault => settings.labs.restore_shortcut_default,
            Self::KeepOnClipboard => settings.labs.keep_on_clipboard,
            Self::CopyOnly => settings.labs.copy_only,
            Self::SpokenFormatting => settings.labs.spoken_formatting,
            Self::FillerWords => settings.labs.filler_words,
            Self::SecondDictationShortcut => settings.labs.second_dictation_shortcut,
            Self::SmartSpacing => settings.labs.smart_spacing,
            Self::SpokenSymbols => settings.labs.spoken_symbols,
            Self::SidedChords => settings.labs.sided_modifier_chords,
            Self::TapOrHold => settings.labs.tap_or_hold,
            Self::StandaloneSpecialKeys => settings.labs.standalone_special_keys,
            Self::HistoryStats => settings.labs.history_stats,
            Self::RecentDictations => settings.labs.recent_dictations,
            Self::OutputFixes => settings.labs.output_fixes,
            Self::SectionKeyAlone => settings.labs.section_key_alone,
            Self::HideMenuBarIcon => settings.labs.hide_menu_bar_icon,
            Self::GnomeClipboardRestore => settings.labs.gnome_clipboard_restore,
        }
    }

    /// Every feature's switch in `settings`, for the Labs lock
    /// (`crate::policy`).
    pub fn switches(settings: &mut AppSettings) -> Vec<&mut bool> {
        let labs = &mut settings.labs;
        vec![
            &mut settings.experimental.hex_microphone,
            &mut settings.paste_with_shift,
            &mut settings.pause_browser_media,
            &mut settings.share_dictation_shortcut,
            &mut labs.playground,
            &mut labs.live_text,
            &mut labs.mouse_shortcuts,
            &mut labs.restore_shortcut_default,
            &mut labs.keep_on_clipboard,
            &mut labs.copy_only,
            &mut labs.spoken_formatting,
            &mut labs.filler_words,
            &mut labs.second_dictation_shortcut,
            &mut labs.smart_spacing,
            &mut labs.spoken_symbols,
            &mut labs.sided_modifier_chords,
            &mut labs.tap_or_hold,
            &mut labs.standalone_special_keys,
            &mut labs.history_stats,
            &mut labs.recent_dictations,
            &mut labs.output_fixes,
            &mut labs.section_key_alone,
            &mut labs.hide_menu_bar_icon,
            &mut labs.gnome_clipboard_restore,
        ]
    }

    /// The managed lock that holds this feature off, if any.
    pub fn managed(self) -> Option<crate::policy::Lock> {
        crate::policy::Policy::current().labs_lock()
    }

    pub fn set(self, settings: &mut AppSettings, on: bool) {
        match self {
            Self::VoiceFilter => settings.experimental.hex_microphone = on,
            Self::Playground => settings.labs.playground = on,
            Self::LiveText => settings.labs.live_text = on,
            Self::TerminalPaste => settings.paste_with_shift = on,
            Self::MouseShortcuts => settings.labs.mouse_shortcuts = on,
            Self::PauseBrowser => settings.pause_browser_media = on,
            Self::ShareShortcut => settings.share_dictation_shortcut = on,
            Self::RestoreShortcutDefault => settings.labs.restore_shortcut_default = on,
            Self::KeepOnClipboard => settings.labs.keep_on_clipboard = on,
            Self::CopyOnly => settings.labs.copy_only = on,
            Self::SpokenFormatting => settings.labs.spoken_formatting = on,
            Self::FillerWords => settings.labs.filler_words = on,
            Self::SecondDictationShortcut => settings.labs.second_dictation_shortcut = on,
            Self::SmartSpacing => settings.labs.smart_spacing = on,
            Self::SpokenSymbols => settings.labs.spoken_symbols = on,
            Self::SidedChords => settings.labs.sided_modifier_chords = on,
            Self::TapOrHold => settings.labs.tap_or_hold = on,
            Self::HistoryStats => settings.labs.history_stats = on,
            Self::RecentDictations => settings.labs.recent_dictations = on,
            Self::OutputFixes => settings.labs.output_fixes = on,
            Self::HideMenuBarIcon => settings.labs.hide_menu_bar_icon = on,
            Self::GnomeClipboardRestore => settings.labs.gnome_clipboard_restore = on,
            // Off, no shortcut keeps a key only the feature allows.
            Self::StandaloneSpecialKeys => {
                settings.labs.standalone_special_keys = on;
                settings.repair_special_keys();
            }
            Self::SectionKeyAlone => {
                settings.labs.section_key_alone = on;
                settings.repair_section_key();
            }
        }
    }
}

/// Set once this process knows its settings.
const KNOWN: u64 = 1;
const LIVE_TEXT: u64 = 1 << 1;
const PLAYGROUND: u64 = 1 << 2;
const MOUSE_SHORTCUTS: u64 = 1 << 4;
const KEEP_ON_CLIPBOARD: u64 = 1 << 6;
const COPY_ONLY: u64 = 1 << 7;
const SPOKEN_FORMATTING: u64 = 1 << 8;
const FILLER_WORDS: u64 = 1 << 9;
const SMART_SPACING: u64 = 1 << 10;
const SPOKEN_SYMBOLS: u64 = 1 << 11;
const SIDED_CHORDS: u64 = 1 << 12;
const TAP_OR_HOLD: u64 = 1 << 13;
const HISTORY_STATS: u64 = 1 << 16;
const RECENT_DICTATIONS: u64 = 1 << 17;
const HIDE_MENU_BAR_ICON: u64 = 1 << 19;
const GNOME_CLIPBOARD_RESTORE: u64 = 1 << 20;

/// This process's `labs` switches: unknown until settings load or one is
/// first read.
static STATE: AtomicU64 = AtomicU64::new(0);

fn on(bit: u64) -> bool {
    state() & bit != 0
}

fn state() -> u64 {
    match STATE.load(Ordering::Acquire) {
        0 => {
            let saved = encode(&AppSettings::saved_labs());
            let _ = STATE.compare_exchange(0, saved, Ordering::AcqRel, Ordering::Acquire);
            STATE.load(Ordering::Acquire)
        }
        state => state,
    }
}

/// Live text: provisional words beside the HUD (read as each dictation starts).
pub fn live_text() -> bool {
    on(LIVE_TEXT)
}

/// The playground in the window.
pub fn playground() -> bool {
    on(PLAYGROUND)
}

/// Mouse buttons may be recorded as shortcuts (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mouse_shortcuts() -> bool {
    on(MOUSE_SHORTCUTS)
}

/// A pasted dictation stays on the clipboard rather than the previous
/// clipboard being restored (macOS; read as each paste commits).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn keep_on_clipboard() -> bool {
    on(KEEP_ON_CLIPBOARD)
}

/// A dictation goes to the clipboard and is never pasted (macOS; read as
/// each dictation's output commits).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn copy_only() -> bool {
    on(COPY_ONLY)
}

/// Spoken formatting phrases become line breaks and quotation marks (read
/// as each dictation's processing starts).
pub fn spoken_formatting() -> bool {
    on(SPOKEN_FORMATTING)
}

/// Spoken punctuation and symbol words become characters joined to their
/// neighbours (read as each dictation's processing starts).
pub fn spoken_symbols() -> bool {
    on(SPOKEN_SYMBOLS)
}

/// Filler words such as "um" and "uh" leave dictations (read as each
/// dictation's processing starts).
pub fn filler_words() -> bool {
    on(FILLER_WORDS)
}

/// A pasted dictation gets one leading space when the character before the
/// cursor asks for one (macOS; read as each paste commits).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn smart_spacing() -> bool {
    on(SMART_SPACING)
}

/// The shortcut recorder keeps each modifier's side in a modifier-only
/// chord of two or more keys (macOS; read as each recording starts).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sided_modifier_chords() -> bool {
    on(SIDED_CHORDS)
}

/// In Toggle mode, a press held past the hold threshold finishes its capture
/// when it lifts instead of locking it (macOS reads it at each release;
/// Linux's listener reads the saved switch with its shortcut options).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn tap_or_hold() -> bool {
    on(TAP_OR_HOLD)
}

/// History's summary of retained dictations and `hex history stats` (read
/// when the History pane reloads and when the command starts).
pub fn history_stats() -> bool {
    on(HISTORY_STATS)
}

/// The menu bar's Copy Recent Dictation submenu (macOS; read each time the
/// menu opens).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn recent_dictations() -> bool {
    on(RECENT_DICTATIONS)
}

/// The menu-bar icon is hidden while nothing records (macOS; read each
/// time the status item refreshes, about twice a second).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn hide_menu_bar_icon() -> bool {
    on(HIDE_MENU_BAR_ICON) && Feature::HideMenuBarIcon.on_this_os()
}

/// A paste on a Wayland desktop without data-control (GNOME) snapshots and
/// restores the clipboard through an already running XWayland (Linux; read
/// as each paste starts).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn gnome_clipboard_restore() -> bool {
    on(GNOME_CLIPBOARD_RESTORE) && Feature::GnomeClipboardRestore.on_this_os()
}

/// Records the switches the loaded or saved settings hold.
pub(crate) fn apply(settings: &LabsSettings) {
    STATE.store(encode(settings), Ordering::Release);
}

fn encode(settings: &LabsSettings) -> u64 {
    let policy = crate::policy::Policy::current();
    encode_where(settings, |feature| {
        feature.shipped() && policy.labs_lock().is_none()
    })
}

/// The switches `settings` hold for the features `reaches` admits; the
/// rest read off while their saved setting stays as it is.
fn encode_where(settings: &LabsSettings, reaches: impl Fn(Feature) -> bool) -> u64 {
    [
        (Feature::LiveText, settings.live_text, LIVE_TEXT),
        (Feature::Playground, settings.playground, PLAYGROUND),
        (
            Feature::MouseShortcuts,
            settings.mouse_shortcuts,
            MOUSE_SHORTCUTS,
        ),
        (
            Feature::KeepOnClipboard,
            settings.keep_on_clipboard,
            KEEP_ON_CLIPBOARD,
        ),
        (Feature::CopyOnly, settings.copy_only, COPY_ONLY),
        (
            Feature::SpokenFormatting,
            settings.spoken_formatting,
            SPOKEN_FORMATTING,
        ),
        (Feature::FillerWords, settings.filler_words, FILLER_WORDS),
        (Feature::SmartSpacing, settings.smart_spacing, SMART_SPACING),
        (
            Feature::SpokenSymbols,
            settings.spoken_symbols,
            SPOKEN_SYMBOLS,
        ),
        (
            Feature::SidedChords,
            settings.sided_modifier_chords,
            SIDED_CHORDS,
        ),
        (Feature::TapOrHold, settings.tap_or_hold, TAP_OR_HOLD),
        (Feature::HistoryStats, settings.history_stats, HISTORY_STATS),
        (
            Feature::RecentDictations,
            settings.recent_dictations,
            RECENT_DICTATIONS,
        ),
        (
            Feature::HideMenuBarIcon,
            settings.hide_menu_bar_icon,
            HIDE_MENU_BAR_ICON,
        ),
        (
            Feature::GnomeClipboardRestore,
            settings.gnome_clipboard_restore,
            GNOME_CLIPBOARD_RESTORE,
        ),
    ]
    .into_iter()
    .filter(|(feature, on, _)| *on && reaches(*feature))
    .fold(KNOWN, |state, (_, _, bit)| state | bit)
}

/// The names of the features on, for `hex status`.
pub fn active_names(settings: &AppSettings) -> Vec<String> {
    Feature::available()
        .filter(|feature| feature.on(settings))
        .map(|feature| feature.name().into())
        .collect()
}

/// Logs the switches to process.log; call once logging is up in a long-lived Hex.
pub fn log_launch() {
    tracing::info!(live_text = live_text(), playground = playground(), "labs");
}

#[derive(Debug, Subcommand)]
pub enum LabsCommand {
    /// List every Labs feature and whether it is on (the default).
    List,
    /// Turn a Labs feature on.
    On { feature: String },
    /// Turn a Labs feature off.
    Off { feature: String },
    /// Print the link that opens Hex's Labs page at a feature's row (macOS).
    #[cfg(target_os = "macos")]
    Link { feature: String },
}

/// `hex labs`: edits `settings.json`; a running Hex applies most changes
/// when it next starts.
pub fn run(command: Option<&LabsCommand>) -> Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(LabsCommand::Link { feature }) = command {
        println!(
            "{}",
            crate::rune_ui::deep_link::labs_link(
                crate::rune_ui::story_link::scheme(),
                resolve(feature, Feature::shipped)?
            )
        );
        return Ok(());
    }
    // Listing, or handing the change to a running Hex, never writes the file.
    let settings = AppSettings::read()?;
    let (name, on) = match command {
        None | Some(LabsCommand::List) => {
            print!("{}", list(&settings));
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        Some(LabsCommand::Link { .. }) => unreachable!("handled above"),
        Some(LabsCommand::On { feature }) => (feature, true),
        Some(LabsCommand::Off { feature }) => (feature, false),
    };
    if let Some((_, note)) = GRADUATED.iter().find(|(graduated, _)| *graduated == name) {
        println!("{note}");
        return Ok(());
    }
    let feature = resolve(name, Feature::shipped)?;
    if let Some(lock) = feature.managed() {
        return Err(eyre!(lock.refusal()));
    }
    if feature == Feature::VoiceFilter
        && !on
        && crate::audio::hex_microphone::driver_bundle_installed()
    {
        return Err(eyre!(
            "voice-filter stays on while the Hex Microphone driver is installed; uninstall the driver in its pane to turn it off"
        ));
    }
    let state = if on { "on" } else { "off" };
    // A running Hex saves and applies it at once, as its Labs row would.
    if let Some(reply) = crate::control::cli::set_in_running_hex(
        feature.settings_key(),
        if on { "true" } else { "false" },
    ) {
        return match reply {
            crate::control::ControlReply::Error { message, .. } => Err(eyre!("{message}")),
            _ => {
                println!("Turned {} {state}.", feature.name());
                Ok(())
            }
        };
    }
    let mut settings = AppSettings::load()?;
    if feature.on(&settings) == on {
        println!("{} is already {state}.", feature.name());
        return Ok(());
    }
    feature.set(&mut settings, on);
    settings.save()?;
    println!("Turned {} {state}.", feature.name());
    Ok(())
}

/// The feature `name` names, if this OS has it and `reaches` admits it.
/// Features that left Labs: `hex labs` still knows their names.
const GRADUATED: [(&str, &str); 1] = [(
    "custom-models",
    "custom-models graduated: adding a model file and models found on this computer are always on now",
)];

fn resolve(name: &str, reaches: impl Fn(Feature) -> bool + Copy) -> Result<Feature> {
    if let Some((_, note)) = GRADUATED.iter().find(|(graduated, _)| *graduated == name) {
        return Err(eyre!("{note}"));
    }
    Feature::from_name(name)
        .filter(|feature| feature.on_this_os() && reaches(*feature))
        .ok_or_else(|| {
            eyre!(
                "unknown Labs feature {name:?}; this Hex has: {}",
                Feature::available_where(reaches)
                    .map(Feature::name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn list(settings: &AppSettings) -> String {
    let width = Feature::available()
        .map(|feature| feature.name().len())
        .max()
        .unwrap_or_default();
    let mut out = String::new();
    for feature in Feature::available() {
        let on = feature.on(settings)
            || (feature == Feature::VoiceFilter
                && crate::audio::hex_microphone::driver_bundle_installed());
        out.push_str(&format!(
            "{:<width$}  {:<7}  {:<8}  {} (hex settings {})\n",
            feature.name(),
            if feature.managed().is_some() {
                "managed"
            } else if on {
                "on"
            } else {
                "off"
            },
            feature.audience().name(),
            feature.description(),
            feature.settings_key(),
        ));
    }
    out
}

/// `hex experiments …`, from before Labs.
pub fn run_retired_experiments() -> Result<()> {
    println!("Experiments are now Labs: hex labs, hex labs on|off <feature>");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graduated_features_answer_by_name() {
        let error = resolve("custom-models", |_| true).unwrap_err().to_string();
        assert!(error.contains("graduated"), "{error}");
        assert_eq!(Feature::from_name("custom-models"), None);
    }

    #[test]
    fn every_feature_is_off_by_default_and_round_trips() {
        let mut settings = AppSettings::default();
        for feature in Feature::ALL {
            assert!(
                !feature.on(&settings),
                "{} is on by default",
                feature.name()
            );
            assert_eq!(Feature::from_name(feature.name()), Some(feature));
            feature.set(&mut settings, true);
            assert!(feature.on(&settings));
        }
    }

    #[test]
    fn switches_hold_every_feature() {
        let mut settings = AppSettings::default();
        for switch in Feature::switches(&mut settings) {
            *switch = true;
        }
        for feature in Feature::ALL {
            assert!(feature.on(&settings), "{} has no switch", feature.name());
        }
        assert_eq!(
            Feature::switches(&mut settings).len(),
            Feature::ALL.len(),
            "one switch per feature"
        );
    }

    #[test]
    fn an_audience_reaches_the_builds_it_names() {
        assert!(Audience::Everyone.reaches(false));
        assert!(Audience::Everyone.reaches(true));
        assert!(Audience::Dev.reaches(true));
        assert!(!Audience::Dev.reaches(false));
    }

    #[test]
    fn settings_keys_name_their_feature() {
        for feature in Feature::ALL {
            assert_eq!(
                Feature::for_settings_key(feature.settings_key()),
                Some(feature)
            );
        }
        assert_eq!(Feature::for_settings_key("dictation_hotkey"), None);
    }

    #[test]
    fn spoken_formatting_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode(&labs) & SPOKEN_FORMATTING, 0);
        labs.spoken_formatting = true;
        assert_ne!(encode(&labs) & SPOKEN_FORMATTING, 0);
    }

    #[test]
    fn filler_words_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode(&labs) & FILLER_WORDS, 0);
        labs.filler_words = true;
        let state = encode(&labs);
        assert_ne!(state & FILLER_WORDS, 0);
        assert_eq!(state & SPOKEN_FORMATTING, 0);
    }

    #[test]
    fn smart_spacing_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode(&labs) & SMART_SPACING, 0);
        labs.smart_spacing = true;
        let state = encode(&labs);
        assert_ne!(state & SMART_SPACING, 0);
        assert_eq!(state & FILLER_WORDS, 0);
    }

    #[test]
    fn spoken_symbols_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode(&labs) & SPOKEN_SYMBOLS, 0);
        labs.spoken_symbols = true;
        let state = encode(&labs);
        assert_ne!(state & SPOKEN_SYMBOLS, 0);
        assert_eq!(state & SPOKEN_FORMATTING, 0);
    }

    #[test]
    fn sided_modifier_chords_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & SIDED_CHORDS, 0);
        labs.sided_modifier_chords = true;
        assert_ne!(encode_where(&labs, |_| true) & SIDED_CHORDS, 0);
    }

    #[test]
    fn tap_or_hold_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & TAP_OR_HOLD, 0);
        labs.tap_or_hold = true;
        let state = encode_where(&labs, |_| true);
        assert_ne!(state & TAP_OR_HOLD, 0);
        assert_eq!(state & SIDED_CHORDS, 0);
    }

    #[test]
    fn history_stats_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & HISTORY_STATS, 0);
        labs.history_stats = true;
        assert_ne!(encode_where(&labs, |_| true) & HISTORY_STATS, 0);
    }

    #[test]
    fn recent_dictations_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & RECENT_DICTATIONS, 0);
        labs.recent_dictations = true;
        let state = encode_where(&labs, |_| true);
        assert_ne!(state & RECENT_DICTATIONS, 0);
        assert_eq!(state & HISTORY_STATS, 0);
    }

    #[test]
    fn hide_menu_bar_icon_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & HIDE_MENU_BAR_ICON, 0);
        labs.hide_menu_bar_icon = true;
        assert_ne!(encode_where(&labs, |_| true) & HIDE_MENU_BAR_ICON, 0);
    }

    #[test]
    fn gnome_clipboard_restore_is_its_own_bit_and_off_by_default() {
        let mut labs = LabsSettings::default();
        assert_eq!(encode_where(&labs, |_| true) & GNOME_CLIPBOARD_RESTORE, 0);
        labs.gnome_clipboard_restore = true;
        let state = encode_where(&labs, |_| true);
        assert_ne!(state & GNOME_CLIPBOARD_RESTORE, 0);
        assert_eq!(state & HIDE_MENU_BAR_ICON, 0);
    }

    #[test]
    fn the_old_master_switch_is_read_and_ignored() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"labs": {"enabled": true, "live_text": true}}"#).unwrap();
        assert!(settings.labs.live_text);
        assert!(!settings.labs.playground);
        let json = serde_json::to_value(&settings).unwrap();
        assert!(json["labs"].get("enabled").is_none());
    }
}
