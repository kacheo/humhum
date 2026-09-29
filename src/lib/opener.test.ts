import { vi } from "vitest";

const openUrl = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (...args: unknown[]) => openUrl(...args) }));

import { openExternal } from "./opener";

afterEach(() => {
  openUrl.mockClear();
});

test("openExternal opens the given url via the system browser", async () => {
  await openExternal("https://example.com");
  expect(openUrl).toHaveBeenCalledWith("https://example.com");
});
