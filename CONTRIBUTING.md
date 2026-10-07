# Contributing to Wipzi

Open an issue before a large change. Contributions are licensed under AGPL-3.0-only, the same license as Wipzi. Focus each PR on one problem and explain the resulting behavior.

## Development

The supported development target is Apple Silicon macOS 13+. Install Node.js 20+, Rust stable, Xcode Command Line Tools, CMake, and Python 3. Run `npm ci`, then `npm run tauri dev`. Unsigned development builds have a different signing identity from releases and may need their own permission grants.

The React frontend lives in `src`: components render onboarding, Preferences, and the recording overlay; Zustand stores hold UI state; `src/lib/ipc.ts` defines calls into Tauri. The Rust backend in `src-tauri/src` handles audio capture, local Whisper transcription, model downloads, optional text cleanup, Keychain preferences, permissions, hotkeys, and pasting. `e2e` exercises the real debug app with a mock server and temporary preferences.

Run before submitting a PR:

```sh
npm test
npm run test:release
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Rust network tests start loopback mock servers, and Keychain tests use an isolated `wipzi-api-test` namespace. Run them in a normal macOS session; restrictive sandboxes may block these operations.

For UI integration checks, build with `npx tauri build --debug --features e2e-harness --bundles app`, then run `npm run test:e2e`. The API-key case is excluded until its session/Keychain behavior is verified; see [CI notes](docs/ci.md). Never enable `e2e-harness` in a production release.

## Review expectations

Include a focused regression test for behavior changes. Explain new permissions, network calls, dependencies, or stored data. Check dependency licenses and regenerate notices before distributing a changed bundle. Use the [manual checklist](docs/manual-test-checklist.md) for release behavior that requires a real Mac.

Do not commit API keys, Apple signing credentials, downloaded model weights, recordings, or private transcripts. Public PR checks use GitHub-hosted runners without signing secrets. Maintainers review and publish releases; contributors do not need Apple credentials.
