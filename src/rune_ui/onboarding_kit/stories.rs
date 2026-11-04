//! The onboarding options as playground stories (the `playground`
//! experiment, ⇧⌘D, group Onboarding). Each plays itself on open and on R,
//! and in a live desktop window pauses on `"Your turn"` with a focused
//! [`rune::TextEdit`] so you can hold `⌥` and test real voice dictation live.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rune::{Cx, El, Play, Playground, Story, TextEdit, TextStyle, Tx, col, el, row, text};

use super::super::controls::Paint;
use super::dictated::Dictated;
use super::fixture::{ANSWER_JAY, ANSWER_TRY, Take};
use super::hud_stage::LiveHud;
use super::live::Live;
use super::sfx::SfxTrack;
use super::{flows, permissions, typing, versus};
use crate::desktop::onboarding::Practice;
use crate::feedback::KeySoundStyle;

static LIVE_WINDOW: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_live_window(live: bool) {
    LIVE_WINDOW.store(live, Ordering::Relaxed);
}

pub(crate) fn is_live_window() -> bool {
    LIVE_WINDOW.load(Ordering::Relaxed)
}

const PERMISSIONS_WAIT: f64 = 0.6;
const TYPING_LEAD: f64 = 1.15;
// Ask for the next display frame, including 120 Hz displays. Rune paces a
// hidden or unpaced window itself rather than spinning on a zero deadline.
pub(super) const FRAME: Duration = Duration::ZERO;
const SIDE_W: f32 = 240.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    Versus,
    FirstRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Parked,
    YourTurn,
    Hearing,
    Heard,
    Done,
    Permissions,
    Try,
    TryHearing,
    TryHeard,
    Ready,
}

impl Step {
    /// The demo and the practice: the pages setup doesn't need, which Skip
    /// passes over.
    pub(crate) fn optional(self) -> bool {
        matches!(
            self,
            Self::Parked
                | Self::YourTurn
                | Self::Hearing
                | Self::Heard
                | Self::Done
                | Self::Try
                | Self::TryHearing
                | Self::TryHeard
        )
    }
}

pub(crate) struct Onboard {
    pub(crate) flow: Flow,
    pub(crate) caps: Vec<String>,
    shortcut_index: usize,
    pub(crate) step: Step,
    pub(crate) now: f64,
    pub(crate) reduced: Cell<bool>,
    pub(crate) motion: super::landing::Motion,
    pub(crate) parked_from: Option<f64>,
    pub(crate) typing_from: Option<f64>,
    pub(crate) voice_from: Option<f64>,
    /// When the resting HUD plate first showed on `"With Mumble"` or
    /// `"Your turn"`: the HUD unfolds from here and keeps running through the
    /// take, so pressing the shortcut never restarts it.
    pub(crate) hud_from: Option<f64>,
    pub(crate) take_from: Option<f64>,
    /// How many `"Without Mumble"` bubbles have flown out of the field so far.
    sent_seen: usize,
    pub(crate) partial: &'static str,
    pub(crate) take: &'static Take,
    pub(crate) meter: (f32, f32),
    pub(crate) granted: bool,
    pub(crate) scripted_try: bool,
    pub(crate) permissions_from: Option<f64>,
    ready_from: Option<f64>,
    manual_override: bool,
    pub(crate) auto_due_at: Option<f64>,
    pub(crate) practice: TextEdit,
    pub(crate) live_hold_from: Option<f64>,
    /// The `"Your turn"` plate, on the microphone's real level.
    pub(crate) hud: LiveHud,
    /// A dictation's text landed since the plate last stepped.
    pub(crate) hud_landed: bool,
    pub(crate) sfx: SfxTrack,
    /// Set when this is Mumble's real first run (Labs):
    /// what the window's tour reports, in place of the fixtures.
    pub(crate) live: Option<Live>,
    /// The dictated words flying into the practice field.
    pub(crate) dictated: Dictated,
    /// A story's stand-in for a real dictation on the interactive try.
    pub(crate) rehearsal: Option<Rehearsal>,
    /// The interactive try as the practice game (`game`).
    pub(crate) game: Option<super::game::Session>,
    /// The interactive try as the plain practice field, not the game.
    pub(crate) practice_field: bool,
    /// Where Mumble lives once onboarding closes, under `"Mumble is ready"`.
    pub(crate) home: super::landing::Home,
}

