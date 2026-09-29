import { useCallback } from "react";
import { useRecordingStore } from "../../stores/recording";
import { useOverlayVisibility } from "../../hooks/useOverlayVisibility";
import { WaveformIndicator } from "./WaveformIndicator";
import { TranscriptFlash } from "./TranscriptFlash";
import "./index.css";

export function RecordingOverlay() {
  const phase = useRecordingStore((s) => s.phase);
  const rawTranscript = useRecordingStore((s) => s.rawTranscript);
  const cleanedTranscript = useRecordingStore((s) => s.cleanedTranscript);
  const errorMessage = useRecordingStore((s) => s.errorMessage);
  const handleFlashDone = useCallback(() => useRecordingStore.getState().reset(), []);

  let body: React.ReactNode = null;
  switch (phase) {
    case "recording":
      body = <div className="overlay__row"><WaveformIndicator isActive /></div>;
      break;
    case "transcribing":
      body = <div className="overlay__status">Transcribing…</div>;
      break;
    case "cleaning":
      body = <div className="overlay__status">Cleaning…</div>;
      break;
    case "pasting":
    case "done":
      body = <TranscriptFlash raw={rawTranscript ?? ""} cleaned={cleanedTranscript ?? rawTranscript ?? ""} onDone={handleFlashDone} />;
      break;
    case "error":
      body = <div className="overlay__error">{errorMessage ?? "Something went wrong."}</div>;
      break;
  }

  useOverlayVisibility(body !== null);
  return body ? <div className="overlay">{body}</div> : null;
}
