//! macOS CGEventTap implementation of [`HotkeyCapture`].
//!
//! A global push-to-talk hotkey: a `CGEventTap` runs on a dedicated thread's
//! `CFRunLoop` (LLD §148/§686). The C callback reduces each event to a
//! [`KeyEvent`] via the pure, unit-tested [`observe`]/[`classify`] in
//! `hotkey/mod.rs` — this module is the thin `unsafe` plumbing around that
//! tested logic. Requires the Input Monitoring grant (Phase 3); without it
//! `CGEventTapCreate` returns null and `register` reports `TapCreationFailed`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

use super::{
    classify, observe, AudioCue, HotkeyCapture, HotkeyCombo, HotkeyError, HotkeyEvent, Modifier,
};

type Handler = Box<dyn Fn(HotkeyEvent) + Send + Sync>;

// ---- CoreGraphics / CoreFoundation FFI ---------------------------------------

type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFStringRef = *const c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventTapCallBack =
    extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(
        mode: CFStringRef,
        seconds: f64,
        return_after_source_handled: bool,
    ) -> i32;
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: *const c_void);
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFRunLoopDefaultMode: CFStringRef;
}

// How long each `CFRunLoopRunInMode` slice blocks before the worker rechecks its
// stop flag. Small enough that teardown latency is imperceptible, large enough
// that the thread is effectively asleep between keystrokes.
const TEARDOWN_POLL_SECONDS: f64 = 0.25;

const KCG_SESSION_EVENT_TAP: u32 = 1;
const KCG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const KCG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;
const KCG_EVENT_KEY_DOWN: u32 = 10;
const KCG_EVENT_KEY_UP: u32 = 11;
const KCG_EVENT_FLAGS_CHANGED: u32 = 12;
const KCG_EVENT_TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const KCG_EVENT_TAP_DISABLED_USER_INPUT: u32 = 0xFFFF_FFFF;
const KCG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
const ESCAPE_KEYCODE: u16 = 53;

// The device-independent portion of CGEventFlags. Raw flags also carry
// device-dependent left/right bits (e.g. 0x20 left-Option, 0x40 right-Option)
// and a non-coalesced bit; mask them off before matching so a modifier-only
// hotkey compares cleanly. (kCGEventFlagMaskNonCoalesced etc. live in the low
// bits; the modifier flags we care about are all in 0xFFFF0000.)
const DEVICE_INDEPENDENT_FLAGS_MASK: u64 = 0xFFFF_0000;

// CGEventFlags modifier bits.
const FLAG_SHIFT: u64 = 1 << 17;
const FLAG_CONTROL: u64 = 1 << 18;
const FLAG_OPTION: u64 = 1 << 19;
const FLAG_COMMAND: u64 = 1 << 20;
const FLAG_FN: u64 = 1 << 23;

fn modifier_mask(modifiers: &[Modifier]) -> u64 {
    modifiers.iter().fold(0, |mask, m| {
        mask | match m {
            Modifier::Shift => FLAG_SHIFT,
            Modifier::Ctrl => FLAG_CONTROL,
            Modifier::Option => FLAG_OPTION,
            Modifier::Cmd => FLAG_COMMAND,
            Modifier::Fn => FLAG_FN,
        }
    })
}

/// US-ANSI virtual keycode for a key name, or `None` if unsupported.
fn keycode_for(key: &str) -> Option<u16> {
    let code = match key {
        "A" => 0,
        "B" => 11,
        "C" => 8,
        "D" => 2,
        "E" => 14,
        "F" => 3,
        "G" => 5,
        "H" => 4,
        "I" => 34,
        "J" => 38,
        "K" => 40,
        "L" => 37,
        "M" => 46,
        "N" => 45,
        "O" => 31,
        "P" => 35,
        "Q" => 12,
        "R" => 15,
        "S" => 1,
        "T" => 17,
        "U" => 32,
        "V" => 9,
        "W" => 13,
        "X" => 7,
        "Y" => 16,
        "Z" => 6,
        "0" => 29,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "5" => 23,
        "6" => 22,
        "7" => 26,
        "8" => 28,
        "9" => 25,
        "Space" => 49,
        "Return" | "Enter" => 36,
        "Tab" => 48,
        "Escape" => 53,
        "Delete" => 51,
        "Left" => 123,
        "Right" => 124,
        "Down" => 125,
        "Up" => 126,
        "F1" => 122,
        "F2" => 120,
        "F3" => 99,
        "F4" => 118,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F8" => 100,
        "F9" => 101,
        "F10" => 109,
        "F11" => 103,
        "F12" => 111,
        _ => return None,
    };
    Some(code)
}

// ---- Tap context (shared with the C callback) --------------------------------

