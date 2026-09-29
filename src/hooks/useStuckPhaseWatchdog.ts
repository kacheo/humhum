import { useEffect } from "react";
import { useRecordingStore } from "../stores/recording";
import type { RecordingPhase } from "../lib/types";

/**
 * Last-resort self-heal for the overlay. The `transcribing`/`cleaning`/`pasting`
 * phases leave only when their follow-up backend event arrives; if one is ever
 * dropped (or a backend step wedges), the overlay would stick forever. This arms
 * a timer whenever we enter a busy phase and force-resets to idle if we're still
 * there when it fires.
 *
 * Backend step timeouts (transcribe/paste) mean this should never fire in
 * practice — it's the belt-and-suspenders catch-all for the whole class of
 * "stuck waiting on an event" bugs. `recording` is intentionally excluded: it's
 * bounded by the user's hold plus the backend max-duration safety cap.
 */
const BUSY_PHASES: readonly RecordingPhase[] = ["transcribing", "cleaning", "pasting"];

/**
 * Past the backend transcribe-timeout floor (180s) so the common short-recording
 * case always lets the backend drive the next transition first; this only catches
 * a truly stuck overlay. For an unusually long recording the backend budget scales
 * with audio length and can exceed this, so the watchdog may fire first — but that
 * is non-destructive: the pipeline still pastes, and a late `transcript-ready`
 * simply re-renders the overlay. It exists for the rare dropped-event case.
 */
const WATCHDOG_MS = 200_000;

export function useStuckPhaseWatchdog(): void {
  const phase = useRecordingStore((s) => s.phase);
  useEffect(() => {
    if (!BUSY_PHASES.includes(phase)) return;
    const timer = setTimeout(() => {
      // Re-check at fire time: only act if still stuck in a busy phase (a normal
      // transition would have re-run this effect and cleared the timer).
      if (BUSY_PHASES.includes(useRecordingStore.getState().phase)) {
        useRecordingStore.getState().reset();
      }
    }, WATCHDOG_MS);
    return () => clearTimeout(timer);
  }, [phase]);
}
