import { useTauriEvent } from "./useTauriEvent";
import { useRecordingStore } from "../stores/recording";
import { ipc } from "../lib/ipc";
import type { ModelDownloadProgressPayload } from "../lib/types";

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
  useTauriEvent<ModelDownloadProgressPayload>("model-download-progress", (e) => {
    const { model, pct } = e.payload;
    useRecordingStore.getState().setDownloadProgress(model, pct);
    if (pct >= 100) {
      ipc
        .getModelStatus(model)
        .then((s) => useRecordingStore.getState().setModelStatus(model, s))
        .catch(() => {});
    }
  });
}
