import { useEffect, useRef } from "react";
import "./TranscriptFlash.css";

/** Briefly shows the final transcript, fades, then
 *  calls onDone after 2s (the Done → Idle transition). Renders text into the
 *  local overlay only — never logged or emitted. */
export function TranscriptFlash({
  raw,
  cleaned,
  onDone,
}: {
  raw: string;
  cleaned: string;
  onDone?: () => void;
}) {
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    const id = setTimeout(() => onDoneRef.current?.(), 2000);
    return () => clearTimeout(id);
  }, []);

  return (
    <div className="transcript-flash" role="status">
      <span className="transcript-flash__label">Transcript</span>
      <p className="transcript-flash__text">{cleaned || raw}</p>
    </div>
  );
}
