import { useEffect } from "react";
import { useTauriEvent } from "./useTauriEvent";
import { useRecordingStore } from "../stores/recording";
import { ipc } from "../lib/ipc";
import type { ModelDownloadProgressPayload, ModelStatus, WhisperModel } from "../lib/types";

const DOWNLOAD_ERROR = "Download did not finish. Check your connection and available disk space, then retry.";

// A stale response must never overwrite a later progress event or retry.
function reconcile(model: WhisperModel, expected: ModelStatus, next: ModelStatus) {
  const store = useRecordingStore.getState();
  if (store.modelStatus[model] !== expected) return;
  if (typeof next === "object" && "downloading" in next) return;
  store.setDownloadProgress(model, 0);
  store.setModelStatus(model, next === "not_downloaded" ? { error: DOWNLOAD_ERROR } : next);
}

/** Track progress and reconcile terminal success/failure against backend status. */
export function useModelEvents(): void {
  // The command starts a background task; a later failure does not reject IPC.
  // Reconcile active downloads against the backend's in-flight registry/disk.
  useEffect(() => {
    let cancelled = false;
    const pending = new Set<WhisperModel>();
    const timer = setInterval(() => {
      const store = useRecordingStore.getState();
      for (const model of ["base_en", "small_en"] as const) {
        const status = store.modelStatus[model];
        const active = typeof status === "object" && "downloading" in status;
        if (!active || pending.has(model)) continue;
        pending.add(model);
        void ipc.getModelStatus(model).then((next) => {
          if (!cancelled) reconcile(model, status, next);
        }).catch(() => {}).finally(() => pending.delete(model));
      }
    }, 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, []);

  useTauriEvent<ModelDownloadProgressPayload>("model-download-progress", (e) => {
    const { model, pct } = e.payload;
    const store = useRecordingStore.getState();
    store.setDownloadProgress(model, pct);
    const status = { downloading: pct };
    store.setModelStatus(model, status);
    if (pct >= 100) {
      ipc
        .getModelStatus(model)
        .then((next) => reconcile(model, status, next))
        .catch(() => {});
    }
  });
}
