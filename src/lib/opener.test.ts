import { vi } from "vitest";

const openUrl = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (...args: unknown[]) => openUrl(...args) }));

import { openExternal } from "./opener";
import capability from "../../src-tauri/capabilities/default.json";

afterEach(() => {
  openUrl.mockClear();
});

test("openExternal opens the given url via the system browser", async () => {
  await openExternal("https://example.com");
  expect(openUrl).toHaveBeenCalledWith("https://example.com");
});

test("the preferences capability allows opening the source and license URL", () => {
  expect(capability.windows).toContain("prefs");
  expect(capability.permissions).toContainEqual({
    identifier: "opener:allow-open-url",
    allow: [{ url: "https://github.com/kacheo/humhum" }],
  });
});
