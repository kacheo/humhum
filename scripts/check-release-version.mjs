import { readFileSync } from "node:fs";

const npmVersion = JSON.parse(readFileSync("package.json", "utf8")).version;
const tauriVersion = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const cargo = readFileSync("src-tauri/Cargo.toml", "utf8");
const packageSection = cargo.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
const cargoVersion = packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!npmVersion || npmVersion !== tauriVersion || npmVersion !== cargoVersion || process.env.GITHUB_REF_NAME !== `v${npmVersion}`) {
  console.error("Release tag, npm, Tauri, and Cargo package versions must match.");
  process.exit(1);
}
console.log(`Release versions match: v${npmVersion}`);
