//! Runtime launch-gating regressions; no native model loading or settings writes.

use super::*;

/// The pump ticks every frame only while the HUD animates; otherwise it
/// idles and senders wake it (#1378).
#[test]
fn the_pump_idles_unless_the_hud_animates() {
    assert!(pump_interval(true) < 0.02);
    assert!(pump_interval(false) >= 0.2);
}

/// ⌘Tab back to Mumble opens its closed window (#1298), but never for a
/// menu-bar-only Mumble, an overlay that took the keyboard, or an open window.
#[test]
fn switching_to_mumble_opens_its_closed_window() {
    assert!(reopen_on_activation(true, false, false));
    assert!(!reopen_on_activation(false, false, false));
    assert!(!reopen_on_activation(true, true, false));
    assert!(!reopen_on_activation(true, false, true));
}

/// Setup or a missing menu-bar item always opens the window; otherwise a
/// shown Dock icon opens it unless macOS started Mumble at login (#1321), and a
/// hidden one never does. `mumble app --hidden` opens it only without the
/// menu-bar item.
#[test]
fn login_item_launch_starts_without_the_window() {
    for setup_ready in [false, true] {
        for onboarding_completed in [false, true] {
            for status_item_available in [false, true] {
                for show_dock_icon in [false, true] {
                    let ready = setup_ready && onboarding_completed && status_item_available;
                    let launch = LaunchWindow::decide(
                        false,
                        show_dock_icon,
                        setup_ready,
                        onboarding_completed,
                        status_item_available,
                    );
                    assert_eq!(launch.opens(true), !ready, "{launch:?} at login");
                    assert_eq!(
                        launch.opens(false),
                        !ready || show_dock_icon,
                        "{launch:?} opened by hand"
                    );
                    let hidden = LaunchWindow::decide(
                        true,
                        show_dock_icon,
                        setup_ready,
                        onboarding_completed,
                        status_item_available,
                    );
                    assert_eq!(
                        hidden.opens(false),
                        !status_item_available,
                        "{hidden:?} hidden"
                    );
                }
            }
        }
    }
}
