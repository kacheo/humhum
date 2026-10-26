use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    OnceLock,
};

mod worker;
use worker::{Engine, ModelCache, Worker};

use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::AudioBuffer;
use crate::models::ModelFile;

#[derive(Debug)]
pub enum TranscriptionError {
    ModelNotReady,
    InferenceFailed(String),
    EmptyResult,
}

impl TranscriptionError {
    /// Stable snake_case wire code (LLD §593).
    pub fn code(&self) -> String {
        match self {
            TranscriptionError::ModelNotReady => "model_not_ready",
            TranscriptionError::InferenceFailed(_) => "inference_failed",
            TranscriptionError::EmptyResult => "empty_result",
        }
        .to_string()
    }
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionError::ModelNotReady => write!(f, "model not downloaded"),
            TranscriptionError::InferenceFailed(e) => write!(f, "inference failed: {e}"),
            TranscriptionError::EmptyResult => write!(f, "empty transcription result"),
        }
    }
}

// The worker alone owns native resources; these atoms expose metadata only.
static WORKER: OnceLock<Result<Worker, String>> = OnceLock::new();
static CACHED_MODEL: AtomicU8 = AtomicU8::new(0);
static INFERENCE_SAMPLES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn memory_snapshot() -> (usize, &'static str, usize) {
    let active = WORKER
        .get()
        .and_then(|worker| worker.as_ref().ok())
        .is_some_and(Worker::is_busy) as usize;
    let model = match CACHED_MODEL.load(Ordering::Relaxed) {
        1 => "base_en",
        2 => "small_en",
        _ => "none",
    };
    (active, model, INFERENCE_SAMPLES.load(Ordering::Relaxed))
}

fn load_ctx_from(path: &Path) -> Result<WhisperContext, TranscriptionError> {
    if !path.exists() {
        return Err(TranscriptionError::ModelNotReady);
    }
    WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|e| TranscriptionError::InferenceFailed(format!("load model: {e}")))
}

struct WhisperEngine {
    cache: ModelCache<WhisperContext>,
}
impl Engine for WhisperEngine {
    fn run(
        &mut self,
        buffer: AudioBuffer,
        model: WhisperModel,
        cancel: &AtomicBool,
    ) -> Result<String, TranscriptionError> {
        check_cancelled(cancel)?;
        if buffer.sample_rate != crate::audio::buffer::TARGET_SAMPLE_RATE {
            return Err(TranscriptionError::InferenceFailed(
                "expected 16 kHz mono audio".into(),
            ));
        }
        INFERENCE_SAMPLES.store(buffer.samples.len(), Ordering::Relaxed);
        struct SamplesGuard;
        impl Drop for SamplesGuard {
            fn drop(&mut self) {
                INFERENCE_SAMPLES.store(0, Ordering::Relaxed);
            }
        }
        let _samples = SamplesGuard;
        let ctx = self.cache.get_or_load(model, || {
            CACHED_MODEL.store(0, Ordering::Relaxed);
            check_cancelled(cancel)?;
            let path = model
                .model_file()
                .path()
                .ok_or(TranscriptionError::ModelNotReady)?;
            let ctx = load_ctx_from(&path)?;
            CACHED_MODEL.store(
                match model {
                    WhisperModel::BaseEn => 1,
                    WhisperModel::SmallEn => 2,
                },
                Ordering::Relaxed,
            );
            Ok(ctx)
        })?;
        check_cancelled(cancel)?;
        let text = transcribe_recording(&buffer.samples, |pcm| infer_recording(ctx, pcm, cancel));
        check_cancelled(cancel)?;
        text
    }
    fn clear(&mut self) {
        self.cache.clear();
        CACHED_MODEL.store(0, Ordering::Relaxed);
    }
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), TranscriptionError> {
    if cancel.load(Ordering::Acquire) {
        Err(worker::cancelled())
    } else {
        Ok(())
    }
}

