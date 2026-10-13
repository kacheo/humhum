//! macOS AXUIElement text injection with clipboard + synthesized-Cmd+V fallback.
//!
//! Thin `unsafe` plumbing around the pure decision seam in `super` (LLD §3.5).
//! Primary path inserts at the caret via `kAXSelectedTextAttribute`, falling back
//! to `kAXValueAttribute`; if neither is settable (sandboxed / read-only app) the
//! text is copied to `NSPasteboard` and a Cmd+V keystroke is synthesized. Runs on
//! `spawn_blocking` because AX calls may block (LLD §704). Requires the
//! Accessibility grant (Phase 3); without it `AXIsProcessTrusted()` is false and
//! `paste` returns `AccessibilityDenied`.

use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSNumber, NSString};

use super::{
    decide, is_continuation, lead_space, refine_continuation, result_for, should_record, AxProbe,
    CaretContext, LastPaste, PasteAction, PasteError, TextPaste,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---- Accessibility / CoreGraphics / CoreFoundation FFI -----------------------

type AXUIElementRef = *mut c_void;
type AXValueRef = *const c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;
type AXError = i32;

/// `CFRange` — `{ CFIndex location; CFIndex length; }`, `CFIndex` being a signed
/// `long` (`isize`). The shape AX wraps for `AXSelectedTextRange` /
/// `AXStringForRange`.
#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

const AX_SUCCESS: AXError = 0;
/// `kAXValueCFRangeType` — the `AXValueType` tag for a wrapped `CFRange`.
const AX_VALUE_CFRANGE_TYPE: u32 = 4;
/// `kCGEventSourceStateHIDSystemState`.
const CG_EVENT_SOURCE_HID: i32 = 1;
/// `kCGHIDEventTap` — post into the HID stream (same level as a real keypress).
const CG_HID_EVENT_TAP: u32 = 0;
/// US-ANSI virtual keycode for "V" (matches hotkey::macos::keycode_for).
const V_KEYCODE: u16 = 9;
/// US-ANSI virtual keycode for the left Command key.
const CMD_KEYCODE: u16 = 55;
/// `kCGEventFlagMaskCommand`.
const FLAG_COMMAND: u64 = 1 << 20;

/// Let the pasteboard write settle before synthesizing Cmd+V.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(20);
/// Give the target time to consume the paste before we restore the clipboard.
const PASTE_CONSUME: Duration = Duration::from_millis(250);

// AX attribute names are CFStrings; we build them from literals (toll-free
// bridged via NSString) rather than linking the framework constant symbols.
const ATTR_FOCUSED_UI_ELEMENT: &str = "AXFocusedUIElement";
const ATTR_SELECTED_TEXT: &str = "AXSelectedText";
/// The focused element's whole text. Read before/after the `AXSelectedText` set to
/// confirm the insert actually landed — Chromium/Electron contenteditable editors
/// report `AXSelectedText` settable and return success but silently no-op the write.
const ATTR_VALUE: &str = "AXValue";
/// Caret/selection as a `CFRange` (location = caret offset when length is 0).
const ATTR_SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";
/// Parameterized attribute: the string for a given `CFRange`. Used to read the
/// single character just before the caret without pulling the whole field value.
const ATTR_STRING_FOR_RANGE: &str = "AXStringForRange";
/// Chromium/Electron expose their a11y tree only after this is set on the app
/// element; setting it turns the system-wide focused-element query into something
/// that resolves to a settable text field (native caret insert, no clipboard).
const ATTR_MANUAL_ACCESSIBILITY: &str = "AXManualAccessibility";
/// `NSPasteboardTypeString`'s UTI value.
const PASTEBOARD_TYPE_STRING: &str = "public.utf8-plain-text";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut u8,
    ) -> AXError;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXValueGetValue(value: AXValueRef, the_type: u32, value_ptr: *mut c_void) -> u8;
    fn AXValueCreate(the_type: u32, value_ptr: *const c_void) -> AXValueRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state_id: i32) -> CGEventSourceRef;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        keycode: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    /// `CFHashCode CFHash(CFTypeRef cf)` — a hash that's guaranteed to agree
    /// wherever `CFEqual` would consider two objects equal. Used as a cheap,
    /// `Copy`-friendly proxy for AX element identity (see `refine_continuation`)
    /// without retaining the live `AXUIElementRef` across the paste-to-paste gap.
    fn CFHash(cf: CFTypeRef) -> std::os::raw::c_ulong;
}

