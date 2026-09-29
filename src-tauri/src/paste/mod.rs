use std::future::Future;
use std::pin::Pin;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub enum PasteError {
    NoFocusedField,
    ReadOnlyField,
    AccessibilityDenied,
    /// The blocking paste task panicked (a bug, not a permission/field issue —
    /// e.g. an FFI call unwound). Carries the panic payload for logging.
    Internal(String),
}

impl PasteError {
    /// Stable snake_case wire code (matches `paste-failed` reasons, LLD §623).
    pub fn code(&self) -> String {
        match self {
            PasteError::NoFocusedField => "no_focused_field",
            PasteError::ReadOnlyField => "read_only_field",
            PasteError::AccessibilityDenied => "accessibility_denied",
            PasteError::Internal(_) => "paste_internal",
        }
        .to_string()
    }
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasteError::NoFocusedField => write!(f, "no focused text field"),
            PasteError::ReadOnlyField => write!(f, "focused field is read-only"),
            PasteError::AccessibilityDenied => write!(f, "accessibility permission denied"),
            PasteError::Internal(e) => write!(f, "internal paste error: {e}"),
        }
    }
}

pub trait TextPaste: Send + Sync {
    fn paste<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<(), PasteError>>;
}

/// What the `unsafe` AX layer observed about the focused element and the set
/// attempt. Built by `macos.rs`; consumed by the pure [`decide`] so all four
/// outcome branches are unit-testable without a live Accessibility session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxProbe {
    /// `AXIsProcessTrusted()` is false — the app holds no Accessibility grant.
    NotTrusted,
    /// No focused UI element (copy of `kAXFocusedUIElement` failed or was null).
    NoFocusedElement,
    /// A settable text attribute accepted the value — text is in the field.
    Inserted,
    /// A focused element exists but no text attribute was settable / set failed.
    SetFailed,
}

/// The side effect the FFI layer should perform, decided purely from [`AxProbe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteAction {
    /// Text already inserted via AX — nothing more to do.
    Inserted,
    /// Copy to the pasteboard, then synthesize Cmd+V (sandboxed / read-only AX, or
    /// a target — e.g. Chromium/Electron — that exposes no settable AX text field).
    ClipboardThenKeystroke,
    /// No AX grant — do nothing, surface the denial.
    Denied,
}

fn decide(probe: AxProbe) -> PasteAction {
    match probe {
        AxProbe::NotTrusted => PasteAction::Denied,
        // Anything short of a successful AX caret-insert (no focused element found
        // *or* a focused element with no settable text attribute) goes through the
        // universal clipboard + Cmd+V path. Many apps with a real cursor — browsers,
        // Electron apps like Signal — never expose a settable AX field to the
        // system-wide query, so copying without pasting would silently drop the
        // dictation. Cmd+V works wherever the app honours paste.
        AxProbe::NoFocusedElement | AxProbe::SetFailed => PasteAction::ClipboardThenKeystroke,
        AxProbe::Inserted => PasteAction::Inserted,
    }
}

/// Map an executed [`PasteAction`] onto the trait result. `keystroke_posted` is
/// only meaningful for `ClipboardThenKeystroke`: a posted Cmd+V means the text
/// reached the field (`Ok`), otherwise the field was effectively read-only.
///
/// Note: `keystroke_posted` reflects event *creation + post*, not delivery
/// confirmation. `CGEventPost` is fire-and-forget with no return status, so
/// `Ok(())` on this path means "best-effort paste dispatched" — not "the text
/// definitely landed." A posted event can still be dropped (focus changed,
/// secure-input field, destination ignores Cmd+V); we have no way to observe that.
fn result_for(action: PasteAction, keystroke_posted: bool) -> Result<(), PasteError> {
    match action {
        PasteAction::Inserted => Ok(()),
        PasteAction::ClipboardThenKeystroke => {
            if keystroke_posted {
                Ok(())
            } else {
                Err(PasteError::ReadOnlyField)
            }
        }
        PasteAction::Denied => Err(PasteError::AccessibilityDenied),
    }
}

// ---- Inter-dictation spacing -------------------------------------------------

