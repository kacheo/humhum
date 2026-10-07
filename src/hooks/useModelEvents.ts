import { useEffect } from "react";
import { useTauriEvent } from "./useTauriEvent";
import { useRecordingStore } from "../stores/recording";
import { ipc } from "../lib/ipc";
import type { ModelDownloadProgressPayload, WhisperModel } from "../lib/types";

/**
 * Wire model-download progress into the recording store for the prefs window.
 * The overlay window gets this via useRecordingEvents; the prefs window mounts
 * only this focused listener so the Model section's badges advance live.
 *
 * There is no explicit "download complete" event, so when progress reaches 100%
 * we re-fetch get_model_status to flip the badge from Downloading → Ready (or
 * Error). Handlers read the store through getState() so the host never
 * subscribes.
 */
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
          if (cancelled || useRecordingStore.getState().modelStatus[model] !== status) return;
          if (typeof next === "object" && "downloading" in next) return;
          store.setDownloadProgress(model, 0);
          store.setModelStatus(model, next === "not_downloaded" ? {
            error: "Download did not finish. Check your connection and available disk space, then retry.",
          } : next);
        }).catch(() => {}).finally(() => pending.delete(model));
      }
    }, 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, []);

  useTauriEvent<ModelDownloadProgressPayload>("model-download-progress", (e) => {
    const { model, pct } = e.payload;
    const store = useRecordingStore.getState();
    store.setDownloadProgress(model, pct);
    store.setModelStatus(model, { downloading: pct });
    if (pct >= 100) {
      ipc
        .getModelStatus(model)
        .then((s) => useRecordingStore.getState().setModelStatus(model, s))
        .catch(() => {});
    }
  });
}
