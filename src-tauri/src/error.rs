use serde::{Serialize, Serializer};

/// Application-wide command error.
///
/// **Wire shape (LLD §587/§593–604):** a command failure serializes to a *flat
/// snake_case code string* (e.g. `"already_recording"`, `"permission_denied"`,
/// `"io_error"`, `"conflict_with:Cmd+Space"`), never an externally-tagged enum.
/// The custom [`Serialize`] impl below emits [`AppError::code`]. Under Tauri v2
/// the `invoke()` promise rejects with that bare string directly (Tauri owns the
/// outer envelope — LLD §587's `{ error: string }` describes the IPC envelope,
/// not the JS-visible value), so the frontend matches on `e === "..."`.
///
/// The granular codes are finer than these variants, so each subsystem-wrapping
/// variant **stores the originating module error's `code()`** (set by the
/// `From` impls), not its `Display` message. The human-readable message still
/// lives on each module error's own `Display` (used by `pipeline-error
/// { code, message }` in Phase 15). Note [`Display`](std::fmt::Display) below
/// therefore renders the *code* for wrapping variants (e.g. `"audio error:
/// device_unavailable"`), not a prose message — it's for logs, not the UI.
///
/// Some LLD codes have no producing variant yet because their commands are
/// unimplemented stubs: `not_recording` (`stop_recording`), `invalid_provider`
/// (`test_api_key`), `already_downloading` (`download_model`). Tracked in a
/// follow-up bead; add them when those commands land.
#[derive(Debug)]
pub enum AppError {
    Hotkey(String),
    Audio(String),
    Transcription(String),
    Cleanup(String),
    Paste(String),
    Prefs(String),
    Model(String),
    AlreadyRecording,
    /// Payload is a human-readable message (e.g. "not implemented"), deliberately
    /// **not** exposed via [`AppError::code`] — `code()` returns the opaque wire
    /// code `"internal"` so internal detail never leaks to the frontend.
    Internal(String),
}

impl AppError {
    /// The stable snake_case wire code for this error (LLD §593–604). For
    /// subsystem-wrapping variants this is the code captured from the module
    /// error at the `From` boundary.
    pub fn code(&self) -> String {
        match self {
            AppError::Hotkey(c)
            | AppError::Audio(c)
            | AppError::Transcription(c)
            | AppError::Cleanup(c)
            | AppError::Paste(c)
            | AppError::Prefs(c)
            | AppError::Model(c) => c.clone(),
            AppError::AlreadyRecording => "already_recording".to_string(),
            AppError::Internal(_) => "internal".to_string(),
        }
    }
}

