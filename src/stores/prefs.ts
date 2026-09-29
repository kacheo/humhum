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
  savePrefs: () => Promise<void>; // invokes set_prefs; clears isDirty
  discardChanges: () => void;
}

export const usePrefsStore = create<PrefsState>((set, get) => ({
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
    set({ isSaving: true });
    try {
      const { prefs } = get();
      await ipc.savePrefs(prefs);
      set({ savedPrefs: { ...prefs }, isDirty: false });
    } finally {
      set({ isSaving: false });
    }
  },
  discardChanges: () =>
    set((s) => ({ prefs: { ...s.savedPrefs }, isDirty: false })),
}));
