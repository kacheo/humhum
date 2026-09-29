//! macOS implementation of the [`Permissions`] trait.
//!
//! - **Microphone** — `+[AVCaptureDevice authorizationStatusForMediaType:]`
//!   (and `requestAccessForMediaType:completionHandler:` for the prompt).
//! - **Accessibility** — `AXIsProcessTrusted()` (ApplicationServices).
//! - **Input monitoring** — `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)`
//!   (IOKit). This is the purpose-built API; it has no side effects, unlike
//!   probing with a throwaway `CGEventTap`.

use std::process::Command;

use block2::RcBlock;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};

use super::{Permissions, PermissionsState};

/// `AVAuthorizationStatusAuthorized` — full access granted.
const AV_AUTH_STATUS_AUTHORIZED: isize = 3;
/// `kIOHIDRequestTypeListenEvent` — passive event listening (input monitoring).
const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
/// `kIOHIDAccessTypeGranted`.
const IOHID_ACCESS_TYPE_GRANTED: u32 = 0;

/// System Settings deep links (Privacy & Security panes).
const ACCESSIBILITY_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const INPUT_MONITORING_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    /// `AVMediaTypeAudio` — an `NSString *` constant identifying audio media.
    static AVMediaTypeAudio: &'static AnyObject;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    // Returns a CoreFoundation `Boolean` (`unsigned char`), NOT C `_Bool`.
    // Declared as `u8` because Rust `bool` is only valid for 0/1 — a stray
    // non-0/1 byte would be undefined behavior.
    fn AXIsProcessTrusted() -> u8;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
}

pub struct MacosPermissions;

impl MacosPermissions {
    fn microphone_granted() -> bool {
        // SAFETY: class method on the AVCaptureDevice class; AVMediaTypeAudio is
        // a framework-provided NSString constant. Returns an enum (isize).
        let status: isize = unsafe {
            msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio]
        };
        status == AV_AUTH_STATUS_AUTHORIZED
    }

    fn accessibility_granted() -> bool {
        // SAFETY: AXIsProcessTrusted takes no args and returns a CF Boolean (u8).
        unsafe { AXIsProcessTrusted() != 0 }
    }

    fn input_monitoring_granted() -> bool {
        // SAFETY: IOHIDCheckAccess takes a request-type enum and returns an
        // access-type enum; no pointers involved.
        let access = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
        access == IOHID_ACCESS_TYPE_GRANTED
    }

    fn open_pane(url: &str) {
        // Fire-and-forget; failure to launch System Settings is non-fatal but
        // worth a log line for debugging (e.g. if `open` is unavailable).
        if let Err(e) = Command::new("open").arg(url).spawn() {
            log::warn!("failed to open System Settings pane {url}: {e}");
        }
    }
}

impl Permissions for MacosPermissions {
    fn check(&self) -> PermissionsState {
        PermissionsState {
            microphone: Self::microphone_granted(),
            accessibility: Self::accessibility_granted(),
            input_monitoring: Self::input_monitoring_granted(),
        }
    }

    fn request_microphone(&self) -> Result<(), ()> {
        // Trigger the system mic prompt. The grant is observed later via
        // `check()` / the startup `permissions-changed` emit, so the completion
        // handler is a no-op — but the block must be non-null.
        let completion = RcBlock::new(|_granted: Bool| {});
        // SAFETY: class method; AVMediaTypeAudio is a valid NSString constant
        // and `completion` is a live block for the duration of the call.
        unsafe {
            let _: () = msg_send![
                class!(AVCaptureDevice),
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*completion,
            ];
        }
        Ok(())
    }

    fn open_accessibility_settings(&self) {
        Self::open_pane(ACCESSIBILITY_PANE);
    }

    fn open_input_monitoring_settings(&self) {
        Self::open_pane(INPUT_MONITORING_PANE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the full FFI path (class lookup, selectors, framework linking)
    /// on the macOS CI runner. Can't assert specific grants — the runner has no
    /// live permission state — only that the call returns without panicking.
    #[test]
    fn check_runs_without_panicking() {
        let state = MacosPermissions.check();
        // Touch each field so the whole struct is materialized.
        let _ = state.microphone && state.accessibility && state.input_monitoring;
    }

    /// Locks the System Settings deep-link URLs to the exact panes.
    #[test]
    fn settings_pane_urls() {
        assert_eq!(
            ACCESSIBILITY_PANE,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
        assert_eq!(
            INPUT_MONITORING_PANE,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        );
    }
}
