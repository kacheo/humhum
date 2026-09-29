import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { ApiKeyInput } from ".";

test("the field is a password input and the Test button is disabled when disabled", () => {
  render(<ApiKeyInput provider="open_ai" disabled />);
  const field = screen.getByLabelText("API key");
  expect(field).toHaveAttribute("type", "password");
  expect(field).toBeDisabled();
  expect(screen.getByRole("button", { name: "Test" })).toBeDisabled();
});

test("calls onTest with the entered key and shows success", async () => {
  const onTest = vi.fn().mockResolvedValue(undefined);
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await userEvent.type(screen.getByLabelText("API key"), "sk-live-123");
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(onTest).toHaveBeenCalledWith("open_ai", "sk-live-123");
  expect(await screen.findByRole("status")).toHaveTextContent("Key verified");
});

test("never logs the key (privacy)", async () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const onTest = vi.fn().mockResolvedValue(undefined);
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await userEvent.type(screen.getByLabelText("API key"), "sk-secret-xyz");
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  const all = [...log.mock.calls, ...warn.mock.calls, ...err.mock.calls].flat().join(" ");
  expect(all).not.toContain("sk-secret-xyz");
  log.mockRestore();
  warn.mockRestore();
  err.mockRestore();
});
