//! CoreAudio-based Bluetooth-avoidance for the recording input device.
//!
//! Opening a Bluetooth headset's microphone forces macOS to switch it from
//! the high-quality A2DP profile to the low-quality bidirectional HFP
//! profile, audibly changing/raising whatever audio the user is currently
//! listening to through it. `run_capture` (`audio/mod.rs`) used to always
//! grab `cpal::default_host().default_input_device()` unconditionally, which
//! triggers this the moment a Bluetooth headset happens to be the system
//! default input. This module detects that case via real CoreAudio
//! transport-type queries (not device-name guessing) and offers a
//! non-Bluetooth alternative instead.
//!
//! `cpal::Device`'s underlying `AudioDeviceID` is private to the `cpal`
//! crate, so device selection here is done entirely with raw CoreAudio calls
//! and only reconnected to a `cpal::Device` by name in `audio/mod.rs`.
#![cfg(target_os = "macos")]

use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;

#[cfg(test)]
use coreaudio_sys::kAudioDeviceTransportTypeUSB;
use coreaudio_sys::{
    kAudioDevicePropertyStreams, kAudioDevicePropertyTransportType,
    kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBluetoothLE,
    kAudioDeviceTransportTypeBuiltIn, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain, kAudioObjectPropertyName,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput, kAudioObjectSystemObject,
    kCFStringEncodingUTF8, AudioDeviceID, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectPropertyAddress, CFRelease, CFStringGetCString,
    CFStringRef,
};

/// Sentinel `preferred_input_device` pref value meaning "always use whatever
/// macOS reports as the default input" (the pre-fix behavior, Bluetooth
/// included). Must match `SYSTEM_DEFAULT_INPUT_DEVICE` in `src/lib/types.ts`.
pub const SYSTEM_DEFAULT_SENTINEL: &str = "system-default";

/// Resolves which device *name* `run_capture` should ask cpal to open in
/// place of its own `default_input_device()`.
///
/// - `Some(SYSTEM_DEFAULT_SENTINEL)` -> `None` (no override; always use
///   cpal's own default, Bluetooth included).
/// - `Some(name)` (anything else) -> `Some(name)` unchanged, an explicit
///   user override trusted as-is (the caller falls back to cpal's default if
///   no connected cpal device actually has this name).
/// - `None` -> auto mode: if the system default input is Bluetooth and a
///   non-Bluetooth input-capable device exists, returns that device's name
///   (built-in preferred); otherwise `None` (nothing to avoid).
pub fn resolve_input_device_name(preferred: Option<&str>) -> Option<String> {
    match preferred {
        Some(SYSTEM_DEFAULT_SENTINEL) => None,
        Some(name) => Some(name.to_string()),
        None => auto_avoid_bluetooth(),
    }
}

/// All input-capable device names, default-input-device first, for the
/// Preferences "Input device" dropdown. Best-effort: any CoreAudio failure
/// yields an empty or partial list rather than propagating an error — a
/// broken device list must never block the rest of Preferences from
/// rendering.
pub fn list_input_device_names() -> Vec<String> {
    let default_id = default_input_device_id();
    let mut devices = input_capable_devices();
    devices.sort_by_key(|d| u8::from(Some(d.id) != default_id));
    devices.into_iter().map(|d| d.name).collect()
}

fn auto_avoid_bluetooth() -> Option<String> {
    let default_id = default_input_device_id()?;
    choose_alternative(device_transport_type(default_id), &input_capable_devices())
}

/// Pure decision logic: given the current default input's transport type
/// (if known) and the full candidate list, decide whether to avoid it and
/// what to use instead. No CoreAudio calls — fully unit-testable.
fn choose_alternative(default_transport: Option<u32>, candidates: &[DeviceInfo]) -> Option<String> {
    let default_transport = default_transport?;
    if !is_bluetooth(default_transport) {
        return None; // default input isn't Bluetooth — nothing to avoid
    }
    candidates
        .iter()
        .filter(|d| !is_bluetooth(d.transport_type))
        .min_by_key(|d| u8::from(d.transport_type != kAudioDeviceTransportTypeBuiltIn))
        .map(|d| d.name.clone())
}

fn is_bluetooth(transport: u32) -> bool {
    transport == kAudioDeviceTransportTypeBluetooth
        || transport == kAudioDeviceTransportTypeBluetoothLE
}

#[derive(Debug, Clone)]
struct DeviceInfo {
    id: AudioDeviceID,
    name: String,
    transport_type: u32,
}

fn input_capable_devices() -> Vec<DeviceInfo> {
    all_device_ids()
        .into_iter()
        .filter(|&id| device_has_input_streams(id))
        .filter_map(|id| {
            Some(DeviceInfo {
                id,
                name: device_name(id)?,
                transport_type: device_transport_type(id)?,
            })
        })
        .collect()
}

/// Fetches a fixed-size CoreAudio scalar property (anything with a known,
/// constant `size_of::<T>()`, e.g. an `AudioDeviceID` or a `u32` transport
/// type) from `object_id`. Returns `None` on any CoreAudio error.
fn get_property<T: Copy>(object_id: AudioDeviceID, selector: u32, scope: u32) -> Option<T> {
    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = std::mem::size_of::<T>() as u32;
    let mut value: std::mem::MaybeUninit<T> = std::mem::MaybeUninit::uninit();
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            &address,
            0,
            ptr::null(),
            &mut size,
            value.as_mut_ptr() as *mut _,
        )
    };
    if status != 0 {
        return None;
    }
    Some(unsafe { value.assume_init() })
}

