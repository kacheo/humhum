import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

vi.mock("../../lib/ipc", () => ({
  ipc: { validateHotkey: vi.fn().mockResolvedValue(undefined) },
}));
import { HotkeyStep } from "./HotkeyStep";
import { ipc } from "../../lib/ipc";
import { usePrefsStore } from "../../stores/prefs";

const initialPrefs = usePrefsStore.getState().prefs;
beforeEach(() => {
  vi.clearAllMocks();
  usePrefsStore.setState({ prefs: { ...initialPrefs }, savedPrefs: { ...initialPrefs }, isDirty: false });
});

test("validates the existing hotkey on mount and reports ready", async () => {
  const onReady = vi.fn();
  render(<HotkeyStep onReady={onReady} />);
  await waitFor(() => expect(ipc.validateHotkey).toHaveBeenCalledWith("Option+Shift+Space"));
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
});

test("a rejected capture reports not-ready and shows the conflict", async () => {
  vi.mocked(ipc.validateHotkey)
    .mockResolvedValueOnce(undefined)                  // mount validate
    .mockRejectedValueOnce("conflict_with:Spotlight"); // capture validate
  const onReady = vi.fn();
  render(<HotkeyStep onReady={onReady} />);
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
  await userEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(screen.getByRole("button", { name: "Press keys…" }), { code: "Space", metaKey: true });
  expect(await screen.findByRole("alert")).toHaveTextContent("conflict_with:Spotlight");
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(false));
});
