//! The onboarding check (`scripts/check-onboarding.sh`, run before every
//! stable release): every onboarding scene against its approved golden in
//! both appearances, and a first run walked through the real onboarding in
//! a clean data folder. Like every golden, these render on Linux only: the
//! macOS and Windows scenes are the same window drawn from fixtures.
//!
//! Transcription is cloud-only (Claude or Codex), so onboarding is the demo,
//! the permissions, and the practice: there is no model step or download.

use super::super::onboarding_kit::live::Page;
use super::super::scenes::portal_fixture;
use super::*;

/// Every onboarding state a person can see, on macOS and on Linux and
/// Windows. An intentional change refreshes these goldens in the same PR
/// (`HEX_UPDATE_GOLDENS=1`).
pub(super) const SCENES: &[&str] = &[
    // A fresh install opens on the demo.
    "onboarding",
    "onboarding-demo",
    // Permissions (macOS): to do, refused, stale entry, waiting for a
    // restart; Windows' microphone privacy switch.
    "mac-onboarding-permissions",
    "mac-onboarding-denied",
    "mac-onboarding-stale",
    "mac-onboarding-restart",
    "windows-onboarding-microphone",
    // GNOME and KDE on Wayland: the desktop's paste permission, explained.
    "onboarding-paste",
    // Your turn: the practice game, and the shortcut that didn't start.
    "onboarding-try",
    "mac-onboarding-try",
    "mac-onboarding-reopen",
    // Hex is ready.
    "onboarding-ready",
    "mac-onboarding-ready",
];

/// The optional pages: the demo and the practice.
const OPTIONAL: &[&str] = &[
    "onboarding-demo",
    "onboarding-try",
    "mac-onboarding-try",
    "mac-onboarding-reopen",
];

/// Skip shows only on the optional pages, the demo and the practice, and
/// passes over them to the next step setup still needs: permissions, else
/// Hex is ready. It never closes onboarding.
#[test]
fn onboarding_skip() {
    for scene in SCENES {
        let rt = runtime(scene, true, PANE_SIZE);
        assert_eq!(
            rt.find("skip").is_some(),
            OPTIONAL.contains(scene),
            "{scene}: Skip"
        );
    }
    let page = |rt: &Runtime<Ui>| super::super::onboarding_kit::live::page(&rt.state);
    let skip = |rt: &mut Runtime<Ui>, scene: &str, next: Page| {
        tap(rt, "skip");
        settle(rt);
        assert_eq!(page(rt), Some(next), "{scene}: Skip lands here");
        assert!(rt.state.tour.is_some(), "{scene}: onboarding stays open");
        assert_eq!(
            rt.find("skip").is_some(),
            next == Page::Try,
            "{scene}: no Skip on {next:?}"
        );
    };

    // macOS, permissions still to grant: the demo skips to them.
    let mut rt = runtime("mac-onboarding-permissions", true, PANE_SIZE);
    rt.update(|ui, _| super::super::onboarding_kit::live::fixture_resume(ui, Page::Demo));
    settle(&mut rt);
    assert_eq!(page(&rt), Some(Page::Demo));
    skip(&mut rt, "mac demo", Page::Permissions);

    // Nothing left to set up: the demo and the practice skip to Hex is ready.
    let mut rt = runtime("onboarding-demo", true, PANE_SIZE);
    skip(&mut rt, "onboarding-demo", Page::Ready);
    for scene in ["onboarding-try", "mac-onboarding-try"] {
        let mut rt = runtime(scene, true, PANE_SIZE);
        skip(&mut rt, scene, Page::Ready);
    }
}

#[test]
fn onboarding_goldens() {
    let _goldens = Goldens::collect();
    for scene in SCENES {
        for dark in [true, false] {
            check_golden(scene, dark, PANE_SIZE);
        }
    }
}

/// A first run on Linux or Windows, in the clean data folder
/// `HEX_APPLICATION_SUPPORT_DIR` names (no settings): the real settings load,
/// the real decision to open onboarding, and every page to Hex is ready.
/// Transcription is cloud-only, so nothing downloads; only
/// scripts/check-onboarding.sh runs it. The window here is a fixture, and
/// nothing writes the completion marker.
#[test]
#[ignore = "walks the real onboarding; run by scripts/check-onboarding.sh"]
fn onboarding_walkthrough() {
    let support = crate::app_paths::support_dir().unwrap();
    assert!(
        std::env::var_os("HEX_APPLICATION_SUPPORT_DIR").is_some(),
        "set HEX_APPLICATION_SUPPORT_DIR to a clean folder"
    );
    assert!(
        std::fs::read_dir(&support).map_or(true, |mut entries| entries.next().is_none()),
        "{} must start empty",
        support.display()
    );

    // A new install: default settings from an empty folder.
    let settings = AppSettings::load().unwrap();
    let mut ui = portal_fixture();
    ui.platform.caps = platform::Capabilities::LINUX;
    ui.settings = settings;
    // As the window opens at launch.
    ui.tour = Some(
        super::super::onboarding::Tour::start(&ui.settings, &ui.platform)
            .expect("a new install opens onboarding"),
    );
    super::super::onboarding_kit::live::open(&mut ui);
    assert!(ui.first_run.is_some(), "the first run shows");
    let mut rt = runtime_for(ui, true, PANE_SIZE);
    let step = |rt: &Runtime<Ui>| rt.state.tour.as_ref().map(|tour| tour.flow.step());

    // The story plays on: the demo, then the practice game, then Hex is ready.
    settle(&mut rt);
    rt.update(|ui, _| super::super::onboarding_kit::live::fixture_resume(ui, Page::Try));
    settle(&mut rt);
    assert_eq!(step(&rt), Some(crate::desktop::onboarding::Step::TryIt));
    assert!(rt.find("skip").is_some(), "Skip shows on the practice too");
    tap(&mut rt, "done");
    assert!(texts(&rt).contains(&"Hex is ready".to_owned()));
    tap(&mut rt, "start-using-hex");
    assert!(rt.state.tour.is_none(), "Start using Hex closes onboarding");
    assert!(rt.state.first_run.is_none());
    println!("onboarding walkthrough: demo -> Your turn -> Hex is ready");
}
