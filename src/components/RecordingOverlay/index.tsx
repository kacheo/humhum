import { useCallback } from "react";
import { useRecordingStore } from "../../stores/recording";
import { useOverlayVisibility } from "../../hooks/useOverlayVisibility";
import { Icon } from "../Icon";
import { WaveformIndicator } from "./WaveformIndicator";
import { TranscriptFlash } from "./TranscriptFlash";
import "../../styles/tokens.css";
import "./index.css";

const phaseLabels = {
  recording: "Recording…",
  transcribing: "Transcribing…",
  cleaning: "Cleaning…",
} as const;

export function RecordingOverlay() {
  const phase = useRecordingStore((s) => s.phase);
  const rawTranscript = useRecordingStore((s) => s.rawTranscript);
  const cleanedTranscript = useRecordingStore((s) => s.cleanedTranscript);
  const errorMessage = useRecordingStore((s) => s.errorMessage);
  const handleFlashDone = useCallback(() => useRecordingStore.getState().reset(), []);

  const visible = phase !== "idle" && phase !== "cancelled";
  useOverlayVisibility(visible);
  if (!visible) return null;

  const isPreview = phase === "pasting" || phase === "done";
  const isError = phase === "error";

  return (
    <div className={`overlay overlay--${phase}`}>
      <div className="overlay__indicator" aria-hidden="true">
        {phase === "recording" ? <WaveformIndicator isActive /> : (
          <Icon name={isError ? "warning" : isPreview ? "check" : phase === "cleaning" ? "sparkles" : "mic"} />
        )}
      </div>
      <div className="overlay__content">
        {isError ? (
          <div className="overlay__error" role="alert">{errorMessage ?? "Something went wrong."}</div>
        ) : isPreview ? (
          <TranscriptFlash raw={rawTranscript ?? ""} cleaned={cleanedTranscript ?? rawTranscript ?? ""} onDone={handleFlashDone} />
        ) : (
          <div className="overlay__status" role="status">{phaseLabels[phase]}</div>
        )}
      </div>
    </div>
  );
}
