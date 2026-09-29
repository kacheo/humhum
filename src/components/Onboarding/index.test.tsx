import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("../../hooks/useTauriEvent", () => ({
  useTauriEvent: (event: string, handler: (e: { payload: unknown }) => void) => { handlers.set(event, handler); },
}));
vi.mock("../../hooks/useRecordingEvents", () => ({ useRecordingEvents: () => {} }));
vi.mock("../../lib/ipc", () => ({
  ipc: {
    checkPermissions: vi.fn().mockResolvedValue({ microphone: true, accessibility: true, inputMonitoring: true }),
    requestMicrophone: vi.fn(), openAccessibilitySettings: vi.fn(), openInputMonitoringSettings: vi.fn(),
    validateHotkey: vi.fn().mockResolvedValue(undefined),
    getModelStatus: vi.fn().mockResolvedValue("ready"),
    downloadModel: vi.fn().mockResolvedValue(undefined),
    savePrefs: vi.fn().mockResolvedValue(undefined),
  },
}));
import { Onboarding } from ".";
import { ipc } from "../../lib/ipc";
import { usePrefsStore } from "../../stores/prefs";
import { useRecordingStore } from "../../stores/recording";

const initialPrefs = usePrefsStore.getState().prefs;
beforeEach(() => {
  vi.clearAllMocks(); handlers.clear();
  usePrefsStore.setState({ prefs: { ...initialPrefs, onboardingComplete: false }, savedPrefs: { ...initialPrefs }, isDirty: false, isSaving: false });
  useRecordingStore.getState().reset();
});

async function clickNext() {
  const next = await screen.findByRole("button", { name: /Next|Finish/ });
  await waitFor(() => expect(next).toBeEnabled());
  await userEvent.click(next);
}

test("walks all five steps and completes onboarding on Skip", async () => {
  render(<Onboarding />);
  expect(screen.getByRole("heading", { name: "Permissions" })).toBeInTheDocument();
  await clickNext();
  expect(screen.getByRole("heading", { name: "Shortcut" })).toBeInTheDocument();
  await clickNext();
  expect(screen.getByRole("heading", { name: "Cleanup" })).toBeInTheDocument();
  await clickNext();
  expect(screen.getByRole("heading", { name: "Model" })).toBeInTheDocument();
  await clickNext();
  expect(screen.getByRole("heading", { name: "Try it" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Skip for now" }));
  await waitFor(() => expect(ipc.savePrefs).toHaveBeenCalled());
  expect(usePrefsStore.getState().prefs.onboardingComplete).toBe(true);
});

test("Next is blocked on the Permissions step until all are granted", async () => {
  vi.mocked(ipc.checkPermissions).mockResolvedValueOnce({ microphone: false, accessibility: true, inputMonitoring: true });
  render(<Onboarding />);
  await waitFor(() => expect(ipc.checkPermissions).toHaveBeenCalled());
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  act(() => handlers.get("permissions-changed")?.({ payload: { microphone: true, accessibility: true, inputMonitoring: true } }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
});

test("Back returns to the previous step", async () => {
  render(<Onboarding />);
  await clickNext(); // → Shortcut
  expect(screen.getByRole("heading", { name: "Shortcut" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByRole("heading", { name: "Permissions" })).toBeInTheDocument();
});
