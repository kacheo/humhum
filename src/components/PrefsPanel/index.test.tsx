import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";
import { PrefsPanel } from ".";
import { usePrefsStore } from "../../stores/prefs";
import { useRecordingStore } from "../../stores/recording";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn().mockResolvedValue("9.8.7") }));
import { getVersion } from "@tauri-apps/api/app";

vi.mock("../../lib/opener", () => ({ openExternal: vi.fn().mockResolvedValue(undefined) }));
import { openExternal } from "../../lib/opener";

// The Shortcut section's HotkeyPermissionNotice subscribes to this event; stub
// it so the panel tests don't reach the real Tauri `listen`.
vi.mock("../../hooks/useTauriEvent", () => ({ useTauriEvent: () => {} }));
vi.mock("../../lib/ipc", () => ({
  ipc: {
    getModelStatus: vi.fn().mockResolvedValue("not_downloaded"),
    downloadModel: vi.fn().mockResolvedValue(undefined),
    validateHotkey: vi.fn().mockResolvedValue(undefined),
    savePrefs: vi.fn().mockResolvedValue(undefined),
    loadPrefs: vi.fn(),
    testApiKey: vi.fn().mockResolvedValue(undefined),
    // All granted so the Permissions section shows status rows with no action
    // buttons and the existing section assertions are unaffected.
    checkPermissions: vi.fn().mockResolvedValue({ microphone: true, accessibility: true, inputMonitoring: true }),
    requestMicrophone: vi.fn().mockResolvedValue(undefined),
    openAccessibilitySettings: vi.fn(),
    openInputMonitoringSettings: vi.fn(),
    listInputDevices: vi.fn().mockResolvedValue(["AirPods Pro"]),
  },
}));
import { ipc } from "../../lib/ipc";

const DEFAULTS = {
  hotkey: "Option+Shift+Space",
  cleanupMode: "none",
  cleanupAggressiveness: "standard",
  apiProvider: "open_ai",
  ollamaModel: "llama3",
  ollamaBaseUrl: "http://localhost:11434/v1/",
  whisperModel: "base_en",
  launchAtLogin: false,
  onboardingComplete: true,
  preferredInputDevice: null,
} as const;

beforeEach(() => {
  vi.clearAllMocks();
  usePrefsStore.setState({ prefs: { ...DEFAULTS }, savedPrefs: { ...DEFAULTS }, isDirty: false, isSaving: false });
  useRecordingStore.getState().reset();
});

async function renderPanel() {
  render(<PrefsPanel />);
  await waitFor(() => expect(ipc.getModelStatus).toHaveBeenCalled()); // flush mount effect
}

test("shows the running app version in the sidebar across tabs", async () => {
  await renderPanel();
  expect(await screen.findByText("Version 9.8.7")).toBeVisible();
  for (const label of ["Shortcut", "AI", "Permissions"]) {
    await userEvent.click(screen.getByRole("tab", { name: label }));
    expect(screen.getByText("Version 9.8.7")).toBeVisible();
  }
});

test("keeps Preferences usable when version lookup fails", async () => {
  vi.mocked(getVersion).mockRejectedValueOnce(new Error("Unavailable"));
  await renderPanel();
  expect(await screen.findByText("Version unavailable")).toBeVisible();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.getByRole("heading", { name: "Text cleanup" })).toBeVisible();
  expect(screen.getByRole("button", { name: /Source and license/i })).toBeEnabled();
});

test("renders all tabs", async () => {
  await renderPanel();
  for (const label of ["General", "Shortcut", "AI", "Permissions"]) {
    expect(screen.getByRole("tab", { name: label })).toBeInTheDocument();
  }
});

test("links to the Humhum source and license", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("button", { name: /Source and license/i }));
  expect(openExternal).toHaveBeenCalledWith("https://github.com/kacheo/humhum");
});

test.each(["{Enter}", " "])("opens the source and license with keyboard activation %s", async (key) => {
  await renderPanel();
  screen.getByRole("button", { name: /Source and license/i }).focus();
  await userEvent.keyboard(key);
  expect(openExternal).toHaveBeenCalledWith("https://github.com/kacheo/humhum");
});

