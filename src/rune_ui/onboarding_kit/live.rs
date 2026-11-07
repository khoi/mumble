//! Mumble's first run: the onboarding story, the one onboarding on every OS.
//! A new install (or an upgrade whose setup isn't ready) opens it at
//! launch, and Run setup again and `mumble onboarding reset` replay it. The
//! window's tour ([`onboarding`](super::super::onboarding)) is the state
//! machine underneath: it asks for the permissions, relaunches when macOS
//! needs it, watches the practice dictation, and records completion. This
//! module shows the story's pages over the window and feeds them that real
//! state each frame.
//!
//! Transcription is cloud-only (Claude or Codex through their own sign-ins),
//! so there is no model to download: the first run is the demo, the
//! permissions, and the practice.

use crate::desktop::i18n::tr;
use std::sync::OnceLock;
use std::time::Instant;

use rune::{El, TextEdit, Transition, Tx, text};

use super::super::controls::Paint;
use super::super::paste_access::{self, PasteAccess};
use super::super::platform::Os;
use super::super::{Ui, onboarding};
use super::stories::{FRAME, Flow, Onboard, Step, advance_to};
use crate::desktop::onboarding::{
    Input, ItemStatus, Practice, SetupItem, SetupStatus, Step as TourStep,
};

/// How long both permission checks show before the page moves on.
const GRANTED_BEAT: f64 = 0.7;
/// How often the first run looks at the permissions.
const CHECK_EVERY: f64 = 0.25;

/// What the real first run reports to the story's pages.
pub(crate) struct Live {
    /// Mumble's own dictation records.
    pub(crate) listening: bool,
    pub(crate) microphone: ItemStatus,
    pub(crate) accessibility: ItemStatus,
    /// General's "Turn off the mic when not dictating" was on when the
    /// first run opened, so the microphone opens only while you dictate.
    pub(crate) release_microphone: bool,
    /// Every permission is on and one applies only after Mumble restarts.
    pub(crate) restart: bool,
    /// macOS asks for Microphone and Accessibility; Windows only shows the
    /// Microphone row while its privacy switch is off.
    pub(crate) accessibility_row: bool,
    /// macOS 27 calls Accessibility "Device Control and Data Access".
    pub(crate) accessibility_renamed: bool,
    /// The shortcut listener failed after setup and Mumble won't relaunch by
    /// itself again: Reopen Mumble.
    pub(crate) reopen: bool,
    /// macOS needed a restart and Mumble couldn't relaunch itself.
    pub(crate) relaunch_failed: bool,
    /// The desktop's paste permission on GNOME and KDE Wayland: the
    /// permissions page explains it and asks before the practice pastes.
    pub(crate) paste: Option<PasteAccess>,
    pub(crate) practice: Practice,
    granted_at: Option<f64>,
    checked: Option<Checked>,
    /// Dictation streams send their live words to the practice field.
    _live_words: crate::dictation::live_text::Demand,
}

/// The last look at the permissions.
#[derive(Clone, Copy)]
struct Checked {
    at: f64,
    status: SetupStatus,
}

impl Live {
    pub(crate) fn granted_for(&self, now: f64) -> bool {
        self.granted_at.is_some_and(|at| now - at >= GRANTED_BEAT)
    }
}

/// Where a fixture's clock starts.
const FIXTURE_START: f64 = 100.0;

/// Seconds on the first run's own clock. Fixtures keep the story's own
/// time, so a scene shows one settled moment.
fn clock(ui: &Ui) -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    if ui.is_fixture() {
        return ui.first_run.as_ref().map_or(FIXTURE_START, |s| s.now);
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f64() + 1.0
}

/// Every permission is ready, the desktop's paste permission included
/// (or passed over), so the story needs no permissions page.
fn granted(status: SetupStatus, paste: Option<PasteAccess>) -> bool {
    use crate::desktop::onboarding::PermissionState::Ready;
    status.microphone == Ready
        && status.accessibility == Ready
        && paste.is_none_or(PasteAccess::settled)
}

