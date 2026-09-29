import type { ProbeResult } from "../../hooks/useAsyncProbe";
import "../../styles/tokens.css";
import "./index.css";

/** Shared presentational result line for probe-and-report widgets
 *  `role="status"` (polite); pass `role="alert"` for assertive announcements. */
export function ProbeStatus({
  result,
  role = "status",
}: {
  result: ProbeResult | null;
  role?: "status" | "alert";
}) {
  if (!result) return null;
  return (
    <span className={`probe-status${result.ok ? "" : " probe-status--error"}`} role={role}>
      {result.msg}
    </span>
  );
}
