import { eventModifiers, eventToCombo, formatHotkey } from "./hotkey";

test("mac renders modifier glyphs with no separators", () => {
  expect(formatHotkey("Option+Shift+Space", "mac")).toBe("⌥⇧Space");
  expect(formatHotkey("Cmd+Ctrl+K", "mac")).toBe("⌘⌃K");
});

test("windows spells modifiers joined by +", () => {
  expect(formatHotkey("Option+Shift+Space", "windows")).toBe("Alt+Shift+Space");
  expect(formatHotkey("Cmd+Ctrl+K", "windows")).toBe("Win+Ctrl+K");
});

test("single letters uppercase; named keys pass through; empty is empty", () => {
  expect(formatHotkey("option+a", "mac")).toBe("⌥A");
  expect(formatHotkey("", "mac")).toBe("");
});

test("trailing separator produces no dangling separator", () => {
  expect(formatHotkey("Cmd+", "windows")).toBe("Win");
  expect(formatHotkey("Cmd+", "mac")).toBe("⌘");
});

test("builds a combo from modifiers + key", () => {
  expect(eventToCombo({ code: "Space", key: " ", altKey: true, shiftKey: true, metaKey: false, ctrlKey: false })).toBe("Option+Shift+Space");
  expect(eventToCombo({ code: "KeyA", key: "a", altKey: false, shiftKey: false, metaKey: true, ctrlKey: false })).toBe("Cmd+A");
});

test("returns null for a modifier-only press", () => {
  expect(eventToCombo({ code: "ShiftLeft", key: "Shift", altKey: false, shiftKey: true, metaKey: false, ctrlKey: false })).toBeNull();
});

test("eventModifiers lists held modifiers in canonical order", () => {
  expect(eventModifiers({ altKey: true, shiftKey: true, metaKey: true, ctrlKey: true })).toEqual(["Cmd", "Ctrl", "Option", "Shift"]);
  expect(eventModifiers({ altKey: true, shiftKey: false, metaKey: false, ctrlKey: false })).toEqual(["Option"]);
  expect(eventModifiers({ altKey: false, shiftKey: false, metaKey: false, ctrlKey: false })).toEqual([]);
});

test("formats a modifier-only combo", () => {
  expect(formatHotkey("Option", "mac")).toBe("⌥");
  expect(formatHotkey("Option+Shift", "mac")).toBe("⌥⇧");
});
