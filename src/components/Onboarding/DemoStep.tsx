import { useEffect } from "react";
import { useRecordingStore } from "../../stores/recording";
import { useRecordingEvents } from "../../hooks/useRecordingEvents";
import { usePrefsStore } from "../../stores/prefs";
import { formatHotkey } from "../../lib/hotkey";

/** Final step: a guided first dictation. Mounts useRecordingEvents (the prefs
 *  window doesn't otherwise wire pipeline events) and succeeds when a non-empty
 *  cleaned transcript arrives (cleanup-done). "Skip for now" completes onboarding.
 *  Privacy: the transcript is rendered into the local DOM only — never logged. */
export function DemoStep({ onReady, onSkip }: {
  onReady: (ready: boolean) => void; onSkip: () => void;
}) {
  useRecordingEvents();
  const hotkey = usePrefsStore((s) => s.prefs.hotkey);
  const transcript = useRecordingStore((s) => s.cleanedTranscript);

  const done = transcript != null && transcript.length > 0;
  useEffect(() => { onReady(done); }, [done, onReady]);

  return (
    <section className="onb-step">
      <h2>Try it</h2>
      <p>Hold <kbd>{formatHotkey(hotkey, "mac")}</kbd>, say a sentence, then release.</p>
      {done ? (
        <p className="demo-result" role="status">✓ Heard: “{transcript}”</p>
      ) : (
        <p className="demo-waiting" role="status">Waiting for your first dictation…</p>
      )}
      <button type="button" className="demo-skip" onClick={onSkip}>Skip for now</button>
    </section>
  );
}
