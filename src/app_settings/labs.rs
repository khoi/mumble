//! The persisted `labs` switches (`crate::labs`).

use super::*;

/// The `labs` switches (`crate::labs`), each off by default; older Labs features keep their own keys, and retired switches (`labs.enabled`, `labs.collapsible_meeting_sections`, `labs.meeting_platform_icons`) are ignored.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LabsSettings {
    /// Live text beside the HUD while dictating.
    pub live_text: bool,
    pub playground: bool,
    /// Mouse buttons may be recorded as shortcuts (macOS, issue 209).
    pub mouse_shortcuts: bool,
    /// Paste last dictation's row offers its default shortcut back (issue 613).
    pub restore_shortcut_default: bool,
    /// A pasted dictation stays on the clipboard, not the previous one (macOS, issue 200).
    pub keep_on_clipboard: bool,
    /// A dictation is put on the clipboard and never pasted (macOS, issue 629).
    pub copy_only: bool,
    /// Spoken "new line", "new paragraph", and "open/close quote" become line breaks and quotes (issue 630).
    pub spoken_formatting: bool,
    /// Filler words such as "um" and "uh" are removed from dictations (issue 631).
    pub filler_words: bool,
    /// `second_dictation_hotkey` dictates like the dictation shortcut (macOS, issue 627).
    pub second_dictation_shortcut: bool,
    /// A paste gets a leading space when the character before the cursor wants one (macOS, issue 632).
    pub smart_spacing: bool,
    /// Spoken symbol words ("comma", "hashtag", "underscore") become characters joined to their words (issue 678).
    pub spoken_symbols: bool,
    /// The recorder keeps each modifier's side in a modifier-only chord, such as Right ⌘ + Right ⌥ (macOS, issue 710).
    pub sided_modifier_chords: bool,
    /// In Toggle mode, a press held past the hold threshold dictates until it lifts (macOS, issue 711).
    pub tap_or_hold: bool,
    /// Menu, Pause, and Scroll Lock may be shortcut keys (Linux, issue 767).
    pub standalone_special_keys: bool,
    pub history_stats: bool,
    pub recent_dictations: bool,
    /// Output's Try card offers Vocabulary and Fix this beside its result (issue 989).
    pub output_fixes: bool,
    /// The ISO § key may be a shortcut on its own (Linux, issue 1050).
    pub section_key_alone: bool,
    /// General's pane starts with a search field that filters its rows.
    pub settings_search: bool,
    pub hide_menu_bar_icon: bool,
    /// ⌘Q closes the Settings window instead of quitting Mumble (macOS).
    pub quit_closes_window: bool,
    /// A paste restores the clipboard through a running XWayland on Wayland desktops without data-control, such as GNOME (Linux, issue 1111).
    pub gnome_clipboard_restore: bool,
    /// With the indicator at the top, the HUD grows out of the notch on a screen with one (macOS).
    pub notch_indicator: bool,
    /// Settings › Labs' groups opened or closed by hand, by `labs::Group`
    /// id; a group not here is open while it holds a lab that's on.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub groups: std::collections::BTreeMap<String, bool>,
}
