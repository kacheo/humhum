import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";

// Stub the hydration hook so the gate is tested in isolation (no IPC).
vi.mock("../hooks/useAppBootstrap", () => ({
  useAppBootstrap: vi.fn(),
}));

// The real Onboarding / PrefsPanel mount-effects fire check_permissions /
// get_model_status on mount. Stub the ipc module so this gate suite never hits a
// real invoke. The stubs reject (mirroring the no-backend environment) and the
// callers' `.catch` swallows them, so no async state update lands after these
// synchronous render-and-assert tests — keeping the suite act()-warning-free.
vi.mock("../lib/ipc", () => ({
  ipc: {
    checkPermissions: vi.fn().mockRejectedValue(new Error("no backend")),
    getModelStatus: vi.fn().mockRejectedValue(new Error("no backend")),
    listInputDevices: vi.fn().mockRejectedValue(new Error("no backend")),
  },
}));

import { useAppBootstrap } from "../hooks/useAppBootstrap";
import { PrefsRoot } from "./prefs";
import { usePrefsStore } from "../stores/prefs";

// Pristine prefs captured before any test mutates the store. Resetting in
// beforeEach (when RTL auto-cleanup has already unmounted the prior test's
// tree) avoids notifying a mounted subscriber outside act().
const initialPrefs = usePrefsStore.getState().prefs;

beforeEach(() => {
  vi.mocked(useAppBootstrap).mockReturnValue({ ready: true });
  usePrefsStore.setState({
    prefs: initialPrefs,
    savedPrefs: initialPrefs,
    isDirty: false,
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

test("gates to Onboarding when onboarding is incomplete", () => {
  usePrefsStore.getState().setField("onboardingComplete", false);
  render(<PrefsRoot />);
  expect(screen.getByRole("heading", { name: "Permissions" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Shortcut" })).not.toBeInTheDocument();
});

test("shows PrefsPanel once onboarding is complete", () => {
  usePrefsStore.getState().setField("onboardingComplete", true);
  render(<PrefsRoot />);
  expect(screen.getByRole("tab", { name: "Shortcut" })).toBeInTheDocument();
  expect(screen.queryByText("Onboarding")).not.toBeInTheDocument();
});

test("renders nothing until stores are hydrated", () => {
  vi.mocked(useAppBootstrap).mockReturnValue({ ready: false });
  usePrefsStore.getState().setField("onboardingComplete", true);
  const { container } = render(<PrefsRoot />);
  expect(container).toBeEmptyDOMElement();
});
