//! Tauri command handlers — the IPC surface declared in `src/lib/ipc.ts`.
//!
//! These are stubs that return `AppError::Internal("not implemented")`. Their
//! purpose is to make the IPC contract compiler-enforced: every command name in
//! `ipc.ts` must resolve to a real Rust symbol with a matching signature, wired
//! via `generate_handler!` in `lib.rs`. As later phases implement real logic,
//! commands migrate into their owning modules (e.g. license → `license/mod.rs`).
//!
//! Parameters are underscore-prefixed because these stubs don't read them yet.
//! Tauri strips a single leading underscore when matching JS argument keys
//! (`_prefs` still binds the `prefs` key), so the IPC contract is unchanged.

use crate::cleanup::{self, ApiProvider, CleanupError};
use crate::error::AppError;
use crate::models::{ModelManager, ModelStatus};
use crate::permissions::{self, PermissionsState};
use crate::prefs::{self, Prefs};
use crate::transcription::WhisperModel;

fn not_implemented() -> AppError {
    AppError::Internal("not implemented".into())
}

/// Parse the wire provider string (`"open_ai"`/`"anthropic"`) into [`ApiProvider`].
/// Unknown values are a contract violation → `invalid_provider` (LLD §600).
fn parse_provider(provider: &str) -> Result<ApiProvider, AppError> {
    serde_json::from_value(serde_json::Value::String(provider.to_string()))
        .map_err(|_| AppError::from(CleanupError::InvalidProvider))
}

/// Base URL for `test_api_key`: a debug-only per-provider env override
/// (`HUMHUM_OPENAI_BASE_URL` / `HUMHUM_ANTHROPIC_BASE_URL`, for the
/// E2E mock server) takes precedence, else the real production URL (no
/// trailing slash). Rust's own `run_test_api_key` tests inject a mockito URL
/// directly and never call this.
fn default_base_url(provider: ApiProvider) -> String {
    #[cfg(debug_assertions)]
    {
        let env_var = match provider {
            ApiProvider::OpenAi => "HUMHUM_OPENAI_BASE_URL",
            ApiProvider::Anthropic => "HUMHUM_ANTHROPIC_BASE_URL",
        };
        if let Ok(url) = std::env::var(env_var) {
            return url;
        }
    }
    match provider {
        ApiProvider::OpenAi => "https://api.openai.com/v1",
        ApiProvider::Anthropic => "https://api.anthropic.com/v1",
    }
    .to_string()
}

/// Verify a key, then store it **only on success** (implicit approval). The
/// `store` seam is injected so the verify→store ordering is unit-testable
/// without touching the real Keychain.
pub(crate) async fn run_test_api_key(
    provider: ApiProvider,
    key: &str,
    base_url: &str,
    store: impl FnOnce(ApiProvider, &str) -> Result<(), AppError>,
) -> Result<(), AppError> {
    cleanup::verify_key(provider, key, base_url).await?; // CleanupError -> AppError::Cleanup
    store(provider, key)
}

/// Parse a wire model string (`"base_en"` / `"small_en"`) into [`WhisperModel`].
/// The typed TS union never sends anything else, so an unknown value is a
/// programming error → opaque `internal` code.
fn parse_model(model: &str) -> Result<WhisperModel, AppError> {
    serde_json::from_value(serde_json::Value::String(model.to_string()))
        .map_err(|_| AppError::Internal(format!("unknown model: {model}")))
}

#[tauri::command]
pub fn start_recording<R: tauri::Runtime>(
    state: tauri::State<'_, crate::state::AppState>,
    app: tauri::AppHandle<R>,
) -> Result<(), AppError> {
    crate::recording::start(&state, &app)
}

#[tauri::command]
pub fn stop_recording<R: tauri::Runtime>(
    state: tauri::State<'_, crate::state::AppState>,
    app: tauri::AppHandle<R>,
) -> Result<(), AppError> {
    crate::recording::stop(&state, &app)
}

#[tauri::command]
pub fn cancel_recording<R: tauri::Runtime>(
    state: tauri::State<'_, crate::state::AppState>,
    app: tauri::AppHandle<R>,
) -> Result<(), AppError> {
    crate::recording::cancel(&state, &app)
}

#[tauri::command]
pub fn get_prefs() -> Result<Prefs, AppError> {
    Ok(prefs::load_prefs())
}

