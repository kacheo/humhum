pub mod buffer;
pub mod cap_timer;
#[cfg(target_os = "macos")]
mod device_select;

use std::future::Future;
use std::pin::Pin;

pub use buffer::AudioBuffer;

/// Resolve which input device name (if any) `run_capture` should ask cpal to
/// open in place of its own `default_input_device()`. `None` means "no
/// override, use cpal's default" — the pre-fix behavior. Non-macOS platforms
/// have no Bluetooth A2DP/HFP switching concern, so this is always `None`
/// there (mirrors the `permissions/mod.rs` cfg-dispatch idiom).
fn resolve_preferred_device_name(preferred: Option<&str>) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        device_select::resolve_input_device_name(preferred)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = preferred;
        None
    }
}

/// All input-capable device names for the Preferences dropdown (empty on
/// non-macOS or on any enumeration failure).
pub fn list_input_device_names() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        device_select::list_input_device_names()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub enum AudioError {
    PermissionDenied,
    DeviceUnavailable,
    StreamError(String),
    /// A `start()` arrived while a capture is already armed. Distinct from a
    /// generic `StreamError` so the controller can map it to the canonical
    /// `AppError::AlreadyRecording` (stable `already_recording` code) rather
    /// than surfacing a less meaningful `stream_error` on a double-start race.
    AlreadyRecording,
}

impl AudioError {
    /// Stable snake_case wire code (LLD §593).
    pub fn code(&self) -> String {
        match self {
            AudioError::PermissionDenied => "permission_denied",
            AudioError::DeviceUnavailable => "device_unavailable",
            AudioError::StreamError(_) => "stream_error",
            AudioError::AlreadyRecording => "already_recording",
        }
        .to_string()
    }
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::PermissionDenied => write!(f, "microphone permission denied"),
            AudioError::DeviceUnavailable => write!(f, "audio device unavailable"),
            AudioError::StreamError(e) => write!(f, "stream error: {e}"),
            AudioError::AlreadyRecording => write!(f, "already recording"),
        }
    }
}

pub trait AudioCapture: Send + Sync {
    /// Synchronously *arm* capture (spawn the capture thread + install the stop
    /// handle) and return the *drain* future, which yields the recorded buffer
    /// when `stop()`/`cancel()` fires. Once this returns `Ok`, a subsequent
    /// `stop()`/`cancel()` is guaranteed to be delivered — this is what closes the
    /// I1 start/stop race (a stop can never land before the handle exists).
    fn start(&self) -> Result<BoxFuture<'static, Result<AudioBuffer, AudioError>>, AudioError>;
    fn stop(&self) -> Result<(), AudioError>;
    fn cancel(&self);

    /// Set the user's preferred input device name (`None`/`Some("system-default")`
    /// handled by [`resolve_preferred_device_name`]) for the *next* `start()`.
    /// Default no-op so existing implementors (test doubles) don't need to change.
    fn set_preferred_device(&self, _preferred: Option<String>) {}
}

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use tokio::sync::oneshot;

use buffer::{duration_ms, resample_to_16k, to_mono_f32, TARGET_SAMPLE_RATE};

/// Absolute ceiling on how long a single capture thread will hold the input
/// stream (and therefore the microphone) before self-terminating, even if it
/// never receives a `stop()`/`cancel()` signal. This is a defense-in-depth
/// backstop: the recording controller's `SAFETY_CAP` (300s, `recording.rs`)
/// already bounds every tier — this sits *just above* it so it only ever fires
/// when the normal stop path failed to deliver (e.g. a lost-signal race), never
/// preempting a legitimate recording. Turns "permanent hot mic" into a bounded
/// worst case and un-wedges `is_recording` (the drain future's `RecordingGuard`
/// runs once the stream is dropped here). Privacy-critical for a dictation app.
const MAX_CAPTURE: Duration = Duration::from_secs(330);

