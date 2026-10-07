import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const recorded = JSON.parse(readFileSync("docs/licenses/lockfiles.json", "utf8"));
for (const path of ["package-lock.json", "src-tauri/Cargo.lock", "about.toml"]) {
  const current = createHash("sha256").update(readFileSync(path)).digest("hex");
  if (recorded[path] !== current) {
    console.error(`Dependency notices need regeneration: ${path} changed. See docs/release.md.`);
    process.exit(1);
  }
}
console.log("Dependency notices match the reviewed lockfiles and license policy.");
