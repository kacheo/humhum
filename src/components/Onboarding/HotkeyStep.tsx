import { useEffect, useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { ipc } from "../../lib/ipc";
import { HotkeyPicker } from "../HotkeyPicker";

/** Shortcut step. Reuses HotkeyPicker. Validates the current hotkey on mount and
 *  on every capture (validate_hotkey); reports ready only while the combo is valid. */
export function HotkeyStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const hotkey = usePrefsStore((s) => s.prefs.hotkey);
  const setField = usePrefsStore.getState().setField;
  const [valid, setValid] = useState(false);

  useEffect(() => {
    ipc.validateHotkey(hotkey).then(() => setValid(true)).catch(() => setValid(false));
    // mount-only: subsequent changes are validated in `validate` below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => { onReady(valid); }, [valid, onReady]);

  async function validate(combo: string) {
    try { await ipc.validateHotkey(combo); setValid(true); }
    catch (e) { setValid(false); throw e; } // rethrow so HotkeyPicker renders the inline error
  }

  return (
    <section className="onb-step">
      <h2>Shortcut</h2>
      <p>Pick the keys you'll hold to dictate — hold them in any app to start recording.</p>
      <HotkeyPicker
        value={hotkey}
        onChange={(combo) => { setField("hotkey", combo); setValid(false); }}
        onValidate={validate}
      />
    </section>
  );
}
