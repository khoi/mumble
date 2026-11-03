//! Rows only some OSes show, chosen by
//! [`Capabilities`](super::platform::Capabilities): macOS's permission
//! warnings, Show Dock icon, and the Hex Microphone driver row Voice
//! Filter shows; Launch at login on macOS and Windows. Their
//! state is [`PlatformState`](super::platform::PlatformState)
//! data and their actions are [`PlatformRequest`]s.

use rune::{El, row};

use super::controls::{ButtonKind, Paint, button, section, setting_row};
use super::lists::{PermissionControl, Tone, permission_control, tone_tag};
use super::paste_access::PasteAccess;
use super::platform::PlatformRequest;
use super::{Pane, ShortcutTarget, Ui};
use crate::app_settings::{AppSettings, HotkeyBinding, ModifierSide};
use crate::audio::hex_microphone::DriverActionState;
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::onboarding::{PermissionAction, PermissionKind, PermissionWarning};

/// A permission's row key, its name as System Settings lists it, and what
/// Hex cannot do without it.
fn permission_copy(
    kind: PermissionKind,
    accessibility_renamed: bool,
) -> (&'static str, &'static str, &'static str) {
    match kind {
        PermissionKind::Microphone => (
            "permission-microphone",
            tr("Microphone"),
            tr("Hex cannot record dictation until microphone access is restored."),
        ),
        PermissionKind::Accessibility => (
            "permission-accessibility",
            accessibility_name(accessibility_renamed),
            tr("Hex cannot recognize the dictation shortcut or paste into other apps."),
        ),
        PermissionKind::SystemAudio => (
            "permission-system-audio",
            tr("Screen & System Audio Recording"),
            tr("Lets Hex hear whether a tab is playing"),
        ),
    }
}

/// What System Settings calls the Accessibility list; macOS 27 renamed it.
pub(super) fn accessibility_name(renamed: bool) -> &'static str {
    if renamed {
        tr("Device Control and Data Access")
    } else {
        tr("Accessibility")
    }
}

/// The missing permissions above everything else, while the tour is closed.
pub(super) fn permissions_section(ui: &Ui, p: Paint) -> Option<El> {
    if ui.tour.is_some() {
        return None;
    }
    if let Some(paste) = paste_permission(ui, p) {
        return Some(section(
            p,
            "permissions",
            tr("Permissions needed"),
            vec![paste],
        ));
    }
    if !ui.platform.caps.permissions {
        return None;
    }
    let warnings = crate::desktop::onboarding::permission_warnings(ui.platform.permissions);
    if warnings.is_empty() {
        return None;
    }
    let rows = warnings
        .into_iter()
        .map(|warning: PermissionWarning| {
            let (key, name, description) =
                permission_copy(warning.kind, ui.platform.accessibility_renamed);
            if warning.action == PermissionAction::Restart {
                return setting_row(
                    p,
                    key,
                    name,
                    Some(tr("Turns on when Hex restarts")),
                    button(
                        p,
                        &format!("{key}-restart"),
                        tr("Restart Hex"),
                        ButtonKind::Secondary,
                        |ui: &mut Ui, _| {
                            ui.ask(PlatformRequest::Permission(PermissionAction::Restart))
                        },
                    ),
                );
            }
            let stale =
                warning.kind == PermissionKind::Accessibility && ui.platform.stale(warning.kind);
            let description = if stale {
                tr("An older Hex entry can keep this off")
            } else {
                description
            };
            let action = if stale {
                PermissionAction::ResetAndPrompt(warning.kind)
            } else {
                warning.action
            };
            let control = permission_control(
                p,
                &format!("{key}-allow"),
                ui.platform.control_for(warning.kind, false),
                ButtonKind::Secondary,
                move |ui: &mut Ui| ui.ask(PlatformRequest::Permission(action)),
            );
            setting_row(p, key, name, Some(description), control)
        })
        .collect();
    Some(section(p, "permissions", tr("Permissions needed"), rows))
}

