//! Post-stop dictation pipeline (LLD §9.1): transcribe → cleanup → paste, with
//! every Tauri event emitted at this orchestration layer. The step traits
//! (`Transcriber`, `CleanupProvider`, `TextPaste`) stay Tauri-free; this module
//! owns all emits via [`PipelineEmitter`]. Branch decisions are pure mappers so
//! every state-machine arm is unit-testable with no model/network/Accessibility.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::audio::AudioBuffer;
use crate::cleanup::{CleanupError, CleanupMode, CleanupProvider};
use crate::events::PasteFailReason;
use crate::hotkey::{AudioCue, HotkeyCapture};
use crate::paste::{PasteError, TextPaste};
use crate::state::RecordingPhase;
use crate::transcription::{Transcriber, TranscriptionError, WhisperModel};

/// How long the terminal overlay lingers before the backend resets to `Idle`
/// (LLD §9.1: `Done`/`Error` → 2s → `Idle`).
const RESET_DELAY: Duration = Duration::from_secs(2);

/// Floor for the per-recording transcription budget — covers the fixed
/// model-load + warm-up cost for short clips where inference itself is tiny.
const TRANSCRIBE_TIMEOUT_FLOOR: Duration = Duration::from_secs(180);

/// Inference budget granted per second of captured audio, on top of the floor.
/// A whole-CPU `small_en` pass stays well under 4× real-time even on the slowest
/// supported Mac, so 4s/1s can never false-kill a legitimate recording — while
/// still finitely bounding a genuinely hung whisper call (which would otherwise
/// wedge `Transcribing` and block the next recording with `AlreadyRecording`).
/// A five-minute recording gets about twenty minutes of inference headroom.
const TRANSCRIBE_BUDGET_PER_AUDIO_SEC: u64 = 4;

/// Per-recording transcription ceiling: the floor, or `PER_AUDIO_SEC ×` the
/// captured duration, whichever is larger.
fn transcribe_timeout(audio_duration_ms: u64) -> Duration {
    let scaled = Duration::from_secs((audio_duration_ms / 1000) * TRANSCRIBE_BUDGET_PER_AUDIO_SEC);
    scaled.max(TRANSCRIBE_TIMEOUT_FLOOR)
}

/// Hard ceiling on the paste step (Accessibility/keystroke injection). Matches
/// the cleanup provider timeout; bounds an unresponsive Accessibility API so the
/// phase can never wedge in `Pasting`.
const PASTE_TIMEOUT: Duration = Duration::from_secs(5);

/// Sink for the four data-carrying pipeline events. Production impl is
/// `tauri::AppHandle`; tests use a recording fake. Kept off the step traits so
/// they remain unit-testable without a Tauri runtime (LLD §430).
pub trait PipelineEmitter: Send + Sync {
    fn emit_transcript_ready(&self, raw: &str);
    fn emit_cleanup_done(&self, text: &str);
    fn emit_pipeline_error(&self, code: &str, message: &str);
    fn emit_paste_failed(&self, reason: PasteFailReason, text_copied_to_clipboard: bool);
    /// Blank audio (`EmptyResult`) — nothing to paste, returning silently to
    /// `Idle`. Data-free, like `recording-cancelled`. Without this emit the
    /// overlay (already in `transcribing`) has no event to leave that phase and
    /// wedges there forever.
    fn emit_transcription_discarded(&self);
}

impl<R: tauri::Runtime> PipelineEmitter for tauri::AppHandle<R> {
    fn emit_transcript_ready(&self, raw: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit(
            "transcript-ready",
            crate::events::TranscriptReady {
                raw: raw.to_string(),
            },
        ) {
            log::warn!("pipeline: emit transcript-ready failed: {e}");
        }
    }
    fn emit_cleanup_done(&self, text: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit(
            "cleanup-done",
            crate::events::CleanupDone {
                text: text.to_string(),
            },
        ) {
            log::warn!("pipeline: emit cleanup-done failed: {e}");
        }
    }
    fn emit_pipeline_error(&self, code: &str, message: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit(
            "pipeline-error",
            crate::events::PipelineError {
                code: code.to_string(),
                message: message.to_string(),
            },
        ) {
            log::warn!("pipeline: emit pipeline-error failed: {e}");
        }
    }
    fn emit_paste_failed(&self, reason: PasteFailReason, text_copied_to_clipboard: bool) {
        use tauri::Emitter;
        if let Err(e) = self.emit(
            "paste-failed",
            crate::events::PasteFailed {
                reason,
                text_copied_to_clipboard,
            },
        ) {
            log::warn!("pipeline: emit paste-failed failed: {e}");
        }
    }
    fn emit_transcription_discarded(&self) {
        use tauri::Emitter;
        if let Err(e) = self.emit("transcription-discarded", ()) {
            log::warn!("pipeline: emit transcription-discarded failed: {e}");
        }
    }
}

