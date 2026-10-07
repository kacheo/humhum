import { useEffect } from "react";
import { ipc } from "../../lib/ipc";
import { useRecordingStore } from "../../stores/recording";
import { ModelStatusBadge } from "../ModelStatusBadge";
import type { ModelStatus } from "../../lib/types";

/** Model step. Fetches base_en status on mount; live progress rides the
 *  model-download-progress event (useModelEvents is mounted in PrefsRoot).
 *  Ready only once base_en is "ready". */
export function ModelDownloadStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const status = useRecordingStore((s) => s.modelStatus.base_en);
  const pct = useRecordingStore((s) => s.downloadProgress.base_en);

  useEffect(() => {
    ipc.getModelStatus("base_en")
      .then((st) => useRecordingStore.getState().setModelStatus("base_en", st))
      .catch(() => {});
  }, []);

  const ready = status === "ready";
  useEffect(() => { onReady(ready); }, [ready, onReady]);

  // An error status must win over stale progress: once a download fails,
  // `downloadProgress` is never reset, so `pct > 0` would otherwise pin the
  // badge to "Downloading" and keep the retry button disabled forever.
  const isError = typeof status === "object" && "error" in status;
  const effective: ModelStatus =
    status === "ready" ? "ready"
    : isError ? status
    : pct > 0 ? { downloading: pct }
    : status;
  const downloading = typeof effective === "object" && "downloading" in effective;

  return (
    <section className="onb-step">
      <h2>Model</h2>
      <p>Download the Base (English) speech model. It runs fully on-device.</p>
      <div className="onb-row">
        <span>Base (English)</span>
        <ModelStatusBadge status={effective} />
        <button type="button" disabled={ready || downloading}
          onClick={() => {
            useRecordingStore.getState().setDownloadProgress("base_en", 0);
            useRecordingStore.getState().setModelStatus("base_en", { downloading: 0 });
            void ipc.downloadModel("base_en").catch(() => {
              useRecordingStore.getState().setModelStatus("base_en", { error: "Could not start the download. Try again." });
            });
          }}>
          Download
        </button>
      </div>
    </section>
  );
}
