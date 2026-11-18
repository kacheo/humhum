import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { CleanupUsageSection } from "./CleanupUsageSection";
import { ipc } from "../../lib/ipc";
import type { CleanupUsage } from "../../lib/types";

const { listen } = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("../../lib/ipc", () => ({ ipc: { getCleanupUsage: vi.fn() } }));

const unreported = { inputTokens: null, outputTokens: null };
const exact = "18446744073709551615";
let current: string;
let usageEvent: () => void;

function response(month = current): CleanupUsage {
  return {
    month,
    currentMonth: current,
    firstMonth: "2023-12",
    days: month === "2024-02" ? {
      "2024-02-01": { inputTokens: exact, outputTokens: "999950" },
      "2024-02-02": unreported,
      "2024-02-29": { inputTokens: null, outputTokens: "0" },
    } : {},
    totals: month === "2024-02" ? { inputTokens: exact, outputTokens: "999950" } : unreported,
    lifetime: { inputTokens: exact, outputTokens: "1000000" },
  };
}

beforeEach(() => {
  current = "2024-02";
  listen.mockImplementation(async (_event, handler) => {
    usageEvent = handler;
    return vi.fn();
  });
  vi.mocked(ipc.getCleanupUsage).mockImplementation(async (month) => response(month));
});

afterEach(() => vi.useRealTimers());

const button = (name: string) => screen.getByRole("button", { name });
const monthLabel = (name: string) => screen.findByRole("heading", { name });

test("defaults to this month and exposes keyboard-selectable exact counts", async () => {
  render(<CleanupUsageSection />);
  await monthLabel("February 2024");
  expect(button("Next month")).toBeDisabled();
  expect(within(screen.getByRole("grid")).getAllByRole("button")).toHaveLength(29);
  expect(screen.getAllByText("18.4 quintillion")[0]).toBeVisible();
  expect(screen.getAllByText("1M")[0]).toBeVisible();
  const allTime = screen.getByText("All-time usage").closest("details");
  expect(allTime).not.toHaveAttribute("open");
  const first = screen.getByRole("button", { name: /February 1, 2024.*18,446,744,073,709,551,615/ });
  act(() => first.focus());
  fireEvent.click(first);
  expect(screen.getByRole("status")).toHaveTextContent("18,446,744,073,709,551,615");
  fireEvent.keyDown(first, { key: "ArrowRight" });
  expect(screen.getByRole("button", { name: /February 2, 2024.*No counts reported/ })).toHaveFocus();
  expect(screen.getByRole("status")).toHaveTextContent("No counts reported");
  fireEvent.click(screen.getByRole("button", { name: /February 29, 2024.*Output 0/ }));
  expect(screen.getByRole("status")).toHaveTextContent("Output 0");
  fireEvent.click(screen.getByRole("button", { name: /February 3, 2024.*No cleanup/ }));
  expect(screen.getByRole("status")).toHaveTextContent("No cleanup");
});

test("browses empty months and stays in history across focus, usage and rollover refreshes", async () => {
  render(<CleanupUsageSection />);
  await monthLabel("February 2024");
  fireEvent.click(button("Previous month"));
  await monthLabel("January 2024");
  expect(await screen.findByText("No cleanup this month")).toBeVisible();
  fireEvent.click(button("Previous month"));
  await monthLabel("December 2023");
  expect(button("Previous month")).toBeDisabled();

  current = "2024-03";
  await act(async () => usageEvent());
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(ipc.getCleanupUsage).toHaveBeenLastCalledWith("2023-12");
  expect(screen.getByRole("heading", { name: "December 2023" })).toBeVisible();
  fireEvent.click(button("This month"));
  await monthLabel("March 2024");
  current = "2024-04";
  await act(async () => usageEvent());
  await monthLabel("April 2024");
  expect(button("Next month")).toBeDisabled();
});

test("refreshes at midnight while Preferences stays open", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2024, 1, 29, 23, 59, 59, 900));
  render(<CleanupUsageSection />);
  await act(async () => {});
  expect(screen.getByRole("heading", { name: "February 2024" })).toBeVisible();
  current = "2024-03";
  await act(async () => vi.advanceTimersByTimeAsync(100));
  expect(screen.getByRole("heading", { name: "March 2024" })).toBeVisible();
});

test("ignores a slow historical response after navigating to another month", async () => {
  render(<CleanupUsageSection />);
  await monthLabel("February 2024");
  let finish!: (value: CleanupUsage) => void;
  vi.mocked(ipc.getCleanupUsage).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  fireEvent.click(button("Previous month"));
  fireEvent.click(button("Previous month"));
  await monthLabel("December 2023");
  await act(async () => finish(response("2024-01")));
  expect(screen.getByRole("heading", { name: "December 2023" })).toBeVisible();
});

test("shows usage unavailable when storage cannot be read", async () => {
  vi.mocked(ipc.getCleanupUsage).mockRejectedValueOnce(new Error("Unreadable usage"));
  render(<CleanupUsageSection />);
  expect(await screen.findByText("Usage unavailable")).toBeVisible();
});

test("keeps legacy totals available while waiting for the first tracked cleanup", async () => {
  vi.mocked(ipc.getCleanupUsage).mockResolvedValueOnce({
    ...response(),
    firstMonth: null,
    days: {},
    totals: unreported,
  });
  render(<CleanupUsageSection />);
  expect(await screen.findByText("Daily usage starts with your next cleanup.")).toBeVisible();
  expect(screen.queryByRole("grid")).not.toBeInTheDocument();
  expect(button("Previous month")).toBeDisabled();
  const lifetime = screen.getByText("All-time usage").closest("details")!;
  expect(lifetime).not.toHaveAttribute("open");
  fireEvent.click(within(lifetime).getByText("All-time usage"));
  expect(within(lifetime).getByText("18.4 quintillion")).toBeVisible();
  await act(async () => usageEvent());
  expect(screen.getByRole("grid")).toBeVisible();
  expect(lifetime).toHaveAttribute("open");
});

test.each([
  ["999", "999"],
  ["1000", "1K"],
  ["999950", "1M"],
  ["1000000000", "1B"],
  ["1000000000000", "1T"],
  ["999950000000000", "1 quadrillion"],
  ["999950000000000000", "1 quintillion"],
])("displays %s as %s", async (inputTokens, display) => {
  const usage = response();
  vi.mocked(ipc.getCleanupUsage).mockResolvedValueOnce({
    ...usage,
    totals: { ...unreported, inputTokens },
  });
  render(<CleanupUsageSection />);
  expect(await screen.findByText(display, { selector: ".usage > .usage__totals dd" })).toBeVisible();
});

test("keeps keyboard focus in month navigation when a control disappears or becomes disabled", async () => {
  const user = userEvent.setup();
  render(<CleanupUsageSection />);
  await monthLabel("February 2024");

  button("Previous month").focus();
  await user.keyboard("{Enter}");
  await monthLabel("January 2024");
  expect(button("Previous month")).toHaveFocus();
  await user.keyboard("{Enter}");
  expect(await monthLabel("December 2023")).toHaveFocus();

  button("This month").focus();
  await user.keyboard("{Enter}");
  expect(await monthLabel("February 2024")).toHaveFocus();

  button("Previous month").focus();
  await user.keyboard("{Enter}");
  await monthLabel("January 2024");
  button("Next month").focus();
  await user.keyboard("{Enter}");
  expect(await monthLabel("February 2024")).toHaveFocus();
});