/// The desktop's paste permission on GNOME and KDE Wayland until it is
/// allowed, for people who skipped onboarding (#932).
fn paste_permission(ui: &Ui, p: Paint) -> Option<El> {
    let paste = ui
        .platform
        .paste
        .filter(|paste| *paste != PasteAccess::Allowed)?;
    let description = if paste == PasteAccess::Declined {
        tr("Without it, Hex copies your words and you press Ctrl+V.")
    } else {
        tr("Hex only sends the paste keystroke. It never sees your screen.")
    };
    let control = permission_control(
        p,
        "permission-paste-allow",
        if paste == PasteAccess::Asking {
            PermissionControl::Waiting
        } else {
            PermissionControl::Allow
        },
        ButtonKind::Secondary,
        super::paste_access::ask,
    );
    Some(setting_row(
        p,
        "permission-paste",
        tr("Paste into apps"),
        Some(description),
        control,
    ))
}

/// Records which untrusted permission rows are currently visible in the window
/// so the macOS host clears retired Hex bundle IDs' stale TCC rows once in the
/// background when a row appears.
pub(super) fn update_shown_permissions(ui: &mut Ui) {
    use crate::desktop::onboarding::PermissionState;
    let visible = ui.window.as_ref().is_none_or(rune::AppHandle::is_visible);
    let mut shown = Vec::new();
    if visible && ui.platform.caps.permissions {
        let setup_showing = ui.first_run.is_some()
            || (ui.tour.is_none() && !ui.labs_open && ui.pane == Pane::General);
        if setup_showing
            && matches!(
                ui.platform.permissions.accessibility,
                PermissionState::NeedsRequest | PermissionState::NeedsSettings
            )
        {
            shown.push(PermissionKind::Accessibility);
        }
    }
    ui.platform.permissions_shown = shown;
}

/// Fixture setup for General's missing-permissions scenes (`waiting` vs `stale`).
pub(super) fn mac_permissions_fixture(ui: &mut Ui, stale: bool) {
    use crate::desktop::onboarding::PermissionState;
    *ui = super::mac_fixture();
    ui.platform.permissions.microphone = PermissionState::NeedsSettings;
    ui.platform.permissions.accessibility = PermissionState::NeedsSettings;
    if stale {
        ui.platform.note_stale(PermissionKind::Accessibility);
    } else {
        ui.platform.note_asked(PermissionKind::Accessibility);
    }
    ui.platform.login_item = Some(crate::desktop::login_item::LoginItemStatus::RequiresApproval);
    ui.platform.accessibility_renamed = true;
}

/// A save error where the OS has no listener card to show it in.
pub(super) fn error_section(ui: &Ui, p: Paint) -> Option<El> {
    if ui.platform.caps.listener_control {
        return None;
    }
    let error = ui.error.as_deref()?;
    Some(section(
        p,
        "save-error",
        tr("Status"),
        super::settings::error_notes(p, error).into(),
    ))
}

/// Whether an organization turned OpenCode processing off.
pub(super) fn opencode_managed() -> bool {
    crate::policy::Policy::current().locks(crate::policy::Lock::OpenCodeProcessing)
}

