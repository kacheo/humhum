import type {
  ModelStatus,
  Prefs,
  RecordingPhase,
} from "./types";

// Type-coverage test: the value is the `tsc -b` typecheck. If a wire shape
// drifts, these `const` assignments stop compiling and the suite fails to build.

test("ModelStatus accepts each of the four externally-tagged forms", () => {
  const forms: ModelStatus[] = [
    "not_downloaded",
    "ready",
    { downloading: 50 },
    { error: "hash mismatch" },
  ];
  expect(forms).toHaveLength(4);
});

test("Prefs and RecordingPhase are importable from lib/types", () => {
  const phase: RecordingPhase = "idle";
  const prefs: Prefs = {
    hotkey: "Option+Shift+Space",
    cleanupMode: "none",
    cleanupAggressiveness: "standard",
    apiProvider: "open_ai",
    ollamaModel: "llama3",
    ollamaBaseUrl: "http://localhost:11434/v1/",
    whisperModel: "base_en",
    launchAtLogin: false,
    onboardingComplete: false,
  };
  expect(phase).toBe("idle");
  expect(prefs.whisperModel).toBe("base_en");
});
