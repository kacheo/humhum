//! Recording controller — the **recording half** of the lifecycle (LLD §717 /
//! §719 / §721). Both the IPC command handlers and the hotkey handler call into
//! here, so `start`/`stop`/`cancel` have a single code path.
//!
//! Per LLD §3.9, event emission lives at this orchestration layer (not in the
//! platform traits, which stay Tauri-free). Emits go through the
//! [`RecordingEmitter`] seam so the controller is unit-testable with a fake
//! emitter and no Tauri runtime; the production impl is `tauri::AppHandle`.
//!
//! `start()` spawns capture and arms the five-minute safety timer. The
//! post-stop pipeline transcribes, cleans up, and pastes the result.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::audio::cap_timer::{safety_cap_timer, CapEvent};
use crate::audio::{AudioCapture, AudioError};
use crate::error::AppError;
use crate::hotkey::AudioCue;
use crate::pipeline::{self, PipelineEmitter};
use crate::state::{AppState, RecordingPhase};

/// Bound a recording that never receives its stop, such as a missed hotkey
/// release across a focus switch.
const SAFETY_CAP: Duration = Duration::from_secs(300);

/// The three data-free recording lifecycle events (LLD §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingSignal {
    Started,
    Stopped,
    Cancelled,
}

impl RecordingSignal {
    fn event_name(self) -> &'static str {
        match self {
            RecordingSignal::Started => "recording-started",
            RecordingSignal::Stopped => "recording-stopped",
            RecordingSignal::Cancelled => "recording-cancelled",
        }
    }
}

/// Sink for the recording lifecycle events. Production impl is `tauri::AppHandle`;
/// tests use a recording fake.
pub trait RecordingEmitter {
    fn emit_recording(&self, signal: RecordingSignal);
    /// A recording attempt failed *before* it started (e.g. a missing permission)
    /// — emitted as `recording-error {code,message}` so the overlay can surface
    /// why nothing happened. `start` returns `Err` before emitting `Started`, so
    /// without this the failure is silent.
    fn emit_recording_error(&self, code: &str, message: &str);
}

impl<R: tauri::Runtime> RecordingEmitter for tauri::AppHandle<R> {
    fn emit_recording(&self, signal: RecordingSignal) {
        use tauri::Emitter;
        if let Err(e) = self.emit(signal.event_name(), ()) {
            log::warn!("recording: emit {} failed: {e}", signal.event_name());
        }
    }
    fn emit_recording_error(&self, code: &str, message: &str) {
        use tauri::Emitter;
        // Reuse the {code,message} payload shape the overlay already understands.
        if let Err(e) = self.emit(
            "recording-error",
            crate::events::PipelineError {
                code: code.to_string(),
                message: message.to_string(),
            },
        ) {
            log::warn!("recording: emit recording-error failed: {e}");
        }
    }
}

