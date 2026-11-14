import { create } from "zustand";
import { ipc } from "../lib/ipc";
import type { Prefs } from "../lib/types";

const defaultPrefs: Prefs = {
  hotkey: "Option+Shift+Space",
  cleanupMode: "none",
  cleanupAggressiveness: "standard",
  apiProvider: "open_ai",
  ollamaModel: "llama3",
  ollamaBaseUrl: "http://localhost:11434/v1/",
  whisperModel: "base_en",
  launchAtLogin: false,
  onboardingComplete: false,
  preferredInputDevice: null,
};

interface PrefsState {
  prefs: Prefs;
  savedPrefs: Prefs; // internal baseline for isDirty / discardChanges
  isDirty: boolean;
  isSaving: boolean;
  loadPrefs: () => Promise<void>; // invokes get_prefs
  setField: <K extends keyof Prefs>(key: K, value: Prefs[K]) => void;
  savePrefs: () => Promise<void>; // serializes snapshots through set_prefs
  discardChanges: () => void;
}

export const usePrefsStore = create<PrefsState>((set, get) => {
  let saveQueue = Promise.resolve();
  let pendingSaves = 0;
  return {
    prefs: { ...defaultPrefs },
    savedPrefs: { ...defaultPrefs },
    isDirty: false,
    isSaving: false,
    loadPrefs: async () => {
      const prefs = await ipc.loadPrefs();
      set({ prefs, savedPrefs: { ...prefs }, isDirty: false });
    },
    setField: (key, value) =>
      set((s) => ({ prefs: { ...s.prefs, [key]: value }, isDirty: true })),
    savePrefs: async () => {
      const prefs = { ...get().prefs };
      pendingSaves += 1;
      set({ isSaving: true });
      const saving = saveQueue.then(async () => {
        await ipc.savePrefs(prefs);
        set((s) => ({
          savedPrefs: prefs,
          isDirty: (Object.keys(prefs) as (keyof Prefs)[]).some((key) => s.prefs[key] !== prefs[key]),
        }));
      });
      // A failed write rejects its caller, but must not block later saves.
      saveQueue = saving.catch(() => {});
      try {
        await saving;
      } finally {
        pendingSaves -= 1;
        set({ isSaving: pendingSaves > 0 });
      }
    },
    discardChanges: () =>
      set((s) => ({ prefs: { ...s.savedPrefs }, isDirty: false })),
  };
});
