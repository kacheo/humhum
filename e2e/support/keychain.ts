// Scratch macOS Keychain lifecycle for the API-key E2E spec, so it never
// touches a real login keychain in CI or locally. `keyring::Entry` (the Rust
// side, prefs/mod.rs) resolves to whatever
// `security default-keychain` currently points at, so pointing the default at
// a fresh, unlocked scratch keychain for the duration of the test is
// sufficient — no capability/entitlement changes needed on the app side.
import { execFile, execFileSync } from "node:child_process";
import { promisify } from "node:util";
import { randomUUID } from "node:crypto";
import path from "node:path";
import os from "node:os";

const exec = promisify(execFile);

const SCRATCH_PASSWORD = "e2e-scratch";

export interface ScratchKeychain {
  path: string;
  previousDefault: string;
  restore(): Promise<void>;
  /**
   * Synchronous best-effort variant of `restore()` for signal/exit handlers,
   * which can't `await`. Reverting the *system default keychain* is global
   * state that must complete before the process exits, so this uses
   * `execFileSync`. Safe to call in addition to (or instead of) `restore()`.
   */
  restoreSync(): void;
}

async function currentDefaultKeychain(): Promise<string> {
  const { stdout } = await exec("security", ["default-keychain"]);
  // Output is a quoted path, e.g. `    "/Users/x/Library/Keychains/login.keychain-db"\n`
  return stdout.trim().replace(/^"|"$/g, "");
}

// The full user keychain search list, as plain paths. Snapshotting the whole
// list (not just the default) lets restore put it back verbatim, so a non-stock
// setup with extra keychains isn't silently trimmed on teardown.
async function currentUserSearchList(): Promise<string[]> {
  const { stdout } = await exec("security", ["list-keychains", "-d", "user"]);
  return stdout
    .split("\n")
    .map((l) => l.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

/**
 * Create a scratch default keychain for the duration of a test.
 *
 * `onRevertReady` is called with the synchronous revert **as soon as the scratch
 * keychain exists and before the global `default-keychain` flip below**, so an
 * interrupt at any point after the scratch is created — including mid-creation,
 * before this function returns — can still fully restore global keychain state
 * (see `run-all.ts`'s signal teardown). Each revert step is idempotent and a
 * no-op if its corresponding flip hasn't happened yet.
 */
export async function createScratchKeychain(
  onRevertReady?: (revertSync: () => void) => void,
): Promise<ScratchKeychain> {
  const previousDefault = await currentDefaultKeychain();
  const previousSearchList = await currentUserSearchList();
  const scratchPath = path.join(os.tmpdir(), `humhum-e2e-${randomUUID()}.keychain-db`);

  await exec("security", ["create-keychain", "-p", SCRATCH_PASSWORD, scratchPath]);

  // Revert steps restore the EXACT prior global state (default keychain + full
  // user search list), then delete the scratch. Shared by the async `restore()`
  // and the sync `restoreSync()` so the two paths can't drift. Built + registered
  // here, before the global flip, to close the mid-creation interrupt window.
  const revertSteps: string[][] = [
    ["default-keychain", "-s", previousDefault],
    ...(previousSearchList.length
      ? [["list-keychains", "-d", "user", "-s", ...previousSearchList]]
      : []),
    ["delete-keychain", scratchPath],
  ];

  const restore = async (): Promise<void> => {
    for (const args of revertSteps) {
      await exec("security", args).catch(() => {});
    }
  };

  const restoreSync = (): void => {
    for (const args of revertSteps) {
      try {
        // `timeout` so a wedged `security` can't block the exit path forever.
        execFileSync("security", args, { stdio: "ignore", timeout: 5000 });
      } catch {
        // best-effort: keep reverting the remaining steps
      }
    }
  };

  onRevertReady?.(restoreSync);

  await exec("security", ["unlock-keychain", "-p", SCRATCH_PASSWORD, scratchPath]);
  // No auto-lock during the (short) test run — avoids a re-prompt mid-spec.
  await exec("security", ["set-keychain-settings", scratchPath]);
  // Global flips — recoverable from here on via the revert registered above.
  await exec("security", ["default-keychain", "-s", scratchPath]);
  await exec("security", ["list-keychains", "-d", "user", "-s", scratchPath, ...previousSearchList]);

  return { path: scratchPath, previousDefault, restore, restoreSync };
}