/// `&NSString` as a `CFStringRef` arg (NSString is toll-free bridged to CFString).
fn cfstr(s: &Retained<NSString>) -> CFStringRef {
    (&**s) as *const NSString as CFStringRef
}

// ---- Primary AX path ---------------------------------------------------------

/// Read the character immediately before the caret of `element`, to decide
/// whether a new dictation needs a leading space (see `super::lead_space`).
/// Best-effort: any AX hiccup → falls back to [`unknown_or_at_start`].
fn read_caret_context(element: AXUIElementRef) -> CaretContext {
    autoreleasepool(|_| {
        // 1. Caret/selection range. `AXSelectedTextRange` yields an AXValue
        //    wrapping a CFRange; unwrap it to the raw struct.
        let range_attr = NSString::from_str(ATTR_SELECTED_TEXT_RANGE);
        let mut range_value: CFTypeRef = std::ptr::null();
        // SAFETY: valid element + attribute; `range_value` receives a +1 copy on success.
        let err =
            unsafe { AXUIElementCopyAttributeValue(element, cfstr(&range_attr), &mut range_value) };
        if err != AX_SUCCESS || range_value.is_null() {
            return unknown_or_at_start(element);
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        // SAFETY: `range_value` is a live AXValue; write the CFRange into `range`.
        let got = unsafe {
            AXValueGetValue(
                range_value as AXValueRef,
                AX_VALUE_CFRANGE_TYPE,
                &mut range as *mut CFRange as *mut c_void,
            )
        };
        // SAFETY: release the +1 range copy.
        unsafe { CFRelease(range_value) };
        if got == 0 {
            return unknown_or_at_start(element);
        }
        // 2. Nothing before the caret → no separator wanted.
        if range.location <= 0 {
            return CaretContext::AtStart;
        }

        // 3. Read the single character at [location-1, 1] via the parameterized
        //    `AXStringForRange` (one char, not the whole field).
        //    NB: AX ranges are in UTF-16 code units, so for a non-BMP char before
        //    the caret (emoji, astral CJK) this reads a lone surrogate half. That's
        //    spacing-safe: `AXStringForRange` returns it as U+FFFD (non-whitespace →
        //    space added) or empty (→ Unknown, and since a char is known to precede
        //    the caret the field can't be empty, so this still resolves to Unknown →
        //    space added), and `to_string()` is lossy, never panics. Don't "fix" this
        //    into a scalar-aware read that could choke on surrogates — the outcome is
        //    already correct.
        let one_before = CFRange {
            location: range.location - 1,
            length: 1,
        };
        // SAFETY: returns a +1 AXValue wrapping the CFRange (or null).
        let param = unsafe {
            AXValueCreate(
                AX_VALUE_CFRANGE_TYPE,
                &one_before as *const CFRange as *const c_void,
            )
        };
        if param.is_null() {
            return unknown_or_at_start(element);
        }
        let str_attr = NSString::from_str(ATTR_STRING_FOR_RANGE);
        let mut str_value: CFTypeRef = std::ptr::null();
        // SAFETY: valid element + attribute + parameter; `str_value` receives a +1 CFString.
        let s_err = unsafe {
            AXUIElementCopyParameterizedAttributeValue(
                element,
                cfstr(&str_attr),
                param as CFTypeRef,
                &mut str_value,
            )
        };
        // SAFETY: release the +1 parameter AXValue.
        unsafe { CFRelease(param) };
        if s_err != AX_SUCCESS || str_value.is_null() {
            return unknown_or_at_start(element);
        }
        // CFString is toll-free bridged to NSString.
        // SAFETY: `str_value` is a live CFString for the duration of this read.
        let s = unsafe { (*(str_value as *const NSString)).to_string() };
        // SAFETY: release the +1 string copy.
        unsafe { CFRelease(str_value) };
        match s.chars().next() {
            Some(c) => CaretContext::PrecededBy(c),
            None => unknown_or_at_start(element),
        }
    })
}

/// When the caret-relative read fails, fall back to asking whether the whole
/// field is empty via `AXValue` — the same attribute `ax_insert` already uses
/// to verify Chromium/Electron inserts (see its before/after check), so it's
/// known to be readable on these controls even when the parameterized
/// caret-range query isn't. An empty field is unambiguously `AtStart`
/// regardless of the `continuation` heuristic: this is what stops a stray
/// leading space when e.g. Signal clears its composer after sending a
/// message and the next dictation lands within the continuation window. A
/// non-empty field (or an unreadable `AXValue`) still can't tell us where the
/// caret is, so it stays `Unknown` and defers to `continuation` as before.
fn unknown_or_at_start(element: AXUIElementRef) -> CaretContext {
    match read_string_attr(element, ATTR_VALUE) {
        Some(s) if s.is_empty() => CaretContext::AtStart,
        _ => CaretContext::Unknown,
    }
}

/// Try to insert `text` at the caret of the focused UI element, prepending a
/// leading space first when the caret context calls for one (so a new dictation
/// doesn't butt against existing text). `continuation` is consulted only when the
/// caret can't be read (`Unknown`) — see `super::lead_space` — and is first
/// narrowed by `refine_continuation` using `last_element_hash` against the
/// newly focused element's `CFHash`, when both are available. Returns the
/// resulting `AxProbe`, whether a space was prepended (so the clipboard
/// fallback in `paste_blocking` can apply the same decision), and the focused
/// element's `CFHash` (`None` if there was none) for the caller to persist as
/// the next `LastPaste.element_hash`. No Tauri; the only side effect is the AX
/// set.
fn ax_insert(
    text: &str,
    continuation: bool,
    last_element_hash: Option<u64>,
) -> (AxProbe, bool, Option<u64>) {
    // SAFETY: no-arg AX call returning a CF Boolean (u8).
    if unsafe { AXIsProcessTrusted() } == 0 {
        return (AxProbe::NotTrusted, false, None);
    }
    autoreleasepool(|_| {
        // Best-effort: wake up the focused app's accessibility tree. Chromium and
        // Electron (Chrome, Safari's web content, Signal, …) don't publish their
        // focused web element to the system-wide query until AXManualAccessibility
        // is set on the app element. Non-Chromium apps simply don't have the
        // attribute, so this is a harmless no-op for them. The tree may build
        // asynchronously, so a first-call miss is expected and absorbed by the
        // clipboard + Cmd+V fallback in `super::decide`.
        activate_frontmost_a11y();

        // SAFETY: returns a +1 system-wide element ref; released below.
        let system = unsafe { AXUIElementCreateSystemWide() };
        let focused_attr = NSString::from_str(ATTR_FOCUSED_UI_ELEMENT);
        let mut focused: CFTypeRef = std::ptr::null();
        // SAFETY: valid element + attribute; `focused` receives a +1 copy on success.
        let err =
            unsafe { AXUIElementCopyAttributeValue(system, cfstr(&focused_attr), &mut focused) };

        // SAFETY: `focused` is a live +1 AXUIElement reference at this point when
        // non-null; CFHash never takes ownership so this doesn't affect the +1.
        let current_hash = if focused.is_null() {
            None
        } else {
            Some(unsafe { CFHash(focused) } as u64)
        };
        let continuation = refine_continuation(continuation, last_element_hash, current_hash);

        let (probe, lead) = if err != AX_SUCCESS || focused.is_null() {
            // No element to read the caret from → Unknown; the leading space then
            // tracks `continuation` so the clipboard path separates back-to-back
            // dictations without stray-spacing a fresh start.
            (
                AxProbe::NoFocusedElement,
                lead_space(CaretContext::Unknown, continuation),
            )
        } else {
            let element = focused as AXUIElementRef;
            let lead = lead_space(read_caret_context(element), continuation);
            let payload = if lead {
                format!(" {text}")
            } else {
                text.to_string()
            };
            let value = NSString::from_str(&payload);
            let value_ref = (&*value) as *const NSString as CFTypeRef;
            // Insert at the caret via AXSelectedText only. We deliberately do NOT
            // fall back to setting AXValue here: AXValue replaces the *entire*
            // field, so on the many controls that expose a settable AXValue but no
            // settable AXSelectedText it would silently wipe the user's existing
            // text. Letting that case become `SetFailed` routes it to the
            // caret-respecting clipboard + Cmd+V path instead ("must not clobber").
            //
            // A successful set is not enough: Chromium/Electron contenteditable
            // editors (Signal, Gmail compose, Slack) report AXSelectedText settable
            // and return AX_SUCCESS but *silently no-op* the insert. Verify by
            // reading the element's text before/after and only trust `Inserted` if
            // it actually changed; an unchanged value means a false success, which
            // we route to the clipboard + Cmd+V fallback via `SetFailed`.
            let before = read_string_attr(element, ATTR_VALUE);
            let set_ok = set_if_settable(element, ATTR_SELECTED_TEXT, value_ref);
            let probe = if !set_ok {
                AxProbe::SetFailed
            } else {
                let after = read_string_attr(element, ATTR_VALUE);
                match (before, after) {
                    // Text demonstrably changed — a real caret insert.
                    (Some(b), Some(a)) if a != b => AxProbe::Inserted,
                    // Value readable but unchanged — false success (contenteditable
                    // no-op). Fall through to clipboard + Cmd+V.
                    (Some(_), Some(_)) => AxProbe::SetFailed,
                    // Value not readable on at least one side: we can't verify.
                    // Trust the set rather than risk a double paste in a field that
                    // genuinely inserted but just doesn't expose AXValue. Preserves
                    // pre-verification behaviour for that rare case.
                    _ => AxProbe::Inserted,
                }
            };
            (probe, lead)
        };

        // SAFETY: release the +1 focused copy (if any) and the system-wide element.
        if !focused.is_null() {
            unsafe { CFRelease(focused) };
        }
        unsafe { CFRelease(system as *const c_void) };
        (probe, lead, current_hash)
    })
}

/// `true` iff `attr` is settable on `element` and the set returned success.
fn set_if_settable(element: AXUIElementRef, attr: &str, value: CFTypeRef) -> bool {
    let attr = NSString::from_str(attr);
    let mut settable: u8 = 0;
    // SAFETY: valid element + attribute; `settable` receives a CF Boolean.
    let s_err = unsafe { AXUIElementIsAttributeSettable(element, cfstr(&attr), &mut settable) };
    if s_err != AX_SUCCESS || settable == 0 {
        return false;
    }
    // SAFETY: `value` is a live CFString (NSString) for the duration of the call.
    unsafe { AXUIElementSetAttributeValue(element, cfstr(&attr), value) == AX_SUCCESS }
}

/// Read `attr` off `element` as a `String`, or `None` if the copy fails, the value
/// is null, or it isn't a CFString. Used to snapshot the focused field's text
/// around an `AXSelectedText` set so we can confirm the insert actually landed.
fn read_string_attr(element: AXUIElementRef, attr: &str) -> Option<String> {
    autoreleasepool(|_| {
        let attr = NSString::from_str(attr);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: valid element + attribute; `value` receives a +1 copy on success.
        let err = unsafe { AXUIElementCopyAttributeValue(element, cfstr(&attr), &mut value) };
        if err != AX_SUCCESS || value.is_null() {
            return None;
        }
        // AXValue for text controls is a CFString (toll-free bridged to NSString),
        // but other element types return non-string AXValues (e.g. CGSize), so
        // confirm the class before treating it as a string — otherwise skip it.
        // SAFETY: `value` is a live CF object; checked, read, then released (+1 copy).
        unsafe {
            let obj = value as *const AnyObject;
            let is_string: bool = msg_send![obj, isKindOfClass: class!(NSString)];
            let s = if is_string {
                Some((*(value as *const NSString)).to_string())
            } else {
                None
            };
            CFRelease(value);
            s
        }
    })
}

// ---- Chromium/Electron accessibility activation ------------------------------

/// PID of the frontmost (active) application, via `NSWorkspace`.
fn frontmost_app_pid() -> Option<i32> {
    autoreleasepool(|_| {
        // SAFETY: standard AppKit messaging; `frontmostApplication` may be nil.
        unsafe {
            let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let app: *mut AnyObject = msg_send![ws, frontmostApplication];
            if app.is_null() {
                return None;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            // `processIdentifier` is -1 for an app with no pid.
            if pid > 0 {
                Some(pid)
            } else {
                None
            }
        }
    })
}

/// Best-effort: set `AXManualAccessibility = true` on the frontmost app element so
/// Chromium/Electron build their a11y tree. Errors are ignored — apps that don't
/// recognise the attribute (every native Cocoa app) are unaffected.
fn activate_frontmost_a11y() {
    let Some(pid) = frontmost_app_pid() else {
        return;
    };
    autoreleasepool(|_| {
        // SAFETY: returns a +1 application element ref; released below.
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return;
        }
        let attr = NSString::from_str(ATTR_MANUAL_ACCESSIBILITY);
        let yes = NSNumber::new_bool(true);
        // NSCFBoolean is toll-free bridged to CFBooleanRef (what AX expects here).
        let value = (&*yes) as *const NSNumber as CFTypeRef;
        // Set directly (no IsAttributeSettable probe): Chromium reports this
        // attribute as not-settable yet still honours the write.
        // SAFETY: live element + attribute + value for the duration of the call.
        unsafe {
            let _ = AXUIElementSetAttributeValue(app, cfstr(&attr), value);
            CFRelease(app as *const c_void);
        }
    });
}

// ---- Clipboard fallback ------------------------------------------------------

/// Copy `text` to the general pasteboard (replacing its contents).
fn copy_to_pasteboard(text: &str) {
    autoreleasepool(|_| {
        let ns = NSString::from_str(text);
        let ty = NSString::from_str(PASTEBOARD_TYPE_STRING);
        // SAFETY: standard AppKit messaging; generalPasteboard is non-null, the
        // NSStrings outlive the call.
        unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let _: isize = msg_send![pb, clearContents];
            let _: bool = msg_send![pb, setString: &*ns, forType: &*ty];
        }
    });
}

