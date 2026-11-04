//! The tray menu the Linux and Windows trays share (status, Show Mumble, Paste
//! last dictation, the update, Enable / Disable Mumble, Quit Mumble). Each tray
//! maps a picked [`TrayCommand`] onto its own runtime.

#[cfg(not(target_os = "macos"))]
pub(crate) use tray::*;

#[cfg(not(target_os = "macos"))]
mod tray {
    use crate::desktop::events::VoiceState;
    use crate::desktop::host::DesktopUpdateStatus;
    use crate::desktop::i18n::tr;

    /// What a tray menu item asks for.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum TrayCommand {
        Show,
        PasteLast,
        /// The update item: restart into it, download it, or switch channel.
        Update,
        ToggleListening,
        Quit,
    }

    impl TrayCommand {
        /// The command behind a menu item's id.
        pub(crate) fn from_id(id: &str) -> Option<Self> {
            Some(match id {
                "show" => Self::Show,
                "paste-last" => Self::PasteLast,
                "update" => Self::Update,
                "toggle" => Self::ToggleListening,
                "quit" => Self::Quit,
                _ => return None,
            })
        }
    }

    /// Everything the tray shows.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(crate) struct TrayView {
        pub(crate) listening: bool,
        pub(crate) state: Option<VoiceState>,
        pub(crate) update: Option<String>,
    }

    impl TrayView {
        pub(crate) fn new(
            listening: bool,
            state: Option<VoiceState>,
            update: &DesktopUpdateStatus,
        ) -> Self {
            Self {
                listening,
                state: state.filter(|_| listening),
                update: update_label(update),
            }
        }

        fn recording(&self) -> bool {
            self.state == Some(VoiceState::Dictating)
        }

        fn transcribing(&self) -> bool {
            self.state == Some(VoiceState::Transcribing)
        }

        pub(crate) fn tooltip(&self) -> &'static str {
            if self.recording() {
                tr("Mumble Dictation: recording")
            } else if self.transcribing() {
                tr("Mumble Dictation: transcribing")
            } else if self.update.is_some() {
                tr("Mumble Dictation: update available")
            } else {
                tr("Mumble Dictation")
            }
        }

        pub(crate) fn menu(&self) -> Vec<rune::TrayItem> {
            use rune::TrayItem;
            let mut menu = Vec::new();
            let status = if self.recording() {
                Some(tr("Recording"))
            } else if self.transcribing() {
                Some(tr("Transcribing…"))
            } else {
                None
            };
            if let Some(status) = status {
                menu.push(TrayItem::action("status", status).enabled(false));
                menu.push(TrayItem::Separator);
            }
            menu.push(TrayItem::action("show", tr("Show Mumble")));
            menu.push(
                TrayItem::action("paste-last", tr("Paste last dictation")).enabled(self.listening),
            );
            menu.push(TrayItem::Separator);
            menu.extend(
                self.update
                    .as_ref()
                    .map(|label| TrayItem::action("update", label.clone())),
            );
            menu.push(TrayItem::action("toggle", tr("Enable / Disable Mumble")));
            menu.push(TrayItem::action("quit", tr("Quit Mumble")));
            menu
        }
    }

    /// The update the window's sidebar offers: Restart to update once a
    /// managed install has staged it, Download update for an install that
    /// cannot apply it itself, or the channel switch.
    fn update_label(status: &DesktopUpdateStatus) -> Option<String> {
        match status {
            DesktopUpdateStatus::ReadyToRestart => Some(tr("Restart to update").to_owned()),
            DesktopUpdateStatus::Available => Some(tr("Download update").to_owned()),
            DesktopUpdateStatus::SwitchAvailable { channel, version } => {
                Some(crate::updater::switch_label(channel, version))
            }
            _ => None,
        }
    }

    /// What the update item does for `status`, if it shows.
    pub(crate) fn update_action(
        status: &DesktopUpdateStatus,
    ) -> Option<crate::desktop::host::DesktopAction> {
        use crate::desktop::host::DesktopAction;
        match status {
            DesktopUpdateStatus::ReadyToRestart => Some(DesktopAction::RestartIntoUpdate),
            DesktopUpdateStatus::Available => Some(DesktopAction::OpenUpdate),
            DesktopUpdateStatus::SwitchAvailable { .. } => Some(DesktopAction::ConfirmUpdateSwitch),
            _ => None,
        }
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn tray_menu_ids_round_trip_and_show_the_state() {
        use crate::desktop::events::VoiceState;
        use crate::desktop::host::DesktopUpdateStatus;
        let view = TrayView::new(
            true,
            Some(VoiceState::Dictating),
            &DesktopUpdateStatus::ReadyToRestart,
        );
        let menu = view.menu();
        let ids: Vec<&str> = menu
            .iter()
            .filter_map(|item| match item {
                rune::TrayItem::Action { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ids,
            ["status", "show", "paste-last", "update", "toggle", "quit"]
        );
        assert_eq!(
            TrayCommand::from_id("paste-last"),
            Some(TrayCommand::PasteLast)
        );
        assert_eq!(TrayCommand::from_id("status"), None);
        assert_eq!(view.tooltip(), "Mumble Dictation: recording");
        let stopped = TrayView::new(
            false,
            Some(VoiceState::Dictating),
            &DesktopUpdateStatus::Current,
        );
        assert_eq!(stopped.tooltip(), "Mumble Dictation");
    }
}
