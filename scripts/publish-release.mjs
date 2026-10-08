import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";

const { WIPZI_DMG: installer, GITHUB_REF_NAME: tag, GITHUB_REPOSITORY: repository } = process.env;
if (!installer || !tag || !repository || !existsSync(installer)) throw new Error("Installer, release tag, and repository are required.");
const name = basename(installer);
if (name !== `Wipzi_${tag.replace(/^v/, "")}_aarch64.dmg`) throw new Error("Installer filename must match the Apple Silicon release tag.");
const scratch = mkdtempSync(join(tmpdir(), "wipzi-publish-"));
function gh(args) {
  const result = spawnSync("gh", args, { encoding: "utf8" });
  if (result.error || result.status !== 0) throw new Error(result.error?.message ?? result.stderr);
  return result.stdout;
}
function checksum(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}
function verifyChecksum(path, expected) {
  const entry = readFileSync(path, "utf8").split(/\r?\n/).map(line => line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/)).find(match => match?.[2] === name);
  if (!entry || entry[1].toLowerCase() !== expected) throw new Error("Existing release checksum does not match its installer; no assets were overwritten.");
}
try {
  const view = spawnSync("gh", ["release", "view", tag, "--repo", repository, "--json", "assets,body"], { encoding: "utf8" });
  const sums = join(scratch, "SHA256SUMS.txt");
  if (view.error) throw view.error;
  if (view.status !== 0) {
    if (!/release not found|HTTP 404/i.test(view.stderr)) throw new Error(view.stderr);
    writeFileSync(sums, `${checksum(installer)}  ${name}\n`);
    const notes = join(scratch, "release-notes.md");
    writeFileSync(notes, `Source: https://github.com/${repository}/tree/${tag}\n\nApple Silicon beta; macOS 13+. Licensed under AGPL-3.0-only. See LICENSE, THIRD_PARTY_NOTICES.md, and dependency source links in docs/licenses/INVENTORY.md at the source tag.\n`);
    gh(["release", "create", tag, installer, sums, "--repo", repository, "--title", `Wipzi ${tag}`, "--notes-file", notes, "--verify-tag"]);
  } else {
    const assets = JSON.parse(view.stdout).assets.map(asset => asset.name);
    let bytes = installer;
    if (assets.includes(name)) {
      gh(["release", "download", tag, "--repo", repository, "--pattern", name, "--dir", scratch]);
      bytes = join(scratch, name);
    }
    const digest = checksum(bytes);
    if (assets.includes("SHA256SUMS.txt")) {
      gh(["release", "download", tag, "--repo", repository, "--pattern", "SHA256SUMS.txt", "--dir", scratch]);
      verifyChecksum(sums, digest);
    } else {
      writeFileSync(sums, `${digest}  ${name}\n`);
    }
    const missing = [];
    if (!assets.includes(name)) missing.push(installer);
    if (!assets.includes("SHA256SUMS.txt")) missing.push(sums);
    if (missing.length) gh(["release", "upload", tag, ...missing, "--repo", repository]);
    // Never edit notes or replace published bytes. On partial-upload recovery,
    // checksums refer to the original downloaded installer, not this rebuild.
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