/// Read the plain-text contents of the general pasteboard, if any. Used to
/// snapshot the user's clipboard before we clobber it for a paste.
fn read_pasteboard_string() -> Option<String> {
    autoreleasepool(|_| {
        let ty = NSString::from_str(PASTEBOARD_TYPE_STRING);
        // SAFETY: standard AppKit messaging; `stringForType:` returns nil when the
        // pasteboard has no string of that type.
        unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let s: *mut NSString = msg_send![pb, stringForType: &*ty];
            if s.is_null() {
                None
            } else {
                Some((*s).to_string())
            }
        }
    })
}

/// Synthesize a Cmd+V keystroke into the HID stream. Returns `false` if the
/// event source / events could not be created (treated as a read-only field).
///
/// We post explicit Command key-down/up edges around the V key (rather than only
/// setting the Command flag on V): some apps track the modifier *key event*, not
/// just the per-event flag mask, and drop a flagged-but-unpressed Cmd+V.
fn synthesize_cmd_v() -> bool {
    // SAFETY: returns a +1 source ref or null.
    let source = unsafe { CGEventSourceCreate(CG_EVENT_SOURCE_HID) };
    if source.is_null() {
        return false;
    }
    // SAFETY: valid source; each call returns a +1 event ref or null.
    let cmd_down = unsafe { CGEventCreateKeyboardEvent(source, CMD_KEYCODE, true) };
    let v_down = unsafe { CGEventCreateKeyboardEvent(source, V_KEYCODE, true) };
    let v_up = unsafe { CGEventCreateKeyboardEvent(source, V_KEYCODE, false) };
    let cmd_up = unsafe { CGEventCreateKeyboardEvent(source, CMD_KEYCODE, false) };
    let ok = !cmd_down.is_null() && !v_down.is_null() && !v_up.is_null() && !cmd_up.is_null();
    if ok {
        // SAFETY: live events; carry the Command modifier on the V edges and post
        // the full chord in order: Cmd↓ V↓ V↑ Cmd↑.
        unsafe {
            CGEventSetFlags(v_down, FLAG_COMMAND);
            CGEventSetFlags(v_up, FLAG_COMMAND);
            CGEventPost(CG_HID_EVENT_TAP, cmd_down);
            CGEventPost(CG_HID_EVENT_TAP, v_down);
            CGEventPost(CG_HID_EVENT_TAP, v_up);
            CGEventPost(CG_HID_EVENT_TAP, cmd_up);
        }
    }
    // SAFETY: release any non-null refs we created.
    unsafe {
        for ev in [cmd_down, v_down, v_up, cmd_up] {
            if !ev.is_null() {
                CFRelease(ev);
            }
        }
        CFRelease(source as *const c_void);
    }
    ok
}

