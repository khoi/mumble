use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use color_eyre::eyre::{Result, eyre};

#[cfg(test)]
use crate::app_settings::HotkeyBinding;
use crate::app_settings::{HOTKEY_MODIFIERS_MASK, RuntimeHotkey, RuntimeHotkeys};
use crate::audio::CaptureInstant;
use crate::dictation::{BOTH_SIDES_WINDOW, MINIMUM_HOLD_DURATION};

mod hotkey;
mod tap;
pub use hotkey::{DictationHotkey, HotkeyAction};
use tap::*;

const DOUBLE_TAP_WINDOW: Duration = MINIMUM_HOLD_DURATION;
const ESCAPE_KEY_CODE: u16 = 53;
/// The key code the Globe (Fn) key reports in its own key events.
const GLOBE_KEY_CODE: u16 = 179;

const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const EVENT_LEFT_MOUSE_UP: u32 = 2;
const EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const EVENT_RIGHT_MOUSE_UP: u32 = 4;
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_KEY_UP: u32 = 11;
const EVENT_FLAGS_CHANGED: u32 = 12;
const EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const EVENT_OTHER_MOUSE_UP: u32 = 26;
const EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;
const MOUSE_EVENT_BUTTON_NUMBER: u32 = 3;
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;
const KEYBOARD_EVENT_KEYCODE: u32 = 9;
const EVENT_SOURCE_USER_DATA: u32 = 42;

// Assistive keyboards inject downstream of HID; observe them alongside hardware input.
const ANNOTATED_SESSION_EVENT_TAP: u32 = 2;
// Another app's global shortcut (Carbon hotkeys such as a window manager's
// Option-1) consumes its key before the session, so only HID sees that press.
const HID_EVENT_TAP: u32 = 0;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const DEFAULT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;
const HID_SYSTEM_STATE: u32 = 1;
const COMBINED_SESSION_STATE: u32 = 0;
const STALE_KEY_NEUTRAL_DURATION: Duration = Duration::from_millis(100);
/// A held key repeats well within this (macOS's slowest repeat setting is
/// under 2 s), so a tracked key with no key event for this long is not held,
/// even when both system key states still read it down (#1008).
const TRACKED_KEY_REPEAT_TIMEOUT: Duration = Duration::from_secs(5);
const PHYSICAL_KEY_DOWN_HISTORY: usize = 8;

type EventRef = *mut c_void;
type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state_id: u32) -> u64;
    fn CGEventSourceKeyState(state_id: u32, key: u16) -> bool;
    fn CGEventSourceButtonState(state_id: u32, button: u32) -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventTapIsEnabled(tap: *mut c_void) -> bool;
    fn CGEventGetFlags(event: EventRef) -> u64;
    fn CGEventGetTimestamp(event: EventRef) -> u64;
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: usize,
        order: isize,
        callout: unsafe extern "C" fn(timer: *mut c_void, info: *mut c_void),
        context: *mut RunLoopTimerContext,
    ) -> *mut c_void;
    fn CFRunLoopAddTimer(run_loop: *mut c_void, timer: *mut c_void, mode: *const c_void);
    fn CFRunLoopTimerInvalidate(timer: *mut c_void);
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFMachPortIsValid(port: *mut c_void) -> u8;
    fn CFRelease(value: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

#[repr(C)]
struct RunLoopTimerContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[derive(Clone, Copy, Debug)]
pub enum InputEvent {
    Flags(u64),
    Key {
        code: u16,
        down: bool,
        flags: u64,
    },
    /// A key press seen at the HID level. It only signals a chord: the session
    /// `Key` events, when they arrive, remain the source of key state.
    PhysicalKeyDown {
        code: u16,
        timing: HidTiming,
    },
    MouseDown,
    TapDisabled,
}

/// How a HID-tap timestamp was read, kept only for diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidTiming {
    pub raw: u64,
    pub clock: HidClock,
    /// When the tap callback ran.
    pub received_at: CaptureInstant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HidClock {
    MachTicks,
    Nanos,
    /// Neither reading was recent, so the event took its arrival time.
    Arrival,
}

impl HidClock {
    fn as_str(self) -> &'static str {
        match self {
            Self::MachTicks => "mach",
            Self::Nanos => "nanos",
            Self::Arrival => "arrival",
        }
    }
}