/// The virtual microphone driver's row: install, update, or uninstall it.
pub(super) fn driver_row(ui: &Ui, p: Paint) -> El {
    let title = tr("Virtual microphone driver");
    let Some(status) = &ui.platform.hex_microphone else {
        return setting_row(
            p,
            "hex-mic-driver",
            title,
            Some(tr("Checking…")),
            row().key("none"),
        );
    };
    let action = |key: &str, label: &str, request: PlatformRequest| {
        button(
            p,
            key,
            label,
            ButtonKind::Secondary,
            move |ui: &mut Ui, _| ui.ask(request.clone()),
        )
    };
    let install = || {
        action(
            "install",
            tr("Install"),
            PlatformRequest::InstallHexMicrophone,
        )
    };
    let uninstall = || {
        action(
            "uninstall",
            tr("Uninstall"),
            PlatformRequest::UninstallHexMicrophone,
        )
    };
    let state_tag = |label: &str, tone: Tone| tone_tag(p, "state", label, tone);
    let controls = row().key("controls").gap(8.0).center_y();
    let (description, controls) = match &status.action_state {
        DriverActionState::Installing => (
            tr("Installing driver and restarting Core Audio…").to_string(),
            controls.child(state_tag(tr("Installing…"), Tone::Neutral)),
        ),
        DriverActionState::Uninstalling => (
            tr("Removing driver and restarting Core Audio…").to_string(),
            controls.child(state_tag(tr("Uninstalling…"), Tone::Neutral)),
        ),
        state if status.needs_update() => {
            let installed = status.installed_version.as_deref().unwrap_or("1.0.0");
            (
                match state {
                    DriverActionState::Cancelled => tr("Update cancelled").into(),
                    DriverActionState::Failed(reason) => {
                        tr_fill("Could not update driver: {}", reason)
                    }
                    _ => tr_fill("Update driver to v{}", &status.bundled_version),
                },
                controls
                    .child(state_tag(
                        &tr_fill("v{} installed", installed),
                        Tone::Accent,
                    ))
                    .child(action(
                        "update",
                        tr("Update"),
                        PlatformRequest::InstallHexMicrophone,
                    ))
                    .child(uninstall()),
            )
        }
        state if status.driver_loaded => {
            let version = status
                .installed_version
                .as_deref()
                .unwrap_or(&status.bundled_version);
            (
                match state {
                    DriverActionState::Cancelled => tr("Uninstall cancelled").into(),
                    DriverActionState::Failed(reason) => {
                        tr_fill("Could not update driver: {}", reason)
                    }
                    _ if !status.active_apps.is_empty() => tr_fill(
                        "Streaming 48 kHz audio to {}",
                        &status.active_apps.join(", "),
                    ),
                    _ => {
                        tr("Choose Hex Microphone as your input in Slack, Zoom, or Discord").into()
                    }
                },
                controls
                    .child(state_tag(&tr_fill("Installed v{}", version), Tone::Success))
                    .child(uninstall()),
            )
        }
        DriverActionState::DoneUninstalled => (
            tr("Driver removed").into(),
            controls
                .child(state_tag(tr("Removed"), Tone::Neutral))
                .child(install()),
        ),
        DriverActionState::Cancelled => (
            tr("Installation cancelled").into(),
            controls
                .child(state_tag(tr("Cancelled"), Tone::Neutral))
                .child(install()),
        ),
        DriverActionState::Failed(reason) => (
            tr_fill("Could not install driver: {}", reason),
            controls
                .child(state_tag(tr("Failed"), Tone::Danger))
                .child(install()),
        ),
        _ => (
            tr("Install the CoreAudio driver to choose Hex Microphone in Slack, Zoom, or Discord")
                .into(),
            controls.child(install()),
        ),
    };
    setting_row(p, "hex-mic-driver", title, Some(&description), controls)
}

/// `target`'s modifier-only shortcut moved to `side`, unless that collides
/// with another shortcut.
pub(super) fn side_binding(
    settings: &AppSettings,
    target: ShortcutTarget,
    side: ModifierSide,
) -> Option<HotkeyBinding> {
    let mut candidate = super::settings::binding(settings, target)?.clone();
    candidate.standalone_side()?;
    candidate.set_standalone_side(side);
    (!super::settings::conflicts(settings, target, &candidate)).then_some(candidate)
}

pub(super) const SIDES: [ModifierSide; 3] = [
    ModifierSide::Left,
    ModifierSide::Either,
    ModifierSide::Right,
];

/// The side [`side_selector`] shows for `target`, if it shows one: the
/// keycaps beside it then leave the side to it.
pub(super) fn picked_side(ui: &Ui, target: ShortcutTarget) -> Option<ModifierSide> {
    if !ui.platform.caps.modifier_sides {
        return None;
    }
    super::settings::binding(&ui.settings, target)?.standalone_side()
}