/// Idle → Recording (LLD §717). Guards on permissions, model-readiness, and
/// phase; plays the start cue, emits `recording-started`, opens CPAL, and (Free
/// tier) arms the cap timer + its auto-stop consumer. On the normal stop path
/// the capture task drives the post-stop pipeline (transcribe → cleanup → paste).
pub fn start<E>(state: &AppState, emitter: &E) -> Result<(), AppError>
where
    E: RecordingEmitter + PipelineEmitter + Clone + 'static,
{
    if !state.permissions.check().all_granted() {
        // Surface the blocked attempt so the press isn't silent — the overlay
        // shows this and points at Preferences → Permissions to fix it.
        emitter.emit_recording_error(
            "permission_denied",
            "Permission needed to record — open Preferences → Permissions to grant access.",
        );
        return Err(AppError::from(AudioError::PermissionDenied));
    }
    // Fail fast if the selected model isn't downloaded — otherwise the miss only
    // surfaces after a full recording (LLD §9.1 / bead mm0.1). The model is
    // captured here and threaded into the pipeline, so transcription uses the
    // model that was armed at start even if prefs change mid-recording (intended:
    // you record against the model you started with).
    let model = crate::prefs::load_prefs().whisper_model;
    if !state.model_gate.is_ready(model) {
        // Surface the blocked attempt so a hotkey press isn't silent — the
        // overlay shows this and points at the transcription-model download in
        // Preferences → AI (mirrors the permission_denied path above).
        emitter.emit_recording_error(
            "model_not_ready",
            "Transcription model isn't downloaded yet — open Preferences → AI to download it.",
        );
        return Err(AppError::from(
            crate::transcription::TranscriptionError::ModelNotReady,
        ));
    }
    // Fast reject (non-authoritative): if we're plainly not in a claimable phase
    // — a recording is in flight, or its post-stop pipeline is still running —
    // reject *without opening the mic*. Otherwise arming only to fail the
    // authoritative claim below would briefly light the capture stream on every
    // rejected re-press (an orange-indicator flicker). Mirrors
    // `claim_recording`'s claimable set; the atomic claim after the arm stays the
    // race-free authority.
    if !matches!(
        state.phase(),
        RecordingPhase::Idle | RecordingPhase::Done | RecordingPhase::Error
    ) {
        return Err(AppError::AlreadyRecording);
    }

    // Synchronous arm FIRST — spawn the capture thread and install the stop
    // handle NOW, *before* the phase becomes `Recording`. This is what closes
    // the lost-signal race: a racing stop()/cancel() can only win its phase CAS
    // once phase == Recording (set just below), which now happens strictly after
    // the stop handle exists — so `audio.stop()`/`cancel()` can never no-op
    // against a not-yet-installed handle and orphan the capture stream (leaving
    // the mic hot forever). Also subsumes the I1 guarantee: any stop after this
    // returns finds the handle installed and buffered on its channel, never lost.
    // A bare arm failure surfaces the error without ever claiming the phase.
    let session = match state.audio.start() {
        Ok(s) => s,
        // A concurrent caller won the audio layer's `is_recording` guard: keep
        // the canonical `AlreadyRecording` contract rather than surfacing a less
        // meaningful `stream_error` on a double-start race.
        Err(AudioError::AlreadyRecording) => return Err(AppError::AlreadyRecording),
        Err(e) => return Err(AppError::from(e)),
    };

    // Atomic Idle→Recording claim: exactly one of two racing callers (hotkey
    // thread + IPC) wins. A genuine race — two callers both passing the fast
    // reject, then one losing the atomic claim — reaches here after arming, so
    // tear down the stream it just opened: explicit `stop()` releases the mic
    // immediately, and dropping the (unpolled) drain future runs its RAII guard
    // to join the capture thread and clear `is_recording`. Return with no
    // `Started` emitted.
    if !state.claim_recording() {
        let _ = state.audio.stop();
        drop(session);
        return Err(AppError::AlreadyRecording);
    }

    state.hotkey.play_cue(AudioCue::Start);
    // Phase is now `Recording` and the capture stream is armed with a live,
    // deliverable stop handle before this emit.
    emitter.emit_recording(RecordingSignal::Started);

    // The drain future yields the AudioBuffer when stop()/cancel() fires. On the
    // normal stop path (phase already `Transcribing`) the task drives the
    // pipeline; on a pre-stop capture *failure* `finish_capture_err` resets the
    // phase — otherwise a `DeviceUnavailable` would wedge `Recording` forever.
    //
    // The task lives ~2s past a successful paste because `run_pipeline`'s
    // terminal overlay reset is awaited inline. That tail is benign — the task is
    // fire-and-forget (nothing joins it), audio teardown already happened when the
    // drain future resolved, and a hotkey press during the window re-claims the
    // terminal phase (see `AppState::claim_recording`), making the pending reset a
    // no-op.
    let phase = state.recording_phase.clone();
    let cap = state.cap_cancel_tx.clone();
    let transcriber = state.transcriber.clone();
    let paste = state.paste.clone();
    let hotkey = state.hotkey.clone();
    let task_emitter = emitter.clone();
    // Snapshot the cleanup config out of the lock now (cheap `Arc` clone) so the
    // pipeline never holds the non-`Send` read guard across an `.await`.
    let (cleanup_mode, cleanup_provider) = {
        let cfg = state.cleanup.read().unwrap();
        (cfg.mode, cfg.provider.clone())
    };
    tauri::async_runtime::spawn(async move {
        match session.await {
            // Normal stop already moved the phase to Transcribing — run the pipeline.
            Ok(buf) if phase.load(Ordering::SeqCst) == RecordingPhase::Transcribing as u8 => {
                pipeline::run_pipeline(
                    buf,
                    model,
                    &transcriber,
                    &paste,
                    &hotkey,
                    &phase,
                    &task_emitter,
                    cleanup_mode,
                    cleanup_provider,
                )
                .await;
            }
            // Ok but the phase isn't Transcribing — e.g. cancelled, or the audio
            // layer's `MAX_CAPTURE` backstop self-terminated the stream. The
            // backstop releases the mic and clears `is_recording`; returning the
            // recording phase to Idle is the controller's `SAFETY_CAP`'s job, so
            // here we just drop the buffer.
            Ok(_) => log::debug!("capture ok but not in Transcribing; dropping buffer"),
            // Capture failed (possibly before any stop) — reset if still Recording.
            Err(e) => finish_capture_err(&phase, &cap, &task_emitter, e),
        }
    });

    // Arm the five-minute safety timer. An auto-stop follows the same path as
    // a hotkey stop, so a missed key release cannot leave capture running.
    let (cancel_tx, cancel_rx) = oneshot::channel();
    *state.cap_cancel_tx.lock().unwrap() = Some(cancel_tx);

    // Cap auto-stop consumer (bead 20h.4 / mm0.2): on `CapEvent::Stop`, run the
    // *same* stop transition as a hotkey stop, converging on the pipeline.
    let (cap_tx, cap_rx) = mpsc::channel::<CapEvent>(1);
    tauri::async_runtime::spawn(run_cap_consumer(
        cap_rx,
        state.recording_phase.clone(),
        state.audio.clone(),
        state.cap_cancel_tx.clone(),
        emitter.clone(),
    ));

    tauri::async_runtime::spawn(safety_cap_timer(SAFETY_CAP, cap_tx, cancel_rx));
    Ok(())
}

