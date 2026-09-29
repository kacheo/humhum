import type { ModelStatus } from "../../lib/types";
import "../../styles/tokens.css";
import "./index.css";

/** Presentational badge for a Whisper model's download status. */
export function ModelStatusBadge({ status }: { status: ModelStatus }) {
  let label: string;
  let mod: string;
  if (status === "ready") {
    label = "Ready";
    mod = "ready";
  } else if (status === "not_downloaded") {
    label = "Not downloaded";
    mod = "idle";
  } else if ("downloading" in status) {
    label = `Downloading ${status.downloading}%`;
    mod = "active";
  } else {
    label = "Error";
    mod = "error";
  }
  return (
    <span className={`model-badge model-badge--${mod}`} role="status">
      {label}
    </span>
  );
}
