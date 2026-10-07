import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

// ApiKeyInput is enabled but never clicked in these tests, so testApiKey is an inert spy.
vi.mock("../../lib/ipc", () => ({ ipc: { testApiKey: vi.fn().mockResolvedValue(undefined) } }));
import { CleanupStep } from "./CleanupStep";
import { usePrefsStore } from "../../stores/prefs";

const initialPrefs = usePrefsStore.getState().prefs;
beforeEach(() => {
  usePrefsStore.setState({ prefs: { ...initialPrefs }, savedPrefs: { ...initialPrefs }, isDirty: false });
});

test("is ready immediately; the Cloud API cleanup mode is selectable and the API key is testable", async () => {
  const onReady = vi.fn();
  render(<CleanupStep onReady={onReady} />);
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
  // Cloud API is selectable as a cleanup *mode* as of Phase 12.
  const cloud = screen.getByLabelText("Cloud API");
  expect(cloud).toBeEnabled();
  await userEvent.click(cloud);
  expect(usePrefsStore.getState().prefs.cleanupMode).toBe("api");
  // Selecting it surfaces an inline hint pointing at the key field.
  expect(screen.getByText(/Requires an API key/)).toBeInTheDocument();
  // The cloud-API *key* can be tested/stored (Phase 11).
  expect(screen.getByLabelText("API key")).toBeEnabled();
});

test("selecting Local reveals the Ollama fields and persists the mode", async () => {
  render(<CleanupStep onReady={() => {}} />);
  await userEvent.click(screen.getByLabelText("Local (Ollama)"));
  expect(usePrefsStore.getState().prefs.cleanupMode).toBe("local");
  expect(screen.getByLabelText("Ollama model")).toBeInTheDocument();
});

test("editing the Ollama model/URL fields persists them, and Off can be reselected", async () => {
  render(<CleanupStep onReady={() => {}} />);
  await userEvent.click(screen.getByLabelText("Local (Ollama)"));

  const modelField = screen.getByLabelText("Ollama model");
  await userEvent.clear(modelField);
  await userEvent.type(modelField, "phi3");
  expect(usePrefsStore.getState().prefs.ollamaModel).toBe("phi3");

  const urlField = screen.getByLabelText("Ollama URL");
  await userEvent.clear(urlField);
  await userEvent.type(urlField, "http://localhost:1234/v1/");
  expect(usePrefsStore.getState().prefs.ollamaBaseUrl).toBe("http://localhost:1234/v1/");

  await userEvent.click(screen.getByLabelText("Off"));
  expect(usePrefsStore.getState().prefs.cleanupMode).toBe("none");
});

test("Test on the Cloud API key calls test_api_key with the typed key", async () => {
  render(<CleanupStep onReady={() => {}} />);
  await userEvent.click(screen.getByLabelText("Cloud API"));
  await userEvent.type(screen.getByLabelText("API key"), "sk-onb-1");
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  const { ipc } = await import("../../lib/ipc");
  expect(ipc.testApiKey).toHaveBeenCalledWith("open_ai", "sk-onb-1");
});

test("cloud credentials are hidden for Off and Local", async () => {
  render(<CleanupStep onReady={() => {}} />);
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  await userEvent.click(screen.getByLabelText("Local (Ollama)"));
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
});
