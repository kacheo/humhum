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

test("edits made during a save remain dirty and discard uses the saved snapshot", async () => {
  let finish!: () => void;
  vi.mocked(ipc.savePrefs).mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  usePrefsStore.getState().setField("hotkey", "saved shortcut");
  const saving = usePrefsStore.getState().savePrefs();
  await vi.waitFor(() => expect(ipc.savePrefs).toHaveBeenCalled());
  usePrefsStore.getState().setField("hotkey", "new draft");
  finish();
  await saving;
  expect(usePrefsStore.getState().prefs.hotkey).toBe("new draft");
  expect(usePrefsStore.getState().isDirty).toBe(true);
  usePrefsStore.getState().discardChanges();
  expect(usePrefsStore.getState().prefs.hotkey).toBe("saved shortcut");
});

test("overlapping saves persist their snapshots in order and retain the latest baseline", async () => {
  let finishFirst!: () => void;
  let finishSecond!: () => void;
  vi.mocked(ipc.savePrefs)
    .mockImplementationOnce(() => new Promise<void>((resolve) => { finishFirst = resolve; }))
    .mockImplementationOnce(() => new Promise<void>((resolve) => { finishSecond = resolve; }));
  usePrefsStore.getState().setField("hotkey", "first");
  const first = usePrefsStore.getState().savePrefs();
  await vi.waitFor(() => expect(ipc.savePrefs).toHaveBeenCalledTimes(1));
  usePrefsStore.getState().setField("hotkey", "second");
  const second = usePrefsStore.getState().savePrefs();
  expect(ipc.savePrefs).toHaveBeenCalledTimes(1);
  finishFirst();
  await first;
  await vi.waitFor(() => expect(ipc.savePrefs).toHaveBeenCalledTimes(2));
  expect(usePrefsStore.getState().isSaving).toBe(true);
  expect(usePrefsStore.getState().savedPrefs.hotkey).toBe("first");
  expect(usePrefsStore.getState().isDirty).toBe(true);
  expect(ipc.savePrefs).toHaveBeenLastCalledWith(expect.objectContaining({ hotkey: "second" }));
  finishSecond();
  await second;
  expect(usePrefsStore.getState().savedPrefs.hotkey).toBe("second");
  expect(usePrefsStore.getState().isDirty).toBe(false);
  expect(usePrefsStore.getState().isSaving).toBe(false);
});

test("a failed queued save preserves the successful baseline and allows retry", async () => {
  vi.mocked(ipc.savePrefs).mockResolvedValueOnce(undefined).mockRejectedValueOnce("keychain_error");
  usePrefsStore.getState().setField("hotkey", "first");
  const first = usePrefsStore.getState().savePrefs();
  usePrefsStore.getState().setField("hotkey", "second");
  const second = usePrefsStore.getState().savePrefs();
  await first;
  await expect(second).rejects.toBe("keychain_error");
  expect(usePrefsStore.getState().savedPrefs.hotkey).toBe("first");
  expect(usePrefsStore.getState().isDirty).toBe(true);
  expect(usePrefsStore.getState().isSaving).toBe(false);
  vi.mocked(ipc.savePrefs).mockResolvedValueOnce(undefined);
  await usePrefsStore.getState().savePrefs();
  expect(usePrefsStore.getState().savedPrefs.hotkey).toBe("second");
  expect(usePrefsStore.getState().isDirty).toBe(false);
});