/// Paste `text` via the clipboard while preserving the user's existing clipboard.
/// Snapshots the current plain-text clipboard, writes `text`, synthesizes Cmd+V,
/// then restores the snapshot once the target has had time to consume the paste.
/// Returns whether the keystroke was posted (best-effort; see `super::result_for`).
///
/// Limitation: only plain-text clipboard content is restored; non-text flavors
/// (images, rich text) are lost. Tracked as a follow-up.
fn paste_via_clipboard(text: &str) -> bool {
    let previous = read_pasteboard_string();
    copy_to_pasteboard(text);
    // Let the write commit before the paste reads it.
    std::thread::sleep(CLIPBOARD_SETTLE);
    let posted = synthesize_cmd_v();
    if posted {
        // Give the target app time to read the pasteboard before we restore it.
        std::thread::sleep(PASTE_CONSUME);
        if let Some(prev) = previous {
            copy_to_pasteboard(&prev);
        }
    }
    posted
}

// ---- Orchestration -----------------------------------------------------------

/// Blocking paste: probe AX, decide the action, perform side effects, map result.
///
/// `last` carries the previous paste across calls so the `Unknown` caret branch can
/// tell a back-to-back continuation from a fresh start (see `super::is_continuation`).
/// We read the frontmost PID once up front for that decision, and record this paste
/// into `last` afterwards unless the action was a denial (nothing landed).
fn paste_blocking(text: String, last: &Mutex<Option<LastPaste>>) -> Result<(), PasteError> {
    let pid = frontmost_app_pid();
    let now = Instant::now();
    // Lock only to read the snapshot; never hold it across the AX/clipboard work.
    let (continuation, last_element_hash) = {
        let guard = last.lock().unwrap_or_else(|e| e.into_inner());
        (
            is_continuation(guard.as_ref(), pid, now),
            guard.as_ref().and_then(|l| l.element_hash),
        )
    };

    // `ax_insert` already prepended the leading space on the AX path; `lead` lets
    // the clipboard fallback apply the same decision so both paths agree.
    // `current_hash` is the newly focused element's identity, persisted below.
    let (probe, lead, current_hash) = ax_insert(&text, continuation, last_element_hash);
    let action = decide(probe);
    let keystroke_posted = match action {
        PasteAction::Inserted | PasteAction::Denied => false,
        PasteAction::ClipboardThenKeystroke => {
            let payload = if lead { format!(" {text}") } else { text };
            paste_via_clipboard(&payload)
        }
    };

    // Record this paste so the next one can detect a same-app continuation. Skip
    // outcomes where nothing landed (denial, or a clipboard path whose keystroke
    // never posted) and an unknown PID (can't key continuation on it).
    if should_record(action, keystroke_posted) {
        if let Some(pid) = pid {
            let mut guard = last.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(LastPaste {
                pid,
                at: Instant::now(),
                element_hash: current_hash,
            });
        }
    }

    result_for(action, keystroke_posted)
}