/// What ended a press as a chord before it became a dictation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscardCause {
    Key,
    HidKey,
    Mouse,
    ExtraModifier,
}

impl DiscardCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::HidKey => "hid-key",
            Self::Mouse => "mouse",
            Self::ExtraModifier => "extra-modifier",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ObservedInputEvent {
    sequence: u64,
    pub event: InputEvent,
    pub capture_at: CaptureInstant,
}

#[derive(Clone, Default)]
pub struct PendingInputEvents(Arc<Mutex<VecDeque<(u64, CaptureInstant)>>>);

impl PendingInputEvents {
    #[cfg(test)]
    pub fn with_pending_for_test(at: CaptureInstant) -> (Self, impl FnOnce()) {
        let pending = Self::default();
        pending.push(0, at);
        let acknowledge = pending.clone();
        (pending, move || acknowledge.acknowledge(0))
    }

    pub fn oldest(&self) -> Option<CaptureInstant> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .front()
            .map(|(_, at)| *at)
    }

    fn push(&self, sequence: u64, at: CaptureInstant) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back((sequence, at));
    }

    fn acknowledge(&self, sequence: u64) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if pending.front().is_some_and(|(next, _)| *next == sequence) {
            pending.pop_front();
        }
    }
}

pub struct InputMonitor {
    pub events: Receiver<ObservedInputEvent>,
    pub activity: InputActivity,
    pending: PendingInputEvents,
    takes_cancel: Arc<AtomicBool>,
    run_loop: Arc<AtomicPtr<c_void>>,
    worker: Option<JoinHandle<()>>,
}

pub struct PendingInputAcknowledgement<'a> {
    pending: &'a PendingInputEvents,
    sequence: u64,
}

impl Drop for PendingInputAcknowledgement<'_> {
    fn drop(&mut self) {
        self.pending.acknowledge(self.sequence);
    }
}

#[derive(Clone, Default)]
pub struct InputActivity(Arc<AtomicU64>);

impl InputActivity {
    pub fn revision(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn invalidate(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn observe(&self, input: InputEvent, suppressed: bool) {
        if !suppressed
            && matches!(
                input,
                InputEvent::Key { down: true, .. } | InputEvent::MouseDown
            )
        {
            self.invalidate();
        }
    }
}

impl InputMonitor {
    pub fn start() -> Result<Self> {
        let (sender, events) = mpsc::channel::<ObservedInputEvent>();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let activity = InputActivity::default();
        let tap_activity = activity.clone();
        let run_loop = Arc::new(AtomicPtr::new(ptr::null_mut()));
        let tap_run_loop = run_loop.clone();
        let takes_cancel = Arc::new(AtomicBool::new(false));
        let tap_takes_cancel = takes_cancel.clone();
        let pending = PendingInputEvents::default();
        let tap_pending = pending.clone();
        let worker = thread::spawn(move || {
            run_event_tap(
                sender,
                tap_activity,
                tap_takes_cancel,
                tap_pending,
                tap_run_loop,
                ready_sender,
            )
        });
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| eyre!("timed out starting the keyboard event tap"))??;
        Ok(Self {
            events,
            activity,
            pending,
            takes_cancel,
            run_loop,
            worker: Some(worker),
        })
    }

    /// Whether the tap keeps Toggle's cancel shortcut from the app in front:
    /// only while a toggled recording runs.
    pub fn set_takes_cancel(&self, enabled: bool) {
        self.takes_cancel.store(enabled, Ordering::Release);
    }

    pub fn pending_events(&self) -> PendingInputEvents {
        self.pending.clone()
    }

    pub fn acknowledge_after(&self, event: ObservedInputEvent) -> PendingInputAcknowledgement<'_> {
        PendingInputAcknowledgement {
            pending: &self.pending,
            sequence: event.sequence,
        }
    }
}

