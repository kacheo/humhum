import { renderHook } from "@testing-library/react";
import { beforeEach, vi } from "vitest";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useOverlayVisibility } from "./useOverlayVisibility";

const win = getCurrentWindow();
beforeEach(() => vi.clearAllMocks());

test("hides when not visible and shows when visible", () => {
  const { rerender } = renderHook(({ v }) => useOverlayVisibility(v), { initialProps: { v: false } });
  expect(win.hide).toHaveBeenCalledTimes(1);
  expect(win.show).not.toHaveBeenCalled();
  rerender({ v: true });
  expect(win.show).toHaveBeenCalledTimes(1);
});

test("does not re-call on an unchanged value", () => {
  const { rerender } = renderHook(({ v }) => useOverlayVisibility(v), { initialProps: { v: true } });
  expect(win.show).toHaveBeenCalledTimes(1);
  rerender({ v: true });
  expect(win.show).toHaveBeenCalledTimes(1); // effect keyed on `visible`
});
