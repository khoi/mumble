//! macOS "System Audio Recording" consent (TCC `kTCCServiceAudioCapture`),
//! which process taps need: Labs › Also pause the browser hears the browser
//! through one. Screen Recording consent does not imply it.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_core_foundation::{CFRetained, CFString};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemAudioAuthorization {
    Granted,
    Denied,
    NotDetermined,
    /// The private TCC query is unavailable; try the tap and let macOS ask.
    Unknown,
}

type TccPreflight = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
type TccRequest = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void);

struct Tcc {
    preflight: TccPreflight,
    request: TccRequest,
}

fn tcc() -> Option<&'static Tcc> {
    static TCC: OnceLock<Option<Tcc>> = OnceLock::new();
    TCC.get_or_init(|| {
        // SAFETY: dlopen/dlsym of a system framework; symbols are checked.
        unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(),
                libc::RTLD_NOW,
            );
            if handle.is_null() {
                return None;
            }
            let preflight = libc::dlsym(handle, c"TCCAccessPreflight".as_ptr());
            let request = libc::dlsym(handle, c"TCCAccessRequest".as_ptr());
            if preflight.is_null() || request.is_null() {
                return None;
            }
            Some(Tcc {
                preflight: std::mem::transmute::<*mut c_void, TccPreflight>(preflight),
                request: std::mem::transmute::<*mut c_void, TccRequest>(request),
            })
        }
    })
    .as_ref()
}

fn audio_capture_service() -> CFRetained<CFString> {
    CFString::from_static_str("kTCCServiceAudioCapture")
}

pub fn authorization() -> SystemAudioAuthorization {
    let Some(tcc) = tcc() else {
        return SystemAudioAuthorization::Unknown;
    };
    let service = audio_capture_service();
    // SAFETY: A CFString service name and no options.
    let result =
        unsafe { (tcc.preflight)((&*service as *const CFString).cast(), std::ptr::null()) };
    match result {
        0 => SystemAudioAuthorization::Granted,
        1 => SystemAudioAuthorization::Denied,
        _ => SystemAudioAuthorization::NotDetermined,
    }
}

/// Whether a process tap can hear system audio.
pub fn ready() -> bool {
    matches!(
        authorization(),
        SystemAudioAuthorization::Granted | SystemAudioAuthorization::Unknown
    )
}

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Allows [`request_access`] to prompt again after a TCC reset returned
/// System Audio Recording to `NotDetermined`.
pub fn reset_request() {
    REQUESTED.store(false, Ordering::Relaxed);
}

/// Shows the macOS System Audio Recording prompt when consent is undecided.
/// Returns whether a prompt was requested.
pub fn request_access() -> bool {
    if authorization() != SystemAudioAuthorization::NotDetermined
        || REQUESTED.swap(true, Ordering::Relaxed)
    {
        return false;
    }
    let Some(tcc) = tcc() else {
        return false;
    };
    let service = audio_capture_service();
    let completion = RcBlock::new(|granted: Bool| {
        tracing::info!(
            granted = granted.as_bool(),
            "system audio recording consent answered"
        );
    });
    // SAFETY: TCC copies the completion block before returning.
    unsafe {
        (tcc.request)(
            (&*service as *const CFString).cast(),
            std::ptr::null(),
            RcBlock::as_ptr(&completion).cast(),
        );
    }
    true
}
