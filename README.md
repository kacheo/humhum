# Wipzi

Wipzi is a free, open-source macOS menu bar app for voice dictation. Hold a global shortcut, speak, and paste locally transcribed text into the active app.

**Public beta target: Apple Silicon Macs, macOS 13 or later, English transcription.** The current installer is Apple Silicon only; Intel Macs are not yet supported.

<img src="assets/screenshots/preferences.png" alt="Wipzi General preferences" width="720">

<img src="assets/screenshots/recording.png" alt="Wipzi recording overlay" width="320">

Screenshots show the existing UI with synthetic state.

The public-launch candidate is being verified; see [release readiness](docs/public-readiness.md).

## Install and first run

1. Download the Apple Silicon `.dmg` from [GitHub Releases](https://github.com/kacheo/wipzi/releases). Releases are Developer ID signed and notarized.
2. Drag Wipzi into Applications, eject the disk image, and open `/Applications/Wipzi.app`.
3. Follow onboarding to grant Microphone, Accessibility, and Input Monitoring permissions.
4. Download Base English (about 148 MB). Small English is optional (about 488 MB). Allow additional free space for downloads and app operation.
5. Focus a text field in another app. Hold **Option+Shift+Space**, speak, and release to transcribe and paste. Escape cancels a recording. Recordings stop automatically after five minutes.

Change the shortcut, input device, model, and cleanup mode in Preferences. The menu-bar menu opens Preferences and quits the app. Closing Preferences hides the window.

See [troubleshooting](docs/troubleshooting.md) for setup problems. This is a beta: review dictated text before sending or submitting it.

## Privacy and optional cleanup

Whisper transcription runs on your Mac. After a model is downloaded, raw transcription works offline. Model downloads contact Hugging Face and are checked against pinned SHA-256 hashes.

Cleanup is off by default:

- **None:** paste the raw local transcript.
- **Local Ollama:** send transcript text to your configured Ollama endpoint. Use a local endpoint to keep processing on your Mac.
- **Cloud API:** send transcript text, not audio, to OpenAI or Anthropic using your own key. Keys are stored in macOS Keychain. Provider terms, retention policies, and usage charges apply.

See [cleanup setup and local data removal](docs/troubleshooting.md). Never include private recordings, transcript text, or API keys in bug reports.

## Build and test

Install Node.js 20+, Rust stable, Xcode Command Line Tools, CMake, and Python 3. On an Apple Silicon Mac:

```sh
git clone https://github.com/kacheo/wipzi.git
cd wipzi
npm ci
npm run tauri dev
```

Run checks separately:

```sh
npm test
npm run test:release
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

`npm run tauri build` creates an unsigned development bundle. Signed releases use [the release guide](docs/release.md). No license key or payment is required to use Wipzi.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for architecture, checks, and pull requests, and the [roadmap](docs/roadmap.md) for priorities. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## License

Copyright © 2026 Kalon Cheong and contributors. Wipzi is licensed under [AGPL-3.0-only](LICENSE). Release notes link to the exact source tag. Dependencies and downloaded models retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
