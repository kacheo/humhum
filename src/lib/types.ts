// ─── Rust serde mirrors ──────────────────────────────────────────────────────
// This module is the single home for every TypeScript mirror of a Rust serde
// type. Stores and ipc import from here so a wire shape is defined exactly once.

// RecordingPhase — mirrors src-tauri/src/state.rs RecordingPhase (snake_case).
export type RecordingPhase =
  | "idle"
  | "recording"
  | "transcribing"
  | "cleaning"
  | "pasting"
  | "done"
  | "cancelled"
  | "error";

// Prefs enums — mirror the Rust serde enums (CleanupMode, CleanupAggressiveness,
// ApiProvider, WhisperModel) with #[serde(rename_all = "snake_case")].
export type CleanupMode = "none" | "api" | "local";
export type CleanupAggressiveness = "light" | "standard" | "heavy";
export type ApiProvider = "open_ai" | "anthropic";
export type WhisperModel = "base_en" | "small_en";

// Prefs — mirrors the Rust Prefs struct (camelCase fields). See get_prefs/set_prefs.
export interface Prefs {
  hotkey: string;
  cleanupMode: CleanupMode;
  cleanupAggressiveness: CleanupAggressiveness;
  apiProvider: ApiProvider;
  ollamaModel: string;
  ollamaBaseUrl: string;
  whisperModel: WhisperModel;
  launchAtLogin: boolean;
  onboardingComplete: boolean;
  // null = auto (avoid a Bluetooth default input when a non-Bluetooth
  // alternative exists); SYSTEM_DEFAULT_INPUT_DEVICE = always use macOS's
  // own default; any other string = an explicit device name override.
  preferredInputDevice: string | null;
}

// Sentinel `preferredInputDevice` value meaning "always use whatever macOS
// reports as the default input" — must match `SYSTEM_DEFAULT_SENTINEL` in
// src-tauri/src/audio/device_select.rs exactly.
export const SYSTEM_DEFAULT_INPUT_DEVICE = "system-default";

// ModelStatus — externally-tagged snake_case (src-tauri/src/models/mod.rs):
//   NotDownloaded → "not_downloaded"       Ready → "ready"
//   Downloading(50) → { downloading: 50 }  Error("x") → { error: "x" }
export type ModelStatus =
  | "not_downloaded"
  | "ready"
  | { downloading: number }
  | { error: string };

// Tauri event payloads (Rust → Frontend), mirroring src-tauri/src/events.rs.
// Field names/shapes follow the canonical LLD §6 contract. recording-started,
// recording-stopped, and recording-cancelled carry no payload.

// transcript-ready — raw transcript, before cleanup.
export interface TranscriptReadyPayload {
  raw: string;
}

// cleanup-done — cleaned transcript (text === raw if cleanup skipped).
export interface CleanupDonePayload {
  text: string;
}

// pipeline-error
export interface PipelineErrorPayload {
  code: string;
  message: string;
}

// model-download-progress
export interface ModelDownloadProgressPayload {
  model: "base_en" | "small_en";
  pct: number;
}

// paste-failed
export interface PasteFailedPayload {
  reason: "no_focused_field" | "read_only_field";
  textCopiedToClipboard: boolean;
}

// permissions-changed — mirrors the Rust PermissionsState struct.
export interface PermissionsState {
  microphone: boolean;
  accessibility: boolean;
  inputMonitoring: boolean;
}
