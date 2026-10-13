//! Whisper model management: status queries, integrity-verified streaming
//! downloads with per-1% progress events.
//!
//! Follows this codebase's "pure core + thin IO/network adapter" seam (cf.
//! `transcription::transcribe_chunks`, `paste::decide`, `prefs::settings_file`):
//! disk-space math, SHA-256 comparison, status determination, and progress
//! quantization are pure and CI-testable; `reqwest`/`statvfs`/filesystem calls
//! are thin wrappers over those cores. Downloads land in `<model>.bin.tmp` and
//! are promoted to `<model>.bin` by an atomic rename only *after* the SHA-256 of
//! the temp file matches the pinned hash — a partial/corrupt download is never
//! visible as a usable model.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::events::ModelDownloadProgress;
use crate::transcription::WhisperModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFile {
    BaseEn,
    SmallEn,
}

impl ModelFile {
    pub fn filename(&self) -> &str {
        match self {
            ModelFile::BaseEn => "base.en.bin",
            ModelFile::SmallEn => "small.en.bin",
        }
    }

    /// Absolute path to this model on disk, or `None` if `$HOME` is unset.
    pub fn path(&self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(model_file_in(Path::new(&home), *self))
    }

    /// Download source + pinned integrity constants. The `sha256`/`size_bytes`
    /// are taken from the file's Hugging Face git-LFS pointer
    /// (`/ggerganov/whisper.cpp/raw/main/<file>` → `oid sha256:…` / `size …`),
    /// so we verify against a known-good hash rather than trusting the bytes we
    /// happen to download. In debug builds, an all-or-nothing per-model env
    /// override (`HUMHUM_MODEL_<FILE>_URL`/`_SHA256`/`_SIZE_BYTES`) points
    /// this at the E2E mock server + fixture instead — all three or none, so a
    /// partially-set override can't produce a silently-wrong hash/size pairing.
    fn spec(&self) -> ModelSpec {
        #[cfg(debug_assertions)]
        if let Some(spec) = self.env_override() {
            return spec;
        }
        match self {
            ModelFile::BaseEn => ModelSpec {
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
                    .to_string(),
                sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
                    .to_string(),
                size_bytes: 147_964_211,
            },
            ModelFile::SmallEn => ModelSpec {
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"
                    .to_string(),
                sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d"
                    .to_string(),
                size_bytes: 487_614_201,
            },
        }
    }

    /// Env-var prefix for this model's E2E override triple.
    #[cfg(debug_assertions)]
    fn env_prefix(&self) -> &'static str {
        match self {
            ModelFile::BaseEn => "HUMHUM_MODEL_BASE_EN",
            ModelFile::SmallEn => "HUMHUM_MODEL_SMALL_EN",
        }
    }

    /// `Some(spec)` only when `_URL`/`_SHA256`/`_SIZE_BYTES` are all set and
    /// `_SIZE_BYTES` parses; otherwise falls back to the pinned production spec.
    #[cfg(debug_assertions)]
    fn env_override(&self) -> Option<ModelSpec> {
        let prefix = self.env_prefix();
        let url = std::env::var(format!("{prefix}_URL")).ok()?;
        let sha256 = std::env::var(format!("{prefix}_SHA256")).ok()?;
        let size_bytes = std::env::var(format!("{prefix}_SIZE_BYTES"))
            .ok()?
            .parse()
            .ok()?;
        Some(ModelSpec {
            url,
            sha256,
            size_bytes,
        })
    }
}

/// Per-model download facts. `String` (not `&'static str`) so a debug-only E2E
/// override can substitute the mock server's URL/hash/size at runtime.
struct ModelSpec {
    url: String,
    sha256: String,
    size_bytes: u64,
}

/// `<home>/Library/Application Support/humhum/models/<filename>` — pure so
/// it is testable without touching `$HOME` (mirrors `prefs::settings_file`).
fn model_file_in(home: &Path, model: ModelFile) -> PathBuf {
    home.join("Library/Application Support/humhum/models")
        .join(model.filename())
}

/// The in-progress temp sibling of a final model path (`…/base.en.bin.tmp`).
fn tmp_for(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Download/availability state of a whisper model. Serializes snake_case;
/// data-carrying variants are externally tagged:
/// `NotDownloaded → "not_downloaded"`, `Downloading(50) → {"downloading":50}`,
/// `Ready → "ready"`, `Error("x") → {"error":"x"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    NotDownloaded,
    Downloading(u8),
    Ready,
    Error(String),
}

#[derive(Debug)]
pub enum ModelError {
    InsufficientDisk(u64, u64),
    DownloadFailed(String),
    HashMismatch,
    IoError(String),
    /// A download for this model is already in flight (LLD `already_downloading`).
    AlreadyDownloading,
}

impl ModelError {
    /// Stable snake_case wire code (LLD §603, `download_model`).
    pub fn code(&self) -> String {
        match self {
            ModelError::InsufficientDisk(_, _) => "insufficient_disk",
            ModelError::DownloadFailed(_) => "download_failed",
            ModelError::HashMismatch => "hash_mismatch",
            ModelError::IoError(_) => "io_error",
            ModelError::AlreadyDownloading => "already_downloading",
        }
        .to_string()
    }
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::InsufficientDisk(need, have) => write!(f, "need {need} bytes, have {have}"),
            ModelError::DownloadFailed(e) => write!(f, "download failed: {e}"),
            ModelError::HashMismatch => write!(f, "SHA-256 hash mismatch"),
            ModelError::IoError(e) => write!(f, "IO error: {e}"),
            ModelError::AlreadyDownloading => write!(f, "a download is already in progress"),
        }
    }
}

// ---- Pure cores --------------------------------------------------------------

/// Headroom required on disk beyond the model itself before we start a download.
const DISK_MARGIN_BYTES: u64 = 100 * 1024 * 1024; // 100 MiB

fn required_bytes(model_size: u64) -> u64 {
    model_size + DISK_MARGIN_BYTES
}

fn has_enough_disk(required: u64, available: u64) -> bool {
    available >= required
}

/// Integer download percent, clamped to 100. `total == 0` reports complete
/// rather than dividing by zero.
fn progress_pct(downloaded: u64, total: u64) -> u8 {
    if total == 0 {
        return 100;
    }
    ((downloaded * 100 / total).min(100)) as u8
}

/// Tracks the last emitted percent so a download fires at most one event per 1%
/// increment (≤101 events for any size), not one per network chunk.
struct ProgressThrottle {
    last: Option<u8>,
}

