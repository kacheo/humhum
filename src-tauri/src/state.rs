use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::oneshot;

/// `Arc`-cloneable cap-timer cancel slot.
pub type CapCancelTx = Arc<Mutex<Option<oneshot::Sender<()>>>>;

use crate::audio::AudioCapture;
use crate::hotkey::HotkeyCapture;
use crate::permissions::Permissions;

/// Shared application state, `manage`d by Tauri and reached from command handlers
/// via `tauri::State<AppState>` (LLD §3.7).
///
/// Holds the **live platform handles** (audio/hotkey/permissions) plus the
/// recording-phase atom and the cap-timer cancel sender. The audio buffer,
/// drop counter, and `is_recording` flag from LLD §651 live *inside*
/// [`crate::audio::CpalAudioCapture`] (Phase 6) — the capture owns its DSP
/// internals and returns the `AudioBuffer` from `start()`, so duplicating them
/// here would be dead state. `whisper_ctx` / `model_manager` are deferred to
/// Phases 9 / 14; `cleanup` holds the Phase-12 cleanup config.
pub struct AppState {
    pub audio: Arc<dyn AudioCapture>,
    pub hotkey: Arc<dyn HotkeyCapture>,
    pub permissions: Arc<dyn Permissions>,
    /// Transcription step (Phase 9), held so the post-stop pipeline (Phase 15)
    /// can run it from the capture task. Trait-object so tests inject a stub.
    pub transcriber: Arc<dyn crate::transcription::Transcriber>,
    /// Text injection (Phase 13), held so the pipeline can paste the result.
    pub paste: Arc<dyn crate::paste::TextPaste>,
    /// "Is the selected model downloaded?" gate for the `start` guard (mm0.1),
    /// injectable so the recording controller is testable without `$HOME`.
    pub model_gate: Arc<dyn crate::models::ModelGate>,
    /// `Arc` so the capture task can reset it on a pre-stop failure (see
    /// [`crate::recording::finish_capture`]).
    pub recording_phase: Arc<AtomicU8>,
    /// Drop the sender to cancel the running cap timer (early stop/cancel).
    pub cap_cancel_tx: CapCancelTx,
    /// Resolved cleanup config (mode + provider), built from prefs on startup and
    /// rebuilt by `set_prefs`. `Arc<dyn>` inside (not `Box`) so the pipeline can
    /// clone a cheap snapshot out of this lock and drop the guard before its
    /// `.await` — a `std::sync` read guard isn't `Send` across await points.
    pub cleanup: Arc<RwLock<crate::cleanup::CleanupConfig>>,
}

impl AppState {
    /// Wire the live platform handles via the [`crate::platform`] factories.
    pub fn new() -> Self {
        Self {
            audio: Arc::from(crate::platform::create_audio()),
            hotkey: Arc::from(crate::platform::create_hotkey()),
            permissions: Arc::from(crate::platform::create_permissions()),
            transcriber: Arc::from(crate::transcription::create()),
            paste: Arc::from(crate::platform::create_paste()),
            model_gate: Arc::from(crate::models::create_model_gate()),
            recording_phase: Arc::new(AtomicU8::new(RecordingPhase::Idle as u8)),
            cap_cancel_tx: Arc::new(Mutex::new(None)),
            // Off until prefs are loaded in `setup()`; dispatch treats this as
            // a no-op pass-through, so the pre-load window never blocks paste.
            cleanup: Arc::new(RwLock::new(crate::cleanup::CleanupConfig {
                mode: crate::cleanup::CleanupMode::None,
                provider: None,
            })),
        }
    }

    /// Current recording phase (any out-of-range discriminant reads as `Idle`).
    pub fn phase(&self) -> RecordingPhase {
        RecordingPhase::try_from(self.recording_phase.load(Ordering::SeqCst))
            .unwrap_or(RecordingPhase::Idle)
    }

    /// Store the recording phase.
    pub fn set_phase(&self, phase: RecordingPhase) {
        self.recording_phase.store(phase as u8, Ordering::SeqCst);
    }

