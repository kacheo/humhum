use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};

use crate::cleanup::{ApiProvider, CleanupAggressiveness, CleanupMode};
use crate::transcription::WhisperModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub hotkey: String,
    pub cleanup_mode: CleanupMode,
    pub cleanup_aggressiveness: CleanupAggressiveness,
    pub api_provider: ApiProvider,
    pub ollama_model: String,
    pub ollama_base_url: String,
    pub whisper_model: WhisperModel,
    pub launch_at_login: bool,
    pub onboarding_complete: bool,
    /// Recording input device: `None` = auto (avoid a Bluetooth default input
    /// when a non-Bluetooth alternative exists — the default fix behavior);
    /// `Some("system-default")` = always use whatever macOS reports as
    /// default (pre-fix behavior); `Some(name)` = explicit user override.
    /// `#[serde(default)]` so an upgrading user's settings.json — which
    /// predates this field — still parses instead of resetting every pref to
    /// default (see `load_from_partial_schema_returns_defaults`).
    #[serde(default)]
    pub preferred_input_device: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            hotkey: "Option+Shift+Space".into(),
            cleanup_mode: CleanupMode::None,
            cleanup_aggressiveness: CleanupAggressiveness::Standard,
            api_provider: ApiProvider::OpenAi,
            ollama_model: "llama3".into(),
            ollama_base_url: "http://localhost:11434/v1/".into(),
            whisper_model: WhisperModel::BaseEn,
            launch_at_login: false,
            onboarding_complete: false,
            preferred_input_device: None,
        }
    }
}

/// Prefs save error. The `String` variants are a deliberate simplification of
/// LLD §362's `io::Error`/`serde_json::Error` — the message is what survives at
/// the `AppError` boundary anyway (errors are stringified there).
#[derive(Debug)]
pub enum PrefsError {
    IoError(String),
    SerializeError(String),
    KeychainError(String),
}

impl PrefsError {
    /// Stable snake_case wire code (LLD §597).
    pub fn code(&self) -> String {
        match self {
            PrefsError::IoError(_) => "io_error",
            PrefsError::SerializeError(_) => "serialize_error",
            PrefsError::KeychainError(_) => "keychain_error",
        }
        .to_string()
    }
}

impl std::fmt::Display for PrefsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrefsError::IoError(e) => write!(f, "IO error: {e}"),
            PrefsError::SerializeError(e) => write!(f, "serialize error: {e}"),
            PrefsError::KeychainError(e) => write!(f, "keychain error: {e}"),
        }
    }
}

/// Keychain service for user-provided cloud cleanup API keys.
///
/// Under `cfg(test)` this uses a `-test` service so the real-Keychain unit tests
/// operate on an isolated namespace: they never collide with items the shipping
/// app created (which would trigger a macOS Keychain ACL `Password:` prompt on
/// the interactive CI runner, since each ad-hoc-signed test build is a different
/// code identity), and never overwrite/delete the user's real stored API keys.
/// `pub(crate)` so `commands.rs`'s real-wrapper test can clean up via the same
/// (cfg-swapped) service instead of a hardcoded literal.
#[cfg(not(test))]
pub(crate) const API_KEYCHAIN_SERVICE: &str = "wipzi-api";
#[cfg(test)]
pub(crate) const API_KEYCHAIN_SERVICE: &str = "wipzi-api-test";

/// Keychain account name per provider. Stable strings — changing them orphans
/// any previously stored key.
fn api_account(provider: ApiProvider) -> &'static str {
    match provider {
        ApiProvider::OpenAi => "openai-key",
        ApiProvider::Anthropic => "anthropic-key",
    }
}

/// Persist a cloud cleanup API key in the macOS Keychain. Never logs the key.
pub fn store_api_key(provider: ApiProvider, key: &str) -> Result<(), PrefsError> {
    let entry = Entry::new(API_KEYCHAIN_SERVICE, api_account(provider))
        .map_err(|e| PrefsError::KeychainError(e.to_string()))?;
    entry
        .set_password(key)
        .map_err(|e| PrefsError::KeychainError(e.to_string()))
}