struct TapContext {
    handler: Arc<Mutex<Option<Handler>>>,
    is_holding: Arc<AtomicBool>,
    /// `None` for a modifier-only combo (no target key; driven by flagsChanged).
    target_keycode: Option<u16>,
    target_mask: u64,
    /// The tap port, stored after creation so the callback can re-enable it if
    /// macOS disables the tap (timeout / user input).
    tap: AtomicPtr<c_void>,
}

extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    etype: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    // SAFETY: `user` is the `Box<TapContext>` leaked in `register`, valid for the
    // tap's lifetime; the callback only ever runs while the tap is installed.
    let ctx = unsafe { &*(user as *const TapContext) };

    if etype == KCG_EVENT_TAP_DISABLED_TIMEOUT || etype == KCG_EVENT_TAP_DISABLED_USER_INPUT {
        let tap = ctx.tap.load(Ordering::Acquire);
        if !tap.is_null() {
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }

    let keycode = unsafe { CGEventGetIntegerValueField(event, KCG_KEYBOARD_EVENT_KEYCODE) } as u16;
    let raw_flags = unsafe { CGEventGetFlags(event) };
    // Drop device-dependent (left/right) and non-coalesced bits before matching.
    let flags = raw_flags & DEVICE_INDEPENDENT_FLAGS_MASK;
    let observed = observe(
        etype == KCG_EVENT_KEY_DOWN,
        etype == KCG_EVENT_FLAGS_CHANGED,
        keycode,
        flags,
        ctx.target_keycode,
        ctx.target_mask,
        ESCAPE_KEYCODE,
    );
    let decision = classify(observed, ctx.is_holding.load(Ordering::Acquire));
    // Diagnostic (PR #34): trace every keyboard event the tap receives so we can
    // see whether the tap fires at all and what it decides. DEBUG-ONLY: this logs
    // a keycode for every keystroke system-wide, so it must never be compiled into
    // a release build that writes to a persistent log file.
    #[cfg(debug_assertions)]
    {
        log::info!(
            "hotkey tap: etype={etype} keycode={keycode} raw_flags={raw_flags:#x} flags={flags:#x} \
             target_keycode={:?} target_mask={:#x} observed={observed:?} emit={:?} swallow={}",
            ctx.target_keycode,
            ctx.target_mask,
            decision.event,
            decision.swallow,
        );
    }
    ctx.is_holding.store(decision.holding, Ordering::Release);
    if let Some(ev) = decision.event {
        if let Ok(guard) = ctx.handler.lock() {
            if let Some(h) = guard.as_ref() {
                h(ev);
            }
        }
    }
    if decision.swallow {
        std::ptr::null_mut()
    } else {
        event
    }
}

// ---- Raw-pointer wrapper so the run loop ref can cross threads ----------------

/// A `CFRunLoopRef` handle. `CFRunLoopStop` is documented thread-safe, so it is
/// sound to hold this on one thread and stop the loop running on another.
struct SendableRunLoop(CFRunLoopRef);
// SAFETY: see the type doc — CFRunLoop control APIs are thread-safe.
unsafe impl Send for SendableRunLoop {}

struct TapHandles {
    run_loop: SendableRunLoop,
    thread: JoinHandle<()>,
    /// Set by `unregister` to break the worker's poll loop. This is the
    /// authoritative stop signal — `CFRunLoopStop` only *wakes* the loop early;
    /// the flag is what guarantees the worker exits even if the stop raced ahead
    /// of the run loop actually starting.
    stop: Arc<AtomicBool>,
}

pub struct MacosHotkeyCapture {
    handler: Arc<Mutex<Option<Handler>>>,
    is_holding: Arc<AtomicBool>,
    handles: Mutex<Option<TapHandles>>,
}

impl MacosHotkeyCapture {
    fn new() -> Self {
        Self {
            handler: Arc::new(Mutex::new(None)),
            is_holding: Arc::new(AtomicBool::new(false)),
            handles: Mutex::new(None),
        }
    }
}

