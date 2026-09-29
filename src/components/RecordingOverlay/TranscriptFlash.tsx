import { useEffect, useRef } from "react";
import "./TranscriptFlash.css";

/** Briefly shows the raw and cleaned transcript side by side, fades, then
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
    <div className="transcript-flash">
      <div className="transcript-flash__col">
        <span className="transcript-flash__label">Heard</span>
        <p className="transcript-flash__text">{raw}</p>
      </div>
      <div className="transcript-flash__col">
        <span className="transcript-flash__label">Cleaned</span>
        <p className="transcript-flash__text">{cleaned}</p>
      </div>
    </div>
  );
}