fn all_device_ids() -> Vec<AudioDeviceID> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject,
            &address,
            0,
            ptr::null(),
            &mut size,
        )
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let count = size as usize / std::mem::size_of::<AudioDeviceID>();
    let mut ids: Vec<AudioDeviceID> = vec![0; count];
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &address,
            0,
            ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut _,
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids
}

fn default_input_device_id() -> Option<AudioDeviceID> {
    get_property(
        kAudioObjectSystemObject,
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal,
    )
}

fn device_transport_type(id: AudioDeviceID) -> Option<u32> {
    get_property(
        id,
        kAudioDevicePropertyTransportType,
        kAudioObjectPropertyScopeGlobal,
    )
}

/// Whether `id` has at least one input stream — excludes output-only devices
/// (e.g. some Bluetooth speakers with no mic, or output-only aggregate
/// devices) from the candidate list.
fn device_has_input_streams(id: AudioDeviceID) -> bool {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyStreams,
        mScope: kAudioObjectPropertyScopeInput,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size: u32 = 0;
    let status = unsafe { AudioObjectGetPropertyDataSize(id, &address, 0, ptr::null(), &mut size) };
    status == 0 && size > 0
}

/// Reads a device's display name via `kAudioObjectPropertyName`. Mirrors
/// `coreaudio-rs`'s own `get_device_name` (`audio_unit/macos_helpers.rs`): a
/// `CFStringRef` "get" property is retained on the caller's behalf per
/// CoreFoundation's Get Rule, so it's `CFRelease`d here after copying into an
/// owned `String`.
fn device_name(id: AudioDeviceID) -> Option<String> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioObjectPropertyName,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    let mut cf_string: CFStringRef = ptr::null();
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            &mut cf_string as *mut _ as *mut _,
        )
    };
    if status != 0 || cf_string.is_null() {
        return None;
    }

    // 512 bytes of headroom beyond typical device names — some third-party
    // audio interfaces embed serial numbers or long descriptors in their
    // CoreAudio name.
    let mut buf = [0 as c_char; 512];
    let ok = unsafe {
        CFStringGetCString(
            cf_string,
            buf.as_mut_ptr(),
            buf.len() as _,
            kCFStringEncodingUTF8,
        )
    };
    let name = if ok != 0 {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };
    unsafe { CFRelease(cf_string as *const _) };

    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure logic, no CoreAudio call — sentinel always means "no override",
    /// independent of what hardware is connected.
    #[test]
    fn resolve_input_device_name_sentinel_returns_none() {
        assert_eq!(
            resolve_input_device_name(Some(SYSTEM_DEFAULT_SENTINEL)),
            None
        );
    }

    /// Pure logic, no CoreAudio call — an explicit non-sentinel preference
    /// passes through unchanged.
    #[test]
    fn resolve_input_device_name_explicit_override_passes_through() {
        assert_eq!(
            resolve_input_device_name(Some("AirPods Pro")),
            Some("AirPods Pro".to_string())
        );
    }

    /// Touches the live CoreAudio HAL; must never panic regardless of what's
    /// actually connected (headless CI has no input device at all).
    #[test]
    fn resolve_input_device_name_auto_never_panics() {
        let _ = resolve_input_device_name(None);
    }

    #[test]
    fn list_input_device_names_never_panics() {
        let _ = list_input_device_names();
    }

    fn device(id: AudioDeviceID, name: &str, transport_type: u32) -> DeviceInfo {
        DeviceInfo {
            id,
            name: name.to_string(),
            transport_type,
        }
    }

    #[test]
    fn prefers_builtin_over_other_non_bluetooth_when_default_is_bluetooth() {
        let candidates = vec![
            device(1, "AirPods Pro", kAudioDeviceTransportTypeBluetooth),
            device(2, "USB Mic", kAudioDeviceTransportTypeUSB),
            device(
                3,
                "MacBook Pro Microphone",
                kAudioDeviceTransportTypeBuiltIn,
            ),
        ];
        assert_eq!(
            choose_alternative(Some(kAudioDeviceTransportTypeBluetooth), &candidates),
            Some("MacBook Pro Microphone".to_string())
        );
    }

    #[test]
    fn returns_none_when_default_is_not_bluetooth() {
        let candidates = vec![device(1, "Built-in", kAudioDeviceTransportTypeBuiltIn)];
        assert_eq!(
            choose_alternative(Some(kAudioDeviceTransportTypeBuiltIn), &candidates),
            None
        );
    }

    #[test]
    fn returns_none_when_bluetooth_default_has_no_alternative() {
        let candidates = vec![device(1, "AirPods", kAudioDeviceTransportTypeBluetooth)];
        assert_eq!(
            choose_alternative(Some(kAudioDeviceTransportTypeBluetooth), &candidates),
            None
        );
    }

    #[test]
    fn bluetooth_le_default_is_also_avoided() {
        let candidates = vec![device(1, "USB Mic", kAudioDeviceTransportTypeUSB)];
        assert_eq!(
            choose_alternative(Some(kAudioDeviceTransportTypeBluetoothLE), &candidates),
            Some("USB Mic".to_string())
        );
    }
}