    /// Atomically claim the recording slot for `recording::start`. Returns `true`
    /// for the single winner, `false` if a recording/pipeline is already in
    /// flight. Each `compare_exchange` is atomic, so two concurrent callers
    /// (hotkey thread + IPC) can never both win.
    ///
    /// Claimable phases: `Idle` **plus the terminal `Done`/`Error`**. The latter
    /// two close a 2s input dead-zone — after a paste the phase lingers at
    /// `Done`/`Error` for [`crate::pipeline`]'s 2s overlay reset, and a dictation
    /// tool must let the user immediately re-record into that window. Claiming
    /// from a terminal phase makes the pending `terminal → Idle` reset a no-op
    /// (its `compare_exchange` now fails). Mid-pipeline phases
    /// (`Transcribing`/`Cleaning`/`Pasting`) are deliberately **not** claimable.
    pub fn claim_recording(&self) -> bool {
        for from in [
            RecordingPhase::Idle,
            RecordingPhase::Done,
            RecordingPhase::Error,
        ] {
            if self
                .recording_phase
                .compare_exchange(
                    from as u8,
                    RecordingPhase::Recording as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    /// Atomically claim the stop transition: `Recording → Transcribing`. Returns
    /// `true` for the single winner, `false` if not currently recording (already
    /// stopped/cancelled, or a concurrent stop/cancel won). Lets `recording::stop`
    /// run its side effects exactly once even when the hotkey thread and an IPC
    /// caller race — mirrors [`claim_recording`].
    pub fn begin_stop(&self) -> bool {
        self.recording_phase
            .compare_exchange(
                RecordingPhase::Recording as u8,
                RecordingPhase::Transcribing as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Atomically claim the cancel transition: `Recording → Idle`. Returns `true`
    /// for the single winner, `false` otherwise. Resets to `Idle` (not `Cancelled`)
    /// because the Phase 15 pipeline that would advance `Cancelled → Idle` is
    /// deferred — landing in `Idle` keeps recording repeatable and matches
    /// [`crate::recording::finish_capture`]'s failure-path reset.
    pub fn begin_cancel(&self) -> bool {
        self.recording_phase
            .compare_exchange(
                RecordingPhase::Recording as u8,
                RecordingPhase::Idle as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordingPhase {
    Idle = 0,
    Recording = 1,
    Transcribing = 2,
    Cleaning = 3,
    Pasting = 4,
    Done = 5,
    Cancelled = 6,
    Error = 7,
}

impl TryFrom<u8> for RecordingPhase {
    type Error = crate::error::AppError;

    fn try_from(v: u8) -> Result<RecordingPhase, crate::error::AppError> {
        match v {
            0 => Ok(RecordingPhase::Idle),
            1 => Ok(RecordingPhase::Recording),
            2 => Ok(RecordingPhase::Transcribing),
            3 => Ok(RecordingPhase::Cleaning),
            4 => Ok(RecordingPhase::Pasting),
            5 => Ok(RecordingPhase::Done),
            6 => Ok(RecordingPhase::Cancelled),
            7 => Ok(RecordingPhase::Error),
            _ => Err(crate::error::AppError::Internal(format!(
                "invalid recording phase discriminant: {v}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_phase_round_trips_every_variant() {
        for (disc, variant) in [
            (0u8, RecordingPhase::Idle),
            (1, RecordingPhase::Recording),
            (2, RecordingPhase::Transcribing),
            (3, RecordingPhase::Cleaning),
            (4, RecordingPhase::Pasting),
            (5, RecordingPhase::Done),
            (6, RecordingPhase::Cancelled),
            (7, RecordingPhase::Error),
        ] {
            assert_eq!(variant as u8, disc);
            assert_eq!(RecordingPhase::try_from(disc).unwrap(), variant);
        }
    }

    #[test]
    fn recording_phase_rejects_out_of_range() {
        assert!(matches!(
            RecordingPhase::try_from(8),
            Err(crate::error::AppError::Internal(_))
        ));
    }

    #[test]
    fn app_state_phase_helpers_get_and_set() {
        let state = AppState::new();
        assert_eq!(state.phase(), RecordingPhase::Idle);
        state.set_phase(RecordingPhase::Recording);
        assert_eq!(state.phase(), RecordingPhase::Recording);
        state.set_phase(RecordingPhase::Done);
        assert_eq!(state.phase(), RecordingPhase::Done);
    }

    #[test]
    fn claim_recording_is_a_one_shot_idle_to_recording_gate() {
        let state = AppState::new();
        // First claim wins and moves to Recording.
        assert!(state.claim_recording());
        assert_eq!(state.phase(), RecordingPhase::Recording);
        // Second claim loses (already Recording) and does not change the phase.
        assert!(!state.claim_recording());
        assert_eq!(state.phase(), RecordingPhase::Recording);
    }

    #[test]
    fn claim_recording_succeeds_from_terminal_phases() {
        // Done/Error are claimable so a press during the 2s overlay window
        // immediately re-records (no `AlreadyRecording` dead-zone).
        for terminal in [RecordingPhase::Done, RecordingPhase::Error] {
            let state = AppState::new();
            state.set_phase(terminal);
            assert!(state.claim_recording(), "claimable from {terminal:?}");
            assert_eq!(state.phase(), RecordingPhase::Recording);
        }
    }

    #[test]
    fn begin_stop_claims_only_from_recording() {
        let state = AppState::new();
        // Not recording yet: no transition to claim.
        assert!(!state.begin_stop());
        assert_eq!(state.phase(), RecordingPhase::Idle);

        state.set_phase(RecordingPhase::Recording);
        assert!(state.begin_stop());
        assert_eq!(state.phase(), RecordingPhase::Transcribing);

        // Second call has nothing left to claim.
        assert!(!state.begin_stop());
        assert_eq!(state.phase(), RecordingPhase::Transcribing);
    }

    #[test]
    fn app_state_default_matches_new() {
        let state = AppState::default();
        assert_eq!(state.phase(), RecordingPhase::Idle);
    }

    #[test]
    fn claim_recording_rejected_mid_pipeline() {
        // A recording in transcription/cleanup/paste must not be clobbered.
        for busy in [
            RecordingPhase::Transcribing,
            RecordingPhase::Cleaning,
            RecordingPhase::Pasting,
        ] {
            let state = AppState::new();
            state.set_phase(busy);
            assert!(!state.claim_recording(), "not claimable from {busy:?}");
            assert_eq!(state.phase(), busy);
        }
    }
}
