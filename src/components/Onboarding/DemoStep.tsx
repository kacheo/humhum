import { useEffect, useState } from "react";
import { useRecordingStore } from "../../stores/recording";
import { useRecordingEvents } from "../../hooks/useRecordingEvents";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { usePrefsStore } from "../../stores/prefs";
import type { CleanupDonePayload, PipelineErrorPayload } from "../../lib/types";
import { formatHotkey } from "../../lib/hotkey";

type Feedback = { kind: "error" | "notice"; message: string };

/** Guided first dictation using the normal recording pipeline. Keep this
 * session's result/errors locally so overlay resets cannot erase the feedback.
 * Transcript text stays in memory/the local DOM and is never logged. */
export function DemoStep({ onReady, onSkip }: {
  onReady: (ready: boolean) => void; onSkip: () => void;
}) {
  useRecordingEvents();
  const hotkey = usePrefsStore((s) => s.prefs.hotkey);
  const phase = useRecordingStore((s) => s.phase);
  const [transcript, setTranscript] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);

  useEffect(() => {
    // Do not clear model-download state, which shares this store.
    useRecordingStore.setState({ phase: "idle", rawTranscript: null, cleanedTranscript: null, errorMessage: null });
  }, []);
  useTauriEvent<void>("recording-started", () => {
    setTranscript(null);
    setFeedback(null);
    useRecordingStore.setState({ rawTranscript: null, cleanedTranscript: null, errorMessage: null });
  });
  useTauriEvent<CleanupDonePayload>("cleanup-done", (event) => setTranscript(event.payload.text));
  useTauriEvent<PipelineErrorPayload>("recording-error", (event) => setFeedback({ kind: "error", message: event.payload.message }));
  useTauriEvent<PipelineErrorPayload>("pipeline-error", (event) => setFeedback({ kind: "error", message: event.payload.message }));
  useTauriEvent<void>("transcription-discarded", () => setFeedback({ kind: "notice", message: "No speech detected. Hold your shortcut and try another sentence." }));
  useTauriEvent<void>("recording-cancelled", () => setFeedback({ kind: "notice", message: "Cancelled. Hold your shortcut to try again." }));

  const done = transcript != null && transcript.trim().length > 0;
  useEffect(() => { onReady(done); }, [done, onReady]);
  const progress = phase === "recording" ? "Recording… Release your shortcut to transcribe."
    : phase === "transcribing" ? "Transcribing…"
    : phase === "cleaning" ? "Cleaning up your transcript…"
    : feedback?.kind === "notice" ? feedback.message
    : feedback?.kind === "error" ? "Hold your shortcut to try again, or go Back to check setup."
    : "Waiting for your first dictation…";

  return (
    <section className="onb-step">
      <h2>Try it</h2>
      <p>Hold <kbd>{formatHotkey(hotkey, "mac")}</kbd>, say a sentence, then release.</p>
      {feedback?.kind === "error" && <p className="onb-error" role="alert">{feedback.message}</p>}
      {done ? (
        <p className="demo-result" role="status">✓ Heard: “{transcript}”</p>
      ) : (
        <p className="demo-waiting" role="status">{progress}</p>
      )}
      <button type="button" className="demo-skip" onClick={onSkip}>Skip for now</button>
    </section>
  );
}