impl ProgressThrottle {
    fn new() -> Self {
        Self { last: None }
    }

    /// Returns `Some(pct)` only when the integer percent strictly increases.
    fn update(&mut self, downloaded: u64, total: u64) -> Option<u8> {
        let pct = progress_pct(downloaded, total);
        if self.last.map_or(true, |l| pct > l) {
            self.last = Some(pct);
            Some(pct)
        } else {
            None
        }
    }
}

/// Cheap status from on-disk size alone (no hashing). A present file whose size
/// matches the pinned size is `Ready`; anything else (absent, or wrong size /
/// truncated) is `NotDownloaded`. In-flight downloads are handled by the caller
/// via the [`DOWNLOADS`] registry, before this is consulted.
fn status_for(actual_size: Option<u64>, expected_size: u64) -> ModelStatus {
    match actual_size {
        Some(sz) if sz == expected_size => ModelStatus::Ready,
        _ => ModelStatus::NotDownloaded,
    }
}

/// Stream a file through SHA-256 in 64 KiB reads (never loads the whole model
/// into memory), returning lowercase hex.
fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify a file against the pinned size + hash: cheap size check first, then
/// SHA-256. Size or hash mismatch → [`ModelError::HashMismatch`]; an unreadable
/// file → [`ModelError::IoError`].
fn verify_file_hash(path: &Path, expected_size: u64, expected_hex: &str) -> Result<(), ModelError> {
    let meta = std::fs::metadata(path).map_err(|e| ModelError::IoError(e.to_string()))?;
    if meta.len() != expected_size {
        return Err(ModelError::HashMismatch);
    }
    let actual = sha256_file(path).map_err(|e| ModelError::IoError(e.to_string()))?;
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(ModelError::HashMismatch)
    }
}

/// Bytes available to a non-privileged user on the filesystem holding `path`.
fn available_disk(path: &Path) -> io::Result<u64> {
    use nix::sys::statvfs::statvfs;
    let stat = statvfs(path).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(stat.blocks_available() as u64 * stat.fragment_size() as u64)
}

// ---- In-flight download registry --------------------------------------------

/// Models with a download currently in flight. A `Vec` (not `HashSet`) so it can
/// be a `const`-initializable `static` without `WhisperModel: Hash`; the set is
/// tiny (≤2). Poisoning is recovered rather than propagated.
static DOWNLOADS: Mutex<Vec<WhisperModel>> = Mutex::new(Vec::new());

/// Registers `model` as downloading, returning a guard that unregisters on drop.
/// `None` if a download for this model is already registered.
fn register_download(model: WhisperModel) -> Option<DownloadGuard> {
    let mut g = DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
    if g.contains(&model) {
        return None;
    }
    g.push(model);
    Some(DownloadGuard(model))
}

fn is_downloading(model: WhisperModel) -> bool {
    DOWNLOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&model)
}

struct DownloadGuard(WhisperModel);

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        let mut g = DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|m| m != &self.0);
    }
}

// ---- Progress emitter seam ---------------------------------------------------

/// Sink for `model-download-progress`. Keeps [`ModelManager::download`] free of
/// Tauri so it is unit-testable with a fake; production impl is `AppHandle`.
pub trait ProgressEmitter {
    fn emit_progress(&self, model: WhisperModel, pct: u8);
}

impl<R: tauri::Runtime> ProgressEmitter for tauri::AppHandle<R> {
    fn emit_progress(&self, model: WhisperModel, pct: u8) {
        use tauri::Emitter;
        if let Err(e) = self.emit(
            "model-download-progress",
            ModelDownloadProgress { model, pct },
        ) {
            log::warn!("models: emit model-download-progress failed: {e}");
        }
    }
}

// ---- Manager -----------------------------------------------------------------

/// Pure predicate: a model is usable for recording only when fully on disk.
pub fn model_ready(status: &ModelStatus) -> bool {
    matches!(status, ModelStatus::Ready)
}

/// Injectable "is the model downloaded?" gate, so the recording controller's
/// start guard (LLD §9.1 / mm0.1) is unit-testable without touching `$HOME`
/// (mirrors the `AudioCapture`/`Permissions` DI seams). Production reads disk.
pub trait ModelGate: Send + Sync {
    fn is_ready(&self, model: WhisperModel) -> bool;
}

pub struct DiskModelGate;

impl ModelGate for DiskModelGate {
    fn is_ready(&self, model: WhisperModel) -> bool {
        model_ready(&ModelManager::new().get_status(model))
    }
}

/// Construct the production model gate (size-based on-disk check).
pub fn create_model_gate() -> Box<dyn ModelGate> {
    Box::new(DiskModelGate)
}

pub struct ModelManager;

impl ModelManager {
    pub fn new() -> Self {
        ModelManager
    }

    /// Cheap status: in-flight (registry) → `Downloading`; else size-based
    /// existence check. The live percent rides on `model-download-progress`
    /// events, so an in-flight status reports `Downloading(0)` as a placeholder.
    pub fn get_status(&self, model: WhisperModel) -> ModelStatus {
        if is_downloading(model) {
            return ModelStatus::Downloading(0);
        }
        let file = model.model_file();
        let actual_size = file
            .path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        status_for(actual_size, file.spec().size_bytes)
    }

    /// Verify the *final* `.bin` against its pinned hash. On failure the corrupt
    /// file is deleted (so the next [`get_status`](Self::get_status) reports
    /// `NotDownloaded` and onboarding re-prompts), then the error propagates.
    pub fn verify_hash(&self, model: WhisperModel) -> Result<(), ModelError> {
        let file = model.model_file();
        let spec = file.spec();
        let path = file
            .path()
            .ok_or_else(|| ModelError::IoError("$HOME is not set".into()))?;
        match verify_file_hash(&path, spec.size_bytes, &spec.sha256) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }

