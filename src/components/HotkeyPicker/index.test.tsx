import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { HotkeyPicker } from ".";

test("shows the prompt when empty and the formatted combo when set", () => {
  const { rerender } = render(<HotkeyPicker value="" onChange={() => {}} onValidate={() => Promise.resolve()} />);
  expect(screen.getByRole("button")).toHaveTextContent("Click to record");
  rerender(<HotkeyPicker value="Option+Shift+Space" onChange={() => {}} onValidate={() => Promise.resolve()} />);
  expect(screen.getByRole("button")).toHaveTextContent("⌥⇧Space");
});

test("focuses the recorder when recording starts so keystrokes are captured", async () => {
  // macOS WebKit does not focus a <button> on click; the recorder must focus
  // itself or its onKeyDown/onKeyUp never fire and recording hangs. (jsdom
  // auto-focuses on click, so this documents intent more than it guards WebKit.)
  render(<HotkeyPicker value="" onChange={() => {}} onValidate={() => Promise.resolve()} />);
  const button = screen.getByRole("button");
  await userEvent.click(button);
  expect(button).toHaveFocus();
});

test("captures a keystroke and reports the combo", async () => {
  const onChange = vi.fn();
  const onValidate = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyPicker value="" onChange={onChange} onValidate={onValidate} />);
  await userEvent.click(screen.getByRole("button"));
  expect(screen.getByRole("button")).toHaveTextContent("Press keys…");
  fireEvent.keyDown(screen.getByRole("button"), { code: "Space", altKey: true, shiftKey: true });
  expect(onChange).toHaveBeenCalledWith("Option+Shift+Space");
  await waitFor(() => expect(onValidate).toHaveBeenCalledWith("Option+Shift+Space"));
});

test("stops recording when the capture button loses focus", async () => {
  render(<HotkeyPicker value="Option+Shift+Space" onChange={() => {}} onValidate={() => Promise.resolve()} />);
  const button = screen.getByRole("button");
  await userEvent.click(button);
  expect(button).toHaveTextContent("Press keys…");
  fireEvent.blur(button);
  expect(button).toHaveTextContent("⌥⇧Space");
});

test("holding then releasing a lone modifier commits it as push-to-talk", async () => {
  const onChange = vi.fn();
  const onValidate = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyPicker value="" onChange={onChange} onValidate={onValidate} />);
  const button = screen.getByRole("button");
  await userEvent.click(button);
  // Hold Option → live preview shows the glyph, nothing committed yet.
  fireEvent.keyDown(button, { code: "AltLeft", altKey: true });
  expect(button).toHaveTextContent("⌥");
  expect(onChange).not.toHaveBeenCalled();
  // Release Option → commit the modifier-only combo.
  fireEvent.keyUp(button, { code: "AltLeft", altKey: false });
  expect(onChange).toHaveBeenCalledWith("Option");
  await waitFor(() => expect(onValidate).toHaveBeenCalledWith("Option"));
});

test("pressing a real key while holding a modifier commits the keyed combo", async () => {
  const onChange = vi.fn();
  const onValidate = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyPicker value="" onChange={onChange} onValidate={onValidate} />);
  const button = screen.getByRole("button");
  await userEvent.click(button);
  fireEvent.keyDown(button, { code: "AltLeft", altKey: true });
  fireEvent.keyDown(button, { code: "Space", altKey: true });
  expect(onChange).toHaveBeenCalledWith("Option+Space");
  expect(onChange).toHaveBeenCalledTimes(1);
  // A trailing modifier key-up must not also commit a modifier-only combo.
  fireEvent.keyUp(button, { code: "AltLeft", altKey: false });
  expect(onChange).toHaveBeenCalledTimes(1);
});

test("surfaces a validation conflict inline", async () => {
  const onValidate = vi.fn().mockRejectedValue("conflict_with:Spotlight");
  render(<HotkeyPicker value="" onChange={() => {}} onValidate={onValidate} />);
  await userEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(screen.getByRole("button"), { code: "Space", metaKey: true });
  expect(await screen.findByRole("alert")).toHaveTextContent("conflict_with:Spotlight");
});
