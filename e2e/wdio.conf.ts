import path from "node:path";

// One `wdio run` process = one spec file, launched by `run-all.ts` (see
// plan Section 4: each flow needs differently-seeded $HOME/settings.json/
// Keychain state, and onPrepare only runs once per `wdio run` process, so
// sharing a session across specs would leak state between them).
//
// `run-all.ts` does all the per-spec setup (scratch $HOME, mock server,
// fixture hashing, Keychain seeding) in its own process *before* spawning
// this one, then hands the results down as env vars on this process — see
// the `WIPZI_E2E_*` / `WIPZI_*` vars read below. That keeps this file a
// static config (no onPrepare/onComplete asymmetry to reason about) and
// lets `wdio run e2e/wdio.conf.ts --spec <file>` also be run standalone for
// manual debugging (falls back to sane defaults when the env vars are
// absent, e.g. no mock server → the app just talks to the real endpoints).

// The service's embedded provider spawn()s this path directly rather than
// resolving into a .app bundle — point at the executable inside
// Contents/MacOS, not the bundle directory itself (spawning the directory
// fails with EACCES despite the docs' .app-bundle example).
const defaultAppPath = path.resolve(
  import.meta.dirname,
  "..",
  "src-tauri/target/debug/bundle/macos/Wipzi.app/Contents/MacOS/wipzi",
);
const appPath = process.env.WIPZI_E2E_APP_PATH ?? defaultAppPath;

// Passed straight through to the spawned app's environment. HOME redirects
// prefs/model/settings storage (both derive their paths from $HOME — see
// prefs::settings_path() and models::mod.rs); the WIPZI_* vars are the
// debug-only overrides added for this harness (base URLs, model fixture
// hash/size, permission force, prefs-on-startup). Anything run-all.ts didn't
// set is simply omitted, so a standalone `wdio run` still boots the app
// against its real defaults.
const passthroughKeys = Object.keys(process.env).filter(
  (k) => k === "HOME" || k.startsWith("WIPZI_"),
);
const appEnv: Record<string, string> = {};
for (const k of passthroughKeys) {
  const v = process.env[k];
  if (v !== undefined) appEnv[k] = v;
}

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./specs/*.e2e.ts"],
  maxInstances: 1,
  services: [
    [
      "@wdio/tauri-service",
      {
        driverProvider: "embedded",
        appBinaryPath: appPath,
        windowLabel: "prefs",
        captureBackendLogs: true,
        captureFrontendLogs: true,
        logLevel: "debug",
        startTimeout: 60000,
        env: appEnv,
      },
    ],
  ],
  capabilities: [
    { browserName: "tauri", "tauri:options": { application: appPath } },
  ],
  logLevel: "debug",
  outputDir: path.resolve(import.meta.dirname, "logs"),
  bail: 0,
  waitforTimeout: 10000,
  connectionRetryTimeout: 90000,
  connectionRetryCount: 3,
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60000 },
  reporters: ["spec"],
};