test("shows a browser-opening error and clears it when retrying", async () => {
  vi.mocked(openExternal).mockRejectedValueOnce(new Error("forbidden URL"));
  await renderPanel();
  const button = screen.getByRole("button", { name: /Source and license/i });
  await userEvent.click(button);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open the browser. Try again.");
  expect(button).toBeEnabled();

  let finishRetry!: () => void;
  vi.mocked(openExternal).mockImplementationOnce(() => new Promise<void>((resolve) => { finishRetry = resolve; }));
  await userEvent.click(button);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await act(async () => { finishRetry(); });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(openExternal).toHaveBeenCalledTimes(2);
});

test("the active tab and its panel are cross-linked (id/aria-controls/aria-labelledby)", async () => {
  await renderPanel();
  const tab = screen.getByRole("tab", { name: "General" });
  const panel = screen.getByRole("tabpanel"); // only one is visible/in the a11y tree at a time
  expect(tab).toHaveAttribute("aria-controls", panel.id);
  expect(panel).toHaveAttribute("aria-labelledby", tab.id);
});

test("General is the default tab and switching tabs shows/hides the corresponding content", async () => {
  await renderPanel();
  expect(screen.getByRole("heading", { name: "Startup" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Text cleanup" })).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.getByRole("heading", { name: "Text cleanup" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Transcription" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Startup" })).not.toBeInTheDocument();
});

test("switching away from the AI tab preserves an in-progress, unsaved API key", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.selectOptions(screen.getByLabelText("Mode"), "api");
  await userEvent.type(screen.getByLabelText("API key"), "sk-test-123");

  await userEvent.click(screen.getByRole("tab", { name: "General" }));
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));

  expect(screen.getByLabelText("API key")).toHaveValue("sk-test-123");
});

test("Cloud API key field only appears in Cloud API mode (regression: previously rendered regardless of mode)", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();

  await userEvent.selectOptions(screen.getByLabelText("Mode"), "local");
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Ollama model")).toBeInTheDocument();

  await userEvent.selectOptions(screen.getByLabelText("Mode"), "api");
  expect(screen.getByLabelText("API key")).toBeInTheDocument();
  expect(screen.queryByLabelText("Ollama model")).not.toBeInTheDocument();
});

test("Aggressiveness is hidden when cleanup mode is None and shown once a backend is selected", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.queryByLabelText("Aggressiveness")).not.toBeInTheDocument();

  await userEvent.selectOptions(screen.getByLabelText("Mode"), "local");
  expect(screen.getByLabelText("Aggressiveness")).toBeInTheDocument();
});

test("Save is gated on dirtiness and persists via the store", async () => {
  await renderPanel();
  const save = screen.getByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  await userEvent.click(screen.getByLabelText("Launch at login"));
  expect(save).toBeEnabled();
  await userEvent.click(save);
  await waitFor(() => expect(ipc.savePrefs).toHaveBeenCalled());
});

test("Discard reverts unsaved changes", async () => {
  await renderPanel();
  const checkbox = screen.getByLabelText("Launch at login") as HTMLInputElement;
  await userEvent.click(checkbox);
  expect(checkbox.checked).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(checkbox.checked).toBe(false);
});

test("validate_hotkey is called when a new shortcut is captured", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "Shortcut" }));
  await userEvent.click(screen.getByRole("button", { name: /Space|record/i }));
  fireEvent.keyDown(screen.getByRole("button", { name: "Press keys…" }), { code: "KeyK", metaKey: true, shiftKey: true });
  await waitFor(() => expect(ipc.validateHotkey).toHaveBeenCalledWith("Cmd+Shift+K"));
});

test("the Small model download is available", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.getByRole("button", { name: "Download Small (English)" })).toBeEnabled();
});

test("the cloud API key is available", async () => {
  usePrefsStore.setState({
    prefs: { ...DEFAULTS, cleanupMode: "api" },
    savedPrefs: { ...DEFAULTS, cleanupMode: "api" },
    isDirty: false,
    isSaving: false,
  });
  await renderPanel();
  // API key Test/Store landed in Phase 11.
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.getByLabelText("API key")).toBeEnabled();
});