/// Opens the first run over the tour, at the tour's step: at launch when
/// this run needs onboarding, and from Run setup again.
pub(in crate::rune_ui) fn open(ui: &mut Ui) {
    let Some(tour_step) = ui.tour.as_ref().map(|tour| tour.flow.step()) else {
        return;
    };
    if ui.first_run.is_some() {
        return;
    }
    let status = ui.platform.setup_status();
    let granted = granted(status, ui.platform.paste);

    let mut s = Onboard::new(Flow::FirstRun);
    s.caps = ui.settings.dictation_hotkey.keycaps();
    s.granted = granted;
    s.home = match ui.platform.caps.os {
        Os::Macos => super::landing::Home::MenuBar,
        Os::Windows => super::landing::Home::Tray,
        Os::Linux => super::landing::Home::Launcher,
    };
    s.live = Some(Live {
        listening: false,
        microphone: ItemStatus::Todo,
        accessibility: ItemStatus::Todo,
        release_microphone: ui.settings.release_microphone_while_idle,
        restart: false,
        accessibility_row: ui.platform.caps.permissions,
        accessibility_renamed: ui.platform.accessibility_renamed,
        reopen: false,
        relaunch_failed: false,
        paste: ui.platform.paste,
        practice: Practice::Waiting,
        granted_at: None,
        checked: None,
        _live_words: crate::dictation::live_text::demand(),
    });
    let now = clock(ui);
    s.now = now;
    match tour_step {
        TourStep::Welcome => s.begin_live(now),
        TourStep::Setup => s.jump_to_permissions(),
        TourStep::TryIt => s.jump_to_try(),
        TourStep::Done => s.jump_to_ready(),
    }
    ui.first_run = Some(s);
    // The tour waits at setup, where it asks for permissions, while the
    // story plays; with everything granted it goes on to the practice.
    if tour_step == TourStep::Welcome {
        onboarding::input(ui, Input::Continue);
    }
}

/// A page of the first run, for scenes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::rune_ui) enum Page {
    /// The demo, finished: `"Twice as fast"` and Continue.
    Demo,
    Permissions,
    /// `"Your turn"`: the practice game.
    Try,
    Ready,
}

/// The page the first run shows, for the window tests (Linux only).
#[cfg(all(test, target_os = "linux"))]
pub(in crate::rune_ui) fn page(ui: &Ui) -> Option<Page> {
    Some(match ui.first_run.as_ref()?.step {
        Step::Parked | Step::YourTurn | Step::Hearing | Step::Heard | Step::Done => Page::Demo,
        Step::Permissions => Page::Permissions,
        Step::Try | Step::TryHearing | Step::TryHeard => Page::Try,
        Step::Ready => Page::Ready,
    })
}

/// Scenes: the first run over the tour a fixture walked, played on its own
/// clock to `page` as a person would see it, then held there.
pub(in crate::rune_ui) fn fixture(ui: &mut Ui, page: Page) {
    ui.first_run = None;
    open(ui);
    if let Some(s) = ui.first_run.as_mut() {
        // From the start, as a new install plays it, whatever the tour's step.
        s.begin_live(FIXTURE_START);
    }
    fixture_resume(ui, page);
}

/// Scenes: plays a fixture's first run on from where it is to `page`.
pub(in crate::rune_ui) fn fixture_resume(ui: &mut Ui, page: Page) {
    // One poll fills in what the tour reports.
    let mut tx = Tx::default();
    poll(ui, &mut tx);
    let Some(s) = ui.first_run.as_mut() else {
        return;
    };
    let reached = |s: &Onboard| match page {
        Page::Demo => s.step == Step::Done,
        Page::Permissions => s.step == Step::Permissions,
        Page::Try => s.step == Step::Try,
        Page::Ready => s.step == Step::Ready,
    };
    if page == Page::Ready {
        s.jump_to_ready();
    }
    for _ in 0..64 {
        if reached(s) {
            break;
        }
        let Some(next) = super::stories::first_run_next(s, s.now) else {
            break;
        };
        s.now += next;
    }
    if page == Page::Try {
        super::game::step(s, s.now);
    }
    // Everything that entered has settled; nothing more is due.
    s.now += 10.0;
    s.auto_due_at = None;
    poll(ui, &mut tx);
}

