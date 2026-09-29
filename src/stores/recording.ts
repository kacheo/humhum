import { create } from "zustand";
import type { ModelStatus, RecordingPhase } from "../lib/types";

type WhisperModelId = "base_en" | "small_en";

interface RecordingState {
  phase: RecordingPhase;
  rawTranscript: string | null;
  cleanedTranscript: string | null;
  errorMessage: string | null;
  modelStatus: Record<WhisperModelId, ModelStatus>;
  downloadProgress: Record<WhisperModelId, number>; // 0–100

  reset: () => void;
  setPhase: (phase: RecordingPhase) => void;
  setRawTranscript: (text: string) => void;
  setCleanedTranscript: (text: string) => void;
  // Narrower than LLD §5 (model: string) on purpose — model ids are a closed set.
  setModelStatus: (model: WhisperModelId, status: ModelStatus) => void;
  setDownloadProgress: (model: WhisperModelId, pct: number) => void;
  setError: (msg: string) => void;
}

const initialState = {
  phase: "idle" as RecordingPhase,
  rawTranscript: null as string | null,
  cleanedTranscript: null as string | null,
  errorMessage: null as string | null,
  modelStatus: {
    base_en: "not_downloaded",
    small_en: "not_downloaded",
  } as Record<WhisperModelId, ModelStatus>,
  downloadProgress: { base_en: 0, small_en: 0 } as Record<WhisperModelId, number>,
};

export const useRecordingStore = create<RecordingState>((set) => ({
  ...initialState,
  reset: () => set({ ...initialState }),
  setPhase: (phase) => set({ phase }),
  setRawTranscript: (rawTranscript) => set({ rawTranscript }),
  setCleanedTranscript: (cleanedTranscript) => set({ cleanedTranscript }),
  setModelStatus: (model, status) =>
    set((s) => ({ modelStatus: { ...s.modelStatus, [model]: status } })),
  setDownloadProgress: (model, pct) =>
    set((s) => ({ downloadProgress: { ...s.downloadProgress, [model]: pct } })),
  setError: (errorMessage) => set({ errorMessage }),
}));
