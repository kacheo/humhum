import "@testing-library/jest-dom";
import { vi } from "vitest";

// Mounting the event hooks in the window roots makes useTauriEvent → listen()
// fire under jsdom, where there is no Tauri IPC. Neutralize it globally so any
// render test stays inert. Targeted unit tests that need to drive events mock
// ./useTauriEvent directly, so this does not interfere with them.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})), // resolves to a no-op unlisten
}));

// RecordingOverlay drives its own window visibility via @tauri-apps/api/window.
// Neutralize it globally (stable spies) so render tests don't hit real IPC.
vi.mock("@tauri-apps/api/window", () => {
  const win = { show: vi.fn(() => Promise.resolve()), hide: vi.fn(() => Promise.resolve()) };
  return { getCurrentWindow: () => win };
});
