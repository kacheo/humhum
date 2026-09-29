//! macOS launch-at-login via `SMAppService` (ServiceManagement, macOS 13+).
//!
//! The app's `minimumSystemVersion` is 13.0, so the older `SMLoginItemSetEnabled`
//! path the LLD mentions for macOS 12 is unreachable and intentionally omitted.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

use super::PrefsError;

// Force-link ServiceManagement so `class!(SMAppService)` resolves.
#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {}

fn error_description(err: *mut AnyObject) -> String {
    if err.is_null() {
        return "unknown ServiceManagement error".into();
    }
    // SAFETY: `err` is a non-null NSError; `-localizedDescription` returns an
    // NSString.
    unsafe {
        let desc: Retained<NSString> = msg_send![err, localizedDescription];
        desc.to_string()
    }
}

/// Register or unregister the app as a login item. Idempotent per call.
pub fn set_launch_at_login(enabled: bool) -> Result<(), PrefsError> {
    autoreleasepool(|_| {
        // SAFETY: `+[SMAppService mainAppService]` returns the shared service;
        // `register/unregisterAndReturnError:` return BOOL and write an
        // out-NSError on failure.
        unsafe {
            let svc: *mut AnyObject = msg_send![class!(SMAppService), mainAppService];
            if svc.is_null() {
                return Err(PrefsError::IoError("SMAppService unavailable".into()));
            }
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let ok: bool = if enabled {
                msg_send![svc, registerAndReturnError: &mut err]
            } else {
                msg_send![svc, unregisterAndReturnError: &mut err]
            };
            if ok {
                Ok(())
            } else {
                Err(PrefsError::IoError(error_description(err)))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the SMAppService FFI path. In a non-bundled test binary
    /// registration typically fails (returns an error) — the point is that the
    /// call resolves the class/selectors and returns a `Result` without panicking.
    #[test]
    fn set_launch_at_login_does_not_panic() {
        let _ = set_launch_at_login(false);
        let _ = set_launch_at_login(true);
        let _ = set_launch_at_login(false);
    }
}
