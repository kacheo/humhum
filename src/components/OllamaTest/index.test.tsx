import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { OllamaTest } from ".";
import { ipc } from "../../lib/ipc";

vi.mock("../../lib/ipc", () => ({ ipc: { testOllama: vi.fn() } }));
const testOllama = vi.mocked(ipc.testOllama);

test("the Test button is disabled when the URL or model is empty", () => {
  render(<OllamaTest baseUrl="" model="llama3" />);
  expect(screen.getByRole("button", { name: "Test" })).toBeDisabled();
});

test("probes the configured server + model and shows success", async () => {
  testOllama.mockResolvedValue(undefined);
  render(<OllamaTest baseUrl="http://localhost:11434/v1/" model="llama3" />);
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(testOllama).toHaveBeenCalledWith("http://localhost:11434/v1/", "llama3");
  expect(await screen.findByRole("status")).toHaveTextContent("Ollama reachable — llama3 ready");
});

test("maps the model_not_found code to a clear message", async () => {
  // A failed invoke rejects with the bare wire code string.
  testOllama.mockRejectedValue("model_not_found");
  render(<OllamaTest baseUrl="http://localhost:11434/v1/" model="llama3" />);
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(await screen.findByRole("status")).toHaveTextContent("Model 'llama3' not found on server");
});

test("maps an unreachable server to a clear message", async () => {
  testOllama.mockRejectedValue("unavailable");
  render(<OllamaTest baseUrl="http://localhost:9999/v1/" model="llama3" />);
  await userEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Can't reach Ollama at http://localhost:9999/v1/",
  );
});
