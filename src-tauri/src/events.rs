//! Tauri event payload structs (Rust → Frontend), one per event in LLD §3.9/§6
//! that carries data. `main.rs` (and the `cap_timer` task) own the actual
//! `AppHandle::emit()` calls; this module only defines the serialized shapes.
//!
//! Events that carry no data (`recording-started`, `recording-stopped`,
//! `recording-cancelled`) have no struct. Permission events reuse
//! [`crate::permissions::PermissionsState`].

use serde::{Deserialize, Serialize};

use crate::transcription::WhisperModel;

/// `transcript-ready` — the raw transcript, before cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptReady {
    pub raw: String,
}

/// `cleanup-done` — the cleaned transcript (`text == raw` if cleanup skipped).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupDone {
    pub text: String,
}

/// `model-download-progress` — emitted every 1% during a model download.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model: WhisperModel, // serializes "base_en" | "small_en"
    pub pct: u8,
}

/// `pipeline-error` — surfaced from the pipeline orchestrator. `code` is a
/// stable machine string (e.g. from a `CleanupError` variant); `message` is the
/// human-readable detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineError {
    pub code: String,
    pub message: String,
}

/// Why a paste failed, as surfaced to the UI. A subset of
/// [`crate::paste::PasteError`] — only the variants the frontend reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteFailReason {
    NoFocusedField,
    ReadOnlyField,
}

/// `paste-failed` — paste could not target a field; `text_copied_to_clipboard`
/// reports whether the clipboard fallback was used.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteFailed {
    pub reason: PasteFailReason,
    pub text_copied_to_clipboard: bool, // wire: textCopiedToClipboard
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transcript_ready_and_cleanup_done_round_trip() {
        let tr = TranscriptReady {
            raw: "hello".into(),
        };
        assert_eq!(
            serde_json::to_value(&tr).unwrap(),
            json!({ "raw": "hello" })
        );
        let back: TranscriptReady = serde_json::from_value(json!({ "raw": "hi" })).unwrap();
        assert_eq!(back.raw, "hi");

        let cd = CleanupDone {
            text: "hello".into(),
        };
        assert_eq!(
            serde_json::to_value(&cd).unwrap(),
            json!({ "text": "hello" })
        );
    }

    #[test]
    fn model_download_progress_wire_shape() {
        let value = serde_json::to_value(ModelDownloadProgress {
            model: WhisperModel::BaseEn,
            pct: 42,
        })
        .unwrap();
        assert_eq!(value, json!({ "model": "base_en", "pct": 42 }));
        let back: ModelDownloadProgress = serde_json::from_value(value).unwrap();
        assert_eq!(back.model, WhisperModel::BaseEn);
        assert_eq!(back.pct, 42);
    }

    #[test]
    fn pipeline_error_round_trip() {
        let value = serde_json::to_value(PipelineError {
            code: "key_rejected".into(),
            message: "401".into(),
        })
        .unwrap();
        assert_eq!(value, json!({ "code": "key_rejected", "message": "401" }));
    }

    #[test]
    fn paste_failed_reason_strings_and_camel_case_key() {
        assert_eq!(
            serde_json::to_value(PasteFailReason::NoFocusedField).unwrap(),
            json!("no_focused_field")
        );
        assert_eq!(
            serde_json::to_value(PasteFailReason::ReadOnlyField).unwrap(),
            json!("read_only_field")
        );

        let value = serde_json::to_value(PasteFailed {
            reason: PasteFailReason::ReadOnlyField,
            text_copied_to_clipboard: true,
        })
        .unwrap();
        assert_eq!(
            value,
            json!({ "reason": "read_only_field", "textCopiedToClipboard": true })
        );
        let back: PasteFailed = serde_json::from_value(value).unwrap();
        assert_eq!(back.reason, PasteFailReason::ReadOnlyField);
        assert!(back.text_copied_to_clipboard);
    }
}