pub struct MacosPaste {
    /// The previous paste, shared so the spacing decision can detect a same-app
    /// back-to-back continuation. The single `MacosPaste` lives for the whole app
    /// run, so this persists across dictations.
    last: Arc<Mutex<Option<LastPaste>>>,
}

impl MacosPaste {
    pub fn new() -> Self {
        Self {
            last: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for MacosPaste {
    fn default() -> Self {
        Self::new()
    }
}

impl TextPaste for MacosPaste {
    fn paste<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<(), PasteError>> {
        let owned = text.to_string();
        let last = self.last.clone();
        Box::pin(async move {
            // AX/CGEvent calls may block — keep them off the async runtime.
            //
            // A JoinError only occurs if the blocking task *panics* (a bug), not an
            // Accessibility denial — map it to Internal so the pipeline still stops
            // loudly rather than silently dropping the dictation, without lying
            // about the cause via AccessibilityDenied.
            tokio::task::spawn_blocking(move || paste_blocking(owned, &last))
                .await
                .unwrap_or_else(|e| Err(PasteError::Internal(format!("paste task panicked: {e}"))))
        })
    }
}

pub fn create() -> Box<dyn TextPaste> {
    Box::new(MacosPaste::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-AX paste. IGNORED by default: it injects a live Cmd+V keystroke and
    /// mutates the system pasteboard, so it must be run manually against a focused
    /// text field on a machine with the Accessibility grant:
    ///   cargo test --manifest-path src-tauri/Cargo.toml \
    ///     paste::macos::tests::manual_real_paste -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn manual_real_paste() {
        let result = MacosPaste::new().paste("humhum paste test").await;
        println!("real paste result: {result:?}");
    }

    /// Manual spacing check. IGNORED like `manual_real_paste`. Focus an empty text
    /// field, then run it: the field should read "hello world" (one space between),
    /// proving back-to-back dictations are separated rather than concatenated
    /// ("helloworld"). The first paste lands with no leading space — at-start in a
    /// readable field, or a non-continuation fresh start in an unreadable one — and
    /// the second is a same-app continuation, so it gets the separating space. Reuse
    /// one `MacosPaste` so the continuation state carries between the two pastes.
    ///   cargo test --manifest-path src-tauri/Cargo.toml \
    ///     paste::macos::tests::manual_consecutive_paste_is_spaced -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn manual_consecutive_paste_is_spaced() {
        let paste = MacosPaste::new();
        let first = paste.paste("hello").await;
        let second = paste.paste("world").await;
        println!("consecutive paste results: {first:?}, {second:?} (expect 'hello world')");
    }

    /// Manual field-switch check. IGNORED like
    /// `manual_real_paste`. In a Chromium/Electron app (Slack, a browser, Signal),
    /// focus one *non-empty* field/tab/document, run this once, then within a few
    /// seconds switch focus to a *different, non-empty* field/tab/document in the
    /// SAME app and run it again. Each run should land with no leading space (the
    /// field-switch narrowing via `CFHash` should defer to a fresh start rather
    /// than the stale same-app continuation). Contrast with
    /// `manual_consecutive_paste_is_spaced`, which back-to-back pastes into the
    /// SAME field and expects the opposite (a separating space).
    ///   cargo test --manifest-path src-tauri/Cargo.toml \
    ///     paste::macos::tests::manual_field_switch_no_stray_space -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn manual_field_switch_no_stray_space() {
        let paste = MacosPaste::new();
        let result = paste.paste("no leading space here").await;
        println!("field-switch paste result: {result:?} (expect no leading space)");
    }
}