    /// Stream-download `model` to a temp file, emitting progress per 1%, verify
    /// its SHA-256, then atomically promote it to the final path. Concurrent
    /// downloads of the same model are rejected with
    /// [`ModelError::AlreadyDownloading`]. Any failure removes the partial
    /// `.tmp` so an aborted download never leaks a ~150–490 MB orphan.
    pub async fn download<E: ProgressEmitter>(
        &self,
        model: WhisperModel,
        emitter: &E,
    ) -> Result<(), ModelError> {
        let _guard = register_download(model).ok_or(ModelError::AlreadyDownloading)?;

        let file = model.model_file();
        let spec = file.spec();
        let final_path = file
            .path()
            .ok_or_else(|| ModelError::IoError("$HOME is not set".into()))?;
        let tmp_path = tmp_for(&final_path);

        let parent = final_path
            .parent()
            .ok_or_else(|| ModelError::IoError("model path has no parent directory".into()))?;
        std::fs::create_dir_all(parent).map_err(|e| ModelError::IoError(e.to_string()))?;
        let available = available_disk(parent).map_err(|e| ModelError::IoError(e.to_string()))?;
        let required = required_bytes(spec.size_bytes);
        if !has_enough_disk(required, available) {
            return Err(ModelError::InsufficientDisk(required, available));
        }

        let result = Self::stream_and_finalize(model, &spec, &final_path, &tmp_path, emitter).await;
        if result.is_err() {
            // A failed/aborted download must not leave a partial `.tmp` orphaned
            // on disk (it is never resumed; would otherwise leak ~150–490 MB). On
            // the Ok path the rename already consumed `.tmp`, so this no-ops.
            let _ = std::fs::remove_file(&tmp_path);
        }
        result
    }

    /// [`stream_and_finalize_with_url`] against `spec`'s pinned production URL.
    async fn stream_and_finalize<E: ProgressEmitter>(
        model: WhisperModel,
        spec: &ModelSpec,
        final_path: &Path,
        tmp_path: &Path,
        emitter: &E,
    ) -> Result<(), ModelError> {
        Self::stream_and_finalize_with_url(
            model,
            &spec.url,
            spec.size_bytes,
            &spec.sha256,
            final_path,
            tmp_path,
            emitter,
        )
        .await
    }

    /// Fetch → stream-to-`.tmp` (throttled progress) → verify → atomically
    /// promote. Split out so [`download`](Self::download) can guarantee `.tmp`
    /// cleanup on every error path with one wrapper. `url`/`expected_size`/
    /// `expected_hex` are explicit params (rather than read off `ModelSpec`) so
    /// this is reachable from a `mockito` server in tests — mirrors
    async fn stream_and_finalize_with_url<E: ProgressEmitter>(
        model: WhisperModel,
        url: &str,
        expected_size: u64,
        expected_hex: &str,
        final_path: &Path,
        tmp_path: &Path,
        emitter: &E,
    ) -> Result<(), ModelError> {
        // `reqwest::Response::chunk()` streams without pulling in a `Stream`
        // extension trait (no new dep), driving the same `ProgressThrottle` core
        // proven in unit tests.
        let mut resp = reqwest::get(url)
            .await
            .map_err(|e| ModelError::DownloadFailed(e.to_string()))?
            .error_for_status()
            .map_err(|e| ModelError::DownloadFailed(e.to_string()))?;
        let total = resp.content_length().unwrap_or(expected_size);

        let mut out =
            std::fs::File::create(tmp_path).map_err(|e| ModelError::IoError(e.to_string()))?;
        let mut throttle = ProgressThrottle::new();
        let mut downloaded: u64 = 0;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| ModelError::DownloadFailed(e.to_string()))?
        {
            out.write_all(&chunk)
                .map_err(|e| ModelError::IoError(e.to_string()))?;
            downloaded += chunk.len() as u64;
            if let Some(pct) = throttle.update(downloaded, total) {
                emitter.emit_progress(model, pct);
            }
        }
        out.flush()
            .map_err(|e| ModelError::IoError(e.to_string()))?;
        drop(out);

        finalize_download(tmp_path, final_path, expected_size, expected_hex)?;
        emitter.emit_progress(model, 100);
        Ok(())
    }
}

/// Verify the freshly downloaded `tmp_path` and, on success, atomically promote
/// it to `final_path`. On a hash/size mismatch, the `.tmp` is left for the
/// caller to clean and a pre-existing `final_path` is deleted **only if it is
/// also corrupt** — we never remove a user's working model just because a
/// re-download served bad bytes (a deliberate safety deviation from bead
/// 5na.5's literal "delete existing .bin").
fn finalize_download(
    tmp_path: &Path,
    final_path: &Path,
    expected_size: u64,
    expected_hex: &str,
) -> Result<(), ModelError> {
    match verify_file_hash(tmp_path, expected_size, expected_hex) {
        Ok(()) => {
            std::fs::rename(tmp_path, final_path).map_err(|e| ModelError::IoError(e.to_string()))
        }
        Err(e) => {
            if final_path.exists()
                && verify_file_hash(final_path, expected_size, expected_hex).is_err()
            {
                let _ = std::fs::remove_file(final_path);
            }
            Err(e)
        }
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Recover a download that finished and hash-verified but was interrupted
/// before [`finalize_download`]'s atomic rename — leaving a full `<model>.tmp`
/// orphaned while `<model>.bin` is absent (seen after the data-dir rename
/// stranded a valid `base.en.bin.tmp`). Called only when the final `.bin` is
/// missing. A hash-valid `.tmp` is promoted via the same atomic rename as a
/// fresh download (returns `true`); a corrupt or short `.tmp` is deleted so the
/// next status check reports `NotDownloaded` and a clean re-download can occur
/// (returns `false`). No `.tmp` at all is a no-op (`false`).
fn recover_orphaned_tmp(final_path: &Path, expected_size: u64, expected_hex: &str) -> bool {
    let tmp_path = tmp_for(final_path);
    if !tmp_path.exists() {
        return false;
    }
    match verify_file_hash(&tmp_path, expected_size, expected_hex) {
        Ok(()) => match std::fs::rename(&tmp_path, final_path) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("models: promoting recovered .tmp failed: {e}");
                false
            }
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            log::info!(
                "models: discarded a corrupt orphaned .tmp ({}); a clean re-download can proceed",
                e.code()
            );
            false
        }
    }
}