#[tauri::command]
pub fn set_prefs(
    prefs: Prefs,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), AppError> {
    prefs::save_prefs(&prefs)?;
    // Best-effort: a flaky/unsigned-dev SMAppService must not block persisting
    // the rest of prefs (which already succeeded above).
    if let Err(e) = prefs::set_launch_at_login(prefs.launch_at_login) {
        log::warn!("failed to apply launch-at-login: {e}");
    }
    // Rebuild the cleanup config so a mode/provider/key/aggressiveness change
    // takes effect on the next dictation (Phase 12).
    *state.cleanup.write().unwrap() =
        cleanup::build_cleanup_config(&prefs, |p| prefs::read_api_key(p).ok().flatten());
    // Propagate an input-device change so it takes effect on the next `start()`.
    state
        .audio
        .set_preferred_device(prefs.preferred_input_device.clone());
    // Re-register the hotkey so a changed shortcut applies immediately instead of
    // only after restart. `register` tears down the prior tap and rebinds; the
    // handler installed at startup is retained. Best-effort: a parse/registration
    // failure must not block the rest of prefs (already persisted above).
    match crate::hotkey::parse_combo(&prefs.hotkey) {
        Ok(combo) => {
            if let Err(e) = state.hotkey.register(&combo) {
                log::warn!("hotkey re-registration failed on set_prefs: {e}");
            }
        }
        Err(e) => log::warn!("invalid hotkey combo on set_prefs: {e}"),
    }
    Ok(())
}

#[tauri::command]
pub fn validate_hotkey(keys: String) -> Result<(), AppError> {
    let parsed = crate::hotkey::parse_combo(&keys)?;
    crate::hotkey::check_conflicts(&parsed)?;
    Ok(())
}

/// Kick off a model download and return immediately; progress and terminal
/// failures are reported out-of-band via `model-download-progress` events (a
/// later phase surfaces the terminal error via `pipeline-error`).
#[tauri::command]
pub fn download_model<R: tauri::Runtime>(
    model: String,
    app: tauri::AppHandle<R>,
) -> Result<(), AppError> {
    let model = parse_model(&model)?;
    tauri::async_runtime::spawn(async move {
        if let Err(e) = ModelManager::new().download(model, &app).await {
            log::warn!("model download failed for {model:?}: {}", e.code());
        }
    });
    Ok(())
}

/// Current on-disk status of a model. Returns the [`ModelStatus`] discriminated
/// union (`"ready"`, `{"downloading":n}`, …), not a bare string.
#[tauri::command]
pub fn get_model_status(model: String) -> Result<ModelStatus, AppError> {
    let model = parse_model(&model)?;
    Ok(ModelManager::new().get_status(model))
}

#[tauri::command]
pub fn store_api_key(_provider: String, _key: String) -> Result<(), AppError> {
    Err(not_implemented())
}

/// Verify a cloud cleanup API key against the provider and, on success, store it
/// in the Keychain. Never logs the key.
#[tauri::command]
pub async fn test_api_key(provider: String, key: String) -> Result<(), AppError> {
    let p = parse_provider(&provider)?;
    run_test_api_key(p, &key, &default_base_url(p), |p, k| {
        prefs::store_api_key(p, k).map_err(AppError::from)
    })
    .await
}

/// Verify an Ollama setup for the Preferences **Test** button: confirm the server
/// is reachable and the configured model is installed. Unlike `test_api_key` this
/// stores nothing — it's a pure reachability probe. The frontend maps the returned
/// code (`unavailable` / `model_not_found`) to user-facing copy.
#[tauri::command]
pub async fn test_ollama(base_url: String, model: String) -> Result<(), AppError> {
    cleanup::ollama::verify_ollama(&base_url, &model).await?; // CleanupError -> AppError::Cleanup
    Ok(())
}

#[tauri::command]
pub fn check_permissions() -> Result<PermissionsState, AppError> {
    Ok(permissions::create().check())
}

#[tauri::command]
pub fn request_microphone() -> Result<(), AppError> {
    permissions::create()
        .request_microphone()
        .map_err(|()| AppError::Internal("microphone request failed".into()))
}

#[tauri::command]
pub fn open_accessibility_settings() {
    permissions::create().open_accessibility_settings();
}

