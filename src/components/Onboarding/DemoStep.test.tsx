import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, vi } from "vitest";

const handlers = vi.hoisted(() => new Map<string, Set<(event: { payload: unknown }) => void>>());
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    const listeners = handlers.get(name) ?? new Set();
    listeners.add(handler); handlers.set(name, listeners);
    return () => listeners.delete(handler);
  }),
}));
import { DemoStep } from "./DemoStep";
import { useRecordingStore } from "../../stores/recording";

function fire(name: string, payload?: unknown) {
  expect(handlers.get(name)?.size).toBeGreaterThan(0);
  act(() => handlers.get(name)!.forEach(handler => handler({ payload })));
}

beforeEach(() => { handlers.clear(); useRecordingStore.getState().reset(); });
afterEach(() => vi.useRealTimers());

test("real recording events show progress and enable completion for a new transcript", async () => {
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  expect(onReady).toHaveBeenLastCalledWith(false);
  await fire("recording-started");
  expect(screen.getByRole("status")).toHaveTextContent("Recording");
  await fire("recording-stopped");
  expect(screen.getByRole("status")).toHaveTextContent("Transcribing");
  await fire("transcript-ready", { raw: "hello world" });
  expect(screen.getByRole("status")).toHaveTextContent("Cleaning");
  await fire("cleanup-done", { text: "Hello world." });
  expect(screen.getByRole("status")).toHaveTextContent("Hello world.");
  expect(onReady).toHaveBeenLastCalledWith(true);
  await fire("paste-failed", { reason: "no_focused_field", textCopiedToClipboard: true });
  expect(onReady).toHaveBeenLastCalledWith(true);
});

test("entering the demo clears an earlier transcript", () => {
  useRecordingStore.getState().setCleanedTranscript("old transcript");
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  expect(screen.queryByText(/old transcript/)).not.toBeInTheDocument();
  expect(onReady).toHaveBeenLastCalledWith(false);
});

test.each(["recording-error", "pipeline-error"])("%s stays visible and a new attempt clears it", async (name) => {
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  vi.useFakeTimers();
  await fire(name, { code: "permission_denied", message: "Microphone permission needed" });
  expect(screen.getByRole("alert")).toHaveTextContent("Microphone permission needed");
  act(() => vi.advanceTimersByTime(5000)); vi.useRealTimers();
  expect(screen.getByRole("alert")).toHaveTextContent("Microphone permission needed");
  await fire("recording-started");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(onReady).toHaveBeenLastCalledWith(false);
});

test("blank speech and cancellation allow retry without finishing", async () => {
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  await fire("recording-started"); await fire("recording-stopped");
  await fire("transcription-discarded");
  expect(screen.getByRole("status")).toHaveTextContent("No speech detected");
  await fire("recording-started"); await fire("recording-cancelled");
  expect(screen.getByRole("status")).toHaveTextContent("Cancelled");
  expect(onReady).toHaveBeenLastCalledWith(false);
});

test("a second attempt clears the previous result", async () => {
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  await fire("recording-started"); await fire("cleanup-done", { text: "previous result" });
  expect(onReady).toHaveBeenLastCalledWith(true);
  await fire("recording-started");
  expect(screen.queryByText(/previous result/)).not.toBeInTheDocument();
  expect(onReady).toHaveBeenLastCalledWith(false);
});

test("Skip for now triggers onSkip", async () => {
  const onSkip = vi.fn();
  render(<DemoStep onReady={() => {}} onSkip={onSkip} />);
  await userEvent.click(screen.getByRole("button", { name: "Skip for now" }));
  expect(onSkip).toHaveBeenCalled();
});

test.each(["recording-error", "recording-cancelled", "transcription-discarded"])("retry progress survives the previous %s timeout", async (name) => {
  render(<DemoStep onReady={() => {}} onSkip={() => {}} />);
  vi.useFakeTimers();
  await fire(name, { code: "permission_denied", message: "Permission needed" });
  await fire("recording-started");
  act(() => vi.advanceTimersByTime(5000)); vi.useRealTimers();
  expect(screen.getByRole("status")).toHaveTextContent("Recording");
});
