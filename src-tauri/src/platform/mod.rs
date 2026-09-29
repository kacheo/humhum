//! Platform factory aggregation (LLD §10 / §808).
//!
//! One import point so `lib.rs` / [`crate::state::AppState`] construct the
//! platform trait objects without reaching into each module's `macos.rs`.
//! `audio` and `permissions` are cross-platform (CPAL / cfg-dispatched in their
//! own modules); `hotkey` and `paste` are macOS-only and dispatched here.

use crate::audio::AudioCapture;
use crate::hotkey::HotkeyCapture;
use crate::paste::TextPaste;
use crate::permissions::Permissions;

/// Global hotkey capture (macOS: CGEventTap).
#[cfg(target_os = "macos")]
pub fn create_hotkey() -> Box<dyn HotkeyCapture> {
    crate::hotkey::macos::create()
}

/// Text injection at the cursor (macOS: AXUIElement + clipboard fallback).
#[cfg(target_os = "macos")]
pub fn create_paste() -> Box<dyn TextPaste> {
    crate::paste::macos::create()
}

/// Audio capture — cross-platform via CPAL.
pub fn create_audio() -> Box<dyn AudioCapture> {
    crate::audio::create()
}

/// Permission checking — cfg-dispatched inside the module.
pub fn create_permissions() -> Box<dyn Permissions> {
    crate::permissions::create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factories_return_live_trait_objects() {
        // Smoke: each factory constructs without panicking. (Behavior is covered
        // in each module's own tests; here we only prove the aggregation wires.)
        let _audio: Box<dyn AudioCapture> = create_audio();
        let _perms: Box<dyn Permissions> = create_permissions();
        #[cfg(target_os = "macos")]
        {
            let _hotkey: Box<dyn HotkeyCapture> = create_hotkey();
            let _paste: Box<dyn TextPaste> = create_paste();
        }
    }
}
