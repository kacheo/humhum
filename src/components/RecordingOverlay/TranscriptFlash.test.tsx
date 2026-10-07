import { render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import { TranscriptFlash } from "./TranscriptFlash";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

test("shows only the final transcript preview", () => {
  render(<TranscriptFlash raw="hello world" cleaned="Hello, world." />);
  expect(screen.queryByText("hello world")).not.toBeInTheDocument();
  expect(screen.getByText("Hello, world.")).toBeInTheDocument();
});

test("calls onDone after 2s", () => {
  const onDone = vi.fn();
  render(<TranscriptFlash raw="a" cleaned="b" onDone={onDone} />);
  expect(onDone).not.toHaveBeenCalled();
  act(() => { vi.advanceTimersByTime(2000); });
  expect(onDone).toHaveBeenCalledTimes(1);
});

test("never logs transcript text to the console (privacy)", () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  render(<TranscriptFlash raw="secret words" cleaned="Secret words." />);
  const all = [...log.mock.calls, ...warn.mock.calls, ...err.mock.calls].flat().join(" ");
  expect(all).not.toContain("secret words");
  log.mockRestore(); warn.mockRestore(); err.mockRestore();
});
