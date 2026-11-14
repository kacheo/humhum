import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";
import { ApiKeyInput } from ".";
import { ipc } from "../../lib/ipc";

vi.mock("../../lib/ipc", () => ({ ipc: {
  hasApiKey: vi.fn(),
  removeApiKey: vi.fn(),
} }));

beforeEach(() => {
  vi.mocked(ipc.hasApiKey).mockReset().mockResolvedValue(false);
  vi.mocked(ipc.removeApiKey).mockReset().mockResolvedValue(undefined);
});

test("the field is a password input and the Test button is disabled when disabled", () => {
  render(<ApiKeyInput provider="open_ai" disabled />);
  const field = screen.getByLabelText("API key");
  expect(field).toHaveAttribute("type", "password");
  expect(field).toBeDisabled();
  expect(screen.getByRole("button", { name: "Verify & Save" })).toBeDisabled();
});

test("calls onTest with the entered key and shows success", async () => {
  const onTest = vi.fn().mockResolvedValue(undefined);
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await userEvent.type(screen.getByLabelText("API key"), "sk-live-123");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  expect(onTest).toHaveBeenCalledWith("open_ai", "sk-live-123");
  expect(await screen.findByRole("status")).toHaveTextContent("Key verified and saved");
  expect(screen.getByLabelText("API key")).toHaveValue("");
  expect(screen.getByText("Key saved in Keychain")).toBeInTheDocument();
});

test("never logs the key (privacy)", async () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const onTest = vi.fn().mockResolvedValue(undefined);
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await userEvent.type(screen.getByLabelText("API key"), "sk-secret-xyz");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  const all = [...log.mock.calls, ...warn.mock.calls, ...err.mock.calls].flat().join(" ");
  expect(all).not.toContain("sk-secret-xyz");
  log.mockRestore();
  warn.mockRestore();
  err.mockRestore();
});

test("shows a previously saved key and removes it", async () => {
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  render(<ApiKeyInput provider="open_ai" />);
  expect(await screen.findByText("Key saved in Keychain")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Remove key" }));
  expect(ipc.removeApiKey).toHaveBeenCalledWith("open_ai");
  expect(await screen.findByText("No key saved")).toBeInTheDocument();
});

test("failed replacement retains saved status and provides an actionable error", async () => {
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  const onTest = vi.fn().mockRejectedValue("rate_limited");
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await screen.findByText("Key saved in Keychain");
  await userEvent.type(screen.getByLabelText("API key"), "replacement");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  expect(await screen.findByRole("status")).toHaveTextContent(/rate limit or quota reached/i);
  expect(screen.getByText("Key saved in Keychain")).toBeInTheDocument();
  expect(screen.getByLabelText("API key")).toHaveValue("replacement");
});

test("switching providers clears the draft and ignores an old provider's lookup", async () => {
  let resolveOld!: (saved: boolean) => void;
  vi.mocked(ipc.hasApiKey).mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
  const { rerender } = render(<ApiKeyInput provider="open_ai" />);
  await userEvent.type(screen.getByLabelText("API key"), "openai-secret");
  rerender(<ApiKeyInput provider="anthropic" />);
  await screen.findByText("No key saved");
  await act(async () => resolveOld(true));
  expect(screen.getByLabelText("API key")).toHaveValue("");
  expect(screen.getByText("No key saved")).toBeInTheDocument();
});

test("disables overlapping actions while verification is in flight", async () => {
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  let finish!: () => void;
  const onTest = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await screen.findByText("Key saved in Keychain");
  await userEvent.type(screen.getByLabelText("API key"), "new-key");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  expect(screen.getByLabelText("API key")).toBeDisabled();
  expect(screen.getByRole("button", { name: "Remove key" })).toBeDisabled();
  await act(async () => finish());
  await waitFor(() => expect(screen.getByLabelText("API key")).toBeEnabled());
});

test("Keychain lookup failures are distinct from absent credentials and can be retried", async () => {
  vi.mocked(ipc.hasApiKey).mockRejectedValueOnce("keychain_error").mockResolvedValueOnce(true);
  render(<ApiKeyInput provider="open_ai" />);
  expect(await screen.findByText("Unable to access Keychain")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Refresh status" }));
  expect(await screen.findByText("Key saved in Keychain")).toBeInTheDocument();
});

test("a remounted editor refreshes when an earlier verification finishes", async () => {
  let finish!: () => void;
  const onTest = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
  const oldEditor = render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await screen.findByText("No key saved");
  await userEvent.type(screen.getByLabelText("API key"), "key-to-save");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  oldEditor.unmount();
  render(<ApiKeyInput provider="open_ai" />);
  await screen.findByText("No key saved");
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  await act(async () => finish());
  expect(await screen.findByText("Key saved in Keychain")).toBeInTheDocument();
});