/// A story's dictation of [`ANSWER_TRY`] on the interactive `"Your turn"`,
/// through the same live words and paste as Mumble's real one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rehearsal {
    /// Whether the model sends live words while the shortcut is held.
    pub(crate) streams: bool,
    from: f64,
    pasted: bool,
}

impl Rehearsal {
    /// How long after `"Your turn"` shows the shortcut goes down.
    const LEAD: f64 = 1.0;
    /// How long the paste takes after the shortcut comes up.
    const TRANSCRIBE: f64 = 0.45;

    pub(crate) fn new(streams: bool) -> Self {
        Self {
            streams,
            from: f64::INFINITY,
            pasted: false,
        }
    }

    fn elapsed(&self, now: f64) -> f64 {
        now - self.from - Self::LEAD
    }

    fn held(&self, now: f64) -> bool {
        let t = self.elapsed(now);
        (0.0..ANSWER_TRY.release).contains(&t)
    }

    fn partial(&self, now: f64) -> Option<crate::transcription::engine::Partial> {
        (self.streams && self.held(now)).then(|| crate::transcription::engine::Partial {
            stable: ANSWER_TRY.partial(self.elapsed(now)).to_owned(),
            tentative: String::new(),
        })
    }

    fn paste_due(&mut self, now: f64) -> bool {
        let due = !self.pasted && self.elapsed(now) >= ANSWER_TRY.release + Self::TRANSCRIBE;
        self.pasted |= due;
        due
    }
}

impl Drop for Onboard {
    fn drop(&mut self) {
        self.sfx.flush_offline_if_requested(self.now.max(1.0));
    }
}

const PRESET_SHORTCUTS: &[&[&str]] = &[&["⌥"], &["fn"], &["⌃", "Space"], &["R⌘"]];

impl Onboard {
    pub(super) fn new(flow: Flow) -> Self {
        Self {
            flow,
            caps: crate::app_settings::AppSettings::default()
                .dictation_hotkey
                .keycaps(),
            shortcut_index: 0,
            step: Step::Parked,
            now: 0.0,
            reduced: Cell::new(false),
            motion: super::landing::Motion::default(),
            parked_from: None,
            typing_from: None,
            voice_from: None,
            hud_from: None,
            take_from: None,
            sent_seen: 0,
            partial: "",
            take: &ANSWER_JAY,
            meter: (0.0, 0.0),
            granted: false,
            scripted_try: false,
            permissions_from: None,
            ready_from: None,
            manual_override: false,
            auto_due_at: None,
            practice: TextEdit::default(),
            live_hold_from: None,
            hud: LiveHud::default(),
            hud_landed: false,
            sfx: SfxTrack::default(),
            live: None,
            dictated: Dictated::default(),
            rehearsal: None,
            game: None,
            practice_field: false,
            home: super::landing::Home::MenuBar,
        }
    }

    /// True when `"Your turn"` should wait for real voice dictation / typing
    /// rather than auto-playing the scripted reply.
    pub(crate) fn interactive_try(&self) -> bool {
        self.flow == Flow::FirstRun
            && (self.live.is_some() || is_live_window() || self.rehearsal.is_some())
            && !self.scripted_try
    }

    /// Cycles the shortcut capsule on `"Your turn"` so clicking it previews
    /// changing the dictation key.
    pub(crate) fn cycle_shortcut(&mut self) {
        self.shortcut_index = (self.shortcut_index + 1) % PRESET_SHORTCUTS.len();
        self.caps = PRESET_SHORTCUTS[self.shortcut_index]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        self.sfx.on_capsule_tap();
    }

