import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";

vi.mock("../../lib/ipc", () => ({
  ipc: {
    getModelStatus: vi.fn().mockResolvedValue("not_downloaded"),
    downloadModel: vi.fn().mockResolvedValue(undefined),
  },
}));
import { ModelDownloadStep } from "./ModelDownloadStep";
import { ipc } from "../../lib/ipc";
import { useRecordingStore } from "../../stores/recording";

beforeEach(() => { vi.clearAllMocks(); useRecordingStore.getState().reset(); });

test("not ready until base_en is Ready; Download invokes download_model", async () => {
  const onReady = vi.fn();
  render(<ModelDownloadStep onReady={onReady} />);
  await waitFor(() => expect(ipc.getModelStatus).toHaveBeenCalledWith("base_en"));
  expect(onReady).toHaveBeenLastCalledWith(false);
  await userEvent.click(screen.getByRole("button", { name: "Download" }));
  expect(ipc.downloadModel).toHaveBeenCalledWith("base_en");
  act(() => useRecordingStore.getState().setModelStatus("base_en", "ready"));
  await waitFor(() => expect(onReady).toHaveBeenLastCalledWith(true));
});

test("shows live download progress in the badge", async () => {
  render(<ModelDownloadStep onReady={() => {}} />);
  await waitFor(() => expect(ipc.getModelStatus).toHaveBeenCalled());
  act(() => useRecordingStore.getState().setDownloadProgress("base_en", 64));
  expect(screen.getByRole("status")).toHaveTextContent("Downloading 64%");
});

test("a failed download surfaces Error and re-enables retry", async () => {
  const onReady = vi.fn();
  render(<ModelDownloadStep onReady={onReady} />);
  await waitFor(() => expect(ipc.getModelStatus).toHaveBeenCalled());
  // A download ran (progress recorded) then the status flipped to error — the
  // error status must win over the stale progress, not stay stuck "Downloading".
  act(() => useRecordingStore.getState().setDownloadProgress("base_en", 100));
  act(() => useRecordingStore.getState().setModelStatus("base_en", { error: "insufficient_disk" }));
  expect(screen.getByRole("status")).toHaveTextContent("Error");
  expect(screen.getByRole("button", { name: "Download" })).toBeEnabled();
  expect(onReady).toHaveBeenLastCalledWith(false);
});

test("marks download active before the first progress event", async () => {
  render(<ModelDownloadStep onReady={() => {}} />);
  await waitFor(() => expect(ipc.getModelStatus).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", { name: "Download" }));
  expect(screen.getByRole("button", { name: "Download" })).toBeDisabled();
  expect(useRecordingStore.getState().modelStatus.base_en).toEqual({ downloading: 0 });
});