/// CPAL-backed [`AudioCapture`]. The stream is built, played and dropped entirely
/// on a dedicated capture thread; this struct holds only control handles. `start`
/// records into a shared `Vec<f32>` until `stop`/`cancel`, then downmixes +
/// resamples to 16 kHz.
pub struct CpalAudioCapture {
    // `Arc` (not a bare `Mutex`) so the `'static` drain future can run the RAII
    // teardown guard without borrowing `&self`. `stop()`/`cancel()` deref through
    // transparently.
    state: Arc<Mutex<Option<Active>>>,
    frames_dropped: Arc<AtomicU64>,
    is_recording: Arc<AtomicBool>,
    // Separate `Mutex` from `state` (never locked together) — no new lock-ordering
    // hazard. Snapshotted into `run_capture` at the start of each `start()`.
    preferred_device: Arc<Mutex<Option<String>>>,
    // Absolute self-terminate ceiling passed to each `run_capture` (see
    // [`MAX_CAPTURE`]). Overridable only in tests so the backstop can be
    // exercised deterministically with a short value.
    max_capture: Duration,
}

struct Active {
    stop_tx: std::sync::mpsc::Sender<()>,
    thread: JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
}

impl CpalAudioCapture {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(None)),
            frames_dropped: Arc::new(AtomicU64::new(0)),
            is_recording: Arc::new(AtomicBool::new(false)),
            preferred_device: Arc::new(Mutex::new(None)),
            max_capture: MAX_CAPTURE,
        }
    }

    /// Test-only constructor that shortens the self-terminate ceiling so the
    /// [`MAX_CAPTURE`] backstop in `run_capture` can be exercised deterministically.
    #[cfg(test)]
    fn with_max_capture(max_capture: Duration) -> Self {
        Self {
            max_capture,
            ..Self::new()
        }
    }
}

impl Default for CpalAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

fn append(buf: &Arc<Mutex<Vec<f32>>>, dropped: &Arc<AtomicU64>, mono: Vec<f32>) {
    // Non-blocking: never stall the OS audio thread on lock contention.
    match buf.try_lock() {
        Ok(mut b) => b.extend_from_slice(&mono),
        Err(_) => {
            dropped.fetch_add(mono.len() as u64, Ordering::Relaxed);
        }
    }
}

fn log_stream_error(e: cpal::Error) {
    log::warn!("audio stream error: {e}");
}

/// Blocks the capture thread until a stop signal arrives on `stop_rx` (a
/// `stop()`/`cancel()` send, or all senders dropping on teardown), OR until
/// `max_capture` elapses — whichever comes first. On timeout it logs a warning
/// and returns; the caller then drops the stream, so the mic can never be held
/// past `max_capture` regardless of any orchestration bug.
///
/// Extracted from `run_capture` so this park/timeout/warn decision is unit-
/// testable device-free (no CoreAudio stream needed to reach it).
fn park_until_stop(stop_rx: &std::sync::mpsc::Receiver<()>, max_capture: Duration) {
    match stop_rx.recv_timeout(max_capture) {
        // Explicit stop/cancel, or all senders dropped on teardown.
        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        // No stop was ever delivered — self-terminate to release the mic.
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "audio: capture self-terminated after {}s with no stop signal — \
                 releasing the mic (a stop was lost or never delivered)",
                max_capture.as_secs()
            );
        }
    }
}

/// Picks the input device to open: resolves `preferred_device` (auto-avoiding
/// a Bluetooth default on macOS, an explicit override, or no override) to a
/// device *name*, then looks that name up among cpal's enumerated input
/// devices. Falls back to cpal's own `default_input_device()` — preserving
/// the pre-fix behavior verbatim — whenever there's no override, the name
/// lookup fails, or enumeration itself fails.
fn select_device(preferred_device: &Option<String>) -> Option<cpal::Device> {
    if let Some(name) = resolve_preferred_device_name(preferred_device.as_deref()) {
        if let Ok(mut devices) = cpal::default_host().input_devices() {
            if let Some(d) = devices.find(|d| {
                d.description()
                    .map(|description| description.name() == name)
                    .unwrap_or(false)
            }) {
                return Some(d);
            }
        }
        log::warn!("preferred input device '{name}' not found — falling back to system default");
    }
    cpal::default_host().default_input_device()
}

