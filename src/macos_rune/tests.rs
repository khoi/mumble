//! Runtime launch-gating regressions; no native model loading or settings writes.

use super::*;

#[test]
fn dockless_startup_stays_quiet_only_when_setup_and_menu_bar_are_ready() {
    for setup_ready in [false, true] {
        for onboarding_completed in [false, true] {
            for status_item_available in [false, true] {
                assert_eq!(
                    should_open_app_on_launch(
                        false,
                        setup_ready,
                        onboarding_completed,
                        status_item_available,
                    ),
                    !(setup_ready && onboarding_completed && status_item_available),
                    "setup_ready={setup_ready}, onboarding_completed={onboarding_completed}, status_item_available={status_item_available}",
                );
            }
        }
    }
}

#[test]
fn dock_visible_startup_always_opens_the_app() {
    for setup_ready in [false, true] {
        for onboarding_completed in [false, true] {
            for status_item_available in [false, true] {
                assert!(should_open_app_on_launch(
                    true,
                    setup_ready,
                    onboarding_completed,
                    status_item_available,
                ));
            }
        }
    }
}