/// What the AX layer could observe about the text immediately before the caret,
/// used to decide whether a new dictation needs a leading space so it doesn't
/// butt against existing text. Built by `macos.rs` (a single-char read at the
/// caret); consumed by the pure [`lead_space`] so the decision is unit-testable
/// without a live Accessibility session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaretContext {
    /// We read the character just before the caret.
    PrecededBy(char),
    /// Caret is at offset 0 / the field is empty — no separator wanted.
    AtStart,
    /// Couldn't read the caret context (browser/Electron clipboard path, or no
    /// focused element).
    Unknown,
}

/// Whether to prepend one leading space so a new dictation reads as a separate
/// word rather than concatenating onto the previous one (`worldhow` → `world
/// how`). When we can read the caret (`AtStart` / `PrecededBy`) that read is
/// authoritative. When we can't (`Unknown` — browsers/Electron on the clipboard
/// path), we defer to `continuation`: a space only separates this dictation from
/// a *recent paste of our own into the same app*. A fresh start (first dictation,
/// app switch, or long idle) is `continuation == false`, so it lands with no stray
/// leading space — otherwise the first sentence of a paragraph would gain one.
///
/// Known limitation (unreadable path only): `continuation` keys on the frontmost
/// app PID (see [`is_continuation`]), narrowed by focused-element identity when
/// available (see [`refine_continuation`]). It still can't see a manual caret
/// move within the *same* field/document/tab — clicking to reposition the caret
/// there can still prepend a stray space. This is the best signal available with
/// no readable caret (which is exactly what's missing here), and it's a large net
/// improvement over the old always-`true` default. The remaining same-element
/// caret-move gap is tracked separately.
///
/// Scope: spacing only. We deliberately do not touch capitalization or
/// punctuation for mid-sentence continuations — each dictation is transcribed
/// independently and the previous segment's trailing period is already pasted and
/// can't be retracted, so true sentence-merging isn't attempted here.
fn lead_space(ctx: CaretContext, continuation: bool) -> bool {
    match ctx {
        CaretContext::PrecededBy(c) => !c.is_whitespace(),
        CaretContext::AtStart => false,
        CaretContext::Unknown => continuation,
    }
}

/// A paste we performed, used to decide whether the next one is a back-to-back
/// continuation in an app we can't read the caret of (see [`is_continuation`]).
#[derive(Debug, Clone, Copy)]
struct LastPaste {
    /// Frontmost app PID at the time of the paste.
    pid: i32,
    /// When the paste happened.
    at: std::time::Instant,
    /// `CFHash` of the focused AX element at the time of the paste, when one was
    /// readable. Used by [`refine_continuation`] to narrow the PID+window
    /// heuristic when we can tell the current and previous focused elements
    /// apart. `None` when there was no focused element to hash.
    element_hash: Option<u64>,
}

/// How long after a paste a following dictation still counts as the same
/// continuous sequence. Generous enough for normal speaking pauses between
/// sentences, short enough that returning to an app later reads as a fresh start.
const CONTINUATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether the dictation about to be pasted continues a recent one: it lands in the
/// same frontmost app as our last paste, within [`CONTINUATION_WINDOW`]. Used only
/// for the `Unknown` caret branch (see [`lead_space`]). A missing last paste, an app
/// switch, an unknown current app, or a stale last paste all read as *not* a
/// continuation. Pure so it's unit-testable without a live Accessibility session.
fn is_continuation(
    last: Option<&LastPaste>,
    current_pid: Option<i32>,
    now: std::time::Instant,
) -> bool {
    match (last, current_pid) {
        (Some(prev), Some(pid)) => {
            prev.pid == pid && now.duration_since(prev.at) < CONTINUATION_WINDOW
        }
        _ => false,
    }
}

/// Narrows `continuation` using the focused element's identity hash, when both
/// the previous and current hash are known. If they differ, this is provably a
/// different field/tab/document even though the PID and time window matched —
/// force `false`. If either hash is unavailable, or they match, defer to
/// `continuation` unchanged. This can only ever turn a `true` into a `false`,
/// never the reverse — it's a strict narrowing of the existing PID+window
/// heuristic (see [`is_continuation`]), not a replacement for it.
fn refine_continuation(
    continuation: bool,
    last_hash: Option<u64>,
    current_hash: Option<u64>,
) -> bool {
    match (last_hash, current_hash) {
        (Some(a), Some(b)) if a != b => false,
        _ => continuation,
    }
}

