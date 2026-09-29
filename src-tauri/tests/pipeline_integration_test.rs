//! Integration test: drives a full
//! dictation cycle through the real, unmodified `AppState` / `recording::*` /
//! `pipeline::run_pipeline` — a wider black-box slice than either module's own
//! inline unit tests, each of which stops short of the other: `pipeline.rs`'s
//! tests call `run_pipeline` directly (skipping `AppState`/`recording.rs`
//! entirely), and `recording.rs`'s tests use a no-op `PipelineEmitter` and
//! never let the transcribe → cleanup → paste chain run to completion.
//!
//! **Two async runtimes:** `recording::start` spawns the post-stop pipeline
//! via `tauri::async_runtime::spawn`, a runtime separate from this test's own
//! `#[tokio::test]` runtime. There is no `JoinHandle` to await, and
//! `start_paused` (used throughout `pipeline.rs`'s own tests) has no effect on
//! it. So these tests use real wall-clock waits — bounded by
//! `tokio::time::timeout` / a bounded poll loop — instead of a paused clock.
//! Expect ~2-4s of real time per test for the `Done` → 2s → `Idle` reset
//! (`pipeline::terminal_reset`) to complete.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::oneshot;

use wipzi_lib::audio::{AudioBuffer, AudioCapture, AudioError};
use wipzi_lib::cleanup::{CleanupConfig, CleanupMode};
use wipzi_lib::events::PasteFailReason;
use wipzi_lib::hotkey::{AudioCue, HotkeyCapture, HotkeyCombo, HotkeyError, HotkeyEvent};
use wipzi_lib::models::ModelGate;
use wipzi_lib::paste::{PasteError, TextPaste};
use wipzi_lib::permissions::{Permissions, PermissionsState};
use wipzi_lib::pipeline::PipelineEmitter;
use wipzi_lib::recording::{self, RecordingEmitter, RecordingSignal};
use wipzi_lib::state::{AppState, RecordingPhase};
use wipzi_lib::transcription::{Transcriber, TranscriptionError, WhisperModel};

// ---- Mocks -------------------------------------------------------------
// Mirrors `recording.rs`'s own inline harness; duplicated here since an
// external `tests/` crate can't reach a `#[cfg(test)]`-only module.

#[derive(Default)]
struct MockAudio {
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}
impl AudioCapture for MockAudio {
    fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
        AudioError,
    > {
        let (tx, rx) = oneshot::channel();
        *self.stop_tx.lock().unwrap() = Some(tx);
        Ok(Box::pin(async move {
            let _ = rx.await; // resolves when stop()/cancel() fires
            Ok(AudioBuffer {
                samples: Vec::new(),
                sample_rate: 16_000,
                duration_ms: 0,
            })
        }))
    }
    fn stop(&self) -> Result<(), AudioError> {
        if let Some(tx) = self.stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
    fn cancel(&self) {
        if let Some(tx) = self.stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
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

struct MockPermissions;
impl Permissions for MockPermissions {
    fn check(&self) -> PermissionsState {
        PermissionsState {
            microphone: true,
            accessibility: true,
            input_monitoring: true,
        }
    }
    fn request_microphone(&self) -> Result<(), ()> {
        Ok(())
    }
    fn open_accessibility_settings(&self) {}
    fn open_input_monitoring_settings(&self) {}
}

struct MockModelGate(bool);
impl ModelGate for MockModelGate {
    fn is_ready(&self, _model: WhisperModel) -> bool {
        self.0
    }
}

struct StubTranscriber(String);
impl Transcriber for StubTranscriber {
    fn transcribe(
        &self,
        _buffer: AudioBuffer,
        _model: WhisperModel,
    ) -> Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send + 'static>> {
        let text = self.0.clone();
        Box::pin(async move { Ok(text) })
    }
}

/// Sends the pasted text once over a stashed oneshot, so the test can await
/// "the pipeline reached paste" without a `JoinHandle` into the spawned task.
struct CapturingPaste(Mutex<Option<oneshot::Sender<String>>>);
impl CapturingPaste {
    fn new() -> (Arc<dyn TextPaste>, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        (Arc::new(CapturingPaste(Mutex::new(Some(tx)))), rx)
    }
}
impl TextPaste for CapturingPaste {
    fn paste<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
        if let Some(tx) = self.0.lock().unwrap().take() {
            let _ = tx.send(text.to_string());
        }
        Box::pin(async { Ok(()) })
    }
}

/// Always fails with a fixed reason (Test 2's paste-failure branch).
struct FailingPaste(Mutex<Option<PasteError>>);
impl FailingPaste {
    fn new(e: PasteError) -> Self {
        FailingPaste(Mutex::new(Some(e)))
    }
}
impl TextPaste for FailingPaste {
    fn paste<'a>(
        &'a self,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
        let e = self.0.lock().unwrap().take().expect("paste called once");
        Box::pin(async move { Err(e) })
    }
}

