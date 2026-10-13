import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("../../hooks/useTauriEvent", () => ({
  useTauriEvent: (event: string, handler: (e: { payload: unknown }) => void) => {
    handlers.set(event, handler);
  },
}));
vi.mock("../../lib/ipc", () => ({
  ipc: {
    checkPermissions: vi.fn(),
    requestMicrophone: vi.fn().mockResolvedValue(undefined),
    openAccessibilitySettings: vi.fn(),
    openInputMonitoringSettings: vi.fn(),
  },
}));
import { PermissionsNotice } from "./PermissionsNotice";
import { ipc } from "../../lib/ipc";

function fire(event: string, payload: unknown) {
  act(() => handlers.get(event)?.({ payload }));
}

beforeEach(() => {
  vi.clearAllMocks();
  handlers.clear();
});

test("shows every permission as granted with no action buttons when all granted", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({
    microphone: true,
    accessibility: true,
    inputMonitoring: true,
  });
  render(<PermissionsNotice />);
  await waitFor(() => expect(ipc.checkPermissions).toHaveBeenCalled());
  expect(screen.getAllByText("Granted ✓")).toHaveLength(3);
  expect(screen.queryByRole("button")).toBeNull();
});

test("Microphone exposes a Grant button that requests the system prompt", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({
    microphone: false,
    accessibility: true,
    inputMonitoring: true,
  });
  render(<PermissionsNotice />);
  const grant = await screen.findByRole("button", { name: "Grant" });
  await userEvent.click(grant);
  expect(ipc.requestMicrophone).toHaveBeenCalled();
  // The other two are granted, so only the mic action is offered.
  expect(screen.queryByRole("button", { name: "Open Settings" })).toBeNull();
});

test("a denied keyboard permission deep-links to its settings pane", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({
    microphone: true,
    accessibility: false,
    inputMonitoring: true,
  });
  render(<PermissionsNotice />);
  const open = await screen.findByRole("button", { name: "Open Settings" });
  await userEvent.click(open);
  expect(ipc.openAccessibilitySettings).toHaveBeenCalled();
});

test("a denied input-monitoring permission deep-links to its settings pane", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({
    microphone: true,
    accessibility: true,
    inputMonitoring: false,
  });
  render(<PermissionsNotice />);
  const open = await screen.findByRole("button", { name: "Open Settings" });
  await userEvent.click(open);
  expect(ipc.openInputMonitoringSettings).toHaveBeenCalled();
});

test("clears the action live when permissions-changed reports a grant", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({
    microphone: false,
    accessibility: true,
    inputMonitoring: true,
  });
  render(<PermissionsNotice />);
  expect(await screen.findByRole("button", { name: "Grant" })).toBeInTheDocument();
  fire("permissions-changed", { microphone: true, accessibility: true, inputMonitoring: true });
  await waitFor(() => expect(screen.queryByRole("button")).toBeNull());
});

test.each([
  { microphone: true, accessibility: false, inputMonitoring: true },
  { microphone: true, accessibility: true, inputMonitoring: false },
])("offers recovery for a denied keyboard permission (%j)", async (perms) => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue(perms);
  render(<PermissionsNotice />);
  const recovery = await screen.findByRole("note", { name: "Permission recovery" });
  expect(recovery).toHaveTextContent("fully quit Humhum");
  fire("permissions-changed", { microphone: false, accessibility: true, inputMonitoring: true });
  await waitFor(() => expect(screen.queryByRole("note", { name: "Permission recovery" })).toBeNull());
  expect(screen.getByRole("button", { name: "Grant" })).toBeInTheDocument();
});

test("does not recommend keyboard permission recovery for a missing microphone grant", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValue({ microphone: false, accessibility: true, inputMonitoring: true });
  render(<PermissionsNotice />);
  await screen.findByRole("button", { name: "Grant" });
  expect(screen.queryByRole("note", { name: "Permission recovery" })).toBeNull();
});