/// Called when the spawned `audio.start()` future resolves with an **error**.
///
/// Only resets state when the capture failed **before** a stop/cancel — detected
/// by an atomic `Recording → Idle` transition. If the phase is already
/// `Cancelled` (cancel path), `Transcribing` (normal stop, with a late
/// `cancelled` error), or a terminal pipeline phase, the transition fails and we
/// leave the orchestrated state untouched — no double emit.
fn finish_capture_err<E: RecordingEmitter>(
    phase: &AtomicU8,
    cap: &Mutex<Option<oneshot::Sender<()>>>,
    emitter: &E,
    e: AudioError,
) {
    if phase
        .compare_exchange(
            RecordingPhase::Recording as u8,
            RecordingPhase::Idle as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok()
    {
        log::warn!("recording failed before stop: {e}; resetting to Idle");
        cap.lock().unwrap().take();
        emitter.emit_recording(RecordingSignal::Cancelled);
    } else {
        // Expected on the cancel path (phase already Cancelled) or a late error
        // after a normal stop (Transcribing).
        log::debug!("capture ended: {e}");
    }
}

/// `Recording → Transcribing` side effects, shared by the hotkey [`stop`] and the
/// cap auto-stop consumer. The atomic claim makes exactly one of N racing callers
/// win; losers are a pure no-op (`false`). Closes CPAL, cancels the cap timer,
/// and emits `recording-stopped` exactly once.
fn perform_stop<E: RecordingEmitter>(
    phase: &AtomicU8,
    audio: &Arc<dyn AudioCapture>,
    cap: &Mutex<Option<oneshot::Sender<()>>>,
    emitter: &E,
) -> bool {
    if phase
        .compare_exchange(
            RecordingPhase::Recording as u8,
            RecordingPhase::Transcribing as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return false;
    }
    let _ = audio.stop();
    cap.lock().unwrap().take(); // drop sender → cancel cap timer
    emitter.emit_recording(RecordingSignal::Stopped);
    true
}

/// Drain cap events for one recording. On `CapEvent::Stop`, run the same stop
/// transition as a hotkey stop (converging on the pipeline) and exit. `Warning`
/// re-loops — the timer already notified the frontend; the consumer takes no
/// action on it.
///
/// **Lifecycle invariant (no cross-recording stop):** the only `cap_tx` owner is
/// the timer task, and every stop/cancel drops the cap *cancel* sender, which
/// returns the timer, which drops `cap_tx` → `recv()` yields `None` → this loop
/// ends. So each recording's consumer terminates before the next one starts.
async fn run_cap_consumer<E: RecordingEmitter>(
    mut cap_rx: mpsc::Receiver<CapEvent>,
    phase: Arc<AtomicU8>,
    audio: Arc<dyn AudioCapture>,
    cap: crate::state::CapCancelTx,
    emitter: E,
) {
    if cap_rx.recv().await.is_some() {
        perform_stop(&phase, &audio, &cap, &emitter);
    }
}

/// Recording → Transcribing (LLD §719). Closes CPAL, cancels the cap timer, and
/// emits `recording-stopped`.
///
/// The `Recording → Transcribing` claim is atomic ([`AppState::begin_stop`]), so
/// of two racing callers (hotkey thread + IPC) exactly one runs the side effects
/// and the loser is a pure no-op — no double emit. Not currently recording is a
/// no-op too (`not_recording` wire code lands with its command, bead 6s5).
pub fn stop<E: RecordingEmitter>(state: &AppState, emitter: &E) -> Result<(), AppError> {
    perform_stop(
        &state.recording_phase,
        &state.audio,
        &state.cap_cancel_tx,
        emitter,
    );
    Ok(())
}

/// Recording → Idle (LLD §721). Drops the buffer (via `cancel`), cancels the cap
/// timer, and emits `recording-cancelled`.
///
/// Like [`stop`], the transition is an atomic claim ([`AppState::begin_cancel`]):
/// only the single winner touches `audio.cancel()` / the cap timer and emits.
/// This is what closes the stop→cancel race — a `cancel` losing to a concurrent
/// `stop` now does nothing, so it can never flip a captured buffer into a
/// `cancelled` error.
pub fn cancel<E: RecordingEmitter>(state: &AppState, emitter: &E) -> Result<(), AppError> {
    if !state.begin_cancel() {
        return Ok(());
    }
    state.audio.cancel();
    state.cap_cancel_tx.lock().unwrap().take();
    emitter.emit_recording(RecordingSignal::Cancelled);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioBuffer, AudioCapture};
    use crate::hotkey::{HotkeyCapture, HotkeyCombo, HotkeyError, HotkeyEvent};
    use crate::permissions::{Permissions, PermissionsState};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    #[derive(Default)]
    struct MockAudio {
        armed: AtomicBool, // set synchronously by start() (the arm)
        stopped: AtomicBool,
        cancelled: AtomicBool,
        // Fired by stop()/cancel() to resolve the drain future — models the real
        // lifecycle where the buffer only returns once capture stops.
        stop_tx: Mutex<Option<oneshot::Sender<()>>>,
    }
    impl AudioCapture for MockAudio {
        fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
            AudioError,
        > {
            self.armed.store(true, Ordering::SeqCst);
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
            self.stopped.store(true, Ordering::SeqCst);
            if let Some(tx) = self.stop_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            Ok(())
        }
        fn cancel(&self) {
            self.cancelled.store(true, Ordering::SeqCst);
            if let Some(tx) = self.stop_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    }

    #[derive(Default)]
    struct MockHotkey {
        start_cues: AtomicBool,
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
            if matches!(cue, AudioCue::Start) {
                self.start_cues.store(true, Ordering::SeqCst);
            }
        }
    }

    use crate::paste::{PasteError, TextPaste};
    use crate::transcription::{Transcriber, TranscriptionError, WhisperModel};

    /// Canned-`Ok` transcriber so the harness can build an `AppState`; the
    /// recording-controller tests don't drive the pipeline (covered in
    /// `pipeline.rs`), so a fixed value suffices.
    struct MockTranscriber;
    impl Transcriber for MockTranscriber {
        fn transcribe(
            &self,
            _buffer: AudioBuffer,
            _model: WhisperModel,
        ) -> Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send + 'static>>
        {
            Box::pin(async { Ok("mock".to_string()) })
        }
    }

    struct MockPaste;
    impl TextPaste for MockPaste {
        fn paste<'a>(
            &'a self,
            _text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), PasteError>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    /// Reports the selected model as downloaded so controller tests exercise the
    /// real start/stop flow without touching `$HOME`.
    struct MockModelGate(bool);
    impl crate::models::ModelGate for MockModelGate {
        fn is_ready(&self, _model: WhisperModel) -> bool {
            self.0
        }
    }

    struct MockPermissions {
        granted: bool,
    }
    impl Permissions for MockPermissions {
        fn check(&self) -> PermissionsState {
            PermissionsState {
                microphone: self.granted,
                accessibility: self.granted,
                input_monitoring: self.granted,
            }
        }
        fn request_microphone(&self) -> Result<(), ()> {
            Ok(())
        }
        fn open_accessibility_settings(&self) {}
        fn open_input_monitoring_settings(&self) {}
    }

    #[derive(Clone, Default)]
    struct MockEmitter {
        signals: Arc<Mutex<Vec<RecordingSignal>>>,
        errors: Arc<Mutex<Vec<(String, String)>>>,
    }
    impl RecordingEmitter for MockEmitter {
        fn emit_recording(&self, signal: RecordingSignal) {
            self.signals.lock().unwrap().push(signal);
        }
        fn emit_recording_error(&self, code: &str, message: &str) {
            self.errors
                .lock()
                .unwrap()
                .push((code.to_string(), message.to_string()));
        }
    }
    // The controller tests don't assert pipeline events (covered in `pipeline.rs`);
    // a no-op impl satisfies `start`'s `PipelineEmitter` bound.
    impl PipelineEmitter for MockEmitter {
        fn emit_transcript_ready(&self, _raw: &str) {}
        fn emit_cleanup_done(&self, _text: &str) {}
        fn emit_pipeline_error(&self, _code: &str, _message: &str) {}
        fn emit_paste_failed(&self, _reason: crate::events::PasteFailReason, _copied: bool) {}
        fn emit_transcription_discarded(&self) {}
    }

    struct Harness {
        state: AppState,
        audio: Arc<MockAudio>,
        hotkey: Arc<MockHotkey>,
        emitter: MockEmitter,
    }

    fn harness(perms_granted: bool) -> Harness {
        let audio = Arc::new(MockAudio::default());
        let hotkey = Arc::new(MockHotkey::default());
        let state = AppState {
            audio: audio.clone(),
            hotkey: hotkey.clone(),
            permissions: Arc::new(MockPermissions {
                granted: perms_granted,
            }),
            recording_phase: Arc::new(AtomicU8::new(RecordingPhase::Idle as u8)),
            cap_cancel_tx: Arc::new(Mutex::new(None)),
            cleanup: Arc::new(RwLock::new(crate::cleanup::CleanupConfig {
                mode: crate::cleanup::CleanupMode::None,
                provider: None,
            })),
            transcriber: Arc::new(MockTranscriber),
            paste: Arc::new(MockPaste),
            model_gate: Arc::new(MockModelGate(true)),
        };
        Harness {
            state,
            audio,
            hotkey,
            emitter: MockEmitter::default(),
        }
    }

    fn signals(e: &MockEmitter) -> Vec<RecordingSignal> {
        e.signals.lock().unwrap().clone()
    }

    /// Polls `condition` until it's true or 2s elapse, instead of a fixed
    /// sleep — keeps tests that wait on the real spawned-capture-task runtime
    /// fast on a quiet machine and non-flaky under full-suite parallel load.
    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !condition() {
            if std::time::Instant::now() >= deadline {
                panic!("condition did not become true within 2s");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn recording_signal_event_names_match_the_wire_contract() {
        assert_eq!(RecordingSignal::Started.event_name(), "recording-started");
        assert_eq!(RecordingSignal::Stopped.event_name(), "recording-stopped");
        assert_eq!(
            RecordingSignal::Cancelled.event_name(),
            "recording-cancelled"
        );
    }

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

    #[test]
    fn mock_permissions_side_effect_methods_are_infallible_no_ops() {
        let perms = MockPermissions { granted: true };
        assert!(perms.request_microphone().is_ok());
        perms.open_accessibility_settings();
        perms.open_input_monitoring_settings();
    }

    #[test]
    fn mock_emitter_pipeline_trait_methods_are_no_ops() {
        let emitter = MockEmitter::default();
        // `PipelineEmitter`: satisfies `start`'s bound so the controller can
        // hand its emitter straight to `run_pipeline`; the controller tests
        // don't assert on pipeline events (covered in `pipeline.rs`), so call
        // every method directly here.
        emitter.emit_transcript_ready("raw");
        emitter.emit_cleanup_done("clean");
        emitter.emit_pipeline_error("code", "message");
        emitter.emit_paste_failed(crate::events::PasteFailReason::NoFocusedField, true);
        emitter.emit_transcription_discarded();
    }

    #[test]
    fn start_idle_transitions_to_recording_and_emits_started() {
        let h = harness(true);
        start(&h.state, &h.emitter).expect("start ok");

        assert_eq!(h.state.phase(), RecordingPhase::Recording);
        assert!(
            h.hotkey.start_cues.load(Ordering::SeqCst),
            "Start cue played"
        );
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Started]);
    }

    /// I1 regression (chp): `start` arms the capture synchronously (the stop
    /// handle is installed before `start` returns), so a `stop` issued right after
    /// reaches the capture and drives the transition — it can never be lost.
    #[test]
    fn start_arms_capture_synchronously_so_a_following_stop_is_delivered() {
        let h = harness(true);
        start(&h.state, &h.emitter).expect("start ok");
        assert!(
            h.audio.armed.load(Ordering::SeqCst),
            "capture armed before start() returned"
        );
        assert_eq!(h.state.phase(), RecordingPhase::Recording);
        // A stop issued immediately after start() reaches the armed capture.
        stop(&h.state, &h.emitter).expect("stop ok");
        assert!(
            h.audio.stopped.load(Ordering::SeqCst),
            "stop delivered to the armed capture"
        );
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
    }

    #[test]
    fn start_rejected_when_already_recording() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        assert!(matches!(
            start(&h.state, &h.emitter),
            Err(AppError::AlreadyRecording)
        ));
        // Fast-reject path (leak fix): a start rejected because a recording is
        // already in flight must NOT open the capture stream at all — no
        // orphaned, or even momentarily-live, mic on rejection.
        assert!(
            !h.audio.armed.load(Ordering::SeqCst),
            "capture must not be armed when rejected before the claim"
        );
        // No spurious emit on the rejected path.
        assert!(signals(&h.emitter).is_empty());
    }

    #[test]
    fn start_rejected_when_permissions_missing() {
        let h = harness(false);
        assert_eq!(
            start(&h.state, &h.emitter).unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(h.state.phase(), RecordingPhase::Idle);
        // The blocked attempt is surfaced (not silent) so the overlay can show it.
        let errors = h.emitter.errors.lock().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "permission_denied");
        // No lifecycle Started/Stopped should have fired on the rejected path.
        assert!(signals(&h.emitter).is_empty());
    }

    #[test]
    fn stop_recording_transitions_to_transcribing_and_emits_stopped() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        stop(&h.state, &h.emitter).expect("stop ok");

        assert!(h.audio.stopped.load(Ordering::SeqCst), "audio.stop called");
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Stopped]);
        assert!(h.state.cap_cancel_tx.lock().unwrap().is_none());
    }

    #[test]
    fn stop_when_idle_is_noop() {
        let h = harness(true);
        stop(&h.state, &h.emitter).expect("stop ok");
        assert!(!h.audio.stopped.load(Ordering::SeqCst));
        assert_eq!(h.state.phase(), RecordingPhase::Idle);
        assert!(signals(&h.emitter).is_empty());
    }

    #[test]
    fn cancel_recording_resets_to_idle_and_emits() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        cancel(&h.state, &h.emitter).expect("cancel ok");

        assert!(
            h.audio.cancelled.load(Ordering::SeqCst),
            "audio.cancel called"
        );
        // Idle (not Cancelled): the Phase 15 pipeline that would advance Cancelled
        // is deferred, so cancel resets to Idle to keep recording repeatable.
        assert_eq!(h.state.phase(), RecordingPhase::Idle);
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Cancelled]);
    }

    /// Owns ALL `IS_PRO` mutation in the test suite so the global atomic can't
    /// race other tests. Verifies a recording-duration cap is armed on **both**
    #[test]
    fn start_rejected_when_model_not_ready() {
        let mut h = harness(true);
        // Override the gate to report the model missing.
        h.state.model_gate = std::sync::Arc::new(MockModelGate(false));
        assert_eq!(
            start(&h.state, &h.emitter).unwrap_err().code(),
            "model_not_ready"
        );
        assert_eq!(h.state.phase(), RecordingPhase::Idle);
        // The blocked attempt is surfaced (not silent) so the overlay can show
        // why nothing happened and point at the model download — otherwise the
        // hotkey handler only logs it and the press looks dead.
        let errors = h.emitter.errors.lock().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "model_not_ready");
        assert!(signals(&h.emitter).is_empty());
    }

    // --- Cap auto-stop / shared stop transition (`perform_stop`) ---

    #[test]
    fn perform_stop_from_recording_transitions_and_emits_once() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        let won = perform_stop(
            &h.state.recording_phase,
            &h.state.audio,
            &h.state.cap_cancel_tx,
            &h.emitter,
        );
        assert!(won, "the single caller wins the stop claim");
        assert!(h.audio.stopped.load(Ordering::SeqCst), "audio.stop called");
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Stopped]);
    }

    #[test]
    fn perform_stop_from_non_recording_is_noop() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Transcribing);
        let won = perform_stop(
            &h.state.recording_phase,
            &h.state.audio,
            &h.state.cap_cancel_tx,
            &h.emitter,
        );
        assert!(!won, "no claim when not Recording");
        assert!(!h.audio.stopped.load(Ordering::SeqCst));
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert!(signals(&h.emitter).is_empty());
    }

    #[tokio::test]
    async fn cap_stop_event_drives_the_same_stop_transition() {
        // The headline cap auto-stop: a `CapEvent::Stop` delivered to the
        // consumer must converge on `perform_stop` (Recording → Transcribing,
        // single `Stopped`), exactly like a hotkey stop.
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        let (cancel_tx, _cancel_rx) = oneshot::channel();
        *h.state.cap_cancel_tx.lock().unwrap() = Some(cancel_tx);

        let (tx, rx) = mpsc::channel(1);
        tx.send(CapEvent::Stop).await.unwrap();
        run_cap_consumer(
            rx,
            h.state.recording_phase.clone(),
            h.state.audio.clone(),
            h.state.cap_cancel_tx.clone(),
            h.emitter.clone(),
        )
        .await;

        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert!(h.audio.stopped.load(Ordering::SeqCst), "audio.stop called");
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Stopped]);
        assert!(
            h.state.cap_cancel_tx.lock().unwrap().is_none(),
            "cap timer cancelled"
        );
    }

    #[tokio::test]
    async fn cap_consumer_exits_when_timer_drops_sender_without_stopping() {
        // Lifecycle invariant: when the timer drops `cap_tx` (every stop/cancel
        // drops the cancel sender → timer returns), the consumer ends without
        // touching state — so it can't stop a *later* recording.
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        let (tx, rx) = mpsc::channel::<CapEvent>(1);
        drop(tx); // timer ended

        run_cap_consumer(
            rx,
            h.state.recording_phase.clone(),
            h.state.audio.clone(),
            h.state.cap_cancel_tx.clone(),
            h.emitter.clone(),
        )
        .await;

        assert_eq!(
            h.state.phase(),
            RecordingPhase::Recording,
            "phase untouched"
        );
        assert!(!h.audio.stopped.load(Ordering::SeqCst));
        assert!(signals(&h.emitter).is_empty());
    }

    // --- I3: capture-completion error handling (`finish_capture_err`) ---

    #[test]
    fn finish_capture_err_before_stop_resets_to_idle_and_emits() {
        let phase = AtomicU8::new(RecordingPhase::Recording as u8);
        let (tx, _rx) = oneshot::channel();
        let cap = Mutex::new(Some(tx));
        let emitter = MockEmitter::default();

        finish_capture_err(&phase, &cap, &emitter, AudioError::DeviceUnavailable);

        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
        assert!(
            cap.lock().unwrap().is_none(),
            "cap timer cancelled on reset"
        );
        assert_eq!(signals(&emitter), vec![RecordingSignal::Cancelled]);
    }

    /// WS4: microphone permission revoked mid-capture surfaces as an `AudioError`
    /// from the stream. Before a stop, that must reset `Recording → Idle` and emit
    /// `Cancelled` so the UI never wedges on a recording the OS just killed.
    #[test]
    fn mic_revoked_mid_capture_resets_to_idle_and_emits_cancelled() {
        let phase = AtomicU8::new(RecordingPhase::Recording as u8);
        let (tx, _rx) = oneshot::channel();
        let cap = Mutex::new(Some(tx));
        let emitter = MockEmitter::default();

        // PermissionDenied models the mid-session revocation killing the stream.
        finish_capture_err(&phase, &cap, &emitter, AudioError::PermissionDenied);

        assert_eq!(phase.load(Ordering::SeqCst), RecordingPhase::Idle as u8);
        assert!(
            cap.lock().unwrap().is_none(),
            "cap timer cancelled on reset"
        );
        assert_eq!(signals(&emitter), vec![RecordingSignal::Cancelled]);
    }

    #[test]
    fn finish_capture_err_on_cancel_path_does_not_double_emit() {
        // cancel() already set Cancelled + emitted; the late Err("cancelled")
        // from audio.start() must not re-emit or reset.
        let phase = AtomicU8::new(RecordingPhase::Cancelled as u8);
        let cap = Mutex::new(None);
        let emitter = MockEmitter::default();

        finish_capture_err(
            &phase,
            &cap,
            &emitter,
            AudioError::StreamError("cancelled".into()),
        );

        assert_eq!(
            phase.load(Ordering::SeqCst),
            RecordingPhase::Cancelled as u8
        );
        assert!(signals(&emitter).is_empty());
    }

    // --- I2: atomic claim under concurrency ---

    #[test]
    fn concurrent_start_has_exactly_one_winner() {
        let _g = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let h = harness(true);
        let state = Arc::new(h.state);
        let emitter = h.emitter.clone();

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let st = state.clone();
                let em = emitter.clone();
                std::thread::spawn(move || start(&st, &em))
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let oks = results.iter().filter(|r| r.is_ok()).count();
        let rejected = results
            .iter()
            .filter(|r| matches!(r, Err(AppError::AlreadyRecording)))
            .count();
        assert_eq!(oks, 1, "exactly one start wins the atomic claim");
        assert_eq!(rejected, 1, "the loser gets AlreadyRecording");
        assert_eq!(
            signals(&emitter),
            vec![RecordingSignal::Started],
            "only the winner emits Started"
        );
        assert!(
            state.cap_cancel_tx.lock().unwrap().is_some(),
            "exactly one cap timer armed"
        );
    }

    #[test]
    fn concurrent_stop_emits_stopped_once() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        let state = Arc::new(h.state);
        let emitter = h.emitter.clone();

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let st = state.clone();
                let em = emitter.clone();
                std::thread::spawn(move || stop(&st, &em))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().expect("stop ok");
        }

        // The atomic Recording→Transcribing claim has exactly one winner: a single
        // Stopped emit, even though both threads observed Recording on entry.
        assert_eq!(state.phase(), RecordingPhase::Transcribing);
        assert_eq!(signals(&emitter), vec![RecordingSignal::Stopped]);
    }

    #[test]
    fn stop_then_cancel_is_single_winner() {
        // After a stop wins (phase = Transcribing), a racing/late cancel must be a
        // pure no-op: no Cancelled emit, no audio.cancel — otherwise it could flip
        // the captured buffer into a `cancelled` error (the stop→cancel data race).
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Recording);
        stop(&h.state, &h.emitter).expect("stop ok");
        cancel(&h.state, &h.emitter).expect("cancel ok");

        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert!(
            !h.audio.cancelled.load(Ordering::SeqCst),
            "audio.cancel not called"
        );
        assert_eq!(signals(&h.emitter), vec![RecordingSignal::Stopped]);
    }

    // --- M1: stop/cancel from a non-Recording phase ---

    #[test]
    fn stop_from_transcribing_is_noop() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Transcribing);
        stop(&h.state, &h.emitter).expect("stop ok");
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert!(!h.audio.stopped.load(Ordering::SeqCst));
        assert!(signals(&h.emitter).is_empty());
    }

    #[test]
    fn cancel_from_transcribing_does_not_emit_or_change_phase() {
        let h = harness(true);
        h.state.set_phase(RecordingPhase::Transcribing);
        cancel(&h.state, &h.emitter).expect("cancel ok");
        assert_eq!(h.state.phase(), RecordingPhase::Transcribing);
        assert!(signals(&h.emitter).is_empty());
    }

    // --- Synchronous and asynchronous `audio.start()` failure paths ---

    /// Fails the synchronous arm itself (`state.audio.start()` returns `Err`
    /// before any future is even created) — models a CPAL device that's gone
    /// missing/busy at the moment `start()` tries to open the stream.
    struct FailingStartAudio;
    impl AudioCapture for FailingStartAudio {
        fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
            AudioError,
        > {
            Err(AudioError::DeviceUnavailable)
        }
        fn stop(&self) -> Result<(), AudioError> {
            Ok(())
        }
        fn cancel(&self) {}
    }

    /// Arms successfully but the drain future resolves `Err` almost
    /// immediately (no external stop/cancel needed to trigger it) — models a
    /// stream that dies on its own right after opening.
    struct ImmediatelyFailingAudio;
    impl AudioCapture for ImmediatelyFailingAudio {
        fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
            AudioError,
        > {
            Ok(Box::pin(async {
                Err(AudioError::StreamError("boom".into()))
            }))
        }
        fn stop(&self) -> Result<(), AudioError> {
            Ok(())
        }
        fn cancel(&self) {}
    }

    /// Arms successfully but, as a side effect of arming, advances the shared
    /// phase out of a claimable state — deterministically forcing `start`'s
    /// post-arm `claim_recording()` to fail, exactly as a genuine two-caller
    /// race would. Holds a clone of the phase atomic so it can mutate it
    /// mid-`start()`; tracks `armed`/`stopped` so the teardown can be asserted.
    struct ClaimStealingAudio {
        phase: Arc<AtomicU8>,
        armed: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
        stop_tx: Mutex<Option<oneshot::Sender<()>>>,
    }
    impl AudioCapture for ClaimStealingAudio {
        fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
            AudioError,
        > {
            self.armed.store(true, Ordering::SeqCst);
            // "Steal" the claim: move the phase to Recording so the controller's
            // own claim_recording() below fails, as if another caller won.
            self.phase
                .store(RecordingPhase::Recording as u8, Ordering::SeqCst);
            let (tx, rx) = oneshot::channel();
            *self.stop_tx.lock().unwrap() = Some(tx);
            Ok(Box::pin(async move {
                let _ = rx.await;
                Ok(AudioBuffer {
                    samples: Vec::new(),
                    sample_rate: 16_000,
                    duration_ms: 0,
                })
            }))
        }
        fn stop(&self) -> Result<(), AudioError> {
            self.stopped.store(true, Ordering::SeqCst);
            if let Some(tx) = self.stop_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            Ok(())
        }
        fn cancel(&self) {}
    }

    /// Deterministic cover for the arm-then-lost-claim teardown branch (the path
    /// a real two-caller race hits): the fast reject passes (phase Idle), the arm
    /// opens the stream, then `claim_recording()` fails — so `start` MUST tear the
    /// just-armed stream down (`stop()`), never leaving an orphaned live mic, and
    /// must reject without emitting `Started`.
    #[test]
    fn start_arm_then_lost_claim_tears_down_the_stream() {
        let phase = Arc::new(AtomicU8::new(RecordingPhase::Idle as u8));
        let armed = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let audio = Arc::new(ClaimStealingAudio {
            phase: phase.clone(),
            armed: armed.clone(),
            stopped: stopped.clone(),
            stop_tx: Mutex::new(None),
        });
        let mut state = state_with_audio(audio);
        // Share the exact phase atomic the audio double mutates mid-arm.
        state.recording_phase = phase;
        let emitter = MockEmitter::default();

        let err = start(&state, &emitter).expect_err("a lost post-arm claim must reject");
        assert!(matches!(err, AppError::AlreadyRecording));
        assert!(
            armed.load(Ordering::SeqCst),
            "capture was armed before the claim"
        );
        assert!(
            stopped.load(Ordering::SeqCst),
            "the armed stream was torn down on the lost claim — no orphaned mic"
        );
        assert!(
            signals(&emitter).is_empty(),
            "no Started emitted on the rejected path"
        );
    }

    /// The audio layer rejects the arm because a capture is already active
    /// (`CpalAudioCapture`'s shared `is_recording` guard) — modeled by returning
    /// `AudioError::AlreadyRecording` from `start()`.
    struct ArmRejectingAudio;
    impl AudioCapture for ArmRejectingAudio {
        fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn Future<Output = Result<AudioBuffer, AudioError>> + Send + 'static>>,
            AudioError,
        > {
            Err(AudioError::AlreadyRecording)
        }
        fn stop(&self) -> Result<(), AudioError> {
            Ok(())
        }
        fn cancel(&self) {}
    }

    /// Error-contract regression: on a genuine concurrent double-start the loser
    /// is rejected by the audio layer's own `is_recording` guard. The controller
    /// must map that to the canonical `AppError::AlreadyRecording` (code
    /// `already_recording`) — NOT leak a generic `stream_error` — and must not
    /// claim the phase or emit `Started`. (`concurrent_start_has_exactly_one_winner`
    /// uses `MockAudio`, which has no shared arm guard, so it only exercises the
    /// claim-level reject; this pins the arm-level path.)
    #[test]
    fn start_maps_audio_already_recording_to_the_canonical_contract() {
        let state = state_with_audio(Arc::new(ArmRejectingAudio));
        let emitter = MockEmitter::default();

        let err = start(&state, &emitter).expect_err("an arm rejection must surface");
        assert!(
            matches!(err, AppError::AlreadyRecording),
            "arm-level reject must map to the canonical AlreadyRecording, not stream_error"
        );
        assert_eq!(err.code(), "already_recording");
        assert_eq!(
            state.phase(),
            RecordingPhase::Idle,
            "no phase claimed on the rejected arm"
        );
        assert!(
            signals(&emitter).is_empty(),
            "no Started on the rejected arm"
        );
    }

    /// `harness()` is hardwired to `MockAudio`, so a raw `AppState` is built
    /// directly here, reusing every other existing fake.
    fn state_with_audio(audio: Arc<dyn AudioCapture>) -> AppState {
        AppState {
            audio,
            hotkey: Arc::new(MockHotkey::default()),
            permissions: Arc::new(MockPermissions { granted: true }),
            recording_phase: Arc::new(AtomicU8::new(RecordingPhase::Idle as u8)),
            cap_cancel_tx: Arc::new(Mutex::new(None)),
            cleanup: Arc::new(RwLock::new(crate::cleanup::CleanupConfig {
                mode: crate::cleanup::CleanupMode::None,
                provider: None,
            })),
            transcriber: Arc::new(MockTranscriber),
            paste: Arc::new(MockPaste),
            model_gate: Arc::new(MockModelGate(true)),
        }
    }

    #[test]
    fn start_synchronous_arm_failure_releases_the_claim_and_returns_err() {
        let state = state_with_audio(Arc::new(FailingStartAudio));
        let emitter = MockEmitter::default();

        let err = start(&state, &emitter).expect_err("a failed synchronous arm must surface");
        assert_eq!(err.code(), "device_unavailable");
        // The arm now runs before the phase claim, so a failed arm returns
        // without ever entering `Recording` — the phase stays `Idle`.
        assert_eq!(state.phase(), RecordingPhase::Idle);
        // No `Started` on the failed-arm path.
        assert!(signals(&emitter).is_empty());
    }

    #[test]
    fn start_with_capture_that_fails_immediately_resets_via_finish_capture_err() {
        let state = state_with_audio(Arc::new(ImmediatelyFailingAudio));
        let emitter = MockEmitter::default();

        start(&state, &emitter).expect("the synchronous arm itself succeeds");
        // (No immediate `phase == Recording` assert: the spawned capture task
        // runs on tauri's own multi-threaded runtime and can observe the drain
        // future's immediate `Err` and reset the phase to `Idle` before this
        // thread's next line — a race unrelated to what this test verifies. The
        // emitted `Started` below already proves the recording entered
        // `Recording`, since `Started` fires only after the phase claim.)

        // The spawned capture task runs on tauri's own async runtime — a real,
        // separate Tokio instance from this test — so poll for it to observe
        // the drain future resolving `Err` and call `finish_capture_err`.
        // `finish_capture_err` flips the phase to `Idle` *before* emitting
        // `Cancelled`, so wait on the signal too or this can race the assert
        // below (phase already Idle, signal not pushed yet).
        wait_until(|| signals(&emitter).len() >= 2);

        assert_eq!(
            state.phase(),
            RecordingPhase::Idle,
            "reset via finish_capture_err"
        );
        assert_eq!(
            signals(&emitter),
            vec![RecordingSignal::Started, RecordingSignal::Cancelled]
        );
    }

    /// Covers both `MockAudio::cancel()`'s tx-send-when-armed branch and the
    /// spawned capture task's `Ok(_) => log::debug!(...)` arm (the phase has
    /// already moved past `Transcribing` by the time the drain future
    /// resolves, so the task drops the buffer instead of running the
    /// pipeline). Every other `cancel()`-calling test sets the phase directly
    /// without calling `start()` first, so `MockAudio`'s `stop_tx` is never
    /// populated when they run.
    #[test]
    fn cancel_after_start_delivers_cancel_to_the_armed_capture() {
        let h = harness(true);
        start(&h.state, &h.emitter).expect("start ok");
        cancel(&h.state, &h.emitter).expect("cancel ok");

        assert!(
            h.audio.cancelled.load(Ordering::SeqCst),
            "cancel delivered to the armed capture"
        );
        assert_eq!(h.state.phase(), RecordingPhase::Idle);
        assert_eq!(
            signals(&h.emitter),
            vec![RecordingSignal::Started, RecordingSignal::Cancelled]
        );

        // Give the spawned capture task (real Tokio runtime) a moment to
        // observe the now-resolved drain future and drop the buffer.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            h.state.phase(),
            RecordingPhase::Idle,
            "capture task must not resurrect the phase"
        );
    }

    // `RecordingEmitter for tauri::AppHandle<R>` is generic over the runtime,
    // so `MockRuntime` (via `tauri::test`) exercises the real production impl
    // without a window/webview runtime.
    #[test]
    fn app_handle_emit_recording_does_not_panic_for_every_signal() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        for signal in [
            RecordingSignal::Started,
            RecordingSignal::Stopped,
            RecordingSignal::Cancelled,
        ] {
            handle.emit_recording(signal);
        }
    }

    #[test]
    fn app_handle_emit_recording_error_does_not_panic() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        handle.emit_recording_error("some_code", "some message");
    }
}
