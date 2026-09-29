import { useRef, useState } from "react";
import { eventModifiers, eventToCombo, formatHotkey } from "../../lib/hotkey";
import "./index.css";

/** Keystroke recorder. Click to record, then either press a modified key combo
 *  (e.g. Option+Space) or hold a lone modifier as push-to-talk (e.g. hold Option).
 *  A keyed combo commits on key-down; a modifier-only combo commits when the held
 *  modifiers are released. While recording, held modifiers show as a live preview.
 *  On capture it reports the canonical combo via onChange and runs onValidate
 *  (validate_hotkey), showing any conflict/invalid code inline. Escape cancels. */
export function HotkeyPicker({
  value,
  onChange,
  onValidate,
}: {
  value: string;
  onChange: (combo: string) => void;
  onValidate: (combo: string) => Promise<void>;
}) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Live preview of held modifiers while recording (formatted, e.g. "⌥").
  const [preview, setPreview] = useState("");
  // Peak set of modifiers seen this session and whether we've already committed,
  // tracked in refs so keydown/keyup share them without re-render churn.
  const heldMods = useRef<string[]>([]);
  const committed = useRef(false);
  const buttonRef = useRef<HTMLButtonElement>(null);

  function startRecording() {
    heldMods.current = [];
    committed.current = false;
    setPreview("");
    setError(null);
    setRecording(true);
    // macOS WebKit (WKWebView) does not focus a <button> on mouse click, so the
    // onKeyDown/onKeyUp handlers would never fire and recording would hang on
    // "Press keys…". Programmatic focus works in WebKit and routes keys here.
    buttonRef.current?.focus();
  }

  function commit(combo: string) {
    committed.current = true;
    setRecording(false);
    setPreview("");
    setError(null);
    onChange(combo);
    onValidate(combo).catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    e.preventDefault();
    if (e.code === "Escape") {
      setRecording(false);
      setPreview("");
      setError(null);
      return;
    }
    const combo = eventToCombo(e);
    if (combo != null) {
      commit(combo); // a real (non-modifier) key was pressed → keyed combo
      return;
    }
    // Modifier-only so far: remember the largest modifier set and preview it.
    const mods = eventModifiers(e);
    if (mods.length > heldMods.current.length) heldMods.current = mods;
    setPreview(formatHotkey(heldMods.current.join("+"), "mac"));
  }

  function handleKeyUp(e: React.KeyboardEvent) {
    e.preventDefault();
    if (committed.current || heldMods.current.length === 0) return;
    // Modifiers released without a real key → commit them as push-to-talk.
    commit(heldMods.current.join("+"));
  }

  return (
    <div className="hotkey-picker">
      <button
        ref={buttonRef}
        type="button"
        className="hotkey-picker__capture"
        onClick={startRecording}
        onKeyDown={recording ? handleKeyDown : undefined}
        onKeyUp={recording ? handleKeyUp : undefined}
        onBlur={() => setRecording(false)}
      >
        {recording
          ? preview || "Press keys…"
          : formatHotkey(value, "mac") || "Click to record"}
      </button>
      {error && (
        <span className="hotkey-picker__error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
