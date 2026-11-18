import { invoke } from "@tauri-apps/api/core";
import type {
  ApiProvider,
  CleanupModelOption,
  CleanupUsage,
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

  storeApiKey: (provider: ApiProvider, key: string) =>
    invoke<void>("store_api_key", { provider, key }),

  testApiKey: (provider: ApiProvider, key: string, model?: string) =>
    invoke<void>("test_api_key", model ? { provider, key, model } : { provider, key }),

  getCleanupModels: () => invoke<CleanupModelOption[]>("get_cleanup_models"),
  getCleanupUsage: () => invoke<CleanupUsage>("get_cleanup_usage"),

  hasApiKey: (provider: ApiProvider) =>
    invoke<boolean>("has_api_key", { provider }),

  removeApiKey: (provider: ApiProvider) =>
    invoke<void>("remove_api_key", { provider }),

  testOllama: (baseUrl: string, model: string) =>
    invoke<void>("test_ollama", { baseUrl, model }),

  checkPermissions: () => invoke<PermissionsState>("check_permissions"),
  requestMicrophone: () => invoke<void>("request_microphone"),
  openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
  openInputMonitoringSettings: () =>
    invoke<void>("open_input_monitoring_settings"),

  listInputDevices: () => invoke<string[]>("list_input_devices"),
};