    pub(crate) fn choose_key_sound(&mut self, style: KeySoundStyle) {
        self.sfx.set_style_and_preview(style, self.now);
    }

    /// Called when the interactive `"Your turn"` [`TextArea`] changes (e.g. a
    /// real dictation paste from Mumble lands or the user types).
    pub(crate) fn on_practice_change(&mut self, _: &mut Tx) {
        if super::game::changed(self) {
            return;
        }
        self.dictated.changed(&self.practice.text, self.now);
        let has_words = !self.practice.text.trim().is_empty();
        self.hud_landed |= has_words;
        if has_words && self.step == Step::Try {
            self.step = Step::TryHeard;
            self.sfx.on_live_dictation_landed(self.now);
        } else if !has_words && self.step == Step::TryHeard {
            self.step = Step::Try;
        }
    }

    /// Clicking `Continue →` immediately advances to the next step.
    pub(crate) fn press_continue(&mut self) {
        self.manual_override = true;
        self.auto_due_at = None;
        self.sfx.on_capsule_tap();
        match self.step {
            // The real first run moves on as the story does, so the
            // permissions it still waits for decide the next page.
            Step::Done | Step::Permissions if self.live.is_some() => {
                self.auto_due_at = Some(self.now);
            }
            Step::Done if self.flow == Flow::FirstRun => {
                if !self.granted {
                    self.jump_to_permissions();
                } else {
                    self.jump_to_try();
                }
            }
            Step::Done => {
                self.jump_to_ready();
            }
            Step::Permissions => {
                self.jump_to_try();
            }
            Step::TryHeard => {
                self.jump_to_ready();
            }
            _ => {}
        }
    }

    /// Skip on the demo or the practice: on to the next step setup still
    /// needs (the permissions), else Mumble is ready.
    pub(crate) fn skip_optional(&mut self) {
        if !self.step.optional() {
            return;
        }
        if !self.granted {
            self.jump_to_permissions();
        } else {
            self.jump_to_ready();
        }
    }

    /// The real first run's next beat is due now.
    pub(crate) fn move_on(&mut self) {
        self.manual_override = true;
        self.auto_due_at = Some(self.now);
    }

    pub(crate) fn jump_to_demo(&mut self) {
        self.manual_override = true;
        self.sfx.clear();
        self.practice.focused = false;
        self.live_hold_from = None;
        self.step = Step::Parked;
        self.parked_from = Some(self.now);
        let tf = self.now + TYPING_LEAD;
        self.typing_from = Some(tf);
        self.voice_from = None;
        self.hud_from = None;
        self.take_from = None;
        self.partial = "";
        self.sfx.on_without_mumble(self.now, tf);
        self.auto_due_at = Some(tf + typing::duration() + 1.15);
    }

    pub(crate) fn jump_to_permissions(&mut self) {
        self.manual_override = true;
        self.sfx.clear();
        self.practice.focused = false;
        self.live_hold_from = None;
        self.step = Step::Permissions;
        self.permissions_from = Some(self.now);
        if self.live.is_some() {
            self.auto_due_at = Some(self.now + PERMISSIONS_WAIT);
        } else {
            self.sfx.on_permissions(self.now);
            self.auto_due_at = Some(self.now + permissions::PERMISSIONS_SECS);
        }
    }

    pub(crate) fn jump_to_try(&mut self) {
        self.manual_override = true;
        self.auto_due_at = None;
        self.sfx.clear();
        self.take_from = None;
        self.hud_from = Some(self.now);
        self.partial = "";
        self.live_hold_from = None;
        self.practice = TextEdit::default();
        self.practice.focused = self.interactive_try();
        self.dictated.reset();
        self.game = None;
        if let Some(rehearsal) = self.rehearsal.as_mut() {
            *rehearsal = Rehearsal::new(rehearsal.streams);
            rehearsal.from = self.now;
        }
        self.step = Step::Try;
        if !self.interactive_try() {
            self.auto_due_at = Some(self.now + 1.2);
        }
    }