/// Serialize to the flat code string so a failed `#[tauri::command]` rejects the
/// JS promise with the documented code (not `{"Variant": "msg"}`).
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.code())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Hotkey(e) => write!(f, "hotkey error: {e}"),
            AppError::Audio(e) => write!(f, "audio error: {e}"),
            AppError::Transcription(e) => write!(f, "transcription error: {e}"),
            AppError::Cleanup(e) => write!(f, "cleanup error: {e}"),
            AppError::Paste(e) => write!(f, "paste error: {e}"),
            AppError::Prefs(e) => write!(f, "prefs error: {e}"),
            AppError::Model(e) => write!(f, "model error: {e}"),
            AppError::AlreadyRecording => write!(f, "already recording"),
            AppError::Internal(e) => write!(f, "internal error: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

/// Generates `impl From<$module_error> for AppError` that wraps the module
/// error's stable `code()` in the given `AppError::$variant` (so the documented
/// snake_case wire code is preserved while the variant tags the originating
/// subsystem). The human message stays on the module error's own `Display`.
macro_rules! impl_from_module_error {
    ($module_error:ty => $variant:ident) => {
        impl From<$module_error> for AppError {
            fn from(e: $module_error) -> Self {
                AppError::$variant(e.code())
            }
        }
    };
}

impl_from_module_error!(crate::hotkey::HotkeyError => Hotkey);
impl_from_module_error!(crate::audio::AudioError => Audio);
impl_from_module_error!(crate::transcription::TranscriptionError => Transcription);
impl_from_module_error!(crate::cleanup::CleanupError => Cleanup);
impl_from_module_error!(crate::paste::PasteError => Paste);
impl_from_module_error!(crate::prefs::PrefsError => Prefs);
impl_from_module_error!(crate::models::ModelError => Model);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioError;
    use crate::cleanup::CleanupError;
    use crate::hotkey::HotkeyError;
    use crate::models::ModelError;
    use crate::paste::PasteError;
    use crate::prefs::PrefsError;
    use crate::transcription::TranscriptionError;

    #[test]
    fn from_module_errors_map_to_matching_variant() {
        assert!(matches!(
            AppError::from(HotkeyError::PermissionDenied),
            AppError::Hotkey(_)
        ));
        assert!(matches!(
            AppError::from(AudioError::DeviceUnavailable),
            AppError::Audio(_)
        ));
        assert!(matches!(
            AppError::from(TranscriptionError::ModelNotReady),
            AppError::Transcription(_)
        ));
        assert!(matches!(
            AppError::from(PasteError::NoFocusedField),
            AppError::Paste(_)
        ));
        assert!(matches!(
            AppError::from(PrefsError::IoError("x".into())),
            AppError::Prefs(_)
        ));
        assert!(matches!(
            AppError::from(ModelError::HashMismatch),
            AppError::Model(_)
        ));
    }

    #[test]
    fn from_captures_module_code() {
        // The wrapped payload is now the module error's stable wire code.
        assert_eq!(
            AppError::from(AudioError::DeviceUnavailable).code(),
            "device_unavailable"
        );
        assert_eq!(
            AppError::from(HotkeyError::ConflictWith("Cmd+Space".into())).code(),
            "conflict_with:Cmd+Space"
        );
        assert_eq!(
            AppError::from(HotkeyError::InvalidCombo("empty combo".into())).code(),
            "invalid_combo:empty combo"
        );
        assert_eq!(
            AppError::from(PrefsError::IoError("x".into())).code(),
            "io_error"
        );
        assert_eq!(
            AppError::from(PrefsError::SerializeError("x".into())).code(),
            "serialize_error"
        );
        assert_eq!(
            AppError::from(TranscriptionError::ModelNotReady).code(),
            "model_not_ready"
        );
        assert_eq!(
            AppError::from(CleanupError::KeyRejected("401".into())).code(),
            "key_rejected"
        );
        assert_eq!(
            AppError::from(CleanupError::ModelNotFound).code(),
            "model_not_found"
        );
        assert_eq!(
            AppError::from(PasteError::NoFocusedField).code(),
            "no_focused_field"
        );
        assert_eq!(
            AppError::from(ModelError::InsufficientDisk(1, 2)).code(),
            "insufficient_disk"
        );
        assert_eq!(
            AppError::from(ModelError::AlreadyDownloading).code(),
            "already_downloading"
        );
    }

    #[test]
    fn serializes_to_flat_code_string() {
        use serde_json::json;
        // Command failures reject with the flat snake_case code (LLD §587/§593).
        assert_eq!(
            serde_json::to_value(AppError::AlreadyRecording).unwrap(),
            json!("already_recording")
        );
        assert_eq!(
            serde_json::to_value(AppError::Internal("not implemented".into())).unwrap(),
            json!("internal")
        );
        assert_eq!(
            serde_json::to_value(AppError::from(AudioError::PermissionDenied)).unwrap(),
            json!("permission_denied")
        );
        // Parameterized hotkey codes survive serialization verbatim.
        assert_eq!(
            serde_json::to_value(AppError::from(HotkeyError::ConflictWith(
                "Cmd+Space".into()
            )))
            .unwrap(),
            json!("conflict_with:Cmd+Space")
        );
        // Never the old externally-tagged shape.
        let v = serde_json::to_value(AppError::from(PrefsError::IoError("disk".into()))).unwrap();
        assert_eq!(v, json!("io_error"));
        assert!(v.get("Prefs").is_none());
    }

    #[test]
    fn display_renders_the_documented_message_for_every_variant() {
        assert_eq!(AppError::Hotkey("x".into()).to_string(), "hotkey error: x");
        assert_eq!(AppError::Audio("x".into()).to_string(), "audio error: x");
        assert_eq!(
            AppError::Transcription("x".into()).to_string(),
            "transcription error: x"
        );
        assert_eq!(
            AppError::Cleanup("x".into()).to_string(),
            "cleanup error: x"
        );
        assert_eq!(AppError::Paste("x".into()).to_string(), "paste error: x");
        assert_eq!(AppError::Prefs("x".into()).to_string(), "prefs error: x");
        assert_eq!(AppError::Model("x".into()).to_string(), "model error: x");
        assert_eq!(AppError::AlreadyRecording.to_string(), "already recording");
        assert_eq!(
            AppError::Internal("x".into()).to_string(),
            "internal error: x"
        );
    }
}
