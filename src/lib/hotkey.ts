// Pure hotkey-combo formatter. Stored combos are "+"-joined tokens like
// "Option+Shift+Space" (see defaultPrefs.hotkey). Mac-first today, but the
// platform parameter keeps the mapping free of mac-only assumptions so a
// Windows table can be switched on later without touching callers.

export type Platform = "mac" | "windows";

// Modifier token (lowercased) → display symbol, per platform. Synonyms collapse
// to one canonical modifier (Option/Opt/Alt; Cmd/Command/Meta/Super; Ctrl/Control).
const MAC_MODIFIERS: Record<string, string> = {
  option: "⌥",
  opt: "⌥",
  alt: "⌥",
  shift: "⇧",
  cmd: "⌘",
  command: "⌘",
  meta: "⌘",
  super: "⌘",
  ctrl: "⌃",
  control: "⌃",
};

const WINDOWS_MODIFIERS: Record<string, string> = {
  option: "Alt",
  opt: "Alt",
  alt: "Alt",
  shift: "Shift",
  cmd: "Win",
  command: "Win",
  meta: "Win",
  super: "Win",
  ctrl: "Ctrl",
  control: "Ctrl",
};

/**
 * Render a hotkey combo for display. On macOS modifiers become glyphs joined
 * with no separator (`⌥⇧Space`); on Windows they are spelled and joined by `+`
 * (`Alt+Shift+Space`). Non-modifier tokens pass through, with single letters
 * uppercased and named keys (Space, Enter, …) left as written.
 */
export function formatHotkey(combo: string, platform: Platform): string {
  if (!combo) return "";
  const modifiers = platform === "mac" ? MAC_MODIFIERS : WINDOWS_MODIFIERS;
  const separator = platform === "mac" ? "" : "+";

  // Drop empty tokens so a malformed combo with a trailing/double separator
  // (e.g. "Cmd+") doesn't leave a dangling separator. A literal "+" key is
  // unrepresentable here because "+" is itself the delimiter — inherent to the
  // combo storage format, not handled by this formatter.
  const tokens = combo
    .split("+")
    .map((raw) => raw.trim())
    .filter((token) => token.length > 0)
    .map((token) => {
      const mod = modifiers[token.toLowerCase()];
      if (mod) return mod;
      // Single-character key → uppercase (e.g. "a" → "A"); named keys unchanged.
      return token.length === 1 ? token.toUpperCase() : token;
    });

  return tokens.join(separator);
}

// Physical key code → canonical combo key name. Modifier-only codes
// (ShiftLeft, MetaRight, AltLeft, ControlLeft, …) are absent, so codeToKey
// returns null for them and the recorder keeps waiting for a real key.
const CODE_TO_KEY: Record<string, string> = {
  Space: "Space",
  Enter: "Return",
  Tab: "Tab",
  Backspace: "Delete",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
};

function codeToKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-2])$/.test(code)) return code;
  return CODE_TO_KEY[code] ?? null;
}

/**
 * Build a canonical "Mod+Mod+Key" combo from a keydown-like event, or null
 * when only modifier keys are held (the caller keeps waiting for a real key).
 * Modifier order is fixed (Cmd, Ctrl, Option, Shift) so the same keystroke
 * always yields the same string; validate_hotkey parses order-independently.
 */
export function eventToCombo(e: {
  code: string;
  key: string;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}): string | null {
  const key = codeToKey(e.code);
  if (key == null) return null;
  return [...eventModifiers(e), key].join("+");
}

/**
 * The modifiers currently held in a keyboard event, in canonical order
 * (Cmd, Ctrl, Option, Shift) so the same keystroke always yields the same list.
 * Used to drive the live recording preview and to commit a modifier-only combo
 * (e.g. hold Option as push-to-talk) when no non-modifier key is pressed.
 */
export function eventModifiers(e: {
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}): string[] {
  const mods: string[] = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Option");
  if (e.shiftKey) mods.push("Shift");
  return mods;
}