    /// The real first run sets these as the story would at its start.
    pub(super) fn begin_live(&mut self, now: f64) {
        self.manual_override = true;
        self.auto_due_at = Some(now + begin_first_run(self, now));
    }

    pub(crate) fn jump_to_ready(&mut self) {
        self.manual_override = true;
        self.auto_due_at = None;
        self.practice.focused = false;
        self.live_hold_from = None;
        self.step = Step::Ready;
        self.ready_from = Some(self.now);
        self.sfx.on_ready(self.now);
    }

    pub(crate) fn listening_peak(&self) -> Option<f32> {
        if self.live_hold_from.is_some() {
            return Some(crate::audio::input_level::recent().unwrap_or(0.0));
        }
        matches!(self.step, Step::Hearing | Step::TryHearing).then_some(self.meter.1)
    }

    /// Cloud transcription streams its words as you speak.
    pub(crate) fn streams(&self) -> bool {
        true
    }
}

pub(crate) fn add<S: 'static>(playground: Playground<S>) -> Playground<S> {
    playground
        .story(
            Story::new(
                "Prototypes/Onboarding/Without Mumble · With Mumble",
                || Onboard::new(Flow::Versus),
                view,
            )
            .poll(poll)
            .trigger('r', "Play", play)
            .side(side),
        )
        .story(first_run())
}

fn first_run() -> Story<Onboard> {
    Story::new(
        "Prototypes/Onboarding/First run",
        || Onboard::new(Flow::FirstRun),
        view,
    )
    .poll(poll)
    .trigger('r', "Play", play)
    .trigger('t', "Jump to Your turn", |s, _| s.jump_to_try())
    .trigger('s', "Next typing sound", |s, _| {
        let next = s.sfx.style.next();
        s.choose_key_sound(next);
    })
    .variant("Permissions already granted", |s| s.granted = true)
    .variant("Auto-play video", |s| s.scripted_try = true)
    .variant("Your turn, model streams", |s| {
        s.rehearsal = Some(Rehearsal::new(true));
    })
    .variant("Your turn, model doesn't stream", |s| {
        s.rehearsal = Some(Rehearsal::new(false));
    })
    .variant("Your turn, practice field", |s| {
        s.rehearsal = Some(Rehearsal::new(true));
        s.practice_field = true;
    })
    .variant("Quick motion", |s| {
        s.motion.enter = 0.28;
        s.motion.exit = 0.20;
        s.motion.keyboard = 0.5;
    })
    .variant("Gentle motion", |s| {
        s.motion.enter = 0.4;
        s.motion.exit = 0.3;
        s.motion.keyboard = 0.8;
    })
    .side(side)
}

fn view(s: &Onboard, cx: &mut Cx<Onboard>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    s.reduced.set(cx.is_reduced_motion());
    flows::view(p, s)
}

fn play(s: &mut Onboard, play: &mut Play<'_, Onboard>) {
    s.now = play.now();
    s.ready_from = None;
    s.manual_override = false;
    s.auto_due_at = None;
    s.practice.focused = false;
    s.live_hold_from = None;
    s.sfx.clear();
    if s.rehearsal.is_some() {
        s.granted = true;
        s.jump_to_try();
        play.wake_after(Duration::ZERO);
        return;
    }
    let delay = begin_first_run(s, play.now());
    play.after(delay, advance);
    play.wake_after(Duration::ZERO);
}

/// Starts the first run at `now`; the story opens on the `"Without Mumble"`
/// demo (transcription is cloud-only, so there is no model to download).
/// The story's next beat is due after the returned seconds.
pub(super) fn begin_first_run(s: &mut Onboard, now: f64) -> f64 {
    s.now = now;
    s.step = Step::Parked;
    s.parked_from = Some(now);
    let tf = now + TYPING_LEAD;
    s.typing_from = Some(tf);
    s.sfx.on_without_mumble(now, tf);
    TYPING_LEAD + typing::duration() + 1.15
}