// SAFETY: user_data points to an AtomicBool owned by the job's Arc. It
// outlives the synchronous state.full call and all of its native callbacks.
// Only atomic access occurs here; no allocation or unwinding crosses C ABI.
unsafe extern "C" fn abort_inference(user_data: *mut std::ffi::c_void) -> bool {
    unsafe { (&*user_data.cast::<AtomicBool>()).load(Ordering::Acquire) }
}

/// Transcribe the full 16 kHz mono recording once, then clean and guard output.
/// Whisper handles its internal audio windows; joining overlapping text here
/// can duplicate fragments or erase intentional repeated instructions.
fn transcribe_recording<F>(samples: &[f32], mut infer: F) -> Result<String, TranscriptionError>
where
    F: FnMut(&[f32]) -> Result<String, TranscriptionError>,
{
    // Whisper can hallucinate words on exact digital silence. Do not apply a
    // volume threshold: even very quiet nonzero recordings must be decoded.
    if samples.iter().all(|sample| *sample == 0.0) {
        return Err(TranscriptionError::EmptyResult);
    }
    let raw = infer(samples)?;
    let cleaned = strip_nonspeech(&raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !is_meaningful(&cleaned) {
        return Err(TranscriptionError::EmptyResult);
    }
    Ok(cleaned)
}

/// Remove whisper's non-speech annotations — bracketed/parenthesized spans like
/// "[BLANK_AUDIO]", "[ Silence ]", "(wind blowing)". These are emitted as
/// segment text on non-speech audio and must never reach the user's document.
fn strip_nonspeech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth: i32 = 0;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// True if `text` carries real speech: at least one alphanumeric char remains
/// after stripping non-speech annotations. Whitespace/punctuation/annotation-
/// only output is treated as blank (LLD §239-240).
fn is_meaningful(text: &str) -> bool {
    strip_nonspeech(text).chars().any(|c| c.is_alphanumeric())
}

/// Run whisper inference over a full 16 kHz mono recording (thin adapter; untested —
/// covered manually via the real-model verification step).
fn infer_recording(
    ctx: &WhisperContext,
    pcm: &[f32],
    cancel: &AtomicBool,
) -> Result<String, TranscriptionError> {
    check_cancelled(cancel)?;
    let mut state = ctx
        .create_state()
        .map_err(|e| TranscriptionError::InferenceFailed(format!("create state: {e}")))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_translate(false);
    // Keep recordings independent. Whisper advances through the full audio
    // using its own timestamps; no application overlap joining is needed.
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    // Use the raw callback API: callback storage is borrowed from the job,
    // rather than the convenience wrapper's closure/user-data ownership.
    unsafe {
        params.set_abort_callback(Some(abort_inference));
        params.set_abort_callback_user_data((cancel as *const AtomicBool).cast_mut().cast());
    }
    state
        .full(params, pcm)
        .map_err(|e| TranscriptionError::InferenceFailed(format!("inference: {e}")))?;
    check_cancelled(cancel)?;
    collect_segment_text(state.as_iter().map(|segment| segment.to_str()))
}

fn collect_segment_text<'a>(
    segments: impl IntoIterator<Item = Result<&'a str, whisper_rs::WhisperError>>,
) -> Result<String, TranscriptionError> {
    let mut text = String::new();
    for (index, segment) in segments.into_iter().enumerate() {
        let value = segment.map_err(|error| {
            TranscriptionError::InferenceFailed(format!("segment {index}: {error}"))
        })?;
        text.push_str(value);
    }
    Ok(text)
}

pub async fn transcribe(
    buffer: AudioBuffer,
    model: WhisperModel,
) -> Result<String, TranscriptionError> {
    let worker = WORKER.get_or_init(|| {
        Worker::start(
            WhisperEngine {
                cache: ModelCache::new(),
            },
            worker::IDLE_UNLOAD,
        )
        .map_err(|error| error.to_string())
    });
    match worker {
        Ok(worker) => worker.submit(buffer, model).await,
        Err(error) => Err(TranscriptionError::InferenceFailed(format!(
            "start native worker: {error}; restart Humhum"
        ))),
    }
}

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Injectable transcription step so the Phase 15 pipeline is testable with a
/// stub (no model file). Production impl calls the existing [`transcribe`].
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        buffer: AudioBuffer,
        model: WhisperModel,
    ) -> BoxFuture<'static, Result<String, TranscriptionError>>;
}

