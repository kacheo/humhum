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

function keyChanged(provider = "open_ai") {
  window.dispatchEvent(new CustomEvent("humhum-api-key-changed", { detail: provider }));
}

test.each([
  ["saving", "success"], ["saving", "failure"],
  ["removing", "success"], ["removing", "failure"],
])("reconciles an external change after %s ends in %s", async (action, outcome) => {
  const removing = action === "removing";
  vi.mocked(ipc.hasApiKey).mockResolvedValue(removing);
  let resolve!: () => void;
  let reject!: (error: string) => void;
  const pending = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  const onTest = vi.fn(() => pending);
  if (removing) vi.mocked(ipc.removeApiKey).mockReturnValueOnce(pending);
  render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await screen.findByText(removing ? "Key saved in Keychain" : "No key saved");
  if (!removing) await userEvent.type(screen.getByLabelText("API key"), "draft-key");
  await userEvent.click(screen.getByRole("button", { name: removing ? "Remove key" : "Verify & Save" }));
  const externallySaved = outcome === "success" ? removing : !removing;
  vi.mocked(ipc.hasApiKey).mockResolvedValue(externallySaved);
  act(() => keyChanged());
  await act(async () => { if (outcome === "success") resolve(); else reject("unavailable"); });
  expect(await screen.findByText(externallySaved ? "Key saved in Keychain" : "No key saved")).toBeInTheDocument();
  expect(ipc.hasApiKey).toHaveBeenCalledTimes(2);
  if (outcome === "failure") {
    expect(screen.getByRole("status")).toHaveTextContent(/could not reach/i);
    if (!removing) expect(screen.getByLabelText("API key")).toHaveValue("draft-key");
  }
});

test.each(["saving", "removing"])("external changes clear obsolete %s success feedback", async (action) => {
  const removing = action === "removing";
  vi.mocked(ipc.hasApiKey).mockResolvedValue(removing);
  render(<ApiKeyInput provider="open_ai" onTest={vi.fn().mockResolvedValue(undefined)} />);
  await screen.findByText(removing ? "Key saved in Keychain" : "No key saved");
  if (!removing) await userEvent.type(screen.getByLabelText("API key"), "key");
  await userEvent.click(screen.getByRole("button", { name: removing ? "Remove key" : "Verify & Save" }));
  expect(screen.getByRole("status")).toHaveTextContent(removing ? "Key removed" : "Key verified and saved");
  act(() => keyChanged("anthropic"));
  expect(screen.getByRole("status")).toBeInTheDocument();
  act(() => keyChanged());
  await screen.findByText(removing ? "Key saved in Keychain" : "No key saved");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

test("an earlier editor's completion is reconciled after a remounted editor's ordinary failure", async () => {
  let finishOld!: () => void;
  const oldEditor = render(<ApiKeyInput provider="open_ai" onTest={() => new Promise<void>((resolve) => { finishOld = resolve; })} />);
  await screen.findByText("No key saved");
  await userEvent.type(screen.getByLabelText("API key"), "old-secret");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  oldEditor.unmount();
  let failNew!: (error: string) => void;
  render(<ApiKeyInput provider="open_ai" onTest={() => new Promise<void>((_, reject) => { failNew = reject; })} />);
  await screen.findByText("No key saved");
  expect(screen.getByLabelText("API key")).toHaveValue("");
  await userEvent.type(screen.getByLabelText("API key"), "new-secret");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  await act(async () => finishOld());
  await act(async () => failNew("key_rejected"));
  expect(await screen.findByText("Key saved in Keychain")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent(/key rejected/i);
  expect(screen.getByLabelText("API key")).toHaveValue("new-secret");
});

test("ignores a stale same-provider lookup after external invalidation", async () => {
  let finishOld!: (saved: boolean) => void;
  vi.mocked(ipc.hasApiKey).mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; })).mockResolvedValue(true);
  render(<ApiKeyInput provider="open_ai" />);
  act(() => keyChanged());
  await screen.findByText("Key saved in Keychain");
  await act(async () => finishOld(false));
  expect(screen.getByText("Key saved in Keychain")).toBeInTheDocument();
});