fn advance(s: &mut Onboard, play: &mut Play<'_, Onboard>) {
    if s.manual_override {
        return;
    }
    s.now = play.now();
    if s.flow == Flow::FirstRun {
        if let Some(secs) = first_run_next(s, play.now()) {
            play.after(secs, advance);
        }
        play.wake_after(Duration::ZERO);
        return;
    }
    let next = match s.step {
        Step::Parked => {
            s.step = Step::YourTurn;
            s.voice_from = Some(play.now());
            s.hud_from = Some(play.now());
            Some(1.1)
        }
        Step::YourTurn => {
            s.step = Step::Hearing;
            s.take_from = Some(play.now());
            s.sfx.on_voice_take(play.now(), &ANSWER_JAY);
            Some(ANSWER_JAY.release + 0.1)
        }
        Step::Hearing => {
            s.step = Step::Heard;
            Some(1.6)
        }
        Step::Heard => {
            s.step = Step::Done;
            Some(1.2)
        }
        Step::Done => {
            s.step = Step::Ready;
            s.ready_from = Some(play.now());
            s.sfx.on_ready(play.now());
            None
        }
        _ => None,
    };
    if let Some(secs) = next {
        play.after(secs, advance);
    }
    play.wake_after(Duration::ZERO);
}

pub(super) fn first_run_next(s: &mut Onboard, now: f64) -> Option<f64> {
    const WAIT: f64 = 0.25;
    match s.step {
        // The real permissions wait for macOS, then hold their checks a beat.
        Step::Permissions if s.live.as_ref().is_some_and(|live| !live.granted_for(now)) => {
            Some(WAIT)
        }
        Step::Parked => {
            s.step = Step::YourTurn;
            s.voice_from = Some(now);
            s.hud_from = Some(now);
            Some(1.15)
        }
        Step::YourTurn => {
            s.step = Step::Hearing;
            s.take = &ANSWER_JAY;
            s.take_from = Some(now);
            s.sfx.on_voice_take(now, &ANSWER_JAY);
            Some(ANSWER_JAY.release + 0.1)
        }
        Step::Hearing => {
            s.step = Step::Heard;
            Some(1.5)
        }
        Step::Heard => {
            s.step = Step::Done;
            Some(1.3)
        }
        Step::Done if !s.granted => {
            s.step = Step::Permissions;
            s.permissions_from = Some(now);
            s.sfx.on_permissions(now);
            Some(permissions::PERMISSIONS_SECS)
        }
        Step::Done | Step::Permissions => {
            s.step = Step::Try;
            s.take_from = None;
            s.hud_from = Some(now);
            s.partial = "";
            s.practice = TextEdit::default();
            s.dictated.reset();
            if s.interactive_try() {
                s.practice.focused = true;
                None
            } else {
                Some(1.2)
            }
        }
        Step::Try => {
            if s.interactive_try() {
                return None;
            }
            s.step = Step::TryHearing;
            s.take = &ANSWER_TRY;
            s.take_from = Some(now);
            s.sfx.on_voice_take(now, &ANSWER_TRY);
            Some(ANSWER_TRY.release + 0.1)
        }
        Step::TryHearing => {
            s.step = Step::TryHeard;
            Some(2.2)
        }
        Step::TryHeard => {
            if s.interactive_try() {
                return None;
            }
            s.step = Step::Ready;
            s.ready_from = Some(now);
            s.sfx.on_ready(now);
            None
        }
        Step::Ready => None,
    }
}

