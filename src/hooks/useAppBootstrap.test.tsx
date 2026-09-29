import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import type { Prefs } from "../lib/types";

// Mock the IPC layer so the stores hydrate from canned backend responses.
// useAppBootstrap delegates to prefs.loadPrefs() (get_prefs).
vi.mock("../lib/ipc", () => ({
  ipc: {
    loadPrefs: vi.fn(),
    savePrefs: vi.fn(),
  },
}));

import { ipc } from "../lib/ipc";
import { useAppBootstrap } from "./useAppBootstrap";
import { usePrefsStore } from "../stores/prefs";

const defaults = usePrefsStore.getState().prefs;

const cannedPrefs: Prefs = {
  hotkey: "Cmd+Shift+D",
  cleanupMode: "api",
  cleanupAggressiveness: "heavy",
  apiProvider: "anthropic",
  ollamaModel: "llama3",
  ollamaBaseUrl: "http://localhost:11434/v1/",
  whisperModel: "small_en",
  launchAtLogin: true,
  onboardingComplete: true,
};

beforeEach(() => {
  usePrefsStore.setState({
    prefs: { ...defaults },
    savedPrefs: { ...defaults },
    isDirty: false,
    isSaving: false,
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

test("hydrates prefs then reports ready", async () => {
  vi.mocked(ipc.loadPrefs).mockResolvedValue(cannedPrefs);

  const { result } = renderHook(() => useAppBootstrap());

  await waitFor(() => expect(result.current.ready).toBe(true));
  expect(usePrefsStore.getState().prefs).toEqual(cannedPrefs);
});

test("a rejected command degrades gracefully — still ready", async () => {
  vi.mocked(ipc.loadPrefs).mockRejectedValue(new Error("backend down"));

  const { result } = renderHook(() => useAppBootstrap());

  await waitFor(() => expect(result.current.ready).toBe(true));
  expect(usePrefsStore.getState().prefs).toEqual(defaults);
});
