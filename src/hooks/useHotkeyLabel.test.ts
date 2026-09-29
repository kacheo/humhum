import { act, renderHook } from "@testing-library/react";
import { useHotkeyLabel } from "./useHotkeyLabel";
import { usePrefsStore } from "../stores/prefs";

const initialPrefs = usePrefsStore.getState().prefs;

afterEach(() => {
  usePrefsStore.setState({ prefs: initialPrefs });
});

test("formats the store's hotkey for the mac platform", () => {
  act(() => {
    usePrefsStore.getState().setField("hotkey", "Option+Shift+Space");
  });
  const { result } = renderHook(() => useHotkeyLabel());
  expect(result.current).toBe("⌥⇧Space");
});

test("reflects hotkey changes reactively", () => {
  act(() => {
    usePrefsStore.getState().setField("hotkey", "Cmd+K");
  });
  const { result } = renderHook(() => useHotkeyLabel());
  expect(result.current).toBe("⌘K");

  act(() => {
    usePrefsStore.getState().setField("hotkey", "Ctrl+Space");
  });
  expect(result.current).toBe("⌃Space");
});