// ---- Pure outcome mappers (one per pipeline stage) ---------------------------

/// What to do after the transcription step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptionOutcome {
    /// Non-empty raw transcript → emit `transcript-ready`, advance to Cleaning.
    Proceed(String),
    /// `EmptyResult` (blank audio) → silently discard, return to Idle.
    SilentIdle,
    /// `ModelNotReady`/`InferenceFailed` → emit `pipeline-error`, go to Error.
    Error { code: String, message: String },
}

pub fn transcription_outcome(result: Result<String, TranscriptionError>) -> TranscriptionOutcome {
    match result {
        Ok(raw) => TranscriptionOutcome::Proceed(raw),
        Err(TranscriptionError::EmptyResult) => TranscriptionOutcome::SilentIdle,
        Err(e) => TranscriptionOutcome::Error {
            code: e.code(),
            message: e.to_string(),
        },
    }
}

/// What to do after the cleanup step (the raw transcript is the fallback text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupOutcome {
    /// `Ok(text)` → emit `cleanup-done{text}`, paste it.
    Paste(String),
    /// `KeyRejected`/`Unavailable` → emit `pipeline-error`, paste raw.
    PasteRawWithError { code: String, message: String },
    /// `NoKeyConfigured` → emit `pipeline-error`, NO paste (hard stop).
    StopWithError { code: String, message: String },
}

/// Human-friendly overlay copy for the raw-paste fallback (the transcript is
/// still pasted; this only tells the user cleanup was skipped). Keeps the cryptic
/// HTTP status off the UI — the stable wire `code` still carries the detail for
/// logs. The provider name comes from `mode`, since the error itself is provider-agnostic.
fn cleanup_fallback_message(mode: CleanupMode, e: &CleanupError) -> String {
    match (mode, e) {
        (CleanupMode::Local, _) => "Cleanup skipped — Ollama unavailable".to_string(),
        (_, CleanupError::KeyRejected(_)) => "Cleanup skipped — API key rejected".to_string(),
        (_, CleanupError::Unavailable(_) | CleanupError::ModelNotFound) => {
            "Cleanup skipped — service unavailable".to_string()
        }
        // No other variant reaches the raw-fallback arm; keep the raw Display as a
        // conservative catch-all rather than panicking.
        _ => e.to_string(),
    }
}

pub fn cleanup_outcome(result: Result<String, CleanupError>, mode: CleanupMode) -> CleanupOutcome {
    match result {
        Ok(text) => CleanupOutcome::Paste(text),
        Err(
            e @ (CleanupError::KeyRejected(_)
            | CleanupError::Unavailable(_)
            | CleanupError::ModelNotFound),
        ) => CleanupOutcome::PasteRawWithError {
            code: e.code(),
            message: cleanup_fallback_message(mode, &e),
        },
        // NoKeyConfigured is a real pipeline stop; InvalidProvider can't occur
        // here (only from parsing a provider string in `test_api_key`) but is a
        // config error, so it shares the conservative hard-stop path.
        Err(e @ (CleanupError::NoKeyConfigured | CleanupError::InvalidProvider)) => {
            CleanupOutcome::StopWithError {
                code: e.code(),
                message: e.to_string(),
            }
        }
    }
}

/// What to do after the paste step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteOutcome {
    /// `Ok` (text inserted/keystroke posted) → play stop cue, Done.
    Done,
    /// `NoFocusedField`/`ReadOnlyField` → clipboard used, emit `paste-failed`, Done.
    FailedToClipboard { reason: PasteFailReason },
    /// `AccessibilityDenied`/`Internal(_)` → emit `pipeline-error`, Error.
    Error { code: String, message: String },
}

