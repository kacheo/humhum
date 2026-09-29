# Contributing to Wipzi

Wipzi is a macOS Tauri app with a Rust backend and React frontend. Open an issue before a large change. For a pull request, run `npm ci`, `npm test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml` on macOS. Include a focused test for behavior changes and explain any new permissions, network calls, or dependencies.

Do not commit API keys, signing credentials, downloaded model files, recordings, or transcripts. Public pull requests run on GitHub-hosted runners without release secrets. Maintainers review and publish releases.

Contributions are licensed under AGPLv3, the same license as the project.