impl Drop for InputMonitor {
    fn drop(&mut self) {
        let run_loop = self.run_loop.load(Ordering::Acquire);
        if !run_loop.is_null() {
            // SAFETY: Core Foundation run loops may be stopped from any thread.
            unsafe { CFRunLoopStop(run_loop) };
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct EventTapContext {
    sender: Sender<ObservedInputEvent>,
    activity: InputActivity,
    takes_cancel: Arc<AtomicBool>,
    key_tap: AtomicPtr<c_void>,
    observation_tap: AtomicPtr<c_void>,
    physical_tap: AtomicPtr<c_void>,
    shortcut_suppression: Mutex<ShortcutSuppression>,
    pending: PendingInputEvents,
    next_sequence: AtomicU64,
}

unsafe extern "C" fn physical_event_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: `user_info` points to the context allocated by `run_event_tap`.
    let context = unsafe { &*(user_info.cast::<EventTapContext>()) };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        // Missing evidence only lets a chord count as a hold, so just resume.
        reenable_disabled_taps(context, event_type, TapKind::Physical.as_str());
        return event;
    }
    if event.is_null()
        || event_type != EVENT_KEY_DOWN
        || crate::app_settings::hotkey_capture_active()
        // SAFETY: CoreGraphics supplied a valid event to this callback.
        || unsafe { CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA) }
            == crate::keyboard::SYNTHETIC_EVENT_MARKER
    {
        return event;
    }
    // SAFETY: CoreGraphics supplied a valid keyboard event.
    let code = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) as u16 };
    // SAFETY: CoreGraphics supplied a valid event.
    let flags = unsafe { CGEventGetFlags(event) };
    if !is_chord_evidence(code, flags, crate::app_settings::runtime_hotkeys()) {
        return event;
    }
    // SAFETY: CoreGraphics supplied a valid event.
    let raw = unsafe { CGEventGetTimestamp(event) };
    let received_at = CaptureInstant::now();
    let (at, clock) = physical_event_instant(raw, received_at);
    send_input(
        context,
        InputEvent::PhysicalKeyDown {
            code,
            timing: HidTiming {
                raw,
                clock,
                received_at,
            },
        },
        at,
    );
    event
}

/// Whether a HID key press can reveal a chord Hex would otherwise miss.
/// Escape ends no dictation, and Hex's own shortcuts act through their session events.
fn is_chord_evidence(code: u16, flags: u64, hotkeys: RuntimeHotkeys) -> bool {
    code != ESCAPE_KEY_CODE
        && !is_own_globe_key(code, hotkeys.dictation)
        && !hotkeys
            .second_dictation
            .is_some_and(|second| is_own_globe_key(code, second))
        && !hotkeys
            .all()
            .iter()
            .flatten()
            .any(|hotkey| hotkey.matches_key_press(code, flags))
}

/// The Globe key's own key event, which some keyboards and remappers
/// (Karabiner-Elements) send beside the Fn modifier change. For a binding
/// that holds Fn, it is the shortcut itself, never a chord.
fn is_own_globe_key(code: u16, binding: RuntimeHotkey) -> bool {
    code == GLOBE_KEY_CODE && binding.modifiers.function && binding.key_code != Some(code)
}

/// HID-tap timestamps arrive as Mach ticks on Apple Silicon and as
/// nanoseconds on some events; keep whichever reading is recent, else `now`.
fn physical_event_instant(raw: u64, now: CaptureInstant) -> (CaptureInstant, HidClock) {
    const RECENT: Duration = Duration::from_secs(5);
    [
        (CaptureInstant::from_mach_ticks(raw), HidClock::MachTicks),
        (CaptureInstant::from_nanos(raw), HidClock::Nanos),
    ]
    .into_iter()
    .find(|(at, _)| {
        now.checked_duration_since(*at)
            .is_some_and(|age| age <= RECENT)
    })
    .unwrap_or((now, HidClock::Arrival))
}

/// Signed milliseconds from `from` to `to`.
fn signed_ms(to: CaptureInstant, from: CaptureInstant) -> f64 {
    (i128::from(to.as_nanos()) - i128::from(from.as_nanos())) as f64 / 1e6
}