/// Read a stored cloud cleanup API key, or `Ok(None)` if none is stored.
pub fn read_api_key(provider: ApiProvider) -> Result<Option<String>, PrefsError> {
    let entry = Entry::new(API_KEYCHAIN_SERVICE, api_account(provider))
        .map_err(|e| PrefsError::KeychainError(e.to_string()))?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(PrefsError::KeychainError(e.to_string())),
    }
}

#[cfg(target_os = "macos")]
mod macos;

/// Apply the launch-at-login OS setting. macOS uses `SMAppService`; other
/// platforms are a no-op (the app ships macOS-only).
#[cfg(target_os = "macos")]
pub fn set_launch_at_login(enabled: bool) -> Result<(), PrefsError> {
    macos::set_launch_at_login(enabled)
}

#[cfg(not(target_os = "macos"))]
pub fn set_launch_at_login(_enabled: bool) -> Result<(), PrefsError> {
    Ok(())
}

pub fn default_prefs() -> Prefs {
    Prefs::default()
}

/// The settings.json path under a given home directory (pure — testable without
/// touching the process environment).
fn settings_file(home: &std::path::Path) -> std::path::PathBuf {
    home.join("Library/Application Support/wipzi/settings.json")
}

/// `~/Library/Application Support/wipzi/settings.json`, or `None` if
/// `$HOME` is unset.
fn settings_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(settings_file(std::path::Path::new(&home)))
}

/// Read prefs from `path`, returning [`default_prefs`] on any error (missing,
/// unreadable, or corrupt) — prefs failure must never block the app (LLD §389).
fn load_from(path: &std::path::Path) -> Prefs {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|_| default_prefs()),
        Err(_) => default_prefs(),
    }
}