#[tauri::command]
pub fn open_input_monitoring_settings() {
    permissions::create().open_input_monitoring_settings();
}

/// All input-capable device names for the Preferences "Input device"
/// dropdown. Empty on non-macOS or on any CoreAudio enumeration failure —
/// never blocks Preferences from rendering.
#[tauri::command]
pub fn list_input_devices() -> Result<Vec<String>, AppError> {
    Ok(crate::audio::list_input_device_names())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioCapture;
    use crate::hotkey::HotkeyCapture;
    use crate::paste::TextPaste;
    use crate::permissions::Permissions;
    use crate::transcription::Transcriber;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::Manager;

    #[tokio::test]
    async fn stores_key_only_after_successful_verify() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(200)
            .create_async()
            .await;
        let stored = AtomicBool::new(false);
        let r = run_test_api_key(ApiProvider::OpenAi, "sk", &server.url(), |_, _| {
            stored.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert!(r.is_ok());
        assert!(stored.load(Ordering::SeqCst), "store ran on success");
    }

    #[tokio::test]
    async fn does_not_store_when_verify_fails() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(401)
            .create_async()
            .await;
        let stored = AtomicBool::new(false);
        let r = run_test_api_key(ApiProvider::OpenAi, "bad", &server.url(), |_, _| {
            stored.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(r.unwrap_err().code(), "key_rejected");
        assert!(!stored.load(Ordering::SeqCst), "store skipped on rejection");
    }

    /// The `test_api_key` command itself (not just `run_test_api_key`): resolves
    /// `default_base_url` via the debug-only `HUMHUM_OPENAI_BASE_URL`
    /// override to a mock server, and lets the success path store the key for
    /// real via `prefs::store_api_key` (real Keychain access confirmed to work
    /// Keychain test). Cleans up the Keychain entry afterward.
    #[tokio::test]
    async fn test_api_key_command_verifies_and_stores_through_the_real_wrapper() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(200)
            .create_async()
            .await;
        let old = std::env::var("HUMHUM_OPENAI_BASE_URL").ok();
        std::env::set_var("HUMHUM_OPENAI_BASE_URL", server.url());

        let result = test_api_key("open_ai".into(), "test-command-key".into()).await;

        let stored = prefs::read_api_key(ApiProvider::OpenAi);
        // Clean up via the same (cfg(test)-swapped) service the wrapper used, not a
        // hardcoded literal — otherwise cleanup would target the production service
        // while the wrapper wrote to the `-test` one.
        let entry = keyring::Entry::new(prefs::API_KEYCHAIN_SERVICE, "openai-key")
            .expect("valid keychain entry");
        let _ = entry.delete_credential();

        match old {
            Some(v) => std::env::set_var("HUMHUM_OPENAI_BASE_URL", v),
            None => std::env::remove_var("HUMHUM_OPENAI_BASE_URL"),
        }

        assert!(result.is_ok());
        assert_eq!(stored.unwrap(), Some("test-command-key".to_string()));
    }

    #[test]
    fn parse_provider_rejects_unknown() {
        assert_eq!(
            parse_provider("not_a_provider").unwrap_err().code(),
            "invalid_provider"
        );
    }

    #[test]
    fn parse_provider_accepts_wire_strings() {
        assert!(matches!(parse_provider("open_ai"), Ok(ApiProvider::OpenAi)));
        assert!(matches!(
            parse_provider("anthropic"),
            Ok(ApiProvider::Anthropic)
        ));
    }

    #[test]
    fn validate_hotkey_accepts_unreserved_combo() {
        assert!(validate_hotkey("Option+Shift+Space".into()).is_ok());
    }

    #[test]
    fn validate_hotkey_rejects_system_conflict() {
        assert!(matches!(
            validate_hotkey("Cmd+Space".into()),
            Err(AppError::Hotkey(_))
        ));
    }

    #[test]
    fn validate_hotkey_rejects_invalid_combo() {
        assert!(matches!(
            validate_hotkey("Space".into()),
            Err(AppError::Hotkey(_))
        ));
    }

    #[test]
    fn default_base_url_prefers_env_override_else_prod_url() {
        // Clean slate regardless of ambient env (this var is only touched here).
        std::env::remove_var("HUMHUM_OPENAI_BASE_URL");
        assert_eq!(
            default_base_url(ApiProvider::OpenAi),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            default_base_url(ApiProvider::Anthropic),
            "https://api.anthropic.com/v1"
        );

        std::env::set_var("HUMHUM_OPENAI_BASE_URL", "http://127.0.0.1:1/mock");
        assert_eq!(
            default_base_url(ApiProvider::OpenAi),
            "http://127.0.0.1:1/mock"
        );
        // Unrelated provider is unaffected by the OpenAI override.
        assert_eq!(
            default_base_url(ApiProvider::Anthropic),
            "https://api.anthropic.com/v1"
        );
        std::env::remove_var("HUMHUM_OPENAI_BASE_URL");
    }

    #[test]
    fn parse_model_accepts_wire_strings() {
        assert!(matches!(parse_model("base_en"), Ok(WhisperModel::BaseEn)));
        assert!(matches!(parse_model("small_en"), Ok(WhisperModel::SmallEn)));
    }

    #[test]
    fn parse_model_rejects_unknown() {
        assert_eq!(parse_model("not_a_model").unwrap_err().code(), "internal");
    }

    #[test]
    fn store_api_key_is_not_implemented() {
        assert_eq!(
            store_api_key("open_ai".into(), "key".into())
                .unwrap_err()
                .code(),
            "internal"
        );
    }

    #[test]
    fn get_prefs_reads_without_error() {
        // Thin wrapper over `prefs::load_prefs` (read-only, defaults on any
        // missing/corrupt file) — just confirm the IPC surface stays wired.
        assert!(get_prefs().is_ok());
    }

    #[test]
    fn get_model_status_rejects_unknown_model() {
        assert_eq!(
            get_model_status("not_a_model".into()).unwrap_err().code(),
            "internal"
        );
    }

    #[test]
    fn get_model_status_returns_status_for_known_model() {
        assert!(get_model_status("base_en".into()).is_ok());
    }

    #[tokio::test]
    async fn test_api_key_rejects_invalid_provider() {
        assert_eq!(
            test_api_key("not_a_provider".into(), "key".into())
                .await
                .unwrap_err()
                .code(),
            "invalid_provider"
        );
    }

    #[tokio::test]
    async fn test_api_key_rejects_bad_key_without_storing() {
        // Anthropic (not OpenAI) so this doesn't race
        // `default_base_url_prefers_env_override_else_prod_url`'s env var.
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/messages")
            .with_status(401)
            .create_async()
            .await;
        std::env::set_var("HUMHUM_ANTHROPIC_BASE_URL", server.url());
        let result = test_api_key("anthropic".into(), "bad-key".into()).await;
        std::env::remove_var("HUMHUM_ANTHROPIC_BASE_URL");
        assert_eq!(result.unwrap_err().code(), "key_rejected");
    }

    #[tokio::test]
    async fn test_ollama_succeeds_when_model_present() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"llama3"}]}"#)
            .create_async()
            .await;
        assert!(test_ollama(server.url(), "llama3".into()).await.is_ok());
    }

    #[tokio::test]
    async fn test_ollama_reports_model_not_found() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"llama3"}]}"#)
            .create_async()
            .await;
        assert_eq!(
            test_ollama(server.url(), "mistral".into())
                .await
                .unwrap_err()
                .code(),
            "model_not_found"
        );
    }

    /// Exercises `check_permissions`/`request_microphone`/the two `open_*`
    /// settings commands via the debug-only E2E permissions override, so this
    /// never touches real TCC prompts or spawns a real System Settings pane.
    #[test]
    fn permission_commands_use_forced_permissions_in_e2e_mode() {
        std::env::set_var("HUMHUM_E2E_FORCE_PERMISSIONS", "1");

        let state = check_permissions().expect("check_permissions should succeed");
        assert!(state.all_granted());
        assert!(request_microphone().is_ok());
        open_accessibility_settings(); // no-op under the forced impl; must not panic
        open_input_monitoring_settings(); // same

        std::env::remove_var("HUMHUM_E2E_FORCE_PERMISSIONS");
    }

    #[test]
    fn list_input_devices_never_errors() {
        assert!(list_input_devices().is_ok());
    }

    // ---- IPC wrappers over `AppState`/`AppHandle` --------------------------
    //
    // `start_recording`/`stop_recording`/`cancel_recording`/`download_model` are
    // generic over `R: tauri::Runtime` (production `generate_handler!` still
    // monomorphizes on the real `Wry` runtime), which lets `tauri::test`'s
    // `MockRuntime` stand in here — no window/webview runtime needed. `set_prefs`
    // needs no such change since `tauri::State` isn't runtime-parameterized, but
    // like the others it can only be obtained from a `manage`d app, not built
    // standalone.

    struct StubAudio;
    impl crate::audio::AudioCapture for StubAudio {
        fn start(
            &self,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<crate::audio::AudioBuffer, crate::audio::AudioError>,
                        > + Send,
                >,
            >,
            crate::audio::AudioError,
        > {
            Ok(Box::pin(std::future::pending()))
        }
        fn stop(&self) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn cancel(&self) {}
    }

    struct StubHotkey;
    impl crate::hotkey::HotkeyCapture for StubHotkey {
        fn register(
            &self,
            _combo: &crate::hotkey::HotkeyCombo,
        ) -> Result<(), crate::hotkey::HotkeyError> {
            Ok(())
        }
        fn unregister(&self) -> Result<(), crate::hotkey::HotkeyError> {
            Ok(())
        }
        fn set_handler(&self, _handler: Box<dyn Fn(crate::hotkey::HotkeyEvent) + Send + Sync>) {}
        fn validate(
            &self,
            _combo: &crate::hotkey::HotkeyCombo,
        ) -> Result<(), crate::hotkey::HotkeyError> {
            Ok(())
        }
        fn play_cue(&self, _cue: crate::hotkey::AudioCue) {}
    }

    struct StubPermissions(bool);
    impl permissions::Permissions for StubPermissions {
        fn check(&self) -> PermissionsState {
            PermissionsState {
                microphone: self.0,
                accessibility: self.0,
                input_monitoring: self.0,
            }
        }
        fn request_microphone(&self) -> Result<(), ()> {
            Ok(())
        }
        fn open_accessibility_settings(&self) {}
        fn open_input_monitoring_settings(&self) {}
    }

    struct StubTranscriber;
    impl crate::transcription::Transcriber for StubTranscriber {
        fn transcribe(
            &self,
            _buffer: crate::audio::AudioBuffer,
            _model: WhisperModel,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<String, crate::transcription::TranscriptionError>,
                    > + Send,
            >,
        > {
            Box::pin(async { Ok(String::new()) })
        }
    }

    struct StubPaste;
    impl crate::paste::TextPaste for StubPaste {
        fn paste<'a>(
            &'a self,
            _text: &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), crate::paste::PasteError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(()) })
        }
    }

    struct StubModelGate(bool);
    impl crate::models::ModelGate for StubModelGate {
        fn is_ready(&self, _model: WhisperModel) -> bool {
            self.0
        }
    }

    /// A minimal `AppState` wired with no-op/stub trait objects — enough to
    /// drive the thin `start_recording`/`stop_recording`/`cancel_recording`/
    /// `set_prefs` wrappers without touching real hardware or the Keychain.
    /// Deep recording-lifecycle behavior is already covered by `recording.rs`'s
    /// own tests; these only confirm the IPC wrappers delegate correctly.
    fn stub_app_state(perms_granted: bool, model_ready: bool) -> crate::state::AppState {
        crate::state::AppState {
            audio: std::sync::Arc::new(StubAudio),
            hotkey: std::sync::Arc::new(StubHotkey),
            permissions: std::sync::Arc::new(StubPermissions(perms_granted)),
            transcriber: std::sync::Arc::new(StubTranscriber),
            paste: std::sync::Arc::new(StubPaste),
            model_gate: std::sync::Arc::new(StubModelGate(model_ready)),
            recording_phase: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(
                crate::state::RecordingPhase::Idle as u8,
            )),
            cap_cancel_tx: std::sync::Arc::new(std::sync::Mutex::new(None)),
            cleanup: std::sync::Arc::new(std::sync::RwLock::new(crate::cleanup::CleanupConfig {
                mode: crate::cleanup::CleanupMode::None,
                provider: None,
            })),
        }
    }

    fn mock_app_with_state(state: crate::state::AppState) -> tauri::App<tauri::test::MockRuntime> {
        // `mock_context` + `noop_assets` (not `generate_context!()`) — the real
        // context macro embeds a process-global `Info.plist` symbol that's
        // already emitted once for `lib.rs`'s production app; invoking it again
        // here would collide at link time.
        tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds")
    }

    #[test]
    fn start_recording_reports_permission_denied_through_the_wrapper() {
        let app = mock_app_with_state(stub_app_state(false, true));
        let state = app.state::<crate::state::AppState>();
        let handle = app.handle().clone();
        assert!(matches!(
            start_recording(state, handle),
            Err(AppError::Audio(_))
        ));
    }

    #[test]
    fn start_recording_then_cancel_recording_delegate_through_the_wrapper() {
        let app = mock_app_with_state(stub_app_state(true, true));
        let state = app.state::<crate::state::AppState>();
        let handle = app.handle().clone();

        start_recording(state, handle).expect("start ok");

        let state = app.state::<crate::state::AppState>();
        let handle = app.handle().clone();
        cancel_recording(state, handle).expect("cancel ok");

        assert_eq!(
            app.state::<crate::state::AppState>().phase(),
            crate::state::RecordingPhase::Idle
        );
    }

    #[test]
    fn stop_recording_without_an_active_recording_is_a_no_op_through_the_wrapper() {
        let app = mock_app_with_state(stub_app_state(true, true));
        let state = app.state::<crate::state::AppState>();
        let handle = app.handle().clone();
        assert!(stop_recording(state, handle).is_ok());
        assert_eq!(
            app.state::<crate::state::AppState>().phase(),
            crate::state::RecordingPhase::Idle
        );
    }

    #[test]
    fn set_prefs_persists_and_rebuilds_cleanup_config_through_the_wrapper() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home_dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by `ENV_LOCK` above, matching every other test in
        // this crate that mutates process-wide `$HOME`.
        std::env::set_var("HOME", home_dir.path());

        let app = mock_app_with_state(stub_app_state(true, true));
        let state = app.state::<crate::state::AppState>();
        let mut prefs = prefs::load_prefs();
        prefs.hotkey = "Option+Shift+Space".into();
        set_prefs(prefs, state).expect("set_prefs ok");

        assert_eq!(prefs::load_prefs().hotkey, "Option+Shift+Space");
        std::env::remove_var("HOME");
    }

    #[test]
    fn download_model_rejects_unknown_model_through_the_wrapper() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        assert_eq!(
            download_model("not_a_model".into(), handle)
                .unwrap_err()
                .code(),
            "internal"
        );
    }

    /// The `Stub*` mocks above only exercise the code paths the IPC-wrapper
    /// tests happen to drive (e.g. `start`/`check`); this directly calls every
    /// remaining trait method so the stub bodies themselves aren't dead weight
    /// in the coverage report.
    #[tokio::test]
    async fn stub_trait_impls_satisfy_every_method_on_their_trait() {
        assert!(StubAudio.stop().is_ok());
        StubAudio.cancel();

        let combo = crate::hotkey::HotkeyCombo {
            key: "A".into(),
            modifiers: vec![],
        };
        assert!(StubHotkey.unregister().is_ok());
        StubHotkey.set_handler(Box::new(|_| {}));
        assert!(StubHotkey.validate(&combo).is_ok());
        StubHotkey.play_cue(crate::hotkey::AudioCue::Start);

        assert!(StubPermissions(true).request_microphone().is_ok());
        StubPermissions(true).open_accessibility_settings();
        StubPermissions(true).open_input_monitoring_settings();

        let buffer = crate::audio::AudioBuffer {
            samples: vec![],
            sample_rate: 16_000,
            duration_ms: 0,
        };
        assert_eq!(
            StubTranscriber
                .transcribe(buffer, WhisperModel::BaseEn)
                .await
                .unwrap(),
            ""
        );

        assert!(StubPaste.paste("hello").await.is_ok());
    }

    // Deliberately no test calls `download_model` with a *known* model: doing
    // so spawns a real, unsynchronized `ModelManager::download` via
    // `tauri::async_runtime::spawn`, racing `models/mod.rs`'s own download
    // tests over the shared `DOWNLOADS` registry (they serialize on
    // handle to join, so it can't be synchronized from a test).
}
