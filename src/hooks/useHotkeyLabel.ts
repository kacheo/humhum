import { usePrefsStore } from "../stores/prefs";
import { formatHotkey } from "../lib/hotkey";

const PLATFORM = "mac" as const; // Mac-first; flip when Windows ships.

/** The configured hotkey, formatted for display (e.g. "⌥⇧Space"). */
export function useHotkeyLabel(): string {
  const hotkey = usePrefsStore((s) => s.prefs.hotkey);
  return formatHotkey(hotkey, PLATFORM);
}
