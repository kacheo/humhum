# Wipzi

Wipzi is a free, open-source macOS menu bar app for voice dictation. Hold a global shortcut, speak, and paste a locally transcribed result into the active app. Audio is processed on your Mac. Optional text cleanup uses local Ollama or an OpenAI/Anthropic API key you provide; cloud cleanup sends transcript text to that provider.

## Install

Download the signed, notarized `.dmg` from [GitHub Releases](https://github.com/kacheo/wipzi/releases). macOS 13 or later is required. Open Wipzi and follow onboarding to grant Microphone, Accessibility, and Input Monitoring permissions. The default shortcut is Option+Shift+Space.

Wipzi starts with fresh preferences, model downloads, and API keys. It does not read or alter the previous app's data. Quit the older app before running Wipzi to avoid a shortcut conflict.

## Build and test

Install Node.js 20+, Rust stable, Xcode Command Line Tools, and CMake. On macOS:

```sh
git clone https://github.com/kacheo/wipzi.git
cd wipzi
npm ci
npm run tauri dev
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run tauri build` creates an unsigned local bundle. Signed releases use the workflow in [docs/release.md](docs/release.md). The app never requires a license key or payment. A recording stops after five minutes as a safety limit.

## Privacy

Whisper transcription runs locally. Model files download from Hugging Face when selected. Ollama cleanup runs against the configured local endpoint. Cloud cleanup sends transcript text, not audio, to the selected provider using your own key; keys are stored in macOS Keychain. See the provider's privacy terms before enabling cloud cleanup.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and pull requests. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## License

Copyright © 2026 Kalon Cheong and contributors. Wipzi is licensed under the [GNU Affero General Public License v3.0](LICENSE). Each GitHub Release links to the source tag used to build it. Dependencies and downloaded models retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
