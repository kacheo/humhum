import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("../../hooks/useTauriEvent", () => ({
  useTauriEvent: (event: string, handler: (e: { payload: unknown }) => void) => { handlers.set(event, handler); },
}));
vi.mock("../../lib/ipc", () => ({
  ipc: {
    checkPermissions: vi.fn().mockResolvedValue({ microphone: false, accessibility: false, inputMonitoring: false }),
    requestMicrophone: vi.fn().mockResolvedValue(undefined),
    openAccessibilitySettings: vi.fn(),
    openInputMonitoringSettings: vi.fn(),
  },
}));
import { PermissionsStep } from "./PermissionsStep";
import { ipc } from "../../lib/ipc";

function fire(event: string, payload: unknown) { act(() => handlers.get(event)?.({ payload })); }

beforeEach(() => { vi.clearAllMocks(); handlers.clear(); });

test("reports not-ready until all three permissions are granted", async () => {
  const onReady = vi.fn();
  render(<PermissionsStep onReady={onReady} />);
  await waitFor(() => expect(ipc.checkPermissions).toHaveBeenCalled());
  expect(onReady).toHaveBeenLastCalledWith(false);
  fire("permissions-changed", { microphone: true, accessibility: true, inputMonitoring: true });
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
});

test("the microphone action invokes request_microphone", async () => {
  render(<PermissionsStep onReady={() => {}} />);
  await waitFor(() => expect(ipc.checkPermissions).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", { name: "Grant" }));
  expect(ipc.requestMicrophone).toHaveBeenCalled();
});

test("the accessibility and input monitoring actions open their System Settings panes", async () => {
  render(<PermissionsStep onReady={() => {}} />);
  await waitFor(() => expect(ipc.checkPermissions).toHaveBeenCalled());
  const [accessibilityButton, inputMonitoringButton] = screen.getAllByRole("button", { name: "Open Settings" });
  await userEvent.click(accessibilityButton);
  expect(ipc.openAccessibilitySettings).toHaveBeenCalled();
  await userEvent.click(inputMonitoringButton);
  expect(ipc.openInputMonitoringSettings).toHaveBeenCalled();
});

test("offers recovery while Accessibility is denied and clears it after a live grant", async () => {
  render(<PermissionsStep onReady={() => {}} />);
  const recovery = await screen.findByRole("note", { name: "Permission recovery" });
  expect(recovery).toHaveTextContent("fully quit Humhum");
  fire("permissions-changed", { microphone: true, accessibility: true, inputMonitoring: true });
  await waitFor(() => expect(screen.queryByRole("note", { name: "Permission recovery" })).toBeNull());
});
