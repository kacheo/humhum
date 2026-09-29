import { beforeEach } from "vitest";
import { useRecordingStore } from "./recording";

beforeEach(() => {
  useRecordingStore.getState().reset();
});

test("initial state", () => {
  const s = useRecordingStore.getState();
  expect(s.phase).toBe("idle");
  expect(s.rawTranscript).toBeNull();
  expect(s.cleanedTranscript).toBeNull();
  expect(s.errorMessage).toBeNull();
  expect(s.modelStatus).toEqual({ base_en: "not_downloaded", small_en: "not_downloaded" });
  expect(s.downloadProgress).toEqual({ base_en: 0, small_en: 0 });
});

test("setPhase updates phase", () => {
  useRecordingStore.getState().setPhase("recording");
  expect(useRecordingStore.getState().phase).toBe("recording");
});

test("transcript setters", () => {
  useRecordingStore.getState().setRawTranscript("x");
  expect(useRecordingStore.getState().rawTranscript).toBe("x");
  useRecordingStore.getState().setCleanedTranscript("y");
  expect(useRecordingStore.getState().cleanedTranscript).toBe("y");
});

test("setError sets message", () => {
  useRecordingStore.getState().setError("boom");
  expect(useRecordingStore.getState().errorMessage).toBe("boom");
});

test("setModelStatus updates only the named model", () => {
  useRecordingStore.getState().setModelStatus("small_en", { downloading: 40 });
  expect(useRecordingStore.getState().modelStatus).toEqual({
    base_en: "not_downloaded",
    small_en: { downloading: 40 },
  });
});

test("setDownloadProgress updates only the named model", () => {
  useRecordingStore.getState().setDownloadProgress("base_en", 50);
  expect(useRecordingStore.getState().downloadProgress).toEqual({ base_en: 50, small_en: 0 });
});

test("reset returns everything to initial", () => {
  const s = useRecordingStore.getState();
  s.setPhase("error");
  s.setRawTranscript("x");
  s.setModelStatus("small_en", { downloading: 10 });
  s.setDownloadProgress("base_en", 99);
  s.reset();
  const r = useRecordingStore.getState();
  expect(r.phase).toBe("idle");
  expect(r.rawTranscript).toBeNull();
  expect(r.modelStatus).toEqual({ base_en: "not_downloaded", small_en: "not_downloaded" });
  expect(r.downloadProgress).toEqual({ base_en: 0, small_en: 0 });
});