test("the Cloud API cleanup mode is selectable (Phase 12)", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  expect(screen.getByRole("option", { name: "Cloud API" })).toBeEnabled();
});

test("the model badge reflects live download progress", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  act(() => useRecordingStore.getState().setDownloadProgress("base_en", 64));
  expect(await screen.findByText("Downloading 64%")).toBeInTheDocument();
});

test("clicking Download invokes download_model for that model", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.click(screen.getByRole("button", { name: "Download Base (English)" }));
  expect(ipc.downloadModel).toHaveBeenCalledWith("base_en");
});

test("switching the transcription model persists whisperModel", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.selectOptions(screen.getByLabelText("Model"), "small_en");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(ipc.savePrefs).toHaveBeenCalledWith(expect.objectContaining({ whisperModel: "small_en" }))
  );
});

test("editing the Ollama model and URL fields updates the store", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.selectOptions(screen.getByLabelText("Mode"), "local");

  const modelField = screen.getByLabelText("Ollama model");
  await userEvent.clear(modelField);
  await userEvent.type(modelField, "mistral");
  const urlField = screen.getByLabelText("Ollama URL");
  await userEvent.clear(urlField);
  await userEvent.type(urlField, "http://localhost:9999/v1/");

  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(ipc.savePrefs).toHaveBeenCalledWith(
      expect.objectContaining({ ollamaModel: "mistral", ollamaBaseUrl: "http://localhost:9999/v1/" })
    )
  );
});

test("changing Aggressiveness persists the new value", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.selectOptions(screen.getByLabelText("Mode"), "local");
  await userEvent.selectOptions(screen.getByLabelText("Aggressiveness"), "heavy");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(ipc.savePrefs).toHaveBeenCalledWith(expect.objectContaining({ cleanupAggressiveness: "heavy" }))
  );
});

test("clicking Test on the Cloud API key calls test_api_key with the provider and key", async () => {
  usePrefsStore.setState({
    prefs: { ...DEFAULTS, cleanupMode: "api" },
    savedPrefs: { ...DEFAULTS, cleanupMode: "api" },
    isDirty: false,
    isSaving: false,
  });
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  await userEvent.type(screen.getByLabelText("API key"), "sk-live-1");
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  await waitFor(() => expect(ipc.testApiKey).toHaveBeenCalledWith("open_ai", "sk-live-1"));
});

test("selecting an input device is picked up as a dirtying change", async () => {
  await renderPanel();
  await waitFor(() => expect(ipc.listInputDevices).toHaveBeenCalled());
  const save = screen.getByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  await userEvent.selectOptions(screen.getByLabelText("Input device"), "AirPods Pro");
  expect(save).toBeEnabled();
  await userEvent.click(save);
  await waitFor(() =>
    expect(ipc.savePrefs).toHaveBeenCalledWith(
      expect.objectContaining({ preferredInputDevice: "AirPods Pro" })
    )
  );
});

test("failed save displays feedback and retains edits for retry", async () => {
  vi.mocked(ipc.savePrefs).mockRejectedValueOnce(new Error("Disk unavailable"));
  await renderPanel();
  await userEvent.click(screen.getByLabelText("Launch at login"));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
  expect(screen.getByLabelText("Launch at login")).toBeChecked();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
});

test("download error takes precedence over stale progress and offers retry", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  act(() => {
    useRecordingStore.getState().setDownloadProgress("base_en", 64);
    useRecordingStore.getState().setModelStatus("base_en", { error: "Network unavailable" });
  });
  expect(screen.queryByText("Downloading 64%")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry Base (English)" })).toBeEnabled();
});

test("active model download disables repeated download clicks", async () => {
  await renderPanel();
  await userEvent.click(screen.getByRole("tab", { name: "AI" }));
  act(() => useRecordingStore.getState().setDownloadProgress("base_en", 64));
  expect(screen.getByRole("button", { name: "Download Base (English)" })).toBeDisabled();
});
