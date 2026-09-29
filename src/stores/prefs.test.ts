import { afterEach, beforeEach, vi } from "vitest";
import type { Prefs } from "../lib/types";

vi.mock("../lib/ipc", () => ({
  ipc: { loadPrefs: vi.fn(), savePrefs: vi.fn() },
}));

import { ipc } from "../lib/ipc";
import { usePrefsStore } from "./prefs";

// Pristine defaults captured before any test mutates the store.
const defaults = usePrefsStore.getState().prefs;

const canned: Prefs = {
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
  vi.clearAllMocks();
});

afterEach(() => {
  vi.clearAllMocks();
});

test("setField updates prefs and marks dirty", () => {
  usePrefsStore.getState().setField("hotkey", "X");
  const s = usePrefsStore.getState();
  expect(s.prefs.hotkey).toBe("X");
  expect(s.isDirty).toBe(true);
});

test("loadPrefs hydrates prefs + savedPrefs and clears dirty", async () => {
  vi.mocked(ipc.loadPrefs).mockResolvedValue(canned);
  await usePrefsStore.getState().loadPrefs();
  const s = usePrefsStore.getState();
  expect(s.prefs).toEqual(canned);
  expect(s.savedPrefs).toEqual(canned);
  expect(s.isDirty).toBe(false);
});

test("discardChanges restores the last saved baseline", () => {
  usePrefsStore.getState().setField("hotkey", "X");
  expect(usePrefsStore.getState().isDirty).toBe(true);
  usePrefsStore.getState().discardChanges();
  const s = usePrefsStore.getState();
  expect(s.prefs).toEqual(defaults);
  expect(s.isDirty).toBe(false);
});

test("savePrefs invokes set_prefs with current prefs and clears dirty", async () => {
  vi.mocked(ipc.savePrefs).mockResolvedValue(undefined);
  usePrefsStore.getState().setField("hotkey", "X");
  await usePrefsStore.getState().savePrefs();
  const s = usePrefsStore.getState();
  expect(ipc.savePrefs).toHaveBeenCalledWith(s.prefs);
  expect(s.savedPrefs.hotkey).toBe("X");
  expect(s.isDirty).toBe(false);
});

test("savePrefs settles isSaving back to false", async () => {
  vi.mocked(ipc.savePrefs).mockResolvedValue(undefined);
  const p = usePrefsStore.getState().savePrefs();
  expect(usePrefsStore.getState().isSaving).toBe(true);
  await p;
  expect(usePrefsStore.getState().isSaving).toBe(false);
  expect(ipc.savePrefs).toHaveBeenCalledTimes(1);
});
