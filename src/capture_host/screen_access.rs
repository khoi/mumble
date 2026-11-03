//! Screen Recording as this running Hex can use it.
//!
//! `CGPreflightScreenCaptureAccess` keeps its first answer for the life of
//! the process, so a grant made while Hex runs never shows through it: the
//! pane kept saying "Required", Allow opened System Settings on a switch
//! already on, and a capture failed with "Screen Recording is off". Once the
//! preflight says no, a fresh `scrawl-helper` (which macOS counts as Hex)
//! reads the grant, and a real one-frame capture tells whether this process
//! can use it or must restart first.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use scrawl_capture::permissions::{Permission, Status};

unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Whether macOS currently allows Screen & System Audio Recording.
pub fn screen_recording_permitted() -> bool {
    // SAFETY: A CoreGraphics query without arguments or side effects.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Asks macOS to list Hex under Screen & System Audio Recording. The system
/// prompts at most once; later requests return the current state.
pub fn request_screen_recording() -> bool {
    // SAFETY: A CoreGraphics request without arguments.
    unsafe { CGRequestScreenCaptureAccess() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenAccess {
    #[default]
    Off,
    /// Granted in System Settings, but this process can't record until
    /// Hex restarts.
    NeedsRestart,
    Allowed,
}

/// How long a fresh read stands before [`screen_access`] reads again.
const FRESH_FOR: Duration = Duration::from_secs(2);

struct Cached {
    access: ScreenAccess,
    at: Option<Instant>,
    reading: bool,
}

static CACHED: Mutex<Cached> = Mutex::new(Cached {
    access: ScreenAccess::Off,
    at: None,
    reading: false,
});

/// Never blocks: the preflight, else the last fresh read, starting another
/// on its own thread when that one is older than [`FRESH_FOR`].
pub fn screen_access() -> ScreenAccess {
    if screen_recording_permitted() {
        return ScreenAccess::Allowed;
    }
    let mut cached = CACHED.lock().unwrap_or_else(|e| e.into_inner());
    let stale = cached.at.is_none_or(|at| at.elapsed() >= FRESH_FOR);
    if stale && !cached.reading {
        cached.reading = true;
        let spawned = std::thread::Builder::new()
            .name("screen-access".into())
            .spawn(|| {
                let access = read();
                let mut cached = CACHED.lock().unwrap_or_else(|e| e.into_inner());
                *cached = Cached {
                    access,
                    at: Some(Instant::now()),
                    reading: false,
                };
            });
        if spawned.is_err() {
            cached.reading = false;
        }
    }
    cached.access
}

/// Marks the cached Screen Recording state stale so the next [`screen_access`]
/// read spawns a fresh check.
pub fn invalidate_screen_access() {
    CACHED.lock().unwrap_or_else(|e| e.into_inner()).at = None;
}

/// Reads it now, blocking for up to the helper's 5 s. Off the UI thread.
pub fn screen_access_now() -> ScreenAccess {
    if screen_recording_permitted() {
        return ScreenAccess::Allowed;
    }
    let access = read();
    *CACHED.lock().unwrap_or_else(|e| e.into_inner()) = Cached {
        access,
        at: Some(Instant::now()),
        reading: false,
    };
    access
}

fn read() -> ScreenAccess {
    let granted = match scrawl_capture::helper::permissions() {
        Ok(statuses) => statuses.get(&Permission::ScreenRecording) == Some(&Status::Granted),
        Err(error) => {
            tracing::debug!(error = %format!("{error:#}"), "could not read Screen Recording afresh");
            false
        }
    };
    if !granted {
        ScreenAccess::Off
    } else if scrawl_capture::capture::screen_capture_usable(true) {
        ScreenAccess::Allowed
    } else {
        tracing::info!("Screen Recording is granted but needs Hex to restart");
        ScreenAccess::NeedsRestart
    }
}