pub fn paste_outcome(result: Result<(), PasteError>) -> PasteOutcome {
    match result {
        Ok(()) => PasteOutcome::Done,
        Err(PasteError::NoFocusedField) => PasteOutcome::FailedToClipboard {
            reason: PasteFailReason::NoFocusedField,
        },
        Err(PasteError::ReadOnlyField) => PasteOutcome::FailedToClipboard {
            reason: PasteFailReason::ReadOnlyField,
        },
        Err(e @ (PasteError::AccessibilityDenied | PasteError::Internal(_))) => {
            PasteOutcome::Error {
                code: e.code(),
                message: e.to_string(),
            }
        }
    }
}

// ---- Async orchestration -----------------------------------------------------

fn set_phase(phase: &AtomicU8, p: RecordingPhase) {
    phase.store(p as u8, Ordering::SeqCst);
}

/// Run transcribe → cleanup → paste. Entered with phase == `Transcribing` (the
/// stop transition already moved it). Owns all pipeline event emits and the
/// terminal 2s phase reset. Cleanup is a pass-through in Phase 15 (Phase 12
/// swaps `dispatch_cleanup` over a real provider into the cleanup step).
#[allow(clippy::too_many_arguments)] // orchestration entry point: all handles + cleanup snapshot
pub async fn run_pipeline<E: PipelineEmitter>(
    buffer: AudioBuffer,
    model: WhisperModel,
    transcriber: &Arc<dyn Transcriber>,
    paste: &Arc<dyn TextPaste>,
    hotkey: &Arc<dyn HotkeyCapture>,
    phase: &AtomicU8,
    emitter: &E,
    cleanup_mode: CleanupMode,
    cleanup_provider: Option<Arc<dyn CleanupProvider>>,
) {
    // --- Transcribe ---
    // Bound the inference (budget scales with audio length so it never false-kills
    // a long recording): a hang must surface as an error and reset, never wedge
    // the phase in `Transcribing`.
    let budget = transcribe_timeout(buffer.duration_ms);
    let transcribe_result =
        match tokio::time::timeout(budget, transcriber.transcribe(buffer, model)).await {
            Ok(r) => r,
            Err(_elapsed) => Err(TranscriptionError::InferenceFailed("timeout".to_string())),
        };
    let raw = match transcription_outcome(transcribe_result) {
        TranscriptionOutcome::Proceed(raw) => {
            emitter.emit_transcript_ready(&raw);
            raw
        }
        TranscriptionOutcome::SilentIdle => {
            // Blank audio: nothing to paste. Tell the overlay to leave
            // `transcribing` (it has no follow-up event otherwise) *before*
            // resetting the phase.
            emitter.emit_transcription_discarded();
            set_phase(phase, RecordingPhase::Idle);
            return;
        }
        TranscriptionOutcome::Error { code, message } => {
            emitter.emit_pipeline_error(&code, &message);
            return terminal_reset(phase, RecordingPhase::Error).await;
        }
    };

    // --- Cleanup ---
    set_phase(phase, RecordingPhase::Cleaning);
    let cleanup_result =
        crate::cleanup::dispatch_cleanup(&raw, cleanup_mode, cleanup_provider.as_deref()).await;
    let text = match cleanup_outcome(cleanup_result, cleanup_mode) {
        CleanupOutcome::Paste(text) => {
            emitter.emit_cleanup_done(&text);
            text
        }
        CleanupOutcome::PasteRawWithError { code, message } => {
            emitter.emit_pipeline_error(&code, &message);
            raw.clone()
        }
        CleanupOutcome::StopWithError { code, message } => {
            emitter.emit_pipeline_error(&code, &message);
            return terminal_reset(phase, RecordingPhase::Error).await;
        }
    };

    // --- Paste ---
    set_phase(phase, RecordingPhase::Pasting);
    // Bound the paste: an unresponsive Accessibility API must error+reset, never
    // wedge the phase in `Pasting`.
    let paste_result = match tokio::time::timeout(PASTE_TIMEOUT, paste.paste(&text)).await {
        Ok(r) => r,
        Err(_elapsed) => {
            emitter.emit_pipeline_error("paste_timeout", "Paste timed out — please try again.");
            return terminal_reset(phase, RecordingPhase::Error).await;
        }
    };
    match paste_outcome(paste_result) {
        PasteOutcome::Done => {
            hotkey.play_cue(AudioCue::Stop);
            terminal_reset(phase, RecordingPhase::Done).await;
        }
        PasteOutcome::FailedToClipboard { reason } => {
            emitter.emit_paste_failed(reason, true); // clipboard fallback already used
            hotkey.play_cue(AudioCue::Stop);
            terminal_reset(phase, RecordingPhase::Done).await;
        }
        PasteOutcome::Error { code, message } => {
            emitter.emit_pipeline_error(&code, &message);
            terminal_reset(phase, RecordingPhase::Error).await;
        }
    }
}

