import "./WaveformIndicator.css";

const BAR_COUNT = 5;

/** Decorative animated bars shown while recording. Not driven by real audio
 *  amplitude — the frontend never sees PCM. Purely a "we're listening" cue. */
export function WaveformIndicator({ isActive }: { isActive: boolean }) {
  return (
    <div className={`waveform${isActive ? " waveform--active" : ""}`} aria-hidden="true">
      {Array.from({ length: BAR_COUNT }, (_, i) => (
        <span key={i} className="waveform__bar" style={{ animationDelay: `${i * 0.1}s` }} />
      ))}
    </div>
  );
}