/// Startup integrity check (LLD §3.x / bead 5na.6): if base.en is present,
/// verify it; a corrupt file is removed by [`ModelManager::verify_hash`] so
/// `get_status` then reports `NotDownloaded` and the onboarding flow prompts a
/// re-download. A clean first launch (no file) is *not* an error. Emits nothing:
/// the blocking download prompt is the frontend's job, gated on `get_model_status`.
pub fn verify_base_model_on_startup() {
    let model = WhisperModel::BaseEn;
    let file = model.model_file();
    let present = file.path().map(|p| p.exists()).unwrap_or(false);
    if !present {
        // A prior download may have completed and hash-verified but been killed
        // before the atomic rename, stranding a valid `.tmp`. Promote it rather
        // than forcing a ~150 MB re-download the (complete) onboarding won't
        // even prompt for. A corrupt `.tmp` is discarded inside the helper.
        //
        // Safe without consulting the `DOWNLOADS` registry: this runs during
        // Tauri `setup()`, before the frontend can trigger a `download_model`,
        // so no in-flight download can be racing the `.tmp` at this point.
        if let Some(final_path) = file.path() {
            let spec = file.spec();
            if recover_orphaned_tmp(&final_path, spec.size_bytes, &spec.sha256) {
                log::info!("models: base.en recovered from an orphaned .tmp at startup");
                return;
            }
        }
        log::info!("models: base.en not present at startup (onboarding will prompt a download)");
        return;
    }
    match ModelManager::new().verify_hash(model) {
        Ok(()) => log::info!("models: base.en integrity verified at startup"),
        Err(e) => log::warn!(
            "models: base.en failed startup integrity check ({}); removed, will re-download",
            e.code()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Serializes every test that mutates process-wide state (`$HOME`, the
    /// debug-only `HUMHUM_MODEL_*` overrides, or the [`DOWNLOADS`]
    /// registry) so they can't race each other under `cargo test`'s default
    /// thread-per-test parallelism. All `#[tokio::test]`s in this module run
    /// on the default (current-thread) runtime, so holding this across an
    /// `.await` is safe — the guard is never moved to another thread.
    const OVERRIDE_VARS: [&str; 6] = [
        "HUMHUM_MODEL_BASE_EN_URL",
        "HUMHUM_MODEL_BASE_EN_SHA256",
        "HUMHUM_MODEL_BASE_EN_SIZE_BYTES",
        "HUMHUM_MODEL_SMALL_EN_URL",
        "HUMHUM_MODEL_SMALL_EN_SHA256",
        "HUMHUM_MODEL_SMALL_EN_SIZE_BYTES",
    ];

    /// RAII env-var guard: restores (or removes) the prior value on drop, so
    /// a panicking test doesn't leak state to whichever test runs next.
    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// The other `EnvVarGuard` uses all start from an unset var, so they only
    /// ever exercise the `None` (remove) drop branch; nesting two guards on
    /// the same key drives the `Some` (restore) branch too.
    #[test]
    fn env_var_guard_restores_a_previously_set_value_on_drop() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        const KEY: &str = "HUMHUM_TEST_ENV_VAR_GUARD_PROBE";
        let outer = EnvVarGuard::set(KEY, "initial");
        {
            let _inner = EnvVarGuard::set(KEY, "overridden");
            assert_eq!(std::env::var(KEY).unwrap(), "overridden");
        }
        assert_eq!(std::env::var(KEY).unwrap(), "initial");
        drop(outer);
    }

    #[test]
    fn model_ready_only_for_ready_status() {
        assert!(model_ready(&ModelStatus::Ready));
        assert!(!model_ready(&ModelStatus::NotDownloaded));
        assert!(!model_ready(&ModelStatus::Downloading(50)));
        assert!(!model_ready(&ModelStatus::Error("x".into())));
    }

    #[test]
    fn fresh_identity_models_do_not_use_legacy_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("Library/Application Support/wipzi/models/base.en.bin");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"legacy-model").unwrap();
        let current = model_file_in(temp.path(), ModelFile::BaseEn);
        assert_eq!(current, temp.path().join("Library/Application Support/humhum/models/base.en.bin"));
        assert!(!current.exists());
        assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy-model");
    }

    #[test]
    fn model_file_in_builds_models_subdir_path() {
        let p = model_file_in(Path::new("/Users/x"), ModelFile::BaseEn);
        assert_eq!(
            p,
            Path::new("/Users/x/Library/Application Support/humhum/models/base.en.bin")
        );
    }

    #[test]
    fn tmp_for_appends_tmp_suffix() {
        assert_eq!(
            tmp_for(Path::new("/m/base.en.bin")),
            Path::new("/m/base.en.bin.tmp")
        );
    }

    /// Locks the exact wire shape of every variant, including the
    /// externally-tagged data-carrying ones, and round-trips each back.
    #[test]
    fn model_status_wire_shapes_round_trip() {
        let cases = [
            (ModelStatus::NotDownloaded, json!("not_downloaded")),
            (ModelStatus::Ready, json!("ready")),
            (ModelStatus::Downloading(50), json!({ "downloading": 50 })),
            (
                ModelStatus::Error("boom".into()),
                json!({ "error": "boom" }),
            ),
        ];
        for (status, expected) in cases {
            let value = serde_json::to_value(&status).expect("serialize");
            assert_eq!(value, expected, "wire shape for {status:?}");
            let back: ModelStatus = serde_json::from_value(value).expect("deserialize");
            assert_eq!(back, status);
        }
    }

    #[test]
    fn already_downloading_has_stable_code() {
        assert_eq!(ModelError::AlreadyDownloading.code(), "already_downloading");
    }

    // ---- disk-space math (5na.3/.8) ----

    #[test]
    fn required_bytes_adds_fixed_margin() {
        assert_eq!(required_bytes(1_000), 1_000 + DISK_MARGIN_BYTES);
    }

    #[test]
    fn has_enough_disk_is_inclusive_at_the_boundary() {
        assert!(has_enough_disk(1_000, 1_000), "exactly enough is enough");
        assert!(has_enough_disk(1_000, 1_001));
        assert!(!has_enough_disk(1_000, 999), "one byte short fails");
    }

    // ---- progress quantization (5na.4/.8) ----

    #[test]
    fn progress_pct_clamps_and_guards_zero_total() {
        assert_eq!(progress_pct(0, 0), 100);
        assert_eq!(progress_pct(0, 200), 0);
        assert_eq!(progress_pct(100, 200), 50);
        assert_eq!(progress_pct(200, 200), 100);
        assert_eq!(progress_pct(999, 200), 100, "clamped");
    }

    #[test]
    fn throttle_emits_once_per_increasing_percent() {
        let mut t = ProgressThrottle::new();
        // total 100 → each byte is 1%.
        assert_eq!(t.update(0, 100), Some(0)); // first observation emits
        assert_eq!(t.update(0, 100), None); // same percent → silent
        assert_eq!(t.update(1, 100), Some(1));
        assert_eq!(t.update(1, 100), None);
        assert_eq!(t.update(2, 100), Some(2));
        assert_eq!(t.update(100, 100), Some(100)); // jumps straight to 100
    }

    #[test]
    fn throttle_caps_event_count_for_large_total() {
        // A realistic byte-by-byte feed must never exceed 101 events (0..=100).
        let total = 4_096u64;
        let mut t = ProgressThrottle::new();
        let emitted = (0..=total)
            .filter(|&d| t.update(d, total).is_some())
            .count();
        assert!(emitted <= 101, "emitted {emitted} > 101");
        assert!(emitted >= 100, "should reach ~every percent, got {emitted}");
    }

    // ---- status (5na.2) ----

    #[test]
    fn status_for_covers_every_branch() {
        assert_eq!(status_for(None, 100), ModelStatus::NotDownloaded);
        assert_eq!(status_for(Some(100), 100), ModelStatus::Ready);
        assert_eq!(
            status_for(Some(50), 100),
            ModelStatus::NotDownloaded,
            "truncated"
        );
        assert_eq!(
            status_for(Some(150), 100),
            ModelStatus::NotDownloaded,
            "oversized"
        );
    }

    // ---- SHA-256 verification (5na.5/.8) ----

    /// Compute the real SHA-256 of `bytes` via the same streaming core.
    fn sha_of(bytes: &[u8], dir: &Path) -> (PathBuf, String) {
        let path = dir.join("blob.bin");
        std::fs::write(&path, bytes).unwrap();
        let hex = sha256_file(&path).unwrap();
        (path, hex)
    }

    #[test]
    fn verify_file_hash_accepts_matching_and_rejects_tampered() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"humhum model bytes";
        let (path, hex) = sha_of(bytes, dir.path());

        // Correct size + hash → Ok.
        assert!(verify_file_hash(&path, bytes.len() as u64, &hex).is_ok());
        // Hash mismatch (right size, wrong expected hash) → HashMismatch.
        let wrong = "0".repeat(64);
        assert!(matches!(
            verify_file_hash(&path, bytes.len() as u64, &wrong),
            Err(ModelError::HashMismatch)
        ));
        // Size mismatch is rejected before hashing.
        assert!(matches!(
            verify_file_hash(&path, bytes.len() as u64 + 1, &hex),
            Err(ModelError::HashMismatch)
        ));
    }

    #[test]
    fn verify_file_hash_missing_file_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.bin");
        assert!(matches!(
            verify_file_hash(&missing, 1, &"0".repeat(64)),
            Err(ModelError::IoError(_))
        ));
    }

    // ---- offline full-cycle: download (throttled write) → verify → promote ----

    /// Mirrors `ModelManager::download`'s write loop without the network, so the
    /// rename-on-success / cleanup-on-mismatch behavior is covered in CI.
    fn write_with_throttle(chunks: &[&[u8]], total: u64, out: &Path) -> Vec<u8> {
        let mut file = std::fs::File::create(out).unwrap();
        let mut throttle = ProgressThrottle::new();
        let mut downloaded = 0u64;
        let mut emitted = Vec::new();
        for chunk in chunks {
            file.write_all(chunk).unwrap();
            downloaded += chunk.len() as u64;
            if let Some(pct) = throttle.update(downloaded, total) {
                emitted.push(pct);
            }
        }
        file.flush().unwrap();
        emitted
    }

    #[test]
    fn full_cycle_promotes_on_match_and_emits_progress() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        // 100-byte payload streamed in four 25-byte chunks → 25/50/75/100%.
        let payload = vec![7u8; 100];
        let chunks: Vec<&[u8]> = payload.chunks(25).collect();
        let emitted = write_with_throttle(&chunks, payload.len() as u64, &tmp_path);
        assert_eq!(emitted, vec![25, 50, 75, 100]);

        // Verify + promote through the real helper.
        let hex = sha256_file(&tmp_path).unwrap();
        finalize_download(&tmp_path, &final_path, payload.len() as u64, &hex).unwrap();
        assert!(final_path.exists() && !tmp_path.exists());
        assert_eq!(
            status_for(
                std::fs::metadata(&final_path).ok().map(|m| m.len()),
                payload.len() as u64
            ),
            ModelStatus::Ready
        );
    }

    #[test]
    fn finalize_preserves_valid_existing_bin_on_bad_download() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        // A *valid* pre-existing model: final's real hash is the expected one.
        let good = vec![b'A'; 64];
        std::fs::write(&final_path, &good).unwrap();
        let expected = sha256_file(&final_path).unwrap();
        let expected_size = good.len() as u64;
        // A corrupt fresh download (same size, different bytes) → so it fails on
        // the hash compare, not the size short-circuit.
        std::fs::write(&tmp_path, vec![b'B'; 64]).unwrap();
        assert_eq!(std::fs::metadata(&tmp_path).unwrap().len(), expected_size);

        let r = finalize_download(&tmp_path, &final_path, expected_size, &expected);
        assert!(matches!(r, Err(ModelError::HashMismatch)));
        assert!(
            final_path.exists(),
            "a working model is NOT clobbered by a bad re-download"
        );
    }

    #[test]
    fn finalize_removes_corrupt_existing_bin() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        // Both the existing model and the fresh download are corrupt vs `expected`.
        std::fs::write(&final_path, b"stale corrupt existing model").unwrap();
        std::fs::write(&tmp_path, b"freshly downloaded but corrupt").unwrap();
        let expected = "0".repeat(64);
        let size = std::fs::metadata(&tmp_path).unwrap().len();

        let r = finalize_download(&tmp_path, &final_path, size, &expected);
        assert!(matches!(r, Err(ModelError::HashMismatch)));
        assert!(
            !final_path.exists(),
            "an also-corrupt existing .bin is cleared"
        );
    }

    // ---- orphaned-.tmp recovery ----

    #[test]
    fn recover_promotes_a_hash_valid_orphaned_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        // A complete, hash-valid download stranded as `.tmp` (no final `.bin`).
        let payload = vec![3u8; 128];
        std::fs::write(&tmp_path, &payload).unwrap();
        let hex = sha256_file(&tmp_path).unwrap();

        let recovered = recover_orphaned_tmp(&final_path, payload.len() as u64, &hex);

        assert!(recovered, "a hash-valid .tmp should be recovered");
        assert!(
            final_path.exists(),
            "the .tmp is promoted to the final .bin"
        );
        assert!(!tmp_path.exists(), "the .tmp is consumed by the rename");
    }

    #[test]
    fn recover_deletes_a_corrupt_orphaned_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        // A .tmp whose bytes don't match the pinned hash (right size, wrong bytes).
        let bytes = vec![1u8; 64];
        std::fs::write(&tmp_path, &bytes).unwrap();
        let wrong = "0".repeat(64);

        let recovered = recover_orphaned_tmp(&final_path, bytes.len() as u64, &wrong);

        assert!(!recovered, "a corrupt .tmp is not a recovery");
        assert!(
            !tmp_path.exists(),
            "the corrupt .tmp is deleted for a clean re-download"
        );
        assert!(!final_path.exists(), "no bad model is promoted");
    }

    #[test]
    fn recover_deletes_a_truncated_orphaned_tmp() {
        // The exact scenario the feature targets: a download killed mid-write
        // leaves a `.tmp` shorter than the expected size. `verify_file_hash`
        // rejects it on the size check (before hashing), so it is discarded.
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");
        let tmp_path = tmp_for(&final_path);

        std::fs::write(&tmp_path, b"short").unwrap();
        // Expect a larger file than what's on disk, and any hash.
        let recovered = recover_orphaned_tmp(&final_path, 4096, &"0".repeat(64));

        assert!(!recovered, "a truncated .tmp is not a recovery");
        assert!(!tmp_path.exists(), "the truncated .tmp is deleted");
        assert!(!final_path.exists());
    }

    #[test]
    fn recover_is_a_noop_when_no_tmp_present() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("base.en.bin");

        let recovered = recover_orphaned_tmp(&final_path, 128, &"0".repeat(64));

        assert!(!recovered, "nothing to recover when there is no .tmp");
        assert!(!final_path.exists());
    }

    // ---- network glue: stream_and_finalize_with_url (mockito) ----

    #[derive(Default)]
    struct FakeProgressEmitter(Mutex<Vec<u8>>);

    impl ProgressEmitter for FakeProgressEmitter {
        fn emit_progress(&self, _model: WhisperModel, pct: u8) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).push(pct);
        }
    }

    #[tokio::test]
    async fn stream_and_finalize_with_url_downloads_verifies_and_promotes() {
        let mut server = mockito::Server::new_async().await;
        let payload = vec![9u8; 64];
        let hex = {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            format!("{:x}", hasher.finalize())
        };
        let m = server
            .mock("GET", "/model.bin")
            .with_status(200)
            .with_body(payload.clone())
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("model.bin");
        let tmp_path = tmp_for(&final_path);
        let emitter = FakeProgressEmitter::default();

        let result = ModelManager::stream_and_finalize_with_url(
            WhisperModel::BaseEn,
            &format!("{}/model.bin", server.url()),
            payload.len() as u64,
            &hex,
            &final_path,
            &tmp_path,
            &emitter,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(std::fs::read(&final_path).unwrap(), payload);
        assert!(
            !tmp_path.exists(),
            "tmp is consumed by the rename on success"
        );
        assert_eq!(emitter.0.lock().unwrap().last(), Some(&100));
        m.assert_async().await;
    }

    #[tokio::test]
    async fn stream_and_finalize_with_url_maps_http_error_to_download_failed() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/model.bin")
            .with_status(500)
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("model.bin");
        let tmp_path = tmp_for(&final_path);
        let emitter = FakeProgressEmitter::default();

        let result = ModelManager::stream_and_finalize_with_url(
            WhisperModel::BaseEn,
            &format!("{}/model.bin", server.url()),
            64,
            &"0".repeat(64),
            &final_path,
            &tmp_path,
            &emitter,
        )
        .await;

        assert!(matches!(result, Err(ModelError::DownloadFailed(_))));
        assert!(
            !tmp_path.exists(),
            "error_for_status fails before File::create"
        );
    }

    #[tokio::test]
    async fn stream_and_finalize_with_url_propagates_hash_mismatch_from_the_wire() {
        let mut server = mockito::Server::new_async().await;
        let payload = vec![3u8; 32];
        server
            .mock("GET", "/model.bin")
            .with_status(200)
            .with_body(payload.clone())
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("model.bin");
        let tmp_path = tmp_for(&final_path);
        let emitter = FakeProgressEmitter::default();

        let result = ModelManager::stream_and_finalize_with_url(
            WhisperModel::BaseEn,
            &format!("{}/model.bin", server.url()),
            payload.len() as u64,
            &"0".repeat(64), // deliberately wrong hash
            &final_path,
            &tmp_path,
            &emitter,
        )
        .await;

        assert!(matches!(result, Err(ModelError::HashMismatch)));
        assert!(
            tmp_path.exists(),
            "finalize_download leaves .tmp for the caller to clean"
        );
        assert!(!final_path.exists());
    }

    // ---- registry (5na.7 concurrency guard) ----

    /// Validates the pinned base.en URL + size + SHA-256 against the live HF
    /// file. IGNORED: downloads ~148 MB. Run on demand:
    ///   cargo test --manifest-path src-tauri/Cargo.toml \
    ///     models::tests::manual_real_base_en_matches_pinned_hash -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn manual_real_base_en_matches_pinned_hash() {
        let spec = ModelFile::BaseEn.spec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ggml-base.en.bin");
        let mut resp = reqwest::get(&spec.url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let mut out = std::fs::File::create(&path).unwrap();
        while let Some(chunk) = resp.chunk().await.unwrap() {
            out.write_all(&chunk).unwrap();
        }
        out.flush().unwrap();
        verify_file_hash(&path, spec.size_bytes, &spec.sha256)
            .expect("pinned base.en size + hash match the live HF file");
    }

    #[test]
    fn register_download_is_exclusive_until_guard_drops() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let model = WhisperModel::SmallEn; // avoid colliding with other tests' BaseEn
        let g = register_download(model).expect("first registration wins");
        assert!(is_downloading(model));
        assert!(register_download(model).is_none(), "second is rejected");
        drop(g);
        assert!(!is_downloading(model), "drop unregisters");
        assert!(
            register_download(model).is_some(),
            "re-registerable after drop"
        );
    }

    // ---- ModelFile: filename/path/spec/env-override (34–89) ----

    #[test]
    fn filename_and_model_file_in_cover_both_variants() {
        assert_eq!(ModelFile::BaseEn.filename(), "base.en.bin");
        assert_eq!(ModelFile::SmallEn.filename(), "small.en.bin");
        let p = model_file_in(Path::new("/Users/x"), ModelFile::SmallEn);
        assert_eq!(
            p,
            Path::new("/Users/x/Library/Application Support/humhum/models/small.en.bin")
        );
    }

    #[test]
    fn path_is_none_without_home_and_joins_home_when_set() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();
        std::env::remove_var("HOME");
        assert_eq!(ModelFile::BaseEn.path(), None, "no $HOME → no path");

        std::env::set_var("HOME", "/Users/x");
        assert_eq!(
            ModelFile::SmallEn.path(),
            Some(PathBuf::from(
                "/Users/x/Library/Application Support/humhum/models/small.en.bin"
            ))
        );

        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn spec_returns_pinned_constants_without_env_override() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let base = ModelFile::BaseEn.spec();
        assert_eq!(base.size_bytes, 147_964_211);
        assert!(base.url.contains("ggml-base.en.bin"));
        assert!(!base.sha256.is_empty());

        let small = ModelFile::SmallEn.spec();
        assert_eq!(small.size_bytes, 487_614_201);
        assert!(small.url.contains("ggml-small.en.bin"));
        assert!(!small.sha256.is_empty());
    }

    #[test]
    fn spec_prefers_a_fully_set_env_override() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }
        let _u = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://example.test/model.bin");
        let _s = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", "deadbeef");
        let _b = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", "123");

        let spec = ModelFile::BaseEn.spec();
        assert_eq!(spec.url, "http://example.test/model.bin");
        assert_eq!(spec.sha256, "deadbeef");
        assert_eq!(spec.size_bytes, 123);
    }

    #[test]
    fn spec_falls_back_to_pinned_when_override_is_only_partially_set() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }
        // URL + SHA256 set but SIZE_BYTES missing → all-or-nothing override
        // rule falls back to the pinned production spec entirely.
        let _u = EnvVarGuard::set("HUMHUM_MODEL_SMALL_EN_URL", "http://example.test/small.bin");
        let _s = EnvVarGuard::set("HUMHUM_MODEL_SMALL_EN_SHA256", "deadbeef");
        std::env::remove_var("HUMHUM_MODEL_SMALL_EN_SIZE_BYTES");

        let spec = ModelFile::SmallEn.spec();
        assert_eq!(spec.size_bytes, 487_614_201, "fell back to the pinned size");
        assert!(
            spec.url.contains("ggml-small.en.bin"),
            "fell back to the pinned url"
        );
    }

    // ---- ModelError: code()/Display for every variant ----

    #[test]
    fn model_error_code_covers_every_variant() {
        assert_eq!(
            ModelError::InsufficientDisk(1, 2).code(),
            "insufficient_disk"
        );
        assert_eq!(
            ModelError::DownloadFailed("x".into()).code(),
            "download_failed"
        );
        assert_eq!(ModelError::HashMismatch.code(), "hash_mismatch");
        assert_eq!(ModelError::IoError("x".into()).code(), "io_error");
        assert_eq!(ModelError::AlreadyDownloading.code(), "already_downloading");
    }

    #[test]
    fn model_error_display_covers_every_variant() {
        assert_eq!(
            ModelError::InsufficientDisk(10, 5).to_string(),
            "need 10 bytes, have 5"
        );
        assert_eq!(
            ModelError::DownloadFailed("boom".into()).to_string(),
            "download failed: boom"
        );
        assert_eq!(
            ModelError::HashMismatch.to_string(),
            "SHA-256 hash mismatch"
        );
        assert_eq!(
            ModelError::IoError("oops".into()).to_string(),
            "IO error: oops"
        );
        assert_eq!(
            ModelError::AlreadyDownloading.to_string(),
            "a download is already in progress"
        );
    }

    // ---- available_disk (real statvfs on a real path) ----

    #[test]
    fn available_disk_reports_a_positive_value_for_a_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = available_disk(dir.path()).expect("statvfs on a real path succeeds");
        assert!(bytes > 0);
    }

    // ---- ModelManager::new / get_status / DiskModelGate / Default ----

    #[test]
    fn disk_model_gate_agrees_with_get_status_on_real_home() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let gate = DiskModelGate;
        let ready = gate.is_ready(WhisperModel::BaseEn);
        assert_eq!(
            ready,
            model_ready(&ModelManager::new().get_status(WhisperModel::BaseEn))
        );
    }

    #[test]
    fn get_status_reports_downloading_when_registered() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let model = WhisperModel::SmallEn;
        let guard = register_download(model).expect("register");
        assert_eq!(
            ModelManager::new().get_status(model),
            ModelStatus::Downloading(0)
        );
        drop(guard);
    }

    #[test]
    fn model_manager_default_constructs() {
        let _ = ModelManager::default();
        let _ = ModelManager::new();
    }

    // ---- verify_hash (real $HOME + debug env-override spec) ----

    #[test]
    fn verify_hash_ok_leaves_file_and_err_removes_corrupt_file() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());

        let payload = b"whisper model fixture bytes".to_vec();
        let hex = {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            format!("{:x}", hasher.finalize())
        };
        let _url = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://example.test/unused");
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &hex);
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", &payload.len().to_string());

        let final_path = ModelFile::BaseEn.path().unwrap();
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(&final_path, &payload).unwrap();

        assert!(ModelManager::new()
            .verify_hash(WhisperModel::BaseEn)
            .is_ok());
        assert!(final_path.exists(), "a verified file is left in place");

        // Corrupt it in place (still fails via the size or hash check either
        // way, both of which map to `HashMismatch`).
        std::fs::write(&final_path, b"corrupted, will not match").unwrap();
        assert!(matches!(
            ModelManager::new().verify_hash(WhisperModel::BaseEn),
            Err(ModelError::HashMismatch)
        ));
        assert!(!final_path.exists(), "verify_hash deletes a corrupt file");
    }

    // ---- verify_base_model_on_startup: absent / ok / corrupt ----

    #[test]
    fn verify_base_model_on_startup_covers_absent_ok_and_corrupt() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // No logger is installed in the test binary, so `log::max_level()`
        // defaults to `Off` and a multi-line `log::warn!(...)` call's
        // argument-evaluation lines never execute (unlike single-line log
        // calls, which read as "hit" regardless). Raising the level lets the
        // warn-branch's format args actually run so it can be covered.
        log::set_max_level(log::LevelFilter::Trace);
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());

        // 1) No file at all → early return, no panic.
        verify_base_model_on_startup();
        assert_eq!(ModelFile::BaseEn.path().map(|p| p.exists()), Some(false));

        // 2) Present + matches the (overridden) pinned hash → verified in place.
        let payload = b"startup fixture bytes".to_vec();
        let hex = {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            format!("{:x}", hasher.finalize())
        };
        let _url = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://example.test/unused");
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &hex);
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", &payload.len().to_string());

        let final_path = ModelFile::BaseEn.path().unwrap();
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(&final_path, &payload).unwrap();

        verify_base_model_on_startup();
        assert!(final_path.exists(), "a verified file is left in place");

        // 3) Present but corrupt → verify_hash removes it, no panic.
        std::fs::write(&final_path, b"tampered").unwrap();
        verify_base_model_on_startup();
        assert!(!final_path.exists(), "a corrupt file is removed");
    }

    #[test]
    fn verify_base_model_on_startup_recovers_a_valid_orphaned_tmp() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        log::set_max_level(log::LevelFilter::Trace);
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());

        let payload = b"orphaned but complete model bytes".to_vec();
        let hex = {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            format!("{:x}", hasher.finalize())
        };
        let _url = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://example.test/unused");
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &hex);
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", &payload.len().to_string());

        // The interrupted-finalize state: a hash-valid `.tmp`, no final `.bin`.
        let final_path = ModelFile::BaseEn.path().unwrap();
        let tmp_path = tmp_for(&final_path);
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(&tmp_path, &payload).unwrap();

        verify_base_model_on_startup();

        assert!(
            final_path.exists(),
            "startup promotes the stranded valid .tmp"
        );
        assert!(!tmp_path.exists(), "the .tmp is consumed by the promotion");
        assert_eq!(std::fs::read(&final_path).unwrap(), payload);
    }

    #[test]
    fn verify_base_model_on_startup_discards_a_corrupt_orphaned_tmp() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        log::set_max_level(log::LevelFilter::Trace);
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());

        let _url = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://example.test/unused");
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &"0".repeat(64));
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", "8");

        // A `.tmp` that will not match the pinned hash, and no final `.bin`.
        let final_path = ModelFile::BaseEn.path().unwrap();
        let tmp_path = tmp_for(&final_path);
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(&tmp_path, b"badbytes").unwrap();

        verify_base_model_on_startup();

        assert!(
            !tmp_path.exists(),
            "a corrupt orphaned .tmp is cleared for a clean re-download"
        );
        assert!(!final_path.exists(), "no bad model is promoted");
    }

    // ---- ModelManager::download: end-to-end via mockito + env-override ----

    #[tokio::test]
    async fn download_streams_verifies_and_promotes_end_to_end() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let mut server = mockito::Server::new_async().await;
        let payload = vec![5u8; 128];
        let hex = {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            format!("{:x}", hasher.finalize())
        };
        let m = server
            .mock("GET", "/base.bin")
            .with_status(200)
            .with_body(payload.clone())
            .create_async()
            .await;

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());
        let _url = EnvVarGuard::set(
            "HUMHUM_MODEL_BASE_EN_URL",
            &format!("{}/base.bin", server.url()),
        );
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &hex);
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", &payload.len().to_string());

        let emitter = FakeProgressEmitter::default();
        let result = ModelManager::new()
            .download(WhisperModel::BaseEn, &emitter)
            .await;
        assert!(result.is_ok(), "{result:?}");

        let final_path = ModelFile::BaseEn.path().unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), payload);
        assert!(
            !tmp_for(&final_path).exists(),
            "tmp consumed by the rename on success"
        );
        assert_eq!(emitter.0.lock().unwrap().last(), Some(&100));
        assert!(
            !is_downloading(WhisperModel::BaseEn),
            "guard drops after completion"
        );
        m.assert_async().await;
    }

    #[tokio::test]
    async fn download_cleans_up_tmp_on_stream_failure() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        // A 500 makes `stream_and_finalize` fail via `error_for_status`,
        // exercising `download`'s `result.is_err()` `.tmp`-cleanup branch
        // (distinct from the pre-flight `InsufficientDisk` short-circuit,
        // which never reaches this code at all).
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/base.bin")
            .with_status(500)
            .create_async()
            .await;

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());
        let _url = EnvVarGuard::set(
            "HUMHUM_MODEL_BASE_EN_URL",
            &format!("{}/base.bin", server.url()),
        );
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &"0".repeat(64));
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", "64");

        let emitter = FakeProgressEmitter::default();
        let result = ModelManager::new()
            .download(WhisperModel::BaseEn, &emitter)
            .await;
        assert!(
            matches!(result, Err(ModelError::DownloadFailed(_))),
            "{result:?}"
        );

        let final_path = ModelFile::BaseEn.path().unwrap();
        assert!(
            !tmp_for(&final_path).exists(),
            "the cleanup path removes any partial .tmp"
        );
        assert!(
            !is_downloading(WhisperModel::BaseEn),
            "guard drops after the error"
        );
    }

    #[tokio::test]
    async fn download_rejects_concurrent_downloads_of_the_same_model() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _guard = register_download(WhisperModel::BaseEn).expect("register first");
        let emitter = FakeProgressEmitter::default();
        let result = ModelManager::new()
            .download(WhisperModel::BaseEn, &emitter)
            .await;
        assert!(matches!(result, Err(ModelError::AlreadyDownloading)));
    }

    #[tokio::test]
    async fn download_fails_fast_on_insufficient_disk() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }

        let home_dir = tempfile::tempdir().unwrap();
        let _home = EnvVarGuard::set("HOME", home_dir.path().to_str().unwrap());
        let _url = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_URL", "http://127.0.0.1:1/unused");
        let _sha = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SHA256", &"0".repeat(64));
        // Comfortably larger than any real disk, with plenty of headroom
        // below `u64::MAX` so `required_bytes`'s `+ DISK_MARGIN_BYTES` can't
        // overflow (debug builds panic on overflow).
        let _size = EnvVarGuard::set("HUMHUM_MODEL_BASE_EN_SIZE_BYTES", "100000000000000000");

        let emitter = FakeProgressEmitter::default();
        let result = ModelManager::new()
            .download(WhisperModel::BaseEn, &emitter)
            .await;
        assert!(
            matches!(result, Err(ModelError::InsufficientDisk(_, _))),
            "{result:?}"
        );
        assert!(!is_downloading(WhisperModel::BaseEn));
    }

    // `ProgressEmitter for tauri::AppHandle<R>` is generic over the runtime,
    // so `MockRuntime` (via `tauri::test`) exercises the real production impl
    // without a window/webview runtime.
    #[test]
    fn app_handle_emit_progress_does_not_panic() {
        let app = tauri::test::mock_app();
        let handle = app.handle();
        handle.emit_progress(WhisperModel::BaseEn, 0);
        handle.emit_progress(WhisperModel::BaseEn, 100);
    }
}