/// "Run setup again" and `mumble onboarding reset`: forgets that onboarding
/// finished and opens it now. Settings, history, and permissions
/// stay as they are, so the run finds them ready and passes their steps.
pub(crate) fn restart(ui: &mut Ui, tx: &mut Tx) -> Result<(), String> {
    if ui.live() {
        crate::desktop::onboarding::reset().map_err(|error| format!("{error:#}"))?;
    }
    ui.select_pane(super::super::Pane::General, tx);
    ui.first_run = None;
    ui.tour = Some(onboarding::Tour::replay(&ui.settings, &ui.platform));
    open(ui);
    tx.settle();
    if ui.first_run.is_none() {
        return Err("Mumble could not open onboarding".into());
    }
    Ok(())
}

/// Feeds the first run fresh state and moves it on; true while it shows.
pub(in crate::rune_ui) fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    if ui.first_run.is_none() {
        return false;
    }
    let now = clock(ui);
    let view = ui.tour.as_ref().map(|tour| tour.flow.view());
    let caps = ui.settings.dictation_hotkey.keycaps();
    // The disk and macOS checks run a few times a second, not every frame.
    let checked = ui
        .first_run
        .as_ref()
        .and_then(|s| s.live.as_ref())
        .and_then(|live| live.checked)
        // A fixture's clock stands still, so it looks every time.
        .filter(|checked| !ui.is_fixture() && now - checked.at < CHECK_EVERY);
    let checked = match checked {
        Some(checked) => checked,
        None => Checked {
            at: now,
            status: ui.platform.setup_status(),
        },
    };
    let status = checked.status;
    // Setup that is already ready never moves the tour past it by itself,
    // and the tour watches the practice dictation only once it has.
    if status.ready()
        && ui
            .tour
            .as_ref()
            .is_some_and(|tour| tour.flow.step() == TourStep::Setup)
    {
        onboarding::input(ui, Input::Continue);
    }
    let asks_permissions = ui.platform.caps.permissions;
    let accessibility_renamed = ui.platform.accessibility_renamed;
    let relaunch_failed = ui.platform.relaunch_failed;
    let paste = ui.platform.paste;
    let live_window = !ui.is_fixture();
    let Some(s) = ui.first_run.as_mut() else {
        return false;
    };
    let Some(live) = s.live.as_mut() else {
        return false;
    };
    live.checked = Some(checked);
    // Linux and Windows ask for no permissions: both rows are ready.
    if !asks_permissions {
        live.microphone = ItemStatus::Ready;
        live.accessibility = ItemStatus::Ready;
    }

    live.paste = paste;

    // Permissions, as the tour's setup rows show them.
    if let Some(view) = &view {
        for row in &view.rows {
            match row.item {
                SetupItem::Microphone => live.microphone = row.status,
                SetupItem::Accessibility => live.accessibility = row.status,
            }
        }
        live.restart = view.restart;
        live.accessibility_renamed = accessibility_renamed;
        live.relaunch_failed = relaunch_failed;
        live.reopen = view.reopen;
        live.practice = view.practice;
    }

    let granted = granted(status, paste);
    if granted {
        live.granted_at.get_or_insert(now);
    } else {
        live.granted_at = None;
    }
    live.listening = live.practice == Practice::Listening;
    s.granted = granted;
    s.caps = caps;

    // Mumble's state arrives between frames, so the first run keeps polling;
    // a fixture shows one moment and stays still.
    advance_to(s, now, tx);
    if live_window {
        tx.wake_after(FRAME);
    }
    true
}