/// Runs on the dedicated capture thread: builds + plays the input stream, parks
/// until stop (or `max_capture` elapses), then drops the stream. Reports the
/// device sample rate (or an error) over `ready_tx`, and fires `done_tx` once
/// stopped.
///
/// `max_capture` is the absolute self-terminate ceiling (see [`MAX_CAPTURE`]);
/// it's a parameter so tests can inject a short value. If it elapses with no
/// stop signal, the thread drops the stream itself — the mic can never be held
/// longer than this regardless of any orchestration bug.
fn run_capture(
    buffer: Arc<Mutex<Vec<f32>>>,
    dropped: Arc<AtomicU64>,
    ready_tx: oneshot::Sender<Result<u32, AudioError>>,
    done_tx: oneshot::Sender<()>,
    stop_rx: std::sync::mpsc::Receiver<()>,
    preferred_device: Option<String>,
    max_capture: Duration,
) {
    let device = match select_device(&preferred_device) {
        Some(d) => d,
        None => {
            let _ = ready_tx.send(Err(AudioError::DeviceUnavailable));
            return;
        }
    };
    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(AudioError::StreamError(e.to_string())));
            return;
        }
    };
    let rate = config.sample_rate();
    let channels = config.channels();
    let fmt = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let build = {
        let buf = buffer.clone();
        let fd = dropped.clone();
        match fmt {
            SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    append(&buf, &fd, to_mono_f32(data, channels))
                },
                log_stream_error,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    append(&buf, &fd, to_mono_f32(data, channels))
                },
                log_stream_error,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    append(&buf, &fd, to_mono_f32(data, channels))
                },
                log_stream_error,
                None,
            ),
            other => {
                let _ = ready_tx.send(Err(AudioError::StreamError(format!(
                    "unsupported sample format {other:?}"
                ))));
                return;
            }
        }
    };
    let stream = match build {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(AudioError::StreamError(e.to_string())));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready_tx.send(Err(AudioError::StreamError(e.to_string())));
        return;
    }
    let _ = ready_tx.send(Ok(rate));

    // Park until stop()/cancel() (or teardown), but never longer than
    // `max_capture` — see [`park_until_stop`].
    park_until_stop(&stop_rx, max_capture);
    drop(stream); // stops capture
    let _ = done_tx.send(());
}

/// RAII teardown for an in-flight `start()`. On **any** exit — normal return,
/// error, or the future being dropped mid-`await` (cancellation/timeout) — this
/// stops + joins the capture thread and clears `is_recording`, so the recording
/// subsystem can never wedge with a stuck flag or a leaked stream.
///
/// Owns `Arc` clones (not `&self`) so it can live inside the `'static` drain
/// future returned by `start`.
struct RecordingGuard {
    state: Arc<Mutex<Option<Active>>>,
    is_recording: Arc<AtomicBool>,
}

impl Drop for RecordingGuard {
    fn drop(&mut self) {
        if let Some(active) = self.state.lock().unwrap().take() {
            let _ = active.stop_tx.send(()); // wake the parked thread (no-op if already stopped)
            let _ = active.thread.join();
        }
        self.is_recording.store(false, Ordering::SeqCst);
    }
}

impl AudioCapture for CpalAudioCapture {
    fn start(&self) -> Result<BoxFuture<'static, Result<AudioBuffer, AudioError>>, AudioError> {
        // --- Synchronous arm: runs entirely before this fn returns ---
        if self.is_recording.swap(true, Ordering::SeqCst) {
            // Another recording owns the flag — return WITHOUT a guard so we
            // don't reset it. Dedicated variant so the controller surfaces the
            // canonical `AlreadyRecording`, not a generic `stream_error`.
            return Err(AudioError::AlreadyRecording);
        }

