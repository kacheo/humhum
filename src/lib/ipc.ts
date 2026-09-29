import { invoke } from "@tauri-apps/api/core";
import type {
  ModelStatus,
  PermissionsState,
  Prefs,
} from "./types";

export const ipc = {
  startRecording: () => invoke<void>("start_recording"),
  stopRecording: () => invoke<void>("stop_recording"),
  cancelRecording: () => invoke<void>("cancel_recording"),

  loadPrefs: () => invoke<Prefs>("get_prefs"),
  savePrefs: (prefs: Prefs) => invoke<void>("set_prefs", { prefs }),

  validateHotkey: (combo: string) =>
    invoke<void>("validate_hotkey", { keys: combo }),

  downloadModel: (model: string) => invoke<void>("download_model", { model }),
  getModelStatus: (model: string) =>
    invoke<ModelStatus>("get_model_status", { model }),

  storeApiKey: (provider: string, key: string) =>
    invoke<void>("store_api_key", { provider, key }),

  testApiKey: (provider: string, key: string) =>
    invoke<void>("test_api_key", { provider, key }),

  testOllama: (baseUrl: string, model: string) =>
    invoke<void>("test_ollama", { baseUrl, model }),

  checkPermissions: () => invoke<PermissionsState>("check_permissions"),
  requestMicrophone: () => invoke<void>("request_microphone"),
  openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
  openInputMonitoringSettings: () =>
    invoke<void>("open_input_monitoring_settings"),

  listInputDevices: () => invoke<string[]>("list_input_devices"),
};