test("late verification cannot update another provider's status, feedback, or secret draft", async () => {
  let finish!: () => void;
  const onTest = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
  const { rerender } = render(<ApiKeyInput provider="open_ai" onTest={onTest} />);
  await screen.findByText("No key saved");
  await userEvent.type(screen.getByLabelText("API key"), "openai-secret");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  rerender(<ApiKeyInput provider="anthropic" onTest={onTest} />);
  await screen.findByText("No key saved");
  expect(screen.getByLabelText("API key")).toHaveValue("");
  await userEvent.type(screen.getByLabelText("API key"), "anthropic-draft");
  await act(async () => finish());
  expect(screen.getByLabelText("API key")).toHaveValue("anthropic-draft");
  expect(screen.getByText("No key saved")).toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(ipc.hasApiKey).toHaveBeenCalledTimes(2);
});

test.each([
  ["key_rejected", /key rejected/i],
  ["keychain_error", /unlock your login keychain/i],
  ["io_error", /could not update cleanup settings/i],
  ["serialize_error", /could not update cleanup settings/i],
  ["key_action_superseded", /newer key action/i],
])("maps %s to safe actionable feedback and keeps the draft", async (code, message) => {
  render(<ApiKeyInput provider="anthropic" onTest={vi.fn().mockRejectedValue(new Error(code))} />);
  await screen.findByText("No key saved");
  await userEvent.type(screen.getByLabelText("API key"), "retry-secret");
  await userEvent.click(screen.getByRole("button", { name: "Verify & Save" }));
  expect(await screen.findByRole("status")).toHaveTextContent(message);
  expect(screen.getByLabelText("API key")).toHaveValue("retry-secret");
  expect(screen.getByRole("button", { name: "Verify & Save" })).toBeEnabled();
});

test("failed removal retains the saved credential and can be retried", async () => {
  vi.mocked(ipc.hasApiKey).mockResolvedValue(true);
  vi.mocked(ipc.removeApiKey).mockRejectedValueOnce("keychain_error");
  const onBusyChange = vi.fn();
  render(<ApiKeyInput provider="anthropic" onBusyChange={onBusyChange} />);
  await screen.findByText("Key saved in Keychain");
  await userEvent.click(screen.getByRole("button", { name: "Remove key" }));
  expect(await screen.findByRole("status")).toHaveTextContent(/unlock your login keychain/i);
  expect(screen.getByText("Key saved in Keychain")).toBeInTheDocument();
  expect(onBusyChange).toHaveBeenLastCalledWith(false);
  await userEvent.click(screen.getByRole("button", { name: "Remove key" }));
  expect(await screen.findByText("No key saved")).toBeInTheDocument();
  expect(ipc.removeApiKey).toHaveBeenLastCalledWith("anthropic");
});

test.each(["saving", "removing"])("never renders or logs secret-bearing %s errors", async (action) => {
  const removing = action === "removing";
  const secret = "sk-private-do-not-expose";
  const spies = (["log", "warn", "error"] as const).map((method) => vi.spyOn(console, method).mockImplementation(() => {}));
  try {
    vi.mocked(ipc.hasApiKey).mockResolvedValue(removing);
    const error = new Error(`keychain_error: provider echoed ${secret}`);
    if (removing) vi.mocked(ipc.removeApiKey).mockRejectedValueOnce(error);
    render(<ApiKeyInput provider="open_ai" onTest={vi.fn().mockRejectedValue(error)} />);
    await screen.findByText(removing ? "Key saved in Keychain" : "No key saved");
    if (!removing) await userEvent.type(screen.getByLabelText("API key"), secret);
    await userEvent.click(screen.getByRole("button", { name: removing ? "Remove key" : "Verify & Save" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Could not complete the key action. Try again.");
    expect(document.body.textContent).not.toContain(secret);
    expect(spies.flatMap((spy) => spy.mock.calls).flat().join(" ")).not.toContain(secret);
  } finally {
    spies.forEach((spy) => spy.mockRestore());
  }
});