/// Records an ordered log of event names across the `recording.rs` /
/// `pipeline.rs` boundary — the actual cross-module ordering guarantee this
/// bead exists to prove.
#[derive(Clone, Default)]
struct EventLogEmitter(Arc<Mutex<Vec<String>>>);
impl EventLogEmitter {
    fn events(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}
impl RecordingEmitter for EventLogEmitter {
    fn emit_recording(&self, signal: RecordingSignal) {
        let name = match signal {
            RecordingSignal::Started => "recording-started",
            RecordingSignal::Stopped => "recording-stopped",
            RecordingSignal::Cancelled => "recording-cancelled",
        };
        self.0.lock().unwrap().push(name.to_string());
    }
    fn emit_recording_error(&self, code: &str, _message: &str) {
        self.0
            .lock()
            .unwrap()
            .push(format!("recording-error:{code}"));
    }
}
impl PipelineEmitter for EventLogEmitter {
    fn emit_transcript_ready(&self, _raw: &str) {
        self.0.lock().unwrap().push("transcript-ready".to_string());
    }
    fn emit_cleanup_done(&self, _text: &str) {
        self.0.lock().unwrap().push("cleanup-done".to_string());
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
            .push("transcription-discarded".to_string());
    }
}

fn build_state(
    transcriber: Arc<dyn Transcriber>,
    paste: Arc<dyn TextPaste>,
) -> (AppState, Arc<MockHotkey>) {
    let audio = Arc::new(MockAudio::default());
    let hotkey = Arc::new(MockHotkey::default());
    let state = AppState {
        audio,
        hotkey: hotkey.clone(),
        permissions: Arc::new(MockPermissions),
        transcriber,
        paste,
        model_gate: Arc::new(MockModelGate(true)),
        recording_phase: Arc::new(AtomicU8::new(RecordingPhase::Idle as u8)),
        cap_cancel_tx: Arc::new(Mutex::new(None)),
        cleanup: Arc::new(RwLock::new(CleanupConfig {
            mode: CleanupMode::None,
            provider: None,
        })),
    };
    (state, hotkey)
}

/// Polls `state.phase()` on a bounded real-time schedule (see the module doc
/// comment on why this can't use a paused clock).
async fn wait_for_idle(state: &AppState) -> bool {
    for _ in 0..80 {
        if state.phase() == RecordingPhase::Idle {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn full_dictation_cycle_through_app_state_start_stop_emits_in_order_and_pastes() {
    let transcriber: Arc<dyn Transcriber> =
        Arc::new(StubTranscriber("integration test transcript".to_string()));
    let (paste, paste_rx) = CapturingPaste::new();
    let (state, _hotkey) = build_state(transcriber, paste);
    let emitter = EventLogEmitter::default();

    recording::start(&state, &emitter).expect("start ok");
    assert_eq!(state.phase(), RecordingPhase::Recording);

    recording::stop(&state, &emitter).expect("stop ok");
    assert_eq!(state.phase(), RecordingPhase::Transcribing);

    let pasted = tokio::time::timeout(Duration::from_secs(5), paste_rx)
        .await
        .expect("paste reached within 5s")
        .expect("paste sender not dropped");
    assert_eq!(pasted, "integration test transcript");

    assert_eq!(
        emitter.events(),
        vec![
            "recording-started".to_string(),
            "recording-stopped".to_string(),
            "transcript-ready".to_string(),
            "cleanup-done".to_string(),
        ]
    );

    assert!(
        wait_for_idle(&state).await,
        "phase reset to Idle within ~4s of real time"
    );
}

#[tokio::test]
async fn paste_failure_surfaces_paste_failed_event_and_still_reaches_done() {
    let transcriber: Arc<dyn Transcriber> = Arc::new(StubTranscriber("hi".to_string()));
    let paste: Arc<dyn TextPaste> = Arc::new(FailingPaste::new(PasteError::NoFocusedField));
    let (state, hotkey) = build_state(transcriber, paste);
    let emitter = EventLogEmitter::default();

    recording::start(&state, &emitter).expect("start ok");
    recording::stop(&state, &emitter).expect("stop ok");

    assert!(
        wait_for_idle(&state).await,
        "phase reset to Idle within ~4s of real time"
    );

    assert_eq!(
        emitter.events(),
        vec![
            "recording-started".to_string(),
            "recording-stopped".to_string(),
            "transcript-ready".to_string(),
            "cleanup-done".to_string(),
            "paste-failed:NoFocusedField:true".to_string(),
        ]
    );
    assert!(hotkey.stop_cues.load(Ordering::SeqCst), "stop cue played");
}
