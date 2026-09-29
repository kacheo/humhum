import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { useStuckPhaseWatchdog } from "./useStuckPhaseWatchdog";
import { useRecordingStore } from "../stores/recording";

beforeEach(() => {
  useRecordingStore.getState().reset();
});

afterEach(() => {
  vi.useRealTimers();
});

test("a stuck busy phase auto-resets to idle after the watchdog window", () => {
  vi.useFakeTimers();
  act(() => useRecordingStore.setState({ phase: "transcribing" }));
  renderHook(() => useStuckPhaseWatchdog());

  // Still stuck just before the window — no reset yet.
  act(() => vi.advanceTimersByTime(199_000));
  expect(useRecordingStore.getState().phase).toBe("transcribing");

  act(() => vi.advanceTimersByTime(2_000));
  expect(useRecordingStore.getState().phase).toBe("idle");
});

test("a normal transition out of the busy phase cancels the watchdog", () => {
  vi.useFakeTimers();
  act(() => useRecordingStore.setState({ phase: "transcribing" }));
  const { rerender } = renderHook(() => useStuckPhaseWatchdog());

  // Progress normally to a terminal phase well before the window elapses.
  act(() => useRecordingStore.setState({ phase: "done" }));
  rerender();

  act(() => vi.advanceTimersByTime(500_000));
  // The watchdog must not have clobbered the (terminal) phase.
  expect(useRecordingStore.getState().phase).toBe("done");
});

test("idle is not a busy phase — no watchdog armed", () => {
  vi.useFakeTimers();
  act(() => useRecordingStore.setState({ phase: "idle" }));
  renderHook(() => useStuckPhaseWatchdog());
  act(() => vi.advanceTimersByTime(500_000));
  expect(useRecordingStore.getState().phase).toBe("idle");
});