/// The first run over the whole window, while it shows.
pub(in crate::rune_ui) fn overlay(ui: &Ui, p: Paint, reduced: bool) -> Option<El> {
    let s = ui.first_run.as_ref()?;
    s.reduced.set(reduced);
    Some(
        super::flows::view(p, s)
            .key("first-run")
            .role("dialog")
            .label(tr("Setting up Mumble"))
            .absolute(0.0, 0.0)
            .on_press(|_: &mut Ui| {}),
    )
}

/// A story handler run on the window's first run.
pub(super) fn on_ui(f: fn(&mut Onboard)) -> impl Fn(&mut Ui) + 'static {
    move |ui: &mut Ui| {
        if let Some(s) = ui.first_run.as_mut() {
            f(s);
        }
    }
}

pub(super) fn press_continue(ui: &mut Ui) {
    if let Some(s) = ui.first_run.as_mut() {
        s.move_on();
    }
}

/// Skip on the demo or the practice: passes over that part to the next
/// step setup still needs. It never records completion; only Mumble is ready
/// does.
pub(in crate::rune_ui) fn skip(ui: &mut Ui) {
    if let Some(s) = ui.first_run.as_mut() {
        s.skip_optional();
    }
}

/// "Start using Mumble": closes the first run and the tour under it.
pub(super) fn finish(ui: &mut Ui) {
    if ui.live()
        && ui.platform.setup_status().ready()
        && !crate::desktop::onboarding::completion_recorded()
        && let Err(error) = crate::desktop::onboarding::record_completion()
    {
        tracing::warn!(%error, "could not record onboarding completion");
    }
    ui.first_run = None;
    ui.tour = None;
}

/// "Reopen Mumble" once the listener failed and Mumble won't relaunch itself.
pub(super) fn reopen(ui: &mut Ui) {
    onboarding::input(ui, Input::Reopen);
}

/// Allow on a permission row.
pub(super) fn allow(item: SetupItem) -> impl Fn(&mut Ui) + 'static {
    move |ui: &mut Ui| onboarding::input(ui, Input::Grant(item))
}

/// Allow on the paste permission: the desktop's dialog opens.
pub(super) fn allow_paste(ui: &mut Ui) {
    paste_access::ask(ui);
}

/// Continue after the desktop's paste permission was refused: the desktop
/// asks again at the first paste.
pub(super) fn paste_later(ui: &mut Ui) {
    paste_access::later(ui);
    if let Some(s) = ui.first_run.as_mut() {
        s.move_on();
    }
}

/// "Restart Mumble" once macOS applies a grant only after a restart.
pub(super) fn restart_mumble(ui: &mut Ui) {
    onboarding::input(ui, Input::Restart);
}

pub(super) fn practice(ui: &mut Ui) -> &mut TextEdit {
    match ui.first_run.as_mut() {
        Some(s) => &mut s.practice,
        // The field shows only with the first run.
        None => Box::leak(Box::default()),
    }
}

pub(super) fn practice_changed(ui: &mut Ui, tx: &mut Tx) {
    if let Some(s) = ui.first_run.as_mut() {
        s.on_practice_change(tx);
    }
}

/// What the practice dictation's last try needs said.
pub(super) fn hint_copy(live: &Live) -> Option<&'static str> {
    Some(match live.practice {
        Practice::TooShort => tr("Hold the shortcut a little longer while you speak."),
        Practice::NothingHeard => tr("Mumble didn't hear any words. Try again a little louder."),
        Practice::Failed => tr("That didn't work. Try again."),
        Practice::Transcribing => tr("Typing what you said…"),
        _ => return None,
    })
}

/// [`hint_copy`] under the practice field.
pub(super) fn hint(p: Paint, live: &Live) -> Option<El> {
    let copy = hint_copy(live)?;
    Some(
        text(copy)
            .key("practice-hint")
            .font(p.k.ty.small)
            .color(p.muted())
            .transition(Transition::opacity()),
    )
}