#[cfg(target_os = "macos")]
pub(super) fn physical_shortcut_held(caps: &[String]) -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: u32) -> u64;
    }
    const HID_SYSTEM_STATE: u32 = 1;
    if caps.is_empty() {
        return false;
    }
    let flags = unsafe { CGEventSourceFlagsState(HID_SYSTEM_STATE) };
    caps.iter().all(|cap| {
        if cap.contains('⌥') || cap.contains("Alt") {
            (flags & 0x0008_0000) != 0
        } else if cap.contains('⌘') {
            (flags & 0x0010_0000) != 0
        } else if cap.contains('⌃') || cap.contains("Ctrl") {
            (flags & 0x0004_0000) != 0
        } else if cap.contains('⇧') || cap.contains("Shift") {
            (flags & 0x0002_0000) != 0
        } else if cap == "fn" {
            (flags & 0x0080_0000) != 0
        } else {
            false
        }
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn physical_shortcut_held(_: &[String]) -> bool {
    false
}

fn poll(s: &mut Onboard, now: f64, tx: &mut Tx) -> bool {
    if advance_to(s, now, tx) {
        tx.wake_after(FRAME);
    }
    true
}

/// Moves the story to `now`; true while anything on it still moves.
pub(super) fn advance_to(s: &mut Onboard, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    tx.trace_channel("onboarding.step", "phase", s.step as u32 as f32);
    tx.trace_channel("onboarding.mic", "level", s.listening_peak().unwrap_or(0.0));
    // A sent `"Without Mumble"` reply lifts out of the field into its bubble.
    let sent = versus::sent_count(s);
    for i in s.sent_seen..sent {
        tx.launch("typed/line0", format!("sent-bubble-{i}"));
    }
    s.sent_seen = sent;
    if s.manual_override
        && let Some(due) = s.auto_due_at
        && now >= due
    {
        s.auto_due_at = first_run_next(s, now).map(|secs| now + secs);
    }

    let trying = s.interactive_try() && matches!(s.step, Step::Try | Step::TryHeard);
    if trying && super::game::step(s, now) {
        s.hud.clear();
    } else if trying {
        // The real first run follows Mumble's own dictation, which sounds its
        // own tones; a rehearsal plays one without them.
        let real = match s.rehearsal {
            Some(rehearsal) => Some(rehearsal.held(now)),
            None => s.live.as_ref().map(|live| live.listening),
        };
        let held = real.unwrap_or_else(|| physical_shortcut_held(&s.caps));
        match (s.live_hold_from, held) {
            (None, true) => {
                s.live_hold_from = Some(now);
                if real.is_none() {
                    s.sfx.on_live_shortcut_down(now);
                }
            }
            (Some(_), false) => {
                s.live_hold_from = None;
                if real.is_none() {
                    s.sfx.on_live_shortcut_up(now);
                }
            }
            _ => {}
        }
        let partial = match s.rehearsal {
            Some(rehearsal) => rehearsal.partial(now),
            None => crate::dictation::live_text::current(),
        };
        s.dictated.hear(partial.as_ref(), now);
        if s.live.as_ref().is_some_and(|live| {
            matches!(
                live.practice,
                Practice::TooShort | Practice::NothingHeard | Practice::Failed
            )
        }) {
            s.dictated.forget();
        }
        if s.rehearsal.as_mut().is_some_and(|r| r.paste_due(now)) {
            s.practice.insert(ANSWER_TRY.text());
            s.on_practice_change(tx);
        }
        s.dictated.settle(now);
        let landed = std::mem::take(&mut s.hud_landed);
        s.hud.step(
            now,
            s.live_hold_from.is_some(),
            crate::audio::input_level::recent(),
            landed,
            s.reduced.get(),
        );
    } else {
        s.game = None;
        s.live_hold_from = None;
        s.hud.clear();
    }

    s.sfx.advance_to(now);
    if s.ready_from.is_some_and(|from| now - from >= 0.25) {
        s.sfx.flush_offline_if_requested(now + 1.5);
    }
    let mut moving = s.sfx.has_pending(now)
        || s.auto_due_at.is_some()
        || s.live_hold_from.is_some()
        || (s.interactive_try() && matches!(s.step, Step::Try | Step::TryHeard));
    moving |= s.dictated.moving();

    match s.take_from {
        Some(from) => {
            let elapsed = now - from;
            let done = s.take.finished(elapsed);
            s.partial = if done || s.streams() {
                s.take.partial(elapsed)
            } else {
                ""
            };
            s.meter = s.take.meter(elapsed);
            moving |= !done;
        }
        None => {
            let hum = ((now * 9.0).sin() * (now * 4.3).sin()).abs() as f32;
            s.meter = (0.03 + 0.04 * hum, 0.05 + 0.06 * hum);
        }
    }
    moving
}

fn side_chip(
    p: Paint,
    key: &str,
    label: &str,
    active: bool,
    on_press: impl Fn(&mut Onboard) + 'static,
) -> El {
    let k = p.k;
    let c = &k.c;
    row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .focusable(true)
        .w_full()
        .h(26.0)
        .px(8.0)
        .gap(7.0)
        .center_y()
        .radius(6.0)
        .fill(if active {
            c.surface_raised
        } else {
            rune::Color::TRANSPARENT
        })
        .border(
            p.hair,
            if active {
                c.border_base
            } else {
                rune::Color::TRANSPARENT
            },
        )
        .hover(c.hover)
        .press(c.pressed)
        .on_press(on_press)
        .child(el().key("dot").size(5.0, 5.0).radius(2.5).fill(if active {
            p.text()
        } else {
            c.border_base
        }))
        .child(
            text(label)
                .key("lbl")
                .font(if active {
                    k.ty.small_medium
                } else {
                    k.ty.small
                })
                .color(if active { p.text() } else { p.muted() }),
        )
}

fn side_section(p: Paint, key: &str, heading: &str, items: Vec<El>) -> El {
    let k = p.k;
    col()
        .key(key.to_string())
        .w(SIDE_W)
        .gap(3.0)
        .child(
            text(heading)
                .key("hdr")
                .font(TextStyle::new(11.0, 560.0).lh(15.0))
                .color(p.faint()),
        )
        .child(
            col()
                .key("box")
                .w_full()
                .padding(3.0)
                .gap(1.0)
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .border(p.hair, k.c.border_weak)
                .children(items),
        )
}

fn side(s: &Onboard, cx: &mut Cx<Onboard>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let mut out = col().key("onboarding-side").w(SIDE_W).gap(12.0);

    if s.flow == Flow::FirstRun {
        let step_items = vec![
            side_chip(
                p,
                "step-demo",
                "1 · Without / With Mumble",
                matches!(
                    s.step,
                    Step::Parked | Step::YourTurn | Step::Hearing | Step::Heard | Step::Done
                ),
                Onboard::jump_to_demo,
            ),
            side_chip(
                p,
                "step-perms",
                "2 · Permissions",
                s.step == Step::Permissions,
                Onboard::jump_to_permissions,
            ),
            side_chip(
                p,
                "step-try",
                "3 · Your turn (live)",
                matches!(s.step, Step::Try | Step::TryHearing | Step::TryHeard),
                Onboard::jump_to_try,
            ),
            side_chip(
                p,
                "step-ready",
                "4 · Mumble is ready",
                s.step == Step::Ready,
                Onboard::jump_to_ready,
            ),
        ];
        out = out.child(side_section(p, "steps-sec", "Jump to step", step_items));
        out = out.child(motion_controls(p, s));
    }

    if matches!(s.flow, Flow::FirstRun | Flow::Versus) {
        let sound_items = KeySoundStyle::ALL
            .into_iter()
            .enumerate()
            .map(|(i, style)| {
                side_chip(
                    p,
                    &format!("snd-{i}"),
                    style.name(),
                    s.sfx.style == style,
                    move |s: &mut Onboard| s.choose_key_sound(style),
                )
            })
            .collect();
        out = out.child(side_section(
            p,
            "sounds-sec",
            "Typing key sound (click to hear)",
            sound_items,
        ));
    }

    let note = if s.interactive_try() {
        "On Your turn, hold ⌥ and speak into your Mac's mic — your running Mumble pastes your real words into the card."
    } else {
        "Mumble transcribes through Claude or Codex; the demo plays the same answer typed, then spoken."
    };
    out.child(
        text(note)
            .key("side-note")
            .font(p.k.ty.small)
            .color(p.muted())
            .wrap()
            .w(SIDE_W),
    )
}

fn motion_controls(p: Paint, s: &Onboard) -> El {
    use super::super::lists::slider_in;
    let m = s.motion;
    col()
        .key("motion-controls")
        .gap(10.0)
        .child(slider_in(
            p,
            "motion-enter",
            "Rise",
            (m.enter - 0.2) / 0.4,
            format!("{:.2} s", m.enter),
            SIDE_W,
            |s: &mut Onboard, v, _| s.motion.enter = 0.2 + v * 0.4,
        ))
        .child(slider_in(
            p,
            "motion-exit",
            "Leave",
            (m.exit - 0.15) / 0.25,
            format!("{:.2} s", m.exit),
            SIDE_W,
            |s: &mut Onboard, v, _| s.motion.exit = 0.15 + v * 0.25,
        ))
        .child(slider_in(
            p,
            "motion-travel",
            "Travel",
            m.travel / 20.0,
            format!("{:.0} pt", m.travel),
            SIDE_W,
            |s: &mut Onboard, v, _| s.motion.travel = v * 20.0,
        ))
        .child(slider_in(
            p,
            "motion-stagger",
            "Stagger",
            m.stagger / 0.08,
            format!("{:.0} ms", m.stagger * 1000.0),
            SIDE_W,
            |s: &mut Onboard, v, _| s.motion.stagger = v * 0.08,
        ))
        .child(slider_in(
            p,
            "motion-keyboard",
            "Keyboard",
            (m.keyboard - 0.35) / 0.65,
            format!("{:.2} s", m.keyboard),
            SIDE_W,
            |s: &mut Onboard, v, _| s.motion.keyboard = 0.35 + v * 0.65,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_voice_take_shows_the_reply_as_it_streams() {
        let mut s = Onboard::new(Flow::Versus);
        s.take_from = Some(0.0);
        let mut tx = Tx::default();
        poll(&mut s, 0.3, &mut tx);
        assert_eq!(s.partial, ANSWER_JAY.partial(0.3));
        poll(&mut s, ANSWER_JAY.release + 0.1, &mut tx);
        assert_eq!(s.partial, ANSWER_JAY.text());
    }

    #[test]
    fn live_try_pauses_and_completes_when_real_words_paste_in() {
        let mut s = Onboard::new(Flow::FirstRun);
        s.jump_to_try();
        assert_eq!(s.step, Step::Try);
        s.practice.text = "Testing live dictation in Mumble.".into();
        let mut tx = Tx::default();
        s.on_practice_change(&mut tx);
        assert_eq!(s.step, Step::TryHeard);
        s.press_continue();
        assert_eq!(s.step, Step::Ready);
    }

    #[test]
    fn reduced_motion_does_not_suppress_onboarding_sounds() {
        let simulate = |reduced: bool| -> Vec<(f64, crate::feedback::Tone)> {
            let mut s = Onboard::new(Flow::FirstRun);
            s.scripted_try = true;
            s.reduced.set(reduced);
            let mut now = 0.0;
            let mut tx = Tx::default();
            begin_first_run(&mut s, now);
            for _ in 0..20 {
                poll(&mut s, now, &mut tx);
                let Some(wait) = first_run_next(&mut s, now) else {
                    poll(&mut s, now + 0.3, &mut tx);
                    break;
                };
                now += wait;
            }
            s.sfx.fired_cues().to_vec()
        };
        let full = simulate(false);
        let reduced = simulate(true);
        assert_eq!(full, reduced);
        assert!(
            full.len() >= 10,
            "expected a full sound track, got {}",
            full.len()
        );
    }
}
