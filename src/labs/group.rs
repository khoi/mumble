//! The area each Labs feature belongs to: Settings › Labs lists them under
//! one collapsible header per group.

use super::Feature;

/// One area of Settings › Labs. Every feature picks one (`Feature::group`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Group {
    Dictation,
    /// Paste, formatting, and spoken punctuation.
    Output,
    History,
    Shortcuts,
}

impl Group {
    /// In the order Labs lists them.
    pub const ALL: [Self; 4] = [
        Self::Dictation,
        Self::Output,
        Self::History,
        Self::Shortcuts,
    ];

    /// Its stable id: `mumble://labs?category=<id>` and `labs.groups` in
    /// settings.json.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::Output => "output",
            Self::History => "history",
            Self::Shortcuts => "shortcuts",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.id() == id)
    }
}

impl Feature {
    /// The group Labs lists it under. Every feature picks one.
    pub const fn group(self) -> Group {
        match self {
            Self::LiveText
            | Self::NotchIndicator
            | Self::PauseBrowser
            | Self::VoiceFilter
            | Self::HideMenuBarIcon
            | Self::QuitClosesWindow
            | Self::SettingsSearch
            | Self::Playground => Group::Dictation,
            Self::SmartSpacing
            | Self::SpokenFormatting
            | Self::SpokenSymbols
            | Self::FillerWords
            | Self::KeepOnClipboard
            | Self::CopyOnly
            | Self::GnomeClipboardRestore
            | Self::TerminalPaste
            | Self::OutputFixes => Group::Output,
            Self::HistoryStats | Self::RecentDictations => Group::History,
            Self::MouseShortcuts
            | Self::SecondDictationShortcut
            | Self::SidedChords
            | Self::TapOrHold
            | Self::RestoreShortcutDefault
            | Self::StandaloneSpecialKeys
            | Self::SectionKeyAlone
            | Self::ShareShortcut => Group::Shortcuts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_has_a_feature_and_round_trips_its_id() {
        for group in Group::ALL {
            assert_eq!(Group::from_id(group.id()), Some(group));
            assert!(
                Feature::ALL.iter().any(|feature| feature.group() == group),
                "{} holds no feature",
                group.id()
            );
        }
    }
}
