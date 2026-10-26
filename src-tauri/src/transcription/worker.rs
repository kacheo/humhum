//! One persistent native worker. UI cancellation never releases admission.
use super::{BoxFuture, TranscriptionError, WhisperModel};
use crate::audio::AudioBuffer;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

type ResultText = Result<String, TranscriptionError>;
pub(super) const IDLE_UNLOAD: Duration = Duration::from_secs(5 * 60);

pub(super) trait Engine: Send + 'static {
    fn run(&mut self, buffer: AudioBuffer, model: WhisperModel, cancel: &AtomicBool) -> ResultText;
    fn clear(&mut self);
}

/// A lease travels with the native job, never with the awaiting UI future.
struct Admission(Arc<AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct Job {
    buffer: AudioBuffer,
    model: WhisperModel,
    cancel: Arc<AtomicBool>,
    result: oneshot::Sender<ResultText>,
    admission: Admission,
}

pub(super) struct Worker {
    sender: mpsc::SyncSender<Job>,
    busy: Arc<AtomicBool>,
}

/// Clock input is explicit so eviction boundaries can be checked without sleep.
struct IdleDeadline {
    since: Instant,
    limit: Duration,
}
impl IdleDeadline {
    fn remaining(&self, now: Instant) -> Duration {
        self.limit
            .saturating_sub(now.saturating_duration_since(self.since))
    }
    fn reset(&mut self, now: Instant) {
        self.since = now;
    }
    fn evict_if_expired(&mut self, now: Instant, engine: &mut impl Engine) {
        if self.remaining(now).is_zero() {
            crate::platform::autorelease_pool(|| engine.clear());
            self.reset(now);
        }
    }
}

impl Worker {
    pub(super) fn start(mut engine: impl Engine, idle_limit: Duration) -> std::io::Result<Self> {
        // Capacity one is only a handoff slot. CAS admission permits exactly one
        // submitted/executing job, so recordings never accumulate in a queue.
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let busy = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("humhum-whisper".into())
            .spawn(move || {
                let mut idle = IdleDeadline {
                    since: Instant::now(),
                    limit: idle_limit,
                };
                loop {
                    let job = match receiver.recv_timeout(idle.remaining(Instant::now())) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            idle.evict_if_expired(Instant::now(), &mut engine);
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            crate::platform::autorelease_pool(|| engine.clear());
                            break;
                        }
                    };
                    // Also check the deadline on receipt: a job arriving at expiry
                    // must not revive a stale cached context.
                    idle.evict_if_expired(Instant::now(), &mut engine);
                    let Job {
                        buffer,
                        model,
                        cancel,
                        result,
                        admission,
                    } = job;
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::platform::autorelease_pool(|| {
                            if cancel.load(Ordering::Acquire) {
                                drop(buffer);
                                return Err(cancelled());
                            }
                            engine.run(buffer, model, &cancel)
                        })
                    }));
                    // A cancelled or panicking job drops native state and its model
                    // before another caller can acquire admission. Never replace a
                    // worker while a native call is still running.
                    let abandoned = cancel.load(Ordering::Acquire) || result.is_closed();
                    if abandoned || outcome.is_err() {
                        crate::platform::autorelease_pool(|| engine.clear());
                    }
                    let outcome = outcome.unwrap_or_else(|_| {
                        Err(TranscriptionError::InferenceFailed(
                            "native worker panicked; please try again".into(),
                        ))
                    });
                    idle.reset(Instant::now()); // idle starts AFTER execution exits
                    drop(admission);
                    if !abandoned {
                        let _ = result.send(outcome);
                    }
                }
            })?;
        Ok(Self { sender, busy })
    }

    pub(super) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub(super) fn submit(
        &self,
        buffer: AudioBuffer,
        model: WhisperModel,
    ) -> BoxFuture<'static, ResultText> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // A native call that ignores cancellation remains here, with no
            // fallback worker/context and no retained retry recording.
            return Box::pin(async {
                Err(TranscriptionError::InferenceFailed(
                "transcription worker is still running; restart Humhum if it remains unavailable".into(),
            ))
            });
        }
        let admission = Admission(self.busy.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_guard = CancelOnDrop(cancel.clone());
        let (result, receiver) = oneshot::channel();
        let sent = self.sender.try_send(Job {
            buffer,
            model,
            cancel,
            result,
            admission,
        });
        Box::pin(async move {
            let _cancel_guard = cancel_guard;
            // Send errors own the job/lease; drop it before returning the error.
            if let Err(error) = sent {
                drop(error);
                return Err(TranscriptionError::InferenceFailed(
                    "native worker exited; restart Humhum".into(),
                ));
            }
            receiver.await.unwrap_or_else(|_| {
                Err(TranscriptionError::InferenceFailed(
                    "native worker exited; restart Humhum".into(),
                ))
            })
        })
    }
}

pub(super) fn cancelled() -> TranscriptionError {
    TranscriptionError::InferenceFailed("cancelled".into())
}

