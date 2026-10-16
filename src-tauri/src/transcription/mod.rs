use std::path::Path;
use std::sync::{Arc, Mutex};

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

struct CachedCtx {
    model: WhisperModel,
    ctx: Arc<WhisperContext>,
}

/// Loaded whisper context, cached across recordings because loading base.en
/// costs ~500ms. Swapped when the requested model changes (LLD §3.3).
/// `std::sync::Mutex::new` is const since Rust 1.63 — no `once_cell` needed,
/// matching this codebase's "no lazy_static/once_cell" convention.
static WHISPER_CTX: Mutex<Option<CachedCtx>> = Mutex::new(None);

/// Load a context from an explicit path. `ModelNotReady` if absent (testable
/// without a real model); `InferenceFailed` if present but unreadable.
fn load_ctx_from(path: &Path) -> Result<Arc<WhisperContext>, TranscriptionError> {
    if !path.exists() {
        return Err(TranscriptionError::ModelNotReady);
    }
    let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|e| TranscriptionError::InferenceFailed(format!("load model: {e}")))?;
    Ok(Arc::new(ctx))
}

/// Return a cached context for `model`, loading on first use or model change.
fn get_or_load_ctx(model: WhisperModel) -> Result<Arc<WhisperContext>, TranscriptionError> {
    // Recover from poisoning rather than panic-cascading every future call: the
    // guard only wraps an Option<CachedCtx>, so the worst case is one reload.
    let mut guard = WHISPER_CTX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = guard.as_ref() {
        if cached.model == model {
            return Ok(cached.ctx.clone());
        }
    }
    let path = model
        .model_file()
        .path()
        .ok_or(TranscriptionError::ModelNotReady)?;
    // Note: the ~500ms load runs under the lock; concurrent first-callers
    // serialize. Acceptable for a single-user app (they'd share the same cached
    // ctx anyway).
    let ctx = load_ctx_from(&path)?;
    *guard = Some(CachedCtx {
        model,
        ctx: ctx.clone(),
    });
    Ok(ctx)
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
fn infer_recording(ctx: &WhisperContext, pcm: &[f32]) -> Result<String, TranscriptionError> {
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
    state
        .full(params, pcm)
        .map_err(|e| TranscriptionError::InferenceFailed(format!("inference: {e}")))?;
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

/// Load the model (cached) and transcribe on the calling (blocking) thread.
fn transcribe_blocking(
    buffer: AudioBuffer,
    model: WhisperModel,
) -> Result<String, TranscriptionError> {
    let ctx = get_or_load_ctx(model)?;
    transcribe_recording(&buffer.samples, |pcm| infer_recording(&ctx, pcm))
}

pub async fn transcribe(
    buffer: AudioBuffer,
    model: WhisperModel,
) -> Result<String, TranscriptionError> {
    tokio::task::spawn_blocking(move || transcribe_blocking(buffer, model))
        .await
        .map_err(|e| TranscriptionError::InferenceFailed(format!("join: {e}")))?
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
    fn missing_model_file_is_model_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("base.en.bin");
        // `unwrap_err()` would require `Debug` on the Ok type (Arc<WhisperContext>),
        // which whisper-rs doesn't implement — match on the result instead.
        assert!(matches!(
            load_ctx_from(&missing),
            Err(TranscriptionError::ModelNotReady)
        ));
    }

    /// `get_or_load_ctx` (not just `load_ctx_from`) needs its own coverage: a
    /// `$HOME` with no model file present drives the `model_file().path()` →
    /// `load_ctx_from` → `ModelNotReady` chain end to end without needing a
    /// real GGML model (the cache-hit/cache-store branches do need one, so
    /// they're left uncovered — same class as the rest of the whisper-rs FFI
    /// surface in this file).
    #[test]
    fn get_or_load_ctx_is_model_not_ready_without_a_downloaded_model() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("HOME").ok();
        let home_dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home_dir.path());

        assert!(matches!(
            get_or_load_ctx(WhisperModel::SmallEn),
            Err(TranscriptionError::ModelNotReady)
        ));

        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
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

    /// `transcribe_blocking`/`transcribe`/`WhisperTranscriber::transcribe` are
    /// thin dispatch (cache lookup → infer); with no model downloaded
    /// they all short-circuit at `get_or_load_ctx` before ever touching real
    /// whisper-rs inference (`infer_recording`, still uncovered — that needs a real
    /// GGML model file and is out of scope for a unit test).
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