unsafe extern "C" fn event_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: `user_info` points to the context allocated by `run_event_tap`.
    let context = unsafe { &*(user_info.cast::<EventTapContext>()) };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        // The key and modifier taps share this callback.
        reenable_disabled_taps(context, event_type, "session");
        send_input(context, InputEvent::TapDisabled, CaptureInstant::ZERO);
        return event;
    }
    if event.is_null() {
        return event;
    }
    // SAFETY: CoreGraphics supplied a valid event to this callback.
    if unsafe { CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA) }
        == crate::keyboard::SYNTHETIC_EVENT_MARKER
    {
        // The chord breaker has already passed the HID and session taps and
        // global monitors that watch for modifier taps; the focused app would
        // only beep at it.
        let chord_breaker = matches!(event_type, EVENT_KEY_DOWN | EVENT_KEY_UP)
            // SAFETY: CoreGraphics supplied a valid keyboard event.
            && unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) }
                == i64::from(crate::keyboard::CHORD_BREAKER_KEY_CODE);
        return if chord_breaker {
            ptr::null_mut()
        } else {
            event
        };
    }
    if matches!(
        event_type,
        EVENT_LEFT_MOUSE_DRAGGED | EVENT_RIGHT_MOUSE_DRAGGED | EVENT_OTHER_MOUSE_DRAGGED
    ) {
        // A swallowed press must not leak a drag into the foreground app.
        // SAFETY: CoreGraphics supplied a valid mouse event.
        let button = unsafe { CGEventGetIntegerValueField(event, MOUSE_EVENT_BUTTON_NUMBER) };
        let suppress = !crate::app_settings::hotkey_capture_active()
            && u16::try_from(button).ok().is_some_and(|button| {
                context
                    .shortcut_suppression
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .suppresses_mouse_drag(button)
            });
        return if suppress { ptr::null_mut() } else { event };
    }
    let input = match event_type {
        EVENT_FLAGS_CHANGED => {
            // SAFETY: CoreGraphics supplied a valid event to this callback.
            InputEvent::Flags(unsafe { CGEventGetFlags(event) })
        }
        EVENT_KEY_DOWN | EVENT_KEY_UP => InputEvent::Key {
            // SAFETY: CoreGraphics supplied a valid keyboard event.
            code: unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) as u16 },
            down: event_type == EVENT_KEY_DOWN,
            // SAFETY: CoreGraphics supplied a valid event.
            flags: unsafe { CGEventGetFlags(event) },
        },
        EVENT_LEFT_MOUSE_DOWN
        | EVENT_RIGHT_MOUSE_DOWN
        | EVENT_OTHER_MOUSE_DOWN
        | EVENT_LEFT_MOUSE_UP
        | EVENT_RIGHT_MOUSE_UP
        | EVENT_OTHER_MOUSE_UP => {
            // SAFETY: CoreGraphics supplied a valid mouse event.
            let button = unsafe { CGEventGetIntegerValueField(event, MOUSE_EVENT_BUTTON_NUMBER) };
            let Ok(button) = u16::try_from(button) else {
                return event;
            };
            let down = matches!(
                event_type,
                EVENT_LEFT_MOUSE_DOWN | EVENT_RIGHT_MOUSE_DOWN | EVENT_OTHER_MOUSE_DOWN
            );
            // SAFETY: CoreGraphics supplied a valid event.
            let flags = unsafe { CGEventGetFlags(event) };
            let input = mouse_input(button, down, flags, crate::app_settings::runtime_hotkeys())
                .or_else(|| {
                    // A settings change can remove the binding before release.
                    // Finish the original press's bookkeeping and suppression.
                    (!down)
                        .then(|| {
                            context
                                .shortcut_suppression
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .tracked_mouse_release(button, flags)
                        })
                        .flatten()
                });
            let Some(input) = input else {
                return event;
            };
            input
        }
        _ => return event,
    };
    // Both taps are annotated-session taps: their timestamps are nanoseconds.
    // Physical HID events used raw Mach ticks on the tested Apple Silicon system.
    // SAFETY: CoreGraphics supplied a valid event to this callback.
    let capture_at = CaptureInstant::from_nanos(unsafe { CGEventGetTimestamp(event) });
    if crate::app_settings::hotkey_capture_active() {
        context.activity.observe(input, false);
        context
            .shortcut_suppression
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reset();
        return event;
    }
    let delivered = send_input(context, input, capture_at);
    let mut suppression = context
        .shortcut_suppression
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut hotkeys = crate::app_settings::runtime_hotkeys();
    if !context.takes_cancel.load(Ordering::Acquire) {
        hotkeys.cancel = None;
    }
    let suppress = suppression.process_all(input, hotkeys, delivered);
    context.activity.observe(input, suppress);
    // A key Hex consumed (its shortcuts) was never the app's chord.
    if suppress { ptr::null_mut() } else { event }
}

