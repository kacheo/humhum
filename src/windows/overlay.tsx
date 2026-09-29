import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RecordingOverlay } from "../components/RecordingOverlay";
import { useAppBootstrap } from "../hooks/useAppBootstrap";
import { useRecordingEvents } from "../hooks/useRecordingEvents";
import { useStuckPhaseWatchdog } from "../hooks/useStuckPhaseWatchdog";

/** Overlay-window root: hydrate stores and wire the recording pipeline. */
export function OverlayRoot() {
  useAppBootstrap();
  useRecordingEvents();
  useStuckPhaseWatchdog();
  return <RecordingOverlay />;
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <OverlayRoot />
    </StrictMode>
  );
} else if (import.meta.env.MODE !== "test") {
  throw new Error("No #root element found");
}