/// Whether a completed paste should be recorded as the new [`LastPaste`] for
/// continuation detection. We record only when text *plausibly reached the field*:
/// a successful AX insert, or a clipboard + Cmd+V whose keystroke was actually
/// posted. A denial (no grant) or a clipboard path whose keystroke never posted
/// (an effectively read-only field) left nothing behind — recording it would make
/// the *next* dictation falsely read as a continuation and gain a stray space.
/// Pure so it's unit-testable; the PID and timestamp are wrapped on by the caller.
fn should_record(action: PasteAction, keystroke_posted: bool) -> bool {
    match action {
        PasteAction::Inserted => true,
        PasteAction::ClipboardThenKeystroke => keystroke_posted,
        PasteAction::Denied => false,
    }
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_error_code_and_display_cover_every_variant() {
        assert_eq!(PasteError::NoFocusedField.code(), "no_focused_field");
        assert_eq!(PasteError::ReadOnlyField.code(), "read_only_field");
        assert_eq!(
            PasteError::AccessibilityDenied.code(),
            "accessibility_denied"
        );
        assert_eq!(PasteError::Internal("x".into()).code(), "paste_internal");

        assert_eq!(
            PasteError::NoFocusedField.to_string(),
            "no focused text field"
        );
        assert_eq!(
            PasteError::ReadOnlyField.to_string(),
            "focused field is read-only"
        );
        assert_eq!(
            PasteError::AccessibilityDenied.to_string(),
            "accessibility permission denied"
        );
        assert_eq!(
            PasteError::Internal("boom".into()).to_string(),
            "internal paste error: boom"
        );
    }

    #[test]
    fn primary_ax_insert_succeeds() {
        assert_eq!(decide(AxProbe::Inserted), PasteAction::Inserted);
        assert!(result_for(PasteAction::Inserted, false).is_ok());
    }

    #[test]
    fn set_failure_uses_clipboard_then_keystroke() {
        assert_eq!(
            decide(AxProbe::SetFailed),
            PasteAction::ClipboardThenKeystroke
        );
        // Posted keystroke → treated as a successful paste (LLD §296).
        assert!(result_for(PasteAction::ClipboardThenKeystroke, true).is_ok());
        // Keystroke could not be posted → the field was effectively read-only.
        assert!(matches!(
            result_for(PasteAction::ClipboardThenKeystroke, false),
            Err(PasteError::ReadOnlyField)
        ));
    }

    #[test]
    fn no_focused_field_uses_clipboard_then_keystroke() {
        // A target with no settable AX field (browsers, Electron apps) must still
        // paste via clipboard + Cmd+V rather than silently copying-only.
        assert_eq!(
            decide(AxProbe::NoFocusedElement),
            PasteAction::ClipboardThenKeystroke
        );
        assert!(result_for(PasteAction::ClipboardThenKeystroke, true).is_ok());
        assert!(matches!(
            result_for(PasteAction::ClipboardThenKeystroke, false),
            Err(PasteError::ReadOnlyField)
        ));
    }

    #[test]
    fn untrusted_process_is_accessibility_denied() {
        assert_eq!(decide(AxProbe::NotTrusted), PasteAction::Denied);
        assert!(matches!(
            result_for(PasteAction::Denied, false),
            Err(PasteError::AccessibilityDenied)
        ));
    }

    #[test]
    fn lead_space_added_after_a_word_char() {
        // A non-whitespace char before the caret means the new dictation would
        // butt against it — separate them. A readable caret is authoritative, so
        // the continuation flag is irrelevant here.
        assert!(lead_space(CaretContext::PrecededBy('o'), false));
        assert!(lead_space(CaretContext::PrecededBy('.'), false));
    }

    #[test]
    fn no_lead_space_when_already_separated() {
        // Existing whitespace (space or newline) before the caret — no double space.
        assert!(!lead_space(CaretContext::PrecededBy(' '), true));
        assert!(!lead_space(CaretContext::PrecededBy('\n'), true));
        assert!(!lead_space(CaretContext::PrecededBy('\t'), true));
    }

    #[test]
    fn no_lead_space_at_field_start() {
        // First word in an empty/at-start field must not get a stray leading space,
        // even if the app thinks this is a continuation.
        assert!(!lead_space(CaretContext::AtStart, true));
    }

    #[test]
    fn lead_space_on_unknown_follows_continuation() {
        // Browsers/Electron (clipboard path) can't be read. A back-to-back
        // continuation gets the separating space; a fresh start does not — so the
        // first sentence of a paragraph lands without a stray leading space.
        assert!(lead_space(CaretContext::Unknown, true));
        assert!(!lead_space(CaretContext::Unknown, false));
    }

    #[test]
    fn continuation_requires_a_prior_paste() {
        // No paste yet → the first dictation is never a continuation.
        assert!(!is_continuation(None, Some(42), std::time::Instant::now()));
    }

    #[test]
    fn continuation_when_same_app_within_window() {
        let now = std::time::Instant::now();
        let last = LastPaste {
            pid: 42,
            at: now,
            element_hash: None,
        };
        assert!(is_continuation(Some(&last), Some(42), now));
    }

    #[test]
    fn no_continuation_after_app_switch() {
        let now = std::time::Instant::now();
        let last = LastPaste {
            pid: 42,
            at: now,
            element_hash: None,
        };
        assert!(!is_continuation(Some(&last), Some(7), now));
    }

    #[test]
    fn no_continuation_after_window_elapses() {
        let now = std::time::Instant::now();
        let last = LastPaste {
            pid: 42,
            at: now - CONTINUATION_WINDOW - std::time::Duration::from_secs(1),
            element_hash: None,
        };
        assert!(!is_continuation(Some(&last), Some(42), now));
    }

    #[test]
    fn no_continuation_exactly_at_window_boundary() {
        // The window is strict-less-than, so a paste exactly CONTINUATION_WINDOW ago
        // is already a fresh start (pins the `<` vs `<=` edge).
        let now = std::time::Instant::now();
        let last = LastPaste {
            pid: 42,
            at: now - CONTINUATION_WINDOW,
            element_hash: None,
        };
        assert!(!is_continuation(Some(&last), Some(42), now));
    }

    #[test]
    fn no_continuation_when_current_app_unknown() {
        let now = std::time::Instant::now();
        let last = LastPaste {
            pid: 42,
            at: now,
            element_hash: None,
        };
        assert!(!is_continuation(Some(&last), None, now));
    }

    #[test]
    fn record_only_when_text_plausibly_landed() {
        // A successful AX insert always landed.
        assert!(should_record(PasteAction::Inserted, false));
        // Clipboard path: record iff the Cmd+V keystroke was actually posted.
        assert!(should_record(PasteAction::ClipboardThenKeystroke, true));
        // Keystroke never posted (effectively read-only) — nothing landed, so
        // recording it would make the next dictation a false continuation.
        assert!(!should_record(PasteAction::ClipboardThenKeystroke, false));
        // A denial never put text in the field.
        assert!(!should_record(PasteAction::Denied, false));
    }

    #[test]
    fn refine_continuation_unaffected_when_hashes_match() {
        // Same element on both sides — defer to the PID+window continuation
        // value, whatever it is.
        assert!(refine_continuation(true, Some(1), Some(1)));
        assert!(!refine_continuation(false, Some(1), Some(1)));
    }

    #[test]
    fn refine_continuation_forces_false_when_hashes_differ() {
        // A provably different focused element overrides a `true` continuation
        // even though PID+window said otherwise.
        assert!(!refine_continuation(true, Some(1), Some(2)));
        // Already false — stays false.
        assert!(!refine_continuation(false, Some(1), Some(2)));
    }

    #[test]
    fn refine_continuation_unaffected_when_either_hash_missing() {
        // Can't disprove identity without both hashes — defer to `continuation`.
        assert!(refine_continuation(true, None, Some(1)));
        assert!(refine_continuation(true, Some(1), None));
        assert!(refine_continuation(true, None, None));
        assert!(!refine_continuation(false, None, Some(1)));
    }
}