impl HotkeyCapture for MacosHotkeyCapture {
    fn register(&self, combo: &HotkeyCombo) -> Result<(), HotkeyError> {
        // Modifier-only combos have no target key — they fire on flagsChanged.
        let keycode = if combo.is_modifier_only() {
            None
        } else {
            Some(keycode_for(&combo.key).ok_or_else(|| {
                HotkeyError::InvalidCombo(format!("unsupported key '{}'", combo.key))
            })?)
        };
        let mask = modifier_mask(&combo.modifiers);

        // Not idempotent: tear down any prior registration first. Otherwise the
        // old run-loop thread would be detached (its tap left installed and still
        // swallowing events) when `self.handles` is overwritten below.
        self.unregister()?;

        let handler = Arc::clone(&self.handler);
        let is_holding = Arc::clone(&self.is_holding);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let (tx, rx) = mpsc::channel::<Result<SendableRunLoop, HotkeyError>>();

        let thread = std::thread::spawn(move || {
            let ctx = Box::new(TapContext {
                handler,
                is_holding,
                target_keycode: keycode,
                target_mask: mask,
                tap: AtomicPtr::new(std::ptr::null_mut()),
            });
            let ctx_ptr = Box::into_raw(ctx);

            let events = (1u64 << KCG_EVENT_KEY_DOWN)
                | (1u64 << KCG_EVENT_KEY_UP)
                | (1u64 << KCG_EVENT_FLAGS_CHANGED);

            // SAFETY: standard CGEventTap setup; ctx_ptr is a live Box leaked for
            // the tap's lifetime and reclaimed below after the run loop exits.
            let tap = unsafe {
                CGEventTapCreate(
                    KCG_SESSION_EVENT_TAP,
                    KCG_HEAD_INSERT_EVENT_TAP,
                    KCG_EVENT_TAP_OPTION_DEFAULT,
                    events,
                    tap_callback,
                    ctx_ptr as *mut c_void,
                )
            };
            if tap.is_null() {
                log::warn!(
                    "hotkey: CGEventTapCreate returned null — the Accessibility grant is not in \
                     effect for this binary (an active/swallowing tap needs Accessibility, not \
                     just Input Monitoring; in `tauri dev` a rebuild can invalidate the grant)"
                );
                let _ = tx.send(Err(HotkeyError::TapCreationFailed));
                // Reclaim the leaked context; the tap never took ownership.
                drop(unsafe { Box::from_raw(ctx_ptr) });
                return;
            }
            // Record the tap for re-enable from the callback.
            unsafe { (*ctx_ptr).tap.store(tap, Ordering::Release) };

            let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
            let run_loop = unsafe { CFRunLoopGetCurrent() };
            unsafe {
                CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);
            }
            let _ = tx.send(Ok(SendableRunLoop(run_loop)));

            // Drive the tap in bounded slices instead of a single blocking
            // `CFRunLoopRun()`. `unregister` sets `stop` and calls
            // `CFRunLoopStop` to wake us promptly — but a `CFRunLoopStop` that
            // arrives before the loop has actually started running is a silent
            // no-op, which with `CFRunLoopRun()` would wedge the thread forever
            // (no events ever arrive to end the run). Rechecking `stop` after
            // each bounded `CFRunLoopRunInMode` makes teardown race-free: worst
            // case we exit one poll interval late, never indefinitely.
            while !stop_worker.load(Ordering::Acquire) {
                unsafe {
                    CFRunLoopRunInMode(kCFRunLoopDefaultMode, TEARDOWN_POLL_SECONDS, false);
                }
            }

            // Teardown.
            unsafe {
                CFRelease(source);
                CFRelease(tap);
            }
            drop(unsafe { Box::from_raw(ctx_ptr) });
        });

        match rx.recv() {
            Ok(Ok(run_loop)) => {
                log::info!(
                    "hotkey: registered {} combo (target_keycode={keycode:?}, mask={mask:#x}); \
                     tap installed and enabled",
                    if combo.is_modifier_only() {
                        "modifier-only"
                    } else {
                        "keyed"
                    },
                );
                *self.handles.lock().unwrap() = Some(TapHandles {
                    run_loop,
                    thread,
                    stop,
                });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(HotkeyError::TapCreationFailed)
            }
        }
    }

    fn unregister(&self) -> Result<(), HotkeyError> {
        if let Some(handles) = self.handles.lock().unwrap().take() {
            // Set the stop flag FIRST so the worker's poll loop exits on its next
            // check even if the CFRunLoopStop below no-ops (loop not yet running).
            handles.stop.store(true, Ordering::Release);
            // SAFETY: CFRunLoopStop is thread-safe; wakes the worker's in-flight
            // CFRunLoopRunInMode so it rechecks the flag and exits promptly rather
            // than waiting out the poll interval.
            unsafe { CFRunLoopStop(handles.run_loop.0) };
            let _ = handles.thread.join();
        }
        self.is_holding.store(false, Ordering::Release);
        Ok(())
    }

    fn set_handler(&self, handler: Handler) {
        *self.handler.lock().unwrap() = Some(handler);
    }

    fn validate(&self, combo: &HotkeyCombo) -> Result<(), HotkeyError> {
        super::check_conflicts(combo)
    }

    fn play_cue(&self, cue: AudioCue) {
        let name = match cue {
            AudioCue::Start => "Tink",
            AudioCue::Stop => "Pop",
        };
        play_system_sound(name);
    }
}

