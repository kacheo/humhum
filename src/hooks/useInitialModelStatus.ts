import { useEffect } from "react";
import { ipc } from "../lib/ipc";
import type { WhisperModel } from "../lib/types";
import { useRecordingStore } from "../stores/recording";

export function useInitialModelStatus(model: WhisperModel) {
  useEffect(() => {
    let active = true;
    const expected = useRecordingStore.getState().modelStatus[model];
    // Active downloads are reconciled by useModelEvents.
    if (typeof expected === "object" && "downloading" in expected) return;

    void ipc.getModelStatus(model).then((status) => {
      const store = useRecordingStore.getState();
      if (active && store.modelStatus[model] === expected) store.setModelStatus(model, status);
    }).catch(() => {});

    return () => { active = false; };
  }, [model]);
}
