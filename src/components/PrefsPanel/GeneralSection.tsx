import { useEffect, useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { ipc } from "../../lib/ipc";
import { SYSTEM_DEFAULT_INPUT_DEVICE } from "../../lib/types";

export function GeneralSection() {
  const prefs = usePrefsStore((s) => s.prefs);
  const setField = usePrefsStore.getState().setField;
  const [inputDevices, setInputDevices] = useState<string[]>([]);

  useEffect(() => {
    ipc
      .listInputDevices()
      .then(setInputDevices)
      .catch(() => {});
  }, []);

  return (
    <>
      <section className="prefs__section">
        <h2 className="prefs__heading">Startup</h2>
        <label className="prefs__row">
          <span>Launch at login</span>
          <input
            type="checkbox"
            checked={prefs.launchAtLogin}
            onChange={(e) => setField("launchAtLogin", e.target.checked)}
          />
        </label>
      </section>

      <section className="prefs__section">
        <h2 className="prefs__heading">Audio</h2>
        <label className="prefs__row">
          <span>Input device</span>
          <select
            value={prefs.preferredInputDevice ?? ""}
            title={prefs.preferredInputDevice === SYSTEM_DEFAULT_INPUT_DEVICE
              ? "System Default"
              : prefs.preferredInputDevice ?? "Auto (avoid Bluetooth mic switching)"}
            aria-describedby="prefs-audio-hint"
            onChange={(e) => setField("preferredInputDevice", e.target.value || null)}
          >
            <option value="">Auto (avoid Bluetooth mic switching)</option>
            <option value={SYSTEM_DEFAULT_INPUT_DEVICE}>System Default</option>
            {inputDevices.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <p className="prefs__hint" id="prefs-audio-hint">
          Auto prefers a built-in or wired microphone when available to preserve Bluetooth playback quality.
        </p>
        <p className="prefs__hint">Recordings automatically stop after five minutes for safety.</p>
      </section>
    </>
  );
}
