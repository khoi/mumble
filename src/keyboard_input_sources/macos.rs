//! Main-thread-only TIS observation. One observer runs for the app's life,
//! and every selected-source change rebuilds the keyboard layout snapshot.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;

type Ref = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISNotifySelectedKeyboardInputSourceChanged: Ref;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFNotificationCenterGetDistributedCenter() -> Ref;
    fn CFNotificationCenterAddObserver(
        center: Ref,
        observer: Ref,
        callback: unsafe extern "C" fn(Ref, *mut c_void, Ref, Ref, Ref),
        name: Ref,
        object: Ref,
        suspension_behavior: isize,
    );
    fn CFNotificationCenterRemoveObserver(center: Ref, observer: Ref, name: Ref, object: Ref);
}

struct Watcher {
    changed: Box<AtomicBool>,
}

thread_local! {
    static WATCHER: RefCell<Option<Watcher>> = const { RefCell::new(None) };
}

unsafe extern "C" fn changed(_: Ref, observer: *mut c_void, _: Ref, _: Ref, _: Ref) {
    // SAFETY: `observer` is the watcher's boxed flag, registered until it drops.
    unsafe { &*observer.cast::<AtomicBool>() }.store(true, Ordering::Release);
}

/// Starts observing input-source changes for the app's life; later calls do
/// nothing.
pub(crate) fn start(_mtm: MainThreadMarker) {
    WATCHER.with(|slot| {
        slot.borrow_mut().get_or_insert_with(Watcher::new);
    });
}

/// Whether the selected source changed since the last call, so the keyboard
/// layout snapshot must be rebuilt.
pub(crate) fn take_layout_change(_mtm: MainThreadMarker) -> bool {
    WATCHER.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|watcher| watcher.changed.swap(false, Ordering::AcqRel))
    })
}

impl Watcher {
    fn new() -> Self {
        let watcher = Self {
            changed: Box::new(AtomicBool::new(false)),
        };
        // SAFETY: the boxed flag has a stable address and outlives registration.
        // 4 = DeliverImmediately, including when Mumble is in the background.
        unsafe {
            CFNotificationCenterAddObserver(
                CFNotificationCenterGetDistributedCenter(),
                (&*watcher.changed as *const AtomicBool).cast(),
                changed,
                kTISNotifySelectedKeyboardInputSourceChanged,
                ptr::null(),
                4,
            );
        }
        watcher
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: unregister on the main thread before dropping the boxed flag.
        unsafe {
            CFNotificationCenterRemoveObserver(
                CFNotificationCenterGetDistributedCenter(),
                (&*self.changed as *const AtomicBool).cast(),
                kTISNotifySelectedKeyboardInputSourceChanged,
                ptr::null(),
            );
        }
    }
}