pub struct WhisperTranscriber;

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &self,
        buffer: AudioBuffer,
        model: WhisperModel,
    ) -> BoxFuture<'static, Result<String, TranscriptionError>> {
        Box::pin(transcribe(buffer, model))
    }
}

/// Construct the production transcriber (whisper-rs, cached context).
pub fn create() -> Box<dyn Transcriber> {
    Box::new(WhisperTranscriber)
}

/// Whisper model selection. Serializes to `base_en`, `small_en` — the strings
/// shared with the TS prefs contract and the model-download IPC commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhisperModel {
    BaseEn,
    SmallEn,
}

impl WhisperModel {
    pub(crate) fn model_file(self) -> ModelFile {
        match self {
            WhisperModel::BaseEn => ModelFile::BaseEn,
            WhisperModel::SmallEn => ModelFile::SmallEn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_abort_callback_reads_owned_cancellation_flag() {
        let cancellation = Box::new(AtomicBool::new(false));
        let data = (&*cancellation as *const AtomicBool).cast_mut().cast();
        assert!(!unsafe { abort_inference(data) });
        cancellation.store(true, Ordering::Release);
        assert!(unsafe { abort_inference(data) });
        assert!(matches!(
            check_cancelled(&cancellation),
            Err(TranscriptionError::InferenceFailed(_))
        ));
    }

    #[test]
    fn cancelled_job_does_not_load_a_model() {
        let mut engine = WhisperEngine {
            cache: ModelCache::new(),
        };
        let error = engine
            .run(empty_buffer(), WhisperModel::BaseEn, &AtomicBool::new(true))
            .unwrap_err();
        assert_eq!(error.to_string(), "inference failed: cancelled");
    }

    #[test]
    fn missing_model_file_is_model_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("base.en.bin");
        // `unwrap_err()` would require `Debug` on the Ok type (WhisperContext),
        // which whisper-rs doesn't implement — match on the result instead.
        assert!(matches!(
            load_ctx_from(&missing),
            Err(TranscriptionError::ModelNotReady)
        ));
    }

    #[test]
    fn whisper_model_maps_to_model_file() {
        assert_eq!(
            WhisperModel::BaseEn.model_file(),
            crate::models::ModelFile::BaseEn
        );
        assert_eq!(
            WhisperModel::SmallEn.model_file(),
            crate::models::ModelFile::SmallEn
        );
    }

    #[test]
    fn transcription_error_codes_are_stable_snake_case() {
        assert_eq!(TranscriptionError::ModelNotReady.code(), "model_not_ready");
        assert_eq!(
            TranscriptionError::InferenceFailed("x".into()).code(),
            "inference_failed"
        );
        assert_eq!(TranscriptionError::EmptyResult.code(), "empty_result");
    }

    #[test]
    fn transcription_error_display_messages() {
        assert_eq!(
            TranscriptionError::ModelNotReady.to_string(),
            "model not downloaded"
        );
        assert_eq!(
            TranscriptionError::InferenceFailed("boom".into()).to_string(),
            "inference failed: boom"
        );
        assert_eq!(
            TranscriptionError::EmptyResult.to_string(),
            "empty transcription result"
        );
    }

    /// A path that exists but isn't a valid GGML file: `load_ctx_from` must
    /// still fail fast (real whisper.cpp header-parse rejection), not hang or
    /// panic, and map the failure to `InferenceFailed` — no real model needed.
    #[test]
    fn present_but_invalid_model_file_is_inference_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bogus.bin");
        std::fs::write(&path, b"not a real ggml model").unwrap();
        assert!(matches!(
            load_ctx_from(&path),
            Err(TranscriptionError::InferenceFailed(_))
        ));
    }

    fn test_pcm(secs: usize) -> Vec<f32> {
        vec![0.125; secs * 16_000]
    }

    fn empty_buffer() -> AudioBuffer {
        AudioBuffer {
            samples: vec![],
            sample_rate: 16_000,
            duration_ms: 0,
        }
    }

