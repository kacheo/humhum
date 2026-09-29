import { vi } from "vitest";

const invoke = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { ipc } from "./ipc";

afterEach(() => {
  invoke.mockClear();
});

test("recording commands invoke with no args", async () => {
  await ipc.startRecording();
  expect(invoke).toHaveBeenCalledWith("start_recording");
  await ipc.stopRecording();
  expect(invoke).toHaveBeenCalledWith("stop_recording");
  await ipc.cancelRecording();
  expect(invoke).toHaveBeenCalledWith("cancel_recording");
});

test("prefs commands round-trip the prefs payload", async () => {
  await ipc.loadPrefs();
  expect(invoke).toHaveBeenCalledWith("get_prefs");
  const prefs = { hotkey: "Cmd+K" } as never;
  await ipc.savePrefs(prefs);
  expect(invoke).toHaveBeenCalledWith("set_prefs", { prefs });
});

test("validateHotkey passes the combo as keys", async () => {
  await ipc.validateHotkey("Cmd+K");
  expect(invoke).toHaveBeenCalledWith("validate_hotkey", { keys: "Cmd+K" });
});

test("model commands pass through the model id", async () => {
  await ipc.downloadModel("base.en");
  expect(invoke).toHaveBeenCalledWith("download_model", { model: "base.en" });
  await ipc.getModelStatus("base.en");
  expect(invoke).toHaveBeenCalledWith("get_model_status", { model: "base.en" });
});

test("api key commands pass provider and key", async () => {
  await ipc.storeApiKey("openai", "sk-1");
  expect(invoke).toHaveBeenCalledWith("store_api_key", { provider: "openai", key: "sk-1" });
  await ipc.testApiKey("openai", "sk-1");
  expect(invoke).toHaveBeenCalledWith("test_api_key", { provider: "openai", key: "sk-1" });
});

test("testOllama passes baseUrl and model", async () => {
  await ipc.testOllama("http://localhost:11434", "llama3");
  expect(invoke).toHaveBeenCalledWith("test_ollama", {
    baseUrl: "http://localhost:11434",
    model: "llama3",
  });
});

test("permissions commands invoke with no args", async () => {
  await ipc.checkPermissions();
  expect(invoke).toHaveBeenCalledWith("check_permissions");
  await ipc.requestMicrophone();
  expect(invoke).toHaveBeenCalledWith("request_microphone");
  await ipc.openAccessibilitySettings();
  expect(invoke).toHaveBeenCalledWith("open_accessibility_settings");
  await ipc.openInputMonitoringSettings();
  expect(invoke).toHaveBeenCalledWith("open_input_monitoring_settings");
});

test("listInputDevices invokes with no args", async () => {
  await ipc.listInputDevices();
  expect(invoke).toHaveBeenCalledWith("list_input_devices");
});
