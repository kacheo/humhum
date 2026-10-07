import { render, screen } from "@testing-library/react";
import { beforeEach, vi } from "vitest";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RecordingOverlay } from ".";
import { useRecordingStore } from "../../stores/recording";

// beforeEach resets before every test (state never leaks forward), so no
// afterEach is needed. An afterEach reset would run LIFO — before RTL's
// cleanup — firing a store update on the still-mounted, subscribed overlay
// outside act(), which produces spurious act() warnings.
beforeEach(() => {
  vi.clearAllMocks(); // reset the window show/hide spies per test
  useRecordingStore.getState().reset();
});

test("renders nothing when idle", () => {
  const { container } = render(<RecordingOverlay />);
  expect(container).toBeEmptyDOMElement();
});

test("hides the overlay window when idle (no content)", () => {
  render(<RecordingOverlay />);
  expect(getCurrentWindow().hide).toHaveBeenCalled();
  expect(getCurrentWindow().show).not.toHaveBeenCalled();
});

test("shows the overlay window when it has content", () => {
  useRecordingStore.setState({ phase: "recording" });
  render(<RecordingOverlay />);
  expect(getCurrentWindow().show).toHaveBeenCalled();
});

test("shows the transcribing status", () => {
  useRecordingStore.setState({ phase: "transcribing" });
  render(<RecordingOverlay />);
  expect(screen.getByText("Transcribing…")).toBeInTheDocument();
});

test("shows the cleaning status", () => {
  useRecordingStore.setState({ phase: "cleaning" });
  render(<RecordingOverlay />);
  expect(screen.getByText("Cleaning…")).toBeInTheDocument();
});

test("flashes raw and cleaned transcript while pasting", () => {
  useRecordingStore.setState({ phase: "pasting", rawTranscript: "hello", cleanedTranscript: "Hello." });
  render(<RecordingOverlay />);
  expect(screen.queryByText("hello")).not.toBeInTheDocument();
  expect(screen.getByText("Hello.")).toBeInTheDocument();
});

test("shows the error message in the error phase", () => {
  useRecordingStore.setState({ phase: "error", errorMessage: "no key" });
  render(<RecordingOverlay />);
  expect(screen.getByText("no key")).toBeInTheDocument();
});

test("shows the waveform while recording", () => {
  useRecordingStore.setState({ phase: "recording" });
  const { container } = render(<RecordingOverlay />);
  expect(container.querySelector(".waveform")).toBeInTheDocument();
});