/// Mark the terminal phase, wait 2s, then reset to Idle **iff** still terminal
/// (never clobber a recording that started in the meantime).
async fn terminal_reset(phase: &AtomicU8, terminal: RecordingPhase) {
    set_phase(phase, terminal);
    tokio::time::sleep(RESET_DELAY).await;
    let _ = phase.compare_exchange(
        terminal as u8,
        RecordingPhase::Idle as u8,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Pure mapper tests (every state-machine arm) -------------------------

    #[test]
    fn transcribe_timeout_scales_with_audio_and_never_below_floor() {
        // Short/blank clips get the fixed floor.
        assert_eq!(transcribe_timeout(0), TRANSCRIBE_TIMEOUT_FLOOR);
        assert_eq!(transcribe_timeout(5_000), TRANSCRIBE_TIMEOUT_FLOOR);
        // A full-length 300s Pro recording (WS5 safety cap) gets 4×/sec = 1200s,
        // comfortably above any real inference — so it can't be false-killed.
        assert_eq!(transcribe_timeout(300_000), Duration::from_secs(1200));
    }

    #[test]
    fn transcription_nonempty_proceeds() {
        assert_eq!(
            transcription_outcome(Ok("hello".into())),
            TranscriptionOutcome::Proceed("hello".into())
        );
    }

    #[test]
    fn transcription_empty_is_silent_idle() {
        assert_eq!(
            transcription_outcome(Err(TranscriptionError::EmptyResult)),
            TranscriptionOutcome::SilentIdle
        );
    }

    #[test]
    fn transcription_model_not_ready_is_error_with_code() {
        assert_eq!(
            transcription_outcome(Err(TranscriptionError::ModelNotReady)),
            TranscriptionOutcome::Error {
                code: "model_not_ready".to_string(),
                message: "model not downloaded".to_string(),
            }
        );
    }

    #[test]
    fn transcription_inference_failed_is_error_with_code() {
        assert_eq!(
            transcription_outcome(Err(TranscriptionError::InferenceFailed("boom".into()))),
            TranscriptionOutcome::Error {
                code: "inference_failed".to_string(),
                message: "inference failed: boom".to_string(),
            }
        );
    }

    #[test]
    fn cleanup_ok_pastes_cleaned_text() {
        assert_eq!(
            cleanup_outcome(Ok("clean".into()), CleanupMode::Api),
            CleanupOutcome::Paste("clean".into())
        );
    }

    #[test]
    fn cleanup_key_rejected_pastes_raw_with_error() {
        assert_eq!(
            cleanup_outcome(
                Err(CleanupError::KeyRejected("401".into())),
                CleanupMode::Api
            ),
            CleanupOutcome::PasteRawWithError {
                code: "key_rejected".to_string(),
                // Friendly copy, never the raw HTTP status.
                message: "Cleanup skipped — API key rejected".to_string(),
            }
        );
    }

    #[test]
    fn cleanup_unavailable_pastes_raw_with_error() {
        assert_eq!(
            cleanup_outcome(
                Err(CleanupError::Unavailable("timeout".into())),
                CleanupMode::Api
            ),
            CleanupOutcome::PasteRawWithError {
                code: "unavailable".to_string(),
                message: "Cleanup skipped — service unavailable".to_string(),
            }
        );
    }

    #[test]
    fn cleanup_local_unavailable_shows_friendly_ollama_message() {
        // The bug fix: a local-Ollama failure must read clearly, not "provider
        // unavailable: 404".
        assert_eq!(
            cleanup_outcome(
                Err(CleanupError::Unavailable("404".into())),
                CleanupMode::Local
            ),
            CleanupOutcome::PasteRawWithError {
                code: "unavailable".to_string(),
                message: "Cleanup skipped — Ollama unavailable".to_string(),
            }
        );
    }

    #[test]
    fn cleanup_no_key_configured_stops_with_error() {
        assert_eq!(
            cleanup_outcome(Err(CleanupError::NoKeyConfigured), CleanupMode::Api),
            CleanupOutcome::StopWithError {
                code: "no_key_configured".to_string(),
                message: "no API key configured".to_string(),
            }
        );
    }

    /// The `cleanup_fallback_message` catch-all: no `cleanup_outcome` caller
    /// reaches it today (every raw-fallback variant is matched explicitly
    /// above), but the private helper itself must not panic on an unmodeled
    /// `(mode, error)` combination — it degrades to the error's own `Display`
    /// rather than assuming a fixed set of callers forever.
    #[test]
    fn cleanup_fallback_message_falls_back_to_display_for_unmodeled_combo() {
        assert_eq!(
            cleanup_fallback_message(CleanupMode::Api, &CleanupError::NoKeyConfigured),
            "no API key configured"
        );
    }

    #[test]
    fn paste_ok_is_done() {
        assert_eq!(paste_outcome(Ok(())), PasteOutcome::Done);
    }

    #[test]
    fn paste_internal_is_error_with_code() {
        assert_eq!(
            paste_outcome(Err(PasteError::Internal("boom".to_string()))),
            PasteOutcome::Error {
                code: "paste_internal".to_string(),
                message: "internal paste error: boom".to_string(),
            }
        );
    }

    #[test]
    fn paste_no_focused_field_is_failed_to_clipboard() {
        assert_eq!(
            paste_outcome(Err(PasteError::NoFocusedField)),
            PasteOutcome::FailedToClipboard {
                reason: PasteFailReason::NoFocusedField
            }
        );
    }

    #[test]
    fn paste_read_only_is_failed_to_clipboard() {
        assert_eq!(
            paste_outcome(Err(PasteError::ReadOnlyField)),
            PasteOutcome::FailedToClipboard {
                reason: PasteFailReason::ReadOnlyField
            }
        );
    }

    #[test]
    fn paste_accessibility_denied_is_error_with_code() {
        assert_eq!(
            paste_outcome(Err(PasteError::AccessibilityDenied)),
            PasteOutcome::Error {
                code: "accessibility_denied".to_string(),
                message: "accessibility permission denied".to_string(),
            }
        );
    }

    // ---- run_pipeline integration tests (stubs + MockEmitter) ----------------

    use crate::hotkey::{HotkeyCombo, HotkeyError, HotkeyEvent};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    struct StubTranscriber(Mutex<Option<Result<String, TranscriptionError>>>);
    impl StubTranscriber {
        fn new(r: Result<String, TranscriptionError>) -> Arc<dyn Transcriber> {
            Arc::new(StubTranscriber(Mutex::new(Some(r))))
        }
    }
    impl Transcriber for StubTranscriber {
        fn transcribe(
            &self,
            _buffer: AudioBuffer,
            _model: WhisperModel,
        ) -> Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send + 'static>>
        {
            let r = self
                .0
                .lock()
                .unwrap()
                .take()
                .expect("transcribe called once");
            Box::pin(async move { r })
        }
    }

    // StubPaste records whether paste() was ever invoked, so tests can assert the
    // EmptyResult path never reaches paste.
    struct StubPaste {
        result: Mutex<Option<Result<(), PasteError>>>,
        called: Arc<AtomicBool>,
    }
    impl StubPaste {
        fn new(r: Result<(), PasteError>) -> (Arc<dyn TextPaste>, Arc<AtomicBool>) {
            let called = Arc::new(AtomicBool::new(false));
            (
                Arc::new(StubPaste {
                    result: Mutex::new(Some(r)),
                    called: called.clone(),
                }),
                called,
            )
        }
    }
    impl TextPaste for StubPaste {
        fn paste<'a>(
            &'a self,
            _text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
            self.called.store(true, Ordering::SeqCst);
            let r = self
                .result
                .lock()
                .unwrap()
                .take()
                .expect("paste called once");
            Box::pin(async move { r })
        }
    }

    #[derive(Default)]
    struct MockHotkey {
        stop_cues: AtomicBool,
    }
    impl HotkeyCapture for MockHotkey {
        fn register(&self, _: &HotkeyCombo) -> Result<(), HotkeyError> {
            Ok(())
        }
        fn unregister(&self) -> Result<(), HotkeyError> {
            Ok(())
        }
        fn set_handler(&self, _: Box<dyn Fn(HotkeyEvent) + Send + Sync>) {}
        fn validate(&self, _: &HotkeyCombo) -> Result<(), HotkeyError> {
            Ok(())
        }
        fn play_cue(&self, cue: AudioCue) {
            if matches!(cue, AudioCue::Stop) {
                self.stop_cues.store(true, Ordering::SeqCst);
            }
        }
    }

    // `run_pipeline` only ever calls `play_cue` on its injected `HotkeyCapture`
    // (registration/validation happen elsewhere in the app), so `MockHotkey`'s
    // other trait methods need a direct call to be exercised at all.
    #[test]
    fn mock_hotkey_trait_methods_are_infallible_no_ops() {
        let hotkey = MockHotkey::default();
        let combo = HotkeyCombo {
            modifiers: vec![],
            key: "Space".to_string(),
        };
        assert!(hotkey.register(&combo).is_ok());
        assert!(hotkey.unregister().is_ok());
        assert!(hotkey.validate(&combo).is_ok());
        hotkey.set_handler(Box::new(|_event| {}));
    }

    /// Records an ordered log of emitted event names (with the salient payload bit).
    #[derive(Clone, Default)]
    struct MockEmitter(Arc<Mutex<Vec<String>>>);
    impl PipelineEmitter for MockEmitter {
        fn emit_transcript_ready(&self, _raw: &str) {
            self.0.lock().unwrap().push("transcript-ready".into());
        }
        fn emit_cleanup_done(&self, _text: &str) {
            self.0.lock().unwrap().push("cleanup-done".into());
        }
        fn emit_pipeline_error(&self, code: &str, _message: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("pipeline-error:{code}"));
        }
        fn emit_paste_failed(&self, reason: PasteFailReason, copied: bool) {
            self.0
                .lock()
                .unwrap()
                .push(format!("paste-failed:{reason:?}:{copied}"));
        }
        fn emit_transcription_discarded(&self) {
            self.0
                .lock()
                .unwrap()
                .push("transcription-discarded".into());
        }
    }

    fn buf() -> AudioBuffer {
        AudioBuffer {
            samples: vec![0.0; 4],
            sample_rate: 16_000,
            duration_ms: 0,
        }
    }

    fn events(e: &MockEmitter) -> Vec<String> {
        e.0.lock().unwrap().clone()
    }

    #[tokio::test(start_paused = true)]
    async fn happy_path_emits_in_order_pastes_and_resets_to_idle() {
        let transcriber = StubTranscriber::new(Ok("hello world".into()));
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(events(&emitter), vec!["transcript-ready", "cleanup-done"]);
        assert!(paste_called.load(Ordering::SeqCst), "paste was invoked");
        // After the inline 2s reset (paused clock advanced inside run_pipeline's
        // sleep), the phase is back to Idle and ready for the next recording.
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_result_discards_and_never_pastes() {
        let transcriber = StubTranscriber::new(Err(TranscriptionError::EmptyResult));
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        // Blank audio must emit `transcription-discarded` so the overlay leaves
        // `transcribing` (the bug: it used to emit nothing and wedge there).
        assert_eq!(events(&emitter), vec!["transcription-discarded"]);
        assert!(
            !paste_called.load(Ordering::SeqCst),
            "paste must not run on blank audio"
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    // Steps that never resolve within their timeout — drive the WS2 timeout arms.
    struct SlowTranscriber;
    impl Transcriber for SlowTranscriber {
        fn transcribe(
            &self,
            _buffer: AudioBuffer,
            _model: WhisperModel,
        ) -> Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send + 'static>>
        {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(600)).await;
                Ok("never reached".to_string())
            })
        }
    }

    struct SlowPaste;
    impl TextPaste for SlowPaste {
        fn paste<'a>(
            &'a self,
            _text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(600)).await;
                Ok(())
            })
        }
    }

    /// WS2: a hung transcription must time out into a `pipeline-error` and reset to
    /// `Idle` — never wedge the phase in `Transcribing` (which would also block the
    /// next recording).
    #[tokio::test(start_paused = true)]
    async fn transcription_timeout_emits_pipeline_error_and_resets() {
        let transcriber: Arc<dyn Transcriber> = Arc::new(SlowTranscriber);
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(events(&emitter), vec!["pipeline-error:inference_failed"]);
        assert!(
            !paste_called.load(Ordering::SeqCst),
            "paste must not run when transcription times out"
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    #[tokio::test(start_paused = true)]
    async fn late_transcription_after_timeout_emits_no_transcript_and_never_pastes() {
        struct DeferredTranscriber(Mutex<Option<tokio::sync::oneshot::Receiver<String>>>);
        impl Transcriber for DeferredTranscriber {
            fn transcribe(
                &self,
                _: AudioBuffer,
                _: WhisperModel,
            ) -> Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send + 'static>>
            {
                let receiver = self.0.lock().unwrap().take().unwrap();
                Box::pin(async move { Ok(receiver.await.unwrap()) })
            }
        }
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let transcriber: Arc<dyn Transcriber> =
            Arc::new(DeferredTranscriber(Mutex::new(Some(receiver))));
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();
        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;
        assert!(reply.send("abandoned transcript".into()).is_err());
        tokio::task::yield_now().await;
        assert_eq!(events(&emitter), vec!["pipeline-error:inference_failed"]);
        assert!(!paste_called.load(Ordering::SeqCst));
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    /// WS2: a hung paste must time out into a `pipeline-error` and reset to `Idle`
    /// — never wedge the phase in `Pasting`.
    #[tokio::test(start_paused = true)]
    async fn paste_timeout_emits_pipeline_error_and_resets() {
        let transcriber = StubTranscriber::new(Ok("hi".into()));
        let paste: Arc<dyn TextPaste> = Arc::new(SlowPaste);
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(
            events(&emitter),
            vec![
                "transcript-ready",
                "cleanup-done",
                "pipeline-error:paste_timeout"
            ]
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    #[tokio::test(start_paused = true)]
    async fn inference_failure_emits_pipeline_error_and_resets_from_error() {
        let transcriber =
            StubTranscriber::new(Err(TranscriptionError::InferenceFailed("x".into())));
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(events(&emitter), vec!["pipeline-error:inference_failed"]);
        assert!(!paste_called.load(Ordering::SeqCst));
        // terminal_reset moved Error → Idle after the 2s sleep.
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    #[tokio::test(start_paused = true)]
    async fn no_focused_field_emits_paste_failed_and_finishes_done() {
        let transcriber = StubTranscriber::new(Ok("hi".into()));
        let (paste, _called) = StubPaste::new(Err(PasteError::NoFocusedField));
        let mock_hotkey = Arc::new(MockHotkey::default());
        let hotkey: Arc<dyn HotkeyCapture> = mock_hotkey.clone();
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(
            events(&emitter),
            vec![
                "transcript-ready".to_string(),
                "cleanup-done".to_string(),
                "paste-failed:NoFocusedField:true".to_string(),
            ]
        );
        assert!(
            mock_hotkey.stop_cues.load(Ordering::SeqCst),
            "stop cue played"
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    #[tokio::test(start_paused = true)]
    async fn accessibility_denied_emits_pipeline_error_not_paste_failed() {
        let transcriber = StubTranscriber::new(Ok("hi".into()));
        let (paste, _called) = StubPaste::new(Err(PasteError::AccessibilityDenied));
        let mock_hotkey = Arc::new(MockHotkey::default());
        let hotkey: Arc<dyn HotkeyCapture> = mock_hotkey.clone();
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::None,
            None,
        )
        .await;

        assert_eq!(
            events(&emitter),
            vec![
                "transcript-ready".to_string(),
                "cleanup-done".to_string(),
                "pipeline-error:accessibility_denied".to_string(),
            ]
        );
        assert!(
            !mock_hotkey.stop_cues.load(Ordering::SeqCst),
            "no stop cue on error"
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    // ---- Cleanup integration (raw -> cleanup -> cleaned) ---------------------

    struct StubCleanupProvider(Mutex<Option<Result<String, CleanupError>>>);
    impl StubCleanupProvider {
        fn new(r: Result<String, CleanupError>) -> Arc<dyn CleanupProvider> {
            Arc::new(StubCleanupProvider(Mutex::new(Some(r))))
        }
    }
    impl CleanupProvider for StubCleanupProvider {
        fn clean<'a>(
            &'a self,
            _text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, CleanupError>> + Send + 'a>> {
            let r = self.0.lock().unwrap().take().expect("clean called once");
            Box::pin(async move { r })
        }
    }

    /// Paste stub that records the exact text it received (to prove cleaned vs raw).
    struct CapturePaste(Arc<Mutex<Option<String>>>);
    impl CapturePaste {
        fn new() -> (Arc<dyn TextPaste>, Arc<Mutex<Option<String>>>) {
            let slot = Arc::new(Mutex::new(None));
            (Arc::new(CapturePaste(slot.clone())), slot)
        }
    }
    impl TextPaste for CapturePaste {
        fn paste<'a>(
            &'a self,
            text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
            *self.0.lock().unwrap() = Some(text.to_string());
            Box::pin(async move { Ok(()) })
        }
    }

    /// e4f.8: a successful cleanup pastes the *cleaned* text, not the raw transcript.
    #[tokio::test(start_paused = true)]
    async fn local_cleanup_pastes_the_cleaned_text() {
        let transcriber = StubTranscriber::new(Ok("raw".into()));
        let (paste, pasted) = CapturePaste::new();
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();
        let provider = StubCleanupProvider::new(Ok("CLEANED".into()));

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::Local,
            Some(provider),
        )
        .await;

        assert_eq!(events(&emitter), vec!["transcript-ready", "cleanup-done"]);
        assert_eq!(pasted.lock().unwrap().as_deref(), Some("CLEANED"));
    }

    /// e4f.7: a cleanup failure emits the error and falls back to pasting the raw
    /// transcript (paste is never blocked for `Unavailable`).
    #[tokio::test(start_paused = true)]
    async fn cleanup_failure_falls_back_to_pasting_raw() {
        let transcriber = StubTranscriber::new(Ok("raw".into()));
        let (paste, pasted) = CapturePaste::new();
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();
        let provider = StubCleanupProvider::new(Err(CleanupError::Unavailable("503".into())));

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::Local,
            Some(provider),
        )
        .await;

        assert_eq!(
            events(&emitter),
            vec![
                "transcript-ready".to_string(),
                "pipeline-error:unavailable".to_string()
            ]
        );
        assert_eq!(pasted.lock().unwrap().as_deref(), Some("raw"));
    }

    /// `CleanupOutcome::StopWithError` (`NoKeyConfigured`): a hard stop — the
    /// pipeline emits `pipeline-error` and resets through `Error`, never
    /// reaching the paste step at all.
    #[tokio::test(start_paused = true)]
    async fn no_key_configured_cleanup_stops_before_paste() {
        let transcriber = StubTranscriber::new(Ok("raw".into()));
        let (paste, paste_called) = StubPaste::new(Ok(()));
        let hotkey: Arc<dyn HotkeyCapture> = Arc::new(MockHotkey::default());
        let phase = AtomicU8::new(RecordingPhase::Transcribing as u8);
        let emitter = MockEmitter::default();
        let provider = StubCleanupProvider::new(Err(CleanupError::NoKeyConfigured));

        run_pipeline(
            buf(),
            WhisperModel::BaseEn,
            &transcriber,
            &paste,
            &hotkey,
            &phase,
            &emitter,
            CleanupMode::Local,
            Some(provider),
        )
        .await;

        assert_eq!(
            events(&emitter),
            vec![
                "transcript-ready".to_string(),
                "pipeline-error:no_key_configured".to_string()
            ]
        );
        assert!(
            !paste_called.load(Ordering::SeqCst),
            "paste must not run after a hard cleanup stop"
        );
        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
    }

    // `PipelineEmitter for tauri::AppHandle<R>` is generic over the runtime, so
    // `MockRuntime` (via `tauri::test`) exercises the real production impl
    // without a window/webview runtime.
    #[test]
    fn app_handle_emit_methods_do_not_panic() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        handle.emit_transcript_ready("raw text");
        handle.emit_cleanup_done("cleaned text");
        handle.emit_pipeline_error("some_code", "some message");
        handle.emit_paste_failed(PasteFailReason::NoFocusedField, true);
        handle.emit_paste_failed(PasteFailReason::ReadOnlyField, false);
        handle.emit_transcription_discarded();
    }
}