        // Hint only: capture runs at the device rate (e.g. 48 kHz), so this
        // 16 kHz-mono sizing may reallocate during a long recording.
        let buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(
            TARGET_SAMPLE_RATE as usize * 65,
        )));
        self.frames_dropped.store(0, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));

        let (ready_tx, ready_rx) = oneshot::channel::<Result<u32, AudioError>>();
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

        let preferred_device = self.preferred_device.lock().unwrap().clone();
        let max_capture = self.max_capture;
        let thread = {
            let buffer = buffer.clone();
            let dropped = self.frames_dropped.clone();
            std::thread::spawn(move || {
                run_capture(
                    buffer,
                    dropped,
                    ready_tx,
                    done_tx,
                    stop_rx,
                    preferred_device,
                    max_capture,
                );
            })
        };

        // Install the stop handle BEFORE returning so a stop()/cancel() racing
        // the caller buffers on `stop_tx` and is consumed when the thread parks —
        // never lost. This synchronous install (vs. doing it inside the drain
        // future) is what closes the I1 start/stop race.
        *self.state.lock().unwrap() = Some(Active {
            stop_tx,
            thread,
            cancelled: cancelled.clone(),
        });

        // --- Drain future: 'static, owns Arc clones so it never borrows &self ---
        // We now own `is_recording`; the guard guarantees teardown on every exit,
        // including the future being dropped mid-`await`.
        let guard = RecordingGuard {
            state: self.state.clone(),
            is_recording: self.is_recording.clone(),
        };
        let frames_dropped = self.frames_dropped.clone();
        Ok(Box::pin(async move {
            let _guard = guard;

            let device_rate = match ready_rx.await {
                Ok(Ok(rate)) => rate,
                Ok(Err(e)) => return Err(e), // guard tears down
                Err(_) => return Err(AudioError::StreamError("capture thread exited".into())),
            };

            // Resolves when stop()/cancel() stops the capture thread.
            let _ = done_rx.await;

            let dropped = frames_dropped.load(Ordering::Relaxed);
            if dropped > 0 {
                log::warn!("audio: dropped {dropped} samples on lock contention");
            }
            if cancelled.load(Ordering::SeqCst) {
                return Err(AudioError::StreamError("cancelled".into())); // guard tears down
            }

            // The thread dropped the stream before firing `done`, so no callback
            // races this take.
            let samples = std::mem::take(&mut *buffer.lock().unwrap());
            let resampled = resample_to_16k(samples, device_rate)?;
            let duration = duration_ms(resampled.len());
            Ok(AudioBuffer {
                samples: resampled,
                sample_rate: TARGET_SAMPLE_RATE,
                duration_ms: duration,
            })
            // `_guard` drops here → joins the thread, clears `is_recording`.
        }))
    }

    fn stop(&self) -> Result<(), AudioError> {
        if let Some(active) = self.state.lock().unwrap().as_ref() {
            let _ = active.stop_tx.send(());
        }
        Ok(())
    }

    fn cancel(&self) {
        if let Some(active) = self.state.lock().unwrap().as_ref() {
            active.cancelled.store(true, Ordering::SeqCst);
            let _ = active.stop_tx.send(());
        }
    }

    fn set_preferred_device(&self, preferred: Option<String>) {
        *self.preferred_device.lock().unwrap() = preferred;
    }
}