/// Owns at most one context and releases it BEFORE loading a different model.
pub(super) struct ModelCache<T> {
    entry: Option<(WhisperModel, T)>,
}
impl<T> ModelCache<T> {
    pub(super) fn new() -> Self {
        Self { entry: None }
    }
    pub(super) fn clear(&mut self) {
        self.entry = None;
    }
    pub(super) fn get_or_load(
        &mut self,
        model: WhisperModel,
        load: impl FnOnce() -> Result<T, TranscriptionError>,
    ) -> Result<&T, TranscriptionError> {
        if self.entry.as_ref().map(|(key, _)| *key) != Some(model) {
            self.clear();
            self.entry = Some((model, load()?));
        }
        Ok(&self.entry.as_ref().expect("loaded context").1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    enum Action {
        Return,
        Error,
        Panic,
    }
    struct ControlledEngine {
        entered: mpsc::Sender<WhisperModel>,
        actions: mpsc::Receiver<Action>,
        clears: Arc<AtomicUsize>,
        executing: Arc<AtomicBool>,
    }
    struct Executing(Arc<AtomicBool>);
    impl Drop for Executing {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    impl Engine for ControlledEngine {
        fn run(
            &mut self,
            _buffer: AudioBuffer,
            model: WhisperModel,
            _cancel: &AtomicBool,
        ) -> ResultText {
            self.executing.store(true, Ordering::Release);
            let _resource = Executing(self.executing.clone());
            self.entered.send(model).unwrap();
            // Deliberately ignores cancellation, just like a stuck native call.
            match self.actions.recv().unwrap() {
                Action::Return => Ok("late transcript".into()),
                Action::Error => Err(TranscriptionError::InferenceFailed("native error".into())),
                Action::Panic => panic!("native worker regression fixture"),
            }
        }
        fn clear(&mut self) {
            assert!(
                !self.executing.load(Ordering::Acquire),
                "context freed during inference"
            );
            self.clears.fetch_add(1, Ordering::AcqRel);
        }
    }
    struct Harness {
        worker: Worker,
        entered: mpsc::Receiver<WhisperModel>,
        actions: mpsc::Sender<Action>,
        clears: Arc<AtomicUsize>,
        executing: Arc<AtomicBool>,
    }
    fn harness() -> Harness {
        let (entered_tx, entered) = mpsc::channel();
        let (actions, actions_rx) = mpsc::channel();
        let clears = Arc::new(AtomicUsize::new(0));
        let executing = Arc::new(AtomicBool::new(false));
        let worker = Worker::start(
            ControlledEngine {
                entered: entered_tx,
                actions: actions_rx,
                clears: clears.clone(),
                executing: executing.clone(),
            },
            IDLE_UNLOAD,
        )
        .unwrap();
        Harness {
            worker,
            entered,
            actions,
            clears,
            executing,
        }
    }
    fn buffer() -> AudioBuffer {
        AudioBuffer {
            samples: vec![0.25; 16],
            sample_rate: 16_000,
            duration_ms: 1,
        }
    }
    async fn wait_available(worker: &Worker) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while worker.is_busy() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker must release admission after execution exits");
    }

    #[tokio::test]
    async fn timeout_keeps_sole_admission_until_native_exit_and_discards_late_result() {
        let h = harness();
        let future = h.worker.submit(buffer(), WhisperModel::BaseEn);
        assert_eq!(
            h.entered.recv_timeout(Duration::from_secs(2)).unwrap(),
            WhisperModel::BaseEn
        );
        assert!(tokio::time::timeout(Duration::from_millis(1), future)
            .await
            .is_err());
        assert!(h.executing.load(Ordering::Acquire));
        assert_eq!(h.clears.load(Ordering::Acquire), 0);
        let rejected = h
            .worker
            .submit(buffer(), WhisperModel::SmallEn)
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("restart Humhum"));
        assert!(h.entered.try_recv().is_err()); // no retry work queued
        h.actions.send(Action::Return).unwrap();
        wait_available(&h.worker).await;
        assert!(!h.executing.load(Ordering::Acquire));
        assert_eq!(h.clears.load(Ordering::Acquire), 1); // abandoned context released
        let next = h.worker.submit(buffer(), WhisperModel::SmallEn);
        assert_eq!(
            h.entered.recv_timeout(Duration::from_secs(2)).unwrap(),
            WhisperModel::SmallEn
        );
        h.actions.send(Action::Return).unwrap();
        assert_eq!(next.await.unwrap(), "late transcript");
    }

    #[tokio::test]
    async fn dropped_unpolled_caller_cancels_without_releasing_running_admission() {
        let h = harness();
        let future = h.worker.submit(buffer(), WhisperModel::BaseEn);
        h.entered.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(future);
        assert!(h.worker.is_busy());
        h.actions.send(Action::Return).unwrap();
        wait_available(&h.worker).await;
        assert_eq!(h.clears.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn dropping_caller_signals_cancellation_to_executing_engine() {
        struct CancellationEngine {
            entered: mpsc::Sender<()>,
            observe: mpsc::Receiver<()>,
            observed: mpsc::Sender<bool>,
        }
        impl Engine for CancellationEngine {
            fn run(&mut self, _: AudioBuffer, _: WhisperModel, cancel: &AtomicBool) -> ResultText {
                self.entered.send(()).unwrap();
                self.observe.recv().unwrap();
                self.observed.send(cancel.load(Ordering::Acquire)).unwrap();
                Ok("abandoned transcript".into())
            }
            fn clear(&mut self) {}
        }
        let (entered_tx, entered) = mpsc::channel();
        let (observe, observe_rx) = mpsc::channel();
        let (observed_tx, observed) = mpsc::channel();
        let worker = Worker::start(
            CancellationEngine {
                entered: entered_tx,
                observe: observe_rx,
                observed: observed_tx,
            },
            IDLE_UNLOAD,
        )
        .unwrap();
        let future = worker.submit(buffer(), WhisperModel::BaseEn);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(future);
        observe.send(()).unwrap();
        assert!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            "dropping the caller must signal the flag consumed by native inference"
        );
        wait_available(&worker).await;
    }

    #[tokio::test]
    async fn error_and_panic_release_admission_and_worker_can_be_reused() {
        let h = harness();
        for (action, expected_error) in [
            (Action::Error, Some("native error")),
            (Action::Panic, Some("native worker panicked")),
            (Action::Return, None),
        ] {
            let future = h.worker.submit(buffer(), WhisperModel::BaseEn);
            h.entered.recv_timeout(Duration::from_secs(2)).unwrap();
            h.actions.send(action).unwrap();
            match (future.await, expected_error) {
                (Err(error), Some(message)) => assert!(error.to_string().contains(message)),
                (Ok(text), None) => assert_eq!(text, "late transcript"),
                (result, expected) => {
                    panic!("unexpected result {result:?}, expected error {expected:?}")
                }
            }
            assert!(!h.worker.is_busy());
            assert!(!h.executing.load(Ordering::Acquire));
        }
        assert_eq!(h.clears.load(Ordering::Acquire), 1); // panic invalidates cache
    }

    struct Context(Arc<AtomicUsize>);
    impl Context {
        fn load(active: &Arc<AtomicUsize>) -> Self {
            assert_eq!(
                active.fetch_add(1, Ordering::AcqRel),
                0,
                "two loaded contexts"
            );
            Self(active.clone())
        }
    }
    impl Drop for Context {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }
    #[test]
    fn changing_model_releases_previous_context_before_loading_and_errors_leave_cache_empty() {
        let active = Arc::new(AtomicUsize::new(0));
        let mut cache = ModelCache::new();
        cache
            .get_or_load(WhisperModel::BaseEn, || Ok(Context::load(&active)))
            .unwrap();
        cache
            .get_or_load(WhisperModel::BaseEn, || panic!("must reuse context"))
            .unwrap();
        cache
            .get_or_load(WhisperModel::SmallEn, || Ok(Context::load(&active)))
            .unwrap();
        assert_eq!(active.load(Ordering::Acquire), 1);
        assert!(cache
            .get_or_load(WhisperModel::BaseEn, || Err(
                TranscriptionError::ModelNotReady
            ))
            .is_err());
        assert_eq!(active.load(Ordering::Acquire), 0);
        cache
            .get_or_load(WhisperModel::BaseEn, || Ok(Context::load(&active)))
            .unwrap();
        cache.clear();
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    struct CachedEngine {
        cache: ModelCache<Context>,
    }
    impl Engine for CachedEngine {
        fn run(&mut self, _: AudioBuffer, _: WhisperModel, _: &AtomicBool) -> ResultText {
            unreachable!()
        }
        fn clear(&mut self) {
            self.cache.clear();
        }
    }
    #[test]
    fn five_minute_eviction_uses_time_since_execution_finished_and_allows_reload() {
        let active = Arc::new(AtomicUsize::new(0));
        let mut engine = CachedEngine {
            cache: ModelCache::new(),
        };
        engine
            .cache
            .get_or_load(WhisperModel::BaseEn, || Ok(Context::load(&active)))
            .unwrap();
        let start = Instant::now();
        let mut idle = IdleDeadline {
            since: start,
            limit: IDLE_UNLOAD,
        };
        // Worker does not call eviction during run; finishing a 10-minute job
        // resets the idle clock before its next timed receive.
        let finished = start + Duration::from_secs(600);
        idle.reset(finished);
        idle.evict_if_expired(finished + Duration::from_secs(299), &mut engine);
        assert_eq!(active.load(Ordering::Acquire), 1);
        idle.evict_if_expired(finished + Duration::from_secs(300), &mut engine);
        assert_eq!(active.load(Ordering::Acquire), 0);
        engine
            .cache
            .get_or_load(WhisperModel::BaseEn, || Ok(Context::load(&active)))
            .unwrap();
        assert_eq!(active.load(Ordering::Acquire), 1);
    }
}
