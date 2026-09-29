import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";

// Capture each (event, handler) registration instead of touching Tauri, so the
// test can drive handlers directly and assert the resulting store state.
const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("./useTauriEvent", () => ({
  useTauriEvent: (event: string, handler: (e: { payload: unknown }) => void) => {
    handlers.set(event, handler);
  },
}));

import { useRecordingEvents } from "./useRecordingEvents";
import { useRecordingStore } from "../stores/recording";

function fire(event: string, payload?: unknown) {
  const handler = handlers.get(event);
  if (!handler) throw new Error(`no handler registered for ${event}`);
  handler({ payload });
}

beforeEach(() => {
  handlers.clear();
  useRecordingStore.getState().reset();
  renderHook(() => useRecordingEvents());
});

afterEach(() => {
  vi.useRealTimers();
});

test("recording-started / recording-stopped drive phase", () => {
  fire("recording-started");
  expect(useRecordingStore.getState().phase).toBe("recording");
  fire("recording-stopped");
  expect(useRecordingStore.getState().phase).toBe("transcribing");
});

test("transcript-ready sets cleaning phase + raw text", () => {
  fire("transcript-ready", { raw: "hi" });
  const s = useRecordingStore.getState();
  expect(s.phase).toBe("cleaning");
  expect(s.rawTranscript).toBe("hi");
});

test("cleanup-done sets pasting phase + cleaned text", () => {
  fire("cleanup-done", { text: "yo" });
  const s = useRecordingStore.getState();
  expect(s.phase).toBe("pasting");
  expect(s.cleanedTranscript).toBe("yo");
});

test("model-download-progress updates the named model", () => {
  fire("model-download-progress", { model: "base_en", pct: 30 });
  expect(useRecordingStore.getState().downloadProgress.base_en).toBe(30);
});

test("paste-failed moves to done", () => {
  fire("paste-failed", { reason: "no_focused_field", textCopiedToClipboard: true });
  expect(useRecordingStore.getState().phase).toBe("done");
});

test("pipeline-error sets error phase + message", () => {
  fire("pipeline-error", { code: "key_rejected", message: "boom" });
  const s = useRecordingStore.getState();
  expect(s.phase).toBe("error");
  expect(s.errorMessage).toBe("boom");
});

test("recording-cancelled sets cancelled then resets after 200ms", () => {
  vi.useFakeTimers();
  fire("recording-cancelled");
  expect(useRecordingStore.getState().phase).toBe("cancelled");
  vi.advanceTimersByTime(200);
  expect(useRecordingStore.getState().phase).toBe("idle");
});

test("transcription-discarded (blank audio) leaves transcribing and resets to idle", () => {
  vi.useFakeTimers();
  // Reproduce the bug scenario: stopped → transcribing, then blank audio.
  fire("recording-stopped");
  expect(useRecordingStore.getState().phase).toBe("transcribing");
  fire("transcription-discarded");
  expect(useRecordingStore.getState().phase).toBe("cancelled");
  vi.advanceTimersByTime(200);
  expect(useRecordingStore.getState().phase).toBe("idle");
});

test("recording-error flashes the error then auto-dismisses to idle", () => {
  vi.useFakeTimers();
  fire("recording-error", { code: "permission_denied", message: "Permission needed to record" });
  const s = useRecordingStore.getState();
  expect(s.phase).toBe("error");
  expect(s.errorMessage).toBe("Permission needed to record");
  vi.advanceTimersByTime(4000);
  expect(useRecordingStore.getState().phase).toBe("idle");
});