    /// Production dispatch reports missing models without invoking native inference.
    #[tokio::test]
    async fn transcribe_is_model_not_ready_without_a_downloaded_model() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("HOME").ok();
        let home_dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home_dir.path());

        assert!(matches!(
            transcribe(empty_buffer(), WhisperModel::SmallEn).await,
            Err(TranscriptionError::ModelNotReady)
        ));
        assert!(matches!(
            WhisperTranscriber
                .transcribe(empty_buffer(), WhisperModel::SmallEn)
                .await,
            Err(TranscriptionError::ModelNotReady)
        ));

        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn segment_read_failure_is_not_silently_dropped() {
        let result =
            collect_segment_text([Ok("hello"), Err(whisper_rs::WhisperError::NullPointer)]);
        assert!(
            matches!(result, Err(TranscriptionError::InferenceFailed(message)) if message.starts_with("segment 1:"))
        );
    }

    #[test]
    fn segment_collection_keeps_text_and_whitespace_in_order() {
        assert_eq!(
            collect_segment_text([Ok("hello"), Ok(" world.")]).unwrap(),
            "hello world."
        );
    }

    #[test]
    fn single_recording_passes_inference_through() {
        let out = transcribe_recording(&test_pcm(1), |_| Ok("hello world".to_string())).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn whole_recording_reaches_inference_once_including_final_sample() {
        let mut samples = test_pcm(70);
        samples[70 * 16_000 - 1] = 0.75;
        let mut calls = 0;
        let out = transcribe_recording(&samples, |pcm| {
            calls += 1;
            assert_eq!(pcm.len(), 1_120_000);
            assert_eq!(pcm.last(), Some(&0.75));
            Ok("The final sentence is intact.".into())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(out, "The final sentence is intact.");
    }

    #[test]
    fn digital_silence_skips_inference_instead_of_hallucinating() {
        let result = transcribe_recording(&[0.0, -0.0, 0.0], |_| {
            panic!("digital silence must not reach Whisper")
        });
        assert!(matches!(result, Err(TranscriptionError::EmptyResult)));
    }

    #[test]
    fn empty_recording_skips_inference() {
        let result = transcribe_recording(&[], |_| panic!("empty PCM must not reach Whisper"));
        assert!(matches!(result, Err(TranscriptionError::EmptyResult)));
    }

    #[test]
    fn intentional_repetitions_and_distinct_instructions_are_preserved() {
        let text = "Choose red lights. Choose bright red lights. Then stop. Again. Again.";
        let out = transcribe_recording(&test_pcm(70), |_| Ok(text.into())).unwrap();
        assert_eq!(out, text);
    }

    #[test]
    fn blank_inference_is_empty_result() {
        let err = transcribe_recording(&test_pcm(1), |_| Ok("   ".to_string())).unwrap_err();
        assert!(matches!(err, TranscriptionError::EmptyResult));
    }

    #[test]
    fn inference_error_propagates() {
        let err = transcribe_recording(&test_pcm(1), |_| {
            Err(TranscriptionError::InferenceFailed("boom".into()))
        })
        .unwrap_err();
        assert!(matches!(err, TranscriptionError::InferenceFailed(_)));
    }

    #[test]
    fn punctuation_only_is_empty_result() {
        let err = transcribe_recording(&test_pcm(1), |_| Ok(" . ".to_string())).unwrap_err();
        assert!(matches!(err, TranscriptionError::EmptyResult));
    }

    #[test]
    fn blank_audio_annotation_is_empty_result() {
        let err =
            transcribe_recording(&test_pcm(1), |_| Ok("[BLANK_AUDIO]".to_string())).unwrap_err();
        assert!(matches!(err, TranscriptionError::EmptyResult));
    }

    #[test]
    fn nonspeech_annotation_is_stripped_from_kept_output() {
        let out =
            transcribe_recording(
                &test_pcm(1),
                |_| Ok("hello [BLANK_AUDIO] world".to_string()),
            )
            .unwrap();
        assert_eq!(out, "hello world");
    }
}

#[cfg(test)]
mod benchmarks;
