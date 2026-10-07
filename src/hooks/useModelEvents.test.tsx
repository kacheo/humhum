import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, vi } from "vitest";

const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("./useTauriEvent", () => ({
  useTauriEvent: (event: string, handler: (e: { payload: unknown }) => void) => {
    handlers.set(event, handler);
  },
}));

vi.mock("../lib/ipc", () => ({
  ipc: { getModelStatus: vi.fn().mockResolvedValue("ready") },
}));

import { useModelEvents } from "./useModelEvents";
import { useRecordingStore } from "../stores/recording";
import { ipc } from "../lib/ipc";

function fire(event: string, payload?: unknown) {
  const handler = handlers.get(event);
  if (!handler) throw new Error(`no handler registered for ${event}`);
  handler({ payload });
}

beforeEach(() => {
  vi.clearAllMocks();
  handlers.clear();
  useRecordingStore.getState().reset();
  renderHook(() => useModelEvents());
});

test("model-download-progress updates the store's downloadProgress", () => {
  fire("model-download-progress", { model: "base_en", pct: 37 });
  expect(useRecordingStore.getState().downloadProgress.base_en).toBe(37);
});

test("re-fetches model status on completion and flips to Ready", async () => {
  fire("model-download-progress", { model: "base_en", pct: 100 });
  expect(ipc.getModelStatus).toHaveBeenCalledWith("base_en");
  await waitFor(() => expect(useRecordingStore.getState().modelStatus.base_en).toBe("ready"));
});

test("does not re-fetch status mid-download", () => {
  fire("model-download-progress", { model: "base_en", pct: 50 });
  expect(ipc.getModelStatus).not.toHaveBeenCalled();
});

test("reconciles an async download failure so Retry becomes available", async () => {
  vi.mocked(ipc.getModelStatus).mockResolvedValueOnce("not_downloaded");
  useRecordingStore.getState().setModelStatus("base_en", { downloading: 0 });
  await waitFor(() => {
    expect(useRecordingStore.getState().modelStatus.base_en).toEqual({
      error: "Download did not finish. Check your connection and available disk space, then retry.",
    });
  }, { timeout: 2000 });
});

test("verification failure after 100 percent clears stale progress and enables retry", async () => {
  vi.mocked(ipc.getModelStatus).mockResolvedValueOnce("not_downloaded");
  fire("model-download-progress", { model: "base_en", pct: 100 });
  await waitFor(() => expect(useRecordingStore.getState().modelStatus.base_en).toEqual({
    error: "Download did not finish. Check your connection and available disk space, then retry.",
  }));
  expect(useRecordingStore.getState().downloadProgress.base_en).toBe(0);
});

test("a stale completion response cannot overwrite a newer retry", async () => {
  let resolve!: (value: "ready") => void;
  vi.mocked(ipc.getModelStatus).mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  fire("model-download-progress", { model: "base_en", pct: 100 });
  useRecordingStore.getState().setModelStatus("base_en", { downloading: 0 });
  resolve("ready");
  await Promise.resolve();
  expect(useRecordingStore.getState().modelStatus.base_en).toEqual({ downloading: 0 });
});