/// Atomically write prefs to `path`: serialize, write `{path}.tmp`, then
/// `rename` over the target (atomic on POSIX, LLD §388). We deliberately do not
/// `fsync` — for a low-frequency prefs file on APFS the rename's atomicity is
/// sufficient and the durability window is acceptable.
fn save_to(path: &std::path::Path, prefs: &Prefs) -> Result<(), PrefsError> {
    let json = serde_json::to_string_pretty(prefs)
        .map_err(|e| PrefsError::SerializeError(e.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PrefsError::IoError(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| PrefsError::IoError(e.to_string()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp); // don't leave a stale tmp behind
        return Err(PrefsError::IoError(e.to_string()));
    }
    Ok(())
}

pub fn load_prefs() -> Prefs {
    match settings_path() {
        Some(path) => load_from(&path),
        None => default_prefs(),
    }
}

pub fn save_prefs(prefs: &Prefs) -> Result<(), PrefsError> {
    let path = settings_path().ok_or_else(|| PrefsError::IoError("$HOME is not set".into()))?;
    save_to(&path, prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that mutate the shared process `$HOME` env var (read
    /// by `settings_path`) — it's process-wide, so concurrent `cargo test`
    /// threads touching it would race. Mirrors `models::tests::ENV_LOCK`'s
    /// established pattern in this crate.
    #[test]
    fn keychain_error_code_is_stable() {
        assert_eq!(
            PrefsError::KeychainError("denied".into()).code(),
            "keychain_error"
        );
    }

    /// Real macOS Keychain round trip for both providers' API keys (confirmed
    /// Keychain test). No `clear_api_key` exists, so cleanup deletes the
    /// `keyring::Entry` directly.
    #[test]
    fn store_and_read_api_key_round_trips_through_the_real_keychain() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        for provider in [ApiProvider::OpenAi, ApiProvider::Anthropic] {
            store_api_key(provider, "test-api-key-value").unwrap();
            assert_eq!(
                read_api_key(provider).unwrap(),
                Some("test-api-key-value".to_string())
            );

            let entry = Entry::new(API_KEYCHAIN_SERVICE, api_account(provider)).unwrap();
            match entry.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => {}
                Err(e) => panic!("cleanup failed: {e}"),
            }
            assert_eq!(read_api_key(provider).unwrap(), None);
        }
    }

    #[test]
    fn prefs_error_display_messages_for_every_variant() {
        assert_eq!(
            PrefsError::IoError("boom".into()).to_string(),
            "IO error: boom"
        );
        assert_eq!(
            PrefsError::SerializeError("boom".into()).to_string(),
            "serialize error: boom"
        );
        assert_eq!(
            PrefsError::KeychainError("boom".into()).to_string(),
            "keychain error: boom"
        );
    }

    /// Delegates straight to `macos::set_launch_at_login` (FFI, exercised
    /// directly and tolerantly in `prefs::macos::tests::
    /// set_launch_at_login_does_not_panic` — that test's own comment notes
    /// registration typically fails in an unbundled test binary). This just
    /// covers the pass-through wrapper itself.
    #[test]
    fn set_launch_at_login_wrapper_does_not_panic() {
        let _ = set_launch_at_login(false);
    }

    /// Without `$HOME`, `settings_path` is `None`; with it set, the path joins
    /// as expected. Locks `ENV_LOCK` for the whole mutation window.
    #[test]
    fn settings_path_is_none_without_home_and_some_with_it() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();

        std::env::remove_var("HOME");
        assert_eq!(settings_path(), None, "no $HOME -> no path");

        std::env::set_var("HOME", "/Users/x");
        assert_eq!(
            settings_path(),
            Some(std::path::PathBuf::from(
                "/Users/x/Library/Application Support/wipzi/settings.json"
            ))
        );

        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    /// `load_prefs`'s `None` arm (no `$HOME`) falls back to defaults.
    #[test]
    fn load_prefs_falls_back_to_defaults_without_home() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();
        std::env::remove_var("HOME");

        let prefs = load_prefs();
        assert_eq!(prefs.hotkey, Prefs::default().hotkey);

        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    /// `save_prefs` surfaces a clear `IoError` when `$HOME` is unset, rather
    /// than panicking or silently no-oping.
    #[test]
    fn save_prefs_errors_without_home() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();
        std::env::remove_var("HOME");

        let err = save_prefs(&Prefs::default()).expect_err("no $HOME -> error");
        assert!(matches!(err, PrefsError::IoError(_)));
        assert_eq!(err.code(), "io_error");

        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    /// With `$HOME` pointed at a scratch dir, the public `save_prefs`/
    /// `load_prefs` wrappers round-trip through the real settings-file path
    /// resolution (never touching the developer's actual prefs file).
    #[test]
    fn save_prefs_then_load_prefs_round_trip_with_home_override() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());

        let prefs = Prefs {
            hotkey: "Cmd+Opt+P".into(),
            launch_at_login: true,
            ..Prefs::default()
        };
        save_prefs(&prefs).expect("save succeeds with $HOME set");
        let back = load_prefs();
        assert_eq!(back.hotkey, "Cmd+Opt+P");
        assert!(back.launch_at_login);

        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    /// `create_dir_all` fails when a path component along the way is a
    /// regular file rather than a directory.
    #[test]
    fn save_to_errors_when_parent_path_component_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("child").join("settings.json");

        let err = save_to(&path, &Prefs::default()).expect_err("blocked parent dir");
        assert!(matches!(err, PrefsError::IoError(_)));
    }

    /// The root path has no parent (`Path::parent` returns `None`), so the
    /// `create_dir_all` step is skipped entirely; the subsequent write then
    /// fails since `/` isn't a writable regular file.
    #[test]
    fn save_to_root_path_skips_parent_creation_and_fails_to_write() {
        let err = save_to(std::path::Path::new("/"), &Prefs::default())
            .expect_err("can't write a file at the filesystem root");
        assert!(matches!(err, PrefsError::IoError(_)));
    }

    /// `rename` fails when the destination is an existing directory (can't
    /// rename a file over a dir); the impl must surface the error and clean
    /// up the `.tmp` sibling rather than leaving it behind.
    #[test]
    fn save_to_errors_and_cleans_up_tmp_when_rename_target_is_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("target_dir");
        std::fs::create_dir_all(&target_dir).unwrap();
        let tmp_sibling = target_dir.with_extension("json.tmp");

        let err = save_to(&target_dir, &Prefs::default()).expect_err("target is a directory");
        assert!(matches!(err, PrefsError::IoError(_)));
        assert!(
            !tmp_sibling.exists(),
            "tmp sibling cleaned up on rename failure"
        );
        assert!(target_dir.is_dir(), "target directory left untouched");
    }

    #[test]
    fn api_account_names_are_distinct_and_stable() {
        assert_eq!(api_account(ApiProvider::OpenAi), "openai-key");
        assert_eq!(api_account(ApiProvider::Anthropic), "anthropic-key");
    }

    #[test]
    fn save_to_then_load_from_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let prefs = Prefs {
            hotkey: "Cmd+Shift+D".into(),
            whisper_model: WhisperModel::SmallEn,
            launch_at_login: true,
            ..Prefs::default()
        };

        save_to(&path, &prefs).expect("save");
        let back = load_from(&path);

        assert_eq!(back.hotkey, "Cmd+Shift+D");
        assert_eq!(back.whisper_model, WhisperModel::SmallEn);
        assert!(back.launch_at_login);
        assert_eq!(back.cleanup_mode, prefs.cleanup_mode);
    }

    #[test]
    fn load_from_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = load_from(&dir.path().join("does-not-exist.json"));
        assert_eq!(prefs.hotkey, Prefs::default().hotkey);
        assert_eq!(prefs.whisper_model, WhisperModel::BaseEn);
    }

    #[test]
    fn load_from_corrupt_json_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ this is not valid json ]").unwrap();
        let prefs = load_from(&path);
        assert_eq!(prefs.api_provider, Prefs::default().api_provider);
    }

    #[test]
    fn save_to_leaves_no_tmp_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save_to(&path, &Prefs::default()).expect("save");
        assert!(path.exists(), "final file written");
        // Independently scan the dir — no `.tmp` file should remain, regardless
        // of how the impl derives the tmp name.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp left behind: {leftovers:?}");
        // Final file re-parses.
        let _: Prefs = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    }

    #[test]
    fn save_to_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save_to(&path, &Prefs::default()).expect("first save");
        let updated = Prefs {
            hotkey: "Ctrl+Alt+K".into(),
            ..Prefs::default()
        };
        save_to(&path, &updated).expect("overwrite");
        assert_eq!(load_from(&path).hotkey, "Ctrl+Alt+K");
    }

    #[test]
    fn settings_file_has_expected_suffix() {
        let p = settings_file(std::path::Path::new("/Users/x"));
        assert_eq!(
            p,
            std::path::Path::new("/Users/x/Library/Application Support/wipzi/settings.json")
        );
    }

    #[test]
    fn load_from_partial_schema_returns_defaults() {
        // Prefs has no #[serde(default)]; a file missing required fields is
        // treated as corrupt -> defaults (not a partial merge).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"hotkey":"Cmd+X"}"#).unwrap();
        assert_eq!(load_from(&path).hotkey, Prefs::default().hotkey);
    }

    /// Defaults must survive a serialize -> deserialize round-trip unchanged.
    #[test]
    fn default_prefs_round_trip() {
        let prefs = Prefs::default();
        let json = serde_json::to_string(&prefs).expect("serialize");
        let back: Prefs = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.cleanup_mode, prefs.cleanup_mode);
        assert_eq!(back.cleanup_aggressiveness, prefs.cleanup_aggressiveness);
        assert_eq!(back.api_provider, prefs.api_provider);
        assert_eq!(back.whisper_model, prefs.whisper_model);
        assert_eq!(back.hotkey, prefs.hotkey);
    }

    /// Locks the wire string for every non-default enum variant. The canonical
    /// schema test below only covers the defaults; these assertions guard the
    /// remaining variants so a future rename can't silently break the TS unions
    /// or the model-download IPC strings (`small_en`, `anthropic`).
    #[test]
    fn non_default_variants_serialize_to_expected_strings() {
        use serde_json::json;
        assert_eq!(
            serde_json::to_value(CleanupMode::Api).unwrap(),
            json!("api")
        );
        assert_eq!(
            serde_json::to_value(CleanupMode::Local).unwrap(),
            json!("local")
        );
        assert_eq!(
            serde_json::to_value(CleanupAggressiveness::Light).unwrap(),
            json!("light")
        );
        assert_eq!(
            serde_json::to_value(CleanupAggressiveness::Heavy).unwrap(),
            json!("heavy")
        );
        assert_eq!(
            serde_json::to_value(ApiProvider::Anthropic).unwrap(),
            json!("anthropic")
        );
        assert_eq!(
            serde_json::to_value(WhisperModel::SmallEn).unwrap(),
            json!("small_en")
        );
    }

    /// Locks the serialized strings to the canonical settings.json schema
    /// (LLD §6) and the TS prefs contract. If serde stops emitting these exact
    /// strings, this test fails.
    #[test]
    fn canonical_settings_json_maps_to_enum_variants() {
        let json = r#"{
            "hotkey": "Option+Shift+Space",
            "cleanupMode": "none",
            "cleanupAggressiveness": "standard",
            "apiProvider": "open_ai",
            "ollamaModel": "llama3",
            "ollamaBaseUrl": "http://localhost:11434/v1/",
            "whisperModel": "base_en",
            "launchAtLogin": false,
            "onboardingComplete": false
        }"#;
        let prefs: Prefs = serde_json::from_str(json).expect("deserialize canonical schema");
        assert_eq!(prefs.cleanup_mode, CleanupMode::None);
        assert_eq!(
            prefs.cleanup_aggressiveness,
            CleanupAggressiveness::Standard
        );
        assert_eq!(prefs.api_provider, ApiProvider::OpenAi);
        assert_eq!(prefs.whisper_model, WhisperModel::BaseEn);

        // Default serialization emits exactly these strings.
        let value = serde_json::to_value(Prefs::default()).expect("serialize");
        assert_eq!(value["cleanupMode"], "none");
        assert_eq!(value["cleanupAggressiveness"], "standard");
        assert_eq!(value["apiProvider"], "open_ai");
        assert_eq!(value["whisperModel"], "base_en");
    }

    /// A settings.json predating `preferredInputDevice` (every other current
    /// field present) must still parse, with the new field defaulting to
    /// `None` — not reset to full defaults. Directly exercises the
    /// `#[serde(default)]` requirement, not just trusting it.
    #[test]
    fn missing_preferred_input_device_field_parses_as_none() {
        let json = r#"{
            "hotkey": "Cmd+Shift+D",
            "cleanupMode": "none",
            "cleanupAggressiveness": "standard",
            "apiProvider": "open_ai",
            "ollamaModel": "llama3",
            "ollamaBaseUrl": "http://localhost:11434/v1/",
            "whisperModel": "base_en",
            "launchAtLogin": true,
            "onboardingComplete": true
        }"#;
        let prefs: Prefs = serde_json::from_str(json).expect("parses despite missing field");
        assert_eq!(prefs.preferred_input_device, None);
        // The rest of the file is honored, not reset to defaults.
        assert_eq!(prefs.hotkey, "Cmd+Shift+D");
        assert!(prefs.launch_at_login);
    }

    #[test]
    fn preferred_input_device_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let prefs = Prefs {
            preferred_input_device: Some("AirPods Pro".into()),
            ..Prefs::default()
        };
        save_to(&path, &prefs).expect("save");
        assert_eq!(
            load_from(&path).preferred_input_device,
            Some("AirPods Pro".into())
        );
    }

    /// An invalid magic string now fails at deserialization instead of panicking
    /// later at provider-select / model-load time.
    #[test]
    fn invalid_enum_value_is_rejected() {
        let json = r#"{
            "hotkey": "Option+Shift+Space",
            "cleanupMode": "none",
            "cleanupAggressiveness": "standard",
            "apiProvider": "bogus_provider",
            "ollamaModel": "llama3",
            "ollamaBaseUrl": "http://localhost:11434/v1/",
            "whisperModel": "base_en",
            "launchAtLogin": false,
            "onboardingComplete": false
        }"#;
        assert!(serde_json::from_str::<Prefs>(json).is_err());
    }
}
