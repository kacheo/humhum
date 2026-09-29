import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

vi.mock("../../hooks/useRecordingEvents", () => ({ useRecordingEvents: () => {} }));
import { DemoStep } from "./DemoStep";
import { useRecordingStore } from "../../stores/recording";

beforeEach(() => { useRecordingStore.getState().reset(); });

test("not ready until a transcript arrives, then renders it", async () => {
  const onReady = vi.fn();
  render(<DemoStep onReady={onReady} onSkip={() => {}} />);
  expect(onReady).toHaveBeenLastCalledWith(false);
  act(() => useRecordingStore.getState().setCleanedTranscript("hello world this is a test"));
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
  expect(screen.getByRole("status")).toHaveTextContent("hello world this is a test");
});

test("Skip for now triggers onSkip", async () => {
  const onSkip = vi.fn();
  render(<DemoStep onReady={() => {}} onSkip={onSkip} />);
  await userEvent.click(screen.getByRole("button", { name: "Skip for now" }));
  expect(onSkip).toHaveBeenCalled();
});
