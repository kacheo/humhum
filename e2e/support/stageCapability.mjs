// Copies/removes src-tauri/capabilities/e2e.json around an e2e-harness build.
// The file must not exist for a normal (non-e2e-harness) `cargo build`/`tauri
// build` — Tauri's build-time permission resolver only knows about
// wdio-webdriver:default when the optional plugin dependency is actually
// compiled in, so a permanently-present capability file referencing it
// breaks every other build. See e2e-capability.json's own description.

import { copyFileSync, existsSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const template = path.join(here, "e2e-capability.json");
const staged = path.join(here, "..", "..", "src-tauri", "capabilities", "e2e.json");

const mode = process.argv[2];
if (mode === "stage") {
  copyFileSync(template, staged);
} else if (mode === "unstage") {
  if (existsSync(staged)) rmSync(staged);
} else {
  console.error("usage: stageCapability.mjs <stage|unstage>");
  process.exit(1);
}
