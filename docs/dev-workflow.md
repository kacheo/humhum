# Testing Humhum on your Mac

Use **Humhum Dev** for native acceptance testing. It installs at `~/Applications/Humhum Dev.app`, with bundle ID `com.kacheo.humhum.dev`. Official Humhum remains at `/Applications/Humhum.app`. Quit one before launching the other so their dictation shortcuts do not compete. The commands never quit the official app for you.

## Signed native testing

Install the normal development prerequisites in CONTRIBUTING.md and run `npm ci`. Configure an available Developer ID Application certificate in your ignored `.env.local`:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
```

The certificate and private key must already be available in your login Keychain. Do not commit or share them. No Apple ID, notarization password, or network notarization step is needed for this local workflow. `security find-identity -v -p codesigning` lists available signing identities.

```sh
npm run dev:install
npm run dev:doctor
npm run dev:open
```

`dev:install` builds a host-architecture release app with bundled frontend assets, signs and verifies it, and replaces the same Dev installation. It does not launch it. Quit Dev before replacing it; official Humhum may remain running during the build/install. `dev:open` refuses to launch while another Humhum bundle or unbundled native development process is running. Stop a hot-reload session with Ctrl-C. Launch commands share the install/cleanup lock: `dev:open` waits for the new process to appear, and `dev:ui` retains the lock until its session exits.

On the first Dev launch, grant its own Microphone, Accessibility, and Input Monitoring permissions. The official app's grants are separate. Follow any macOS first-open security prompt. Later builds use the same certificate, bundle ID, and install path; verify permission retention across two builds before relying on it. Signing checks alone do not prove macOS retained the grants.

Dev starts with fresh settings and onboarding. Settings and model downloads are in `~/Library/Application Support/humhum-dev`; API keys use the `humhum-api-dev` Keychain service, with the existing provider account names. Logs are under `~/Library/Logs/com.kacheo.humhum.dev`. Production data and credentials are not imported or modified. Whisper transcription still runs locally.

## Fast UI iteration

```sh
npm run dev:ui
```

This starts Tauri/Vite hot reload with the isolated Dev profile. It does not require a signing certificate. The unbundled process is a different permission-testing target, so use the signed installed app for native acceptance. Both Dev loops share Dev settings/models/keys; switching signing identities may cause additional Keychain access prompts. Do not run them together.

The raw `npm run tauri dev` command retains the production profile for compatibility; prefer `dev:ui` for isolated work. Do not use raw builds to test permission retention. Contributors do not need maintainer signing credentials for frontend tests or UI development.

## App copies and recovery

```sh
npm run dev:doctor
npm run dev:clean
npm run dev:clean -- --apply
```

Doctor reports the signing requirement, running processes, indexed copies, and a cleanup preview; it never reads credential values. Cleanup only archives known build bundles, the old PR66 installation, and legacy development backups. It skips unknown identities and symlinked paths, refuses running copies, and protects official Humhum and the canonical Dev installation. Unknown app copies are inventory-only: review those manually. Spotlight may take time to drop archived entries.

Backups are compressed archives in `~/Library/Application Support/humhum-dev/build-backups`, rather than loose launchable apps. `previous.tar.gz` holds the most recently replaced installation. Cleanup archives live in `legacy/`; a leftover build is archived as `last-build.tar.gz` before rebuilding. Successful installation removes the loose Dev build bundle.

Failed builds/signature checks leave the installed app intact. A failed swap restores the previous installation, including an interruption during the rename. If restoration itself fails, the error names the retained recovery app and compressed backup. A signing-requirement change stops replacement: restore the original certificate/configuration rather than resetting permissions. To intentionally change signing identity, archive the old installation manually, remove it, and expect new grants; never overwrite official Humhum. Backup archives can be unpacked to a temporary directory for inspection/recovery; do not leave extracted backups among installed apps.

## Acceptance checklist

- Install build A, grant the three permissions, and test dictation into another app.
- Save, replace, remove, and cold-restart a Dev API key. Confirm official settings and keys remain independent.
- Install changed build B at the same path. Confirm the designated requirement matches and dictation works without granting permissions again.
- Check both local-only dictation and an explicitly selected live cleanup provider. Automated mock-provider tests do not establish live-provider behavior.
- Run doctor/cleanup preview and confirm only official Humhum and the canonical Dev app remain among managed loose app copies.

## Automated E2E

Keep the E2E harness separate from installable Dev builds. It uses temporary HOME and scratch-Keychain isolation; do not point it at your Dev or production settings. The `dev-app` and `e2e-harness` features cannot be combined.

```sh
(
  set -e
  node e2e/support/stageCapability.mjs stage
  trap 'node e2e/support/stageCapability.mjs unstage' EXIT
  npx tauri build --debug --features e2e-harness --bundles app
)
npm run test:e2e -- apiKeyTest
```

Always unstage the capability even if building fails. The default suite includes credential storage, cold restart, and removal, using mock providers and the real scratch Keychain. Never install an E2E-harness bundle as Humhum Dev.