/// Construct the platform `AudioCapture` (CPAL — cross-platform; LLD §800).
pub fn create() -> Box<dyn AudioCapture> {
    Box::new(CpalAudioCapture::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Serializes the tests that spawn a real cpal capture thread, so
    /// concurrent CoreAudio device open/teardown from separate test threads
    /// can't contend and blow each other's timeout budgets .
    static AUDIO_HW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Exercises the real CPAL path. On a headless CI runner (no input device)
    /// `start` resolves quickly to `Err(DeviceUnavailable)`; on a machine with a
    /// mic it begins recording, so we cap it with a timeout. Either way: no
    /// panic, no hang.
    #[tokio::test]
    async fn start_resolves_or_records_without_hanging() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = create();
        if let Ok(session) = cap.start() {
            let _ = tokio::time::timeout(Duration::from_millis(300), session).await;
        }
        // Whatever the outcome, the recording subsystem must not be wedged.
        // (Covered concretely by `dropped_start_future_resets_recording_flag`.)
    }

    /// I1 regression: a `stop()` issued *immediately after the synchronous arm,
    /// before the drain future is awaited* must be delivered (buffered on the stop
    /// channel and consumed when the thread parks) — never lost. The drain
    /// resolves; it must not hang.
    #[tokio::test]
    async fn stop_immediately_after_arm_is_delivered_not_lost() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = CpalAudioCapture::new();
        match cap.start() {
            // Mic machine: arm succeeds; the early stop resolves the drain.
            Ok(session) => {
                cap.stop().expect("stop ok");
                let r = tokio::time::timeout(Duration::from_millis(2000), session).await;
                assert!(r.is_ok(), "drain resolved — the early stop was not lost");
            }
            // Headless CI (no input device): arming may still succeed (device-open
            // is async), so an Err here is acceptable — either way, no wedge.
            Err(_) => {}
        }
    }

    /// A `start()` while already recording is rejected and does NOT reset the
    /// owning recording's flag. Device-free (the swap guard fires before any
    /// device access), so the arm returns `Err` synchronously with no future.
    #[test]
    fn start_while_recording_errors() {
        let cap = CpalAudioCapture::new();
        cap.is_recording.store(true, Ordering::SeqCst); // simulate an active recording
        let r = cap.start().map(|_| ());
        assert!(matches!(r, Err(AudioError::AlreadyRecording)));
        assert!(
            cap.is_recording.load(Ordering::SeqCst),
            "the early reject must not clear another recording's flag"
        );
    }

    /// stop()/cancel() on an idle capture are harmless no-ops.
    #[test]
    fn stop_and_cancel_when_idle_are_noops() {
        let cap = CpalAudioCapture::new();
        assert!(cap.stop().is_ok());
        cap.cancel(); // must not panic
        assert!(!cap.is_recording.load(Ordering::SeqCst));
    }

    #[test]
    fn preferred_device_defaults_to_none_and_is_settable() {
        let cap = CpalAudioCapture::new();
        assert_eq!(*cap.preferred_device.lock().unwrap(), None);
        cap.set_preferred_device(Some("AirPods Pro".to_string()));
        assert_eq!(
            *cap.preferred_device.lock().unwrap(),
            Some("AirPods Pro".to_string())
        );
        cap.set_preferred_device(None);
        assert_eq!(*cap.preferred_device.lock().unwrap(), None);
    }

    /// An unmatched preferred-device name must fall back to the system default,
    /// not hang or panic — mirrors `start_resolves_or_records_without_hanging`.
    #[tokio::test]
    async fn start_with_unmatched_preferred_device_falls_back_without_hanging() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = CpalAudioCapture::new();
        cap.set_preferred_device(Some("definitely-not-a-real-device-name".to_string()));
        if let Ok(session) = cap.start() {
            let _ = tokio::time::timeout(Duration::from_millis(300), session).await;
        }
    }

    /// CRIT-1 regression: dropping the in-flight `start()` future (here via a
    /// timeout) must run the RAII guard, clearing `is_recording` so the next
    /// `start()` isn't permanently blocked. On a mic machine the future is
    /// dropped mid-recording; on headless CI `start` errors first — both reset.
    #[tokio::test]
    async fn dropped_start_future_resets_recording_flag() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = CpalAudioCapture::new();
        if let Ok(session) = cap.start() {
            let _ = tokio::time::timeout(Duration::from_millis(300), session).await;
        }
        assert!(
            !cap.is_recording.load(Ordering::SeqCst),
            "guard must clear is_recording on drop/error"
        );
    }

    /// Backstop regression (mic-leak fix): the capture thread must self-terminate
    /// and release the input stream after `max_capture` even when `stop()`/
    /// `cancel()` is *never* delivered — so a lost/dropped stop signal can't hold
    /// the microphone open forever. The drain future must still resolve (no hang)
    /// and `is_recording` must end clear (the app un-wedges for the next start).
    /// On a mic machine the thread times out at the injected 200ms cap; on
    /// headless CI arming resolves to `Err(DeviceUnavailable)` first — both leave
    /// the flag clear without hanging.
    #[tokio::test]
    async fn capture_self_terminates_after_max_capture_without_a_stop() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = CpalAudioCapture::with_max_capture(Duration::from_millis(200));
        if let Ok(session) = cap.start() {
            // No stop()/cancel() is ever issued: the drain future must resolve on
            // its own once the thread self-terminates at max_capture (≪ this 2s
            // timeout), proving the mic is released with no external signal.
            let r = tokio::time::timeout(Duration::from_secs(2), session).await;
            assert!(
                r.is_ok(),
                "drain resolved — capture self-terminated, no hang"
            );
        }
        assert!(
            !cap.is_recording.load(Ordering::SeqCst),
            "is_recording cleared after self-terminate — no orphaned mic, app un-wedged"
        );
    }

    // Device-free unit tests for the park/timeout/warn decision, so all three
    // arms (signalled / disconnected / timed-out backstop) are covered
    // regardless of whether the CI runner has an input device.

    /// Normal path: a delivered stop returns promptly (the `Ok(())` arm).
    #[test]
    fn park_until_stop_returns_on_signal() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        tx.send(()).unwrap();
        // A generous cap: must return via the signal, not the timeout.
        park_until_stop(&rx, Duration::from_secs(30));
    }

    /// Teardown path: when every sender drops, `recv_timeout` reports
    /// `Disconnected` and the thread proceeds to release the stream.
    #[test]
    fn park_until_stop_returns_when_all_senders_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        park_until_stop(&rx, Duration::from_secs(30));
    }

    /// Backstop path: with the sender kept alive but no stop ever sent, the
    /// park must self-terminate via the `Timeout` arm (the mic-leak safety net).
    /// Holding `_tx` guarantees the return is a timeout, not a disconnect.
    #[test]
    fn park_until_stop_self_terminates_on_timeout() {
        let (_tx, rx) = std::sync::mpsc::channel::<()>();
        let started = std::time::Instant::now();
        park_until_stop(&rx, Duration::from_millis(50));
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "must wait out max_capture before self-terminating (timeout arm)"
        );
    }

    /// Full record cycle — needs a real microphone + grant, so run manually:
    /// `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn full_cycle_returns_16k_buffer() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap: Arc<dyn AudioCapture> = Arc::from(create());
        let stopper = cap.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            stopper.stop().unwrap();
        });
        let buf = cap.start().expect("arm").await.expect("recording");
        assert_eq!(buf.sample_rate, TARGET_SAMPLE_RATE);
        assert!(!buf.samples.is_empty());
        assert!(buf.duration_ms > 0);
    }

    /// Dispatch-only: just exercises the macOS-vs-not cfg branch without
    /// touching a real recording session. Empty (no devices / enumeration
    /// failure) is a valid result on headless CI — we only assert no panic.
    #[test]
    fn list_input_device_names_does_not_panic() {
        let _ = list_input_device_names();
    }

    #[test]
    fn audio_error_codes_are_stable_snake_case() {
        assert_eq!(AudioError::PermissionDenied.code(), "permission_denied");
        assert_eq!(AudioError::DeviceUnavailable.code(), "device_unavailable");
        assert_eq!(AudioError::StreamError("x".into()).code(), "stream_error");
        assert_eq!(AudioError::AlreadyRecording.code(), "already_recording");
    }

    #[test]
    fn audio_error_display_messages() {
        assert_eq!(
            AudioError::PermissionDenied.to_string(),
            "microphone permission denied"
        );
        assert_eq!(
            AudioError::DeviceUnavailable.to_string(),
            "audio device unavailable"
        );
        assert_eq!(
            AudioError::StreamError("boom".into()).to_string(),
            "stream error: boom"
        );
        assert_eq!(
            AudioError::AlreadyRecording.to_string(),
            "already recording"
        );
    }

    /// Minimal `AudioCapture` implementor that does NOT override
    /// `set_preferred_device`, so calling it exercises the trait's default
    /// no-op body (the only thing that body needs to prove).
    struct NoopCapture;
    impl AudioCapture for NoopCapture {
        fn start(&self) -> Result<BoxFuture<'static, Result<AudioBuffer, AudioError>>, AudioError> {
            Err(AudioError::DeviceUnavailable)
        }
        fn stop(&self) -> Result<(), AudioError> {
            Ok(())
        }
        fn cancel(&self) {}
    }

    #[test]
    fn default_set_preferred_device_is_a_noop() {
        let cap = NoopCapture;
        // Must not panic; the default body does nothing and returns nothing.
        cap.set_preferred_device(Some("AirPods".to_string()));
        cap.set_preferred_device(None);
    }

    #[test]
    fn default_impl_constructs_an_idle_capture() {
        let cap = CpalAudioCapture::default();
        assert!(!cap.is_recording.load(Ordering::SeqCst));
        assert!(cap.stop().is_ok());
    }

    #[test]
    fn append_extends_buffer_when_lock_is_free() {
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        append(&buf, &dropped, vec![1.0, 2.0, 3.0]);
        assert_eq!(*buf.lock().unwrap(), vec![1.0, 2.0, 3.0]);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    /// The OS audio callback must never block on lock contention: when the
    /// buffer is already held elsewhere, `append` drops the frames and counts
    /// them instead of waiting.
    #[test]
    fn append_drops_and_counts_frames_when_lock_is_contended() {
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let _held = buf.lock().unwrap(); // simulate contention from another thread
        append(&buf, &dropped, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(dropped.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn log_stream_error_does_not_panic_for_any_variant() {
        for kind in [
            cpal::ErrorKind::DeviceNotAvailable,
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::StreamInvalidated,
            cpal::ErrorKind::Xrun,
            cpal::ErrorKind::BackendError,
        ] {
            log_stream_error(cpal::Error::new(kind));
            log_stream_error(cpal::Error::with_message(kind, "capture failed"));
        }
    }

    /// `cancel()` while a recording is genuinely active (not idle) must both
    /// flip the `cancelled` flag and wake the parked capture thread via
    /// `stop_tx` — the branch `stop_and_cancel_when_idle_are_noops` can't
    /// reach. Fabricates the `Active` state directly (private field, same
    /// module) instead of needing a real device, so this is deterministic.
    #[test]
    fn cancel_with_active_state_flips_flag_and_signals_stop() {
        let cap = CpalAudioCapture::new();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn(|| {}); // trivial handle; Active just needs one
        *cap.state.lock().unwrap() = Some(Active {
            stop_tx,
            thread,
            cancelled: cancelled.clone(),
        });

        cap.cancel();

        assert!(cancelled.load(Ordering::SeqCst));
        assert!(
            stop_rx.try_recv().is_ok(),
            "cancel() must signal the parked capture thread"
        );
    }

    /// Best-effort, mirrors the other conditional hardware tests in this file:
    /// on a mic machine, cancelling a genuinely in-flight recording must
    /// surface `AudioError::StreamError("cancelled")` rather than a buffer.
    /// On headless CI arming may error first — either way, no hang.
    #[tokio::test]
    async fn cancel_after_start_yields_cancelled_error_when_recording() {
        let _g = AUDIO_HW_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cap = CpalAudioCapture::new();
        if let Ok(session) = cap.start() {
            cap.cancel();
            let r = tokio::time::timeout(Duration::from_millis(2000), session).await;
            if let Ok(Err(AudioError::StreamError(msg))) = r {
                assert_eq!(msg, "cancelled");
            }
        }
    }
}
