import { useTauriEvent } from "./useTauriEvent";
import { useRecordingStore } from "../stores/recording";
import type {
  CleanupDonePayload,
  ModelDownloadProgressPayload,
  PipelineErrorPayload,
  TranscriptReadyPayload,
} from "../lib/types";

/**
 * Wire the Rust recording pipeline events into the recording store, per the
 * LLD §5 event→action table. Handlers read actions through `getState()` so the
 * hosting component never subscribes (no re-renders on store changes). Mounted
 * in the overlay window root. Several events are wired ahead of their Rust
 * emitters (pipeline phases land in later phases); this is intentional.
 *
 * Privacy: transcript/cleaned text is placed into the in-memory store only —
 * never logged.
 */
export function useRecordingEvents(): void {
  useTauriEvent<void>("recording-started", () =>
    useRecordingStore.getState().setPhase("recording")
  );
  useTauriEvent<void>("recording-stopped", () =>
    useRecordingStore.getState().setPhase("transcribing")
  );
  useTauriEvent<void>("recording-cancelled", () => {
    useRecordingStore.getState().setPhase("cancelled");
    // Not unmount-guarded: harmless — the store is module-global and resetting an idle store is a no-op.
    setTimeout(() => {
      // A retry may already have started before this dismissal fires.
      if (useRecordingStore.getState().phase === "cancelled") useRecordingStore.getState().reset();
    }, 200);
  });
  // Blank audio: the backend transcribed silence and returned to idle without a
  // transcript. There's no follow-up pipeline event to leave `transcribing`, so
  // this drives the overlay back to hidden/idle (mirrors recording-cancelled).
  useTauriEvent<void>("transcription-discarded", () => {
    useRecordingStore.getState().setPhase("cancelled");
    setTimeout(() => {
      // A retry may already have started before this dismissal fires.
      if (useRecordingStore.getState().phase === "cancelled") useRecordingStore.getState().reset();
    }, 200);
  });
  useTauriEvent<TranscriptReadyPayload>("transcript-ready", (e) => {
    const s = useRecordingStore.getState();
    s.setPhase("cleaning");
    s.setRawTranscript(e.payload.raw);
  });
  useTauriEvent<CleanupDonePayload>("cleanup-done", (e) => {
    const s = useRecordingStore.getState();
    s.setPhase("pasting");
    s.setCleanedTranscript(e.payload.text);
  });
  useTauriEvent<ModelDownloadProgressPayload>("model-download-progress", (e) =>
    useRecordingStore.getState().setDownloadProgress(e.payload.model, e.payload.pct)
  );
  useTauriEvent<void>("paste-failed", () =>
    useRecordingStore.getState().setPhase("done")
  );
  useTauriEvent<PipelineErrorPayload>("pipeline-error", (e) => {
    const s = useRecordingStore.getState();
    s.setPhase("error");
    s.setError(e.payload.message);
  });
  // A recording attempt blocked before it started (e.g. a missing permission).
  // Flash the reason in the overlay, then auto-dismiss back to idle since no
  // recording is in flight to drive the next transition.
  useTauriEvent<PipelineErrorPayload>("recording-error", (e) => {
    const s = useRecordingStore.getState();
    s.setPhase("error");
    s.setError(e.payload.message);
    setTimeout(() => {
      if (useRecordingStore.getState().phase === "error") useRecordingStore.getState().reset();
    }, 4000);
  });
}