fn send_input(context: &EventTapContext, event: InputEvent, capture_at: CaptureInstant) -> bool {
    let sequence = context.next_sequence.fetch_add(1, Ordering::Relaxed);
    context.pending.push(sequence, capture_at);
    match context.sender.send(ObservedInputEvent {
        sequence,
        event,
        capture_at,
    }) {
        Ok(()) => true,
        Err(_) => {
            context.pending.acknowledge(sequence);
            // SAFETY: The callback runs on the event-tap thread's run loop.
            unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
            false
        }
    }
}

#[derive(Default)]
struct ShortcutSuppression {
    // Repeats and releases keep the original press's suppression decision.
    key_presses: HashMap<u16, bool>,
}

impl ShortcutSuppression {
    fn tracked_mouse_release(&self, button: u16, flags: u64) -> Option<InputEvent> {
        let code = crate::app_settings::HotkeyKey::mouse_code(button)?;
        self.key_presses
            .contains_key(&code)
            .then_some(InputEvent::Key {
                code,
                down: false,
                flags,
            })
    }

    fn suppresses_mouse_drag(&self, button: u16) -> bool {
        crate::app_settings::HotkeyKey::mouse_code(button)
            .and_then(|code| self.key_presses.get(&code))
            .copied()
            .unwrap_or(false)
    }

    fn reset(&mut self) {
        self.key_presses.clear();
    }

    /// Whether the tap keeps `input` from the app in front: Hex's shortcuts
    /// and `hotkeys.cancel`, which the caller leaves set only while a
    /// toggled recording runs.
    fn process_all(&mut self, input: InputEvent, hotkeys: RuntimeHotkeys, delivered: bool) -> bool {
        let bindings = hotkeys.all();
        match input {
            InputEvent::Key {
                code,
                down: true,
                flags,
            } => *self.key_presses.entry(code).or_insert_with(|| {
                delivered
                    && bindings
                        .iter()
                        .flatten()
                        .chain(hotkeys.cancel.iter())
                        .any(|hotkey| hotkey.matches_key_press(code, flags))
            }),
            InputEvent::Key {
                code, down: false, ..
            } => self.key_presses.remove(&code).unwrap_or(false),
            _ => false,
        }
    }

    #[cfg(test)]
    fn process(
        &mut self,
        input: InputEvent,
        paste_key_code: u16,
        hotkey: RuntimeHotkey,
        delivered: bool,
    ) -> bool {
        let mut paste_last = HotkeyBinding::paste_last_default().runtime();
        paste_last.key_code = Some(paste_key_code);
        self.process_all(
            input,
            RuntimeHotkeys {
                dictation: hotkey,
                paste_last: Some(paste_last),
                raw_dictation: None,
                second_dictation: None,
                cancel: None,
            },
            delivered,
        )
    }
}

fn paste_action(input: InputEvent, hotkeys: RuntimeHotkeys) -> Option<HotkeyAction> {
    let InputEvent::Key {
        code,
        down: true,
        flags,
    } = input
    else {
        return None;
    };
    hotkeys
        .paste_last
        .filter(|binding| binding.matches_key_press(code, flags))
        .map(|_| HotkeyAction::PasteLast)
}