fn play_system_sound(name: &str) {
    // `+[NSSound soundNamed:]` returns an autoreleased object; drain it via a
    // local pool since this may run off the main thread.
    autoreleasepool(|_| {
        let ns_name: Retained<NSString> = NSString::from_str(name);
        // SAFETY: `+[NSSound soundNamed:]` takes an NSString and returns a
        // possibly-null NSSound; `-play` returns BOOL. AppKit is linked by the
        // app bundle.
        unsafe {
            let sound: *mut AnyObject = msg_send![class!(NSSound), soundNamed: &*ns_name];
            if !sound.is_null() {
                let _: bool = msg_send![sound, play];
            }
        }
    });
}

pub fn create() -> Box<dyn HotkeyCapture> {
    Box::new(MacosHotkeyCapture::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_mask_combines_bits() {
        assert_eq!(
            modifier_mask(&[Modifier::Option, Modifier::Shift]),
            FLAG_OPTION | FLAG_SHIFT
        );
        assert_eq!(modifier_mask(&[Modifier::Fn]), FLAG_FN);
        assert_eq!(modifier_mask(&[]), 0);
    }

    #[test]
    fn device_independent_mask_strips_left_right_bits_keeps_modifier() {
        // Raw Option flags carry a device-dependent left/right bit (e.g. 0x20)
        // plus the non-coalesced bit (0x100); masking must keep FLAG_OPTION and
        // drop the rest so a modifier-only match compares cleanly.
        let raw_left_option = FLAG_OPTION | 0x20 | 0x100;
        assert_eq!(raw_left_option & DEVICE_INDEPENDENT_FLAGS_MASK, FLAG_OPTION);
        // A pure modifier combination survives the mask unchanged.
        let raw_opt_shift = FLAG_OPTION | FLAG_SHIFT | 0x40;
        assert_eq!(
            raw_opt_shift & DEVICE_INDEPENDENT_FLAGS_MASK,
            FLAG_OPTION | FLAG_SHIFT
        );
    }

    #[test]
    fn keycode_for_known_and_unknown_keys() {
        assert_eq!(keycode_for("Space"), Some(49));
        assert_eq!(keycode_for("A"), Some(0));
        assert_eq!(keycode_for("Escape"), Some(53));
        assert_eq!(keycode_for("Up"), Some(126));
        assert_eq!(keycode_for("F5"), Some(96));
        assert_eq!(keycode_for("Nonsense"), None);
    }

    #[test]
    fn register_unsupported_key_is_invalid_combo() {
        let cap = MacosHotkeyCapture::new();
        let combo = HotkeyCombo {
            key: "Nonsense".into(),
            modifiers: vec![Modifier::Option],
        };
        assert!(matches!(
            cap.register(&combo),
            Err(HotkeyError::InvalidCombo(_))
        ));
    }

    /// A modifier-only combo (empty key) must not be rejected as an invalid key —
    /// it registers via flagsChanged. Either grant state is acceptable; the point
    /// is that the empty key does not surface as `InvalidCombo`.
    #[test]
    fn register_modifier_only_combo_is_well_formed() {
        let cap = MacosHotkeyCapture::new();
        let combo = HotkeyCombo {
            key: String::new(),
            modifiers: vec![Modifier::Option],
        };
        match cap.register(&combo) {
            Ok(()) => assert!(cap.unregister().is_ok()),
            Err(e) => assert!(matches!(
                e,
                HotkeyError::TapCreationFailed | HotkeyError::PermissionDenied
            )),
        }
    }

    /// Exercises the real CGEventTap FFI path. Without an Input Monitoring grant
    /// (the CI runner) `register` returns `TapCreationFailed`; if the dev machine
    /// has granted it, `register` succeeds and we immediately tear down. Either
    /// way the call must not panic and `unregister` must be clean.
    #[test]
    fn register_then_unregister_is_well_formed() {
        let cap = MacosHotkeyCapture::new();
        let combo = HotkeyCombo {
            key: "Space".into(),
            modifiers: vec![Modifier::Option, Modifier::Shift],
        };
        match cap.register(&combo) {
            Ok(()) => assert!(cap.unregister().is_ok()),
            Err(e) => assert!(matches!(
                e,
                HotkeyError::TapCreationFailed | HotkeyError::PermissionDenied
            )),
        }
    }

    #[test]
    fn play_cue_does_not_panic() {
        let cap = MacosHotkeyCapture::new();
        cap.play_cue(AudioCue::Start);
        cap.play_cue(AudioCue::Stop);
    }

    /// Re-registering must tear down the prior tap, not leak it. Either grant
    /// state is acceptable; the point is that a second `register` + `unregister`
    /// stays well-formed and doesn't panic or hang.
    #[test]
    fn re_register_is_well_formed() {
        let cap = MacosHotkeyCapture::new();
        let combo = HotkeyCombo {
            key: "Space".into(),
            modifiers: vec![Modifier::Option, Modifier::Shift],
        };
        let _ = cap.register(&combo);
        let _ = cap.register(&combo); // must tear down the first internally
        assert!(cap.unregister().is_ok());
    }
}
