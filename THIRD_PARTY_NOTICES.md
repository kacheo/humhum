# Third-party components

Copyright © 2026 Kalon Cheong and contributors. Humhum is AGPL-3.0-only. Dependencies retain their own licenses; they are not relicensed by this project.

The locked Apple Silicon production/build dependency inventory is in [docs/licenses/INVENTORY.md](docs/licenses/INVENTORY.md). Full required license and attribution texts are in [RUST_NOTICES.txt](docs/licenses/RUST_NOTICES.txt) and [NPM_NOTICES.txt](docs/licenses/NPM_NOTICES.txt). Build-only components are conservatively included; development tools and other target platforms are excluded. Regeneration instructions are in [the release guide](docs/release.md).

Rust licenses include MIT, Apache-2.0, BSD, ISC, Unicode-3.0, MPL-2.0, Zlib, and Unlicense. The notices include the separate MIT license for vendored whisper.cpp/ggml and native/BoringSSL notices when ring is included in the target graph. Where published crates omit a project-specific license file, exact upstream license declarations and package attribution are preserved in [upstream notice sources](docs/licenses/upstream/manifest.json). New dependency versions require fresh review rather than copying old attribution blindly.

MPL-covered components retain MPL-2.0 file-level terms. Their preferred-form source is available in the exact crates.io source packages linked from the inventory; maintain that source availability when redistributing. Exact Rust source packages can also be obtained as `https://crates.io/api/v1/crates/<name>/<version>/download`, using the inventory and Cargo.lock checksums. Humhum does not modify these dependency sources.

The license and notice files are packaged under `Humhum.app/Contents/Resources/licenses` in corrected builds. The historical v0.1.3 installer omitted these packaged files; do not overwrite that asset. See [the preparation record](docs/public-readiness.md) for the corrected launch candidate.

## Downloaded models and optional services

Whisper `ggml-base.en.bin` and `ggml-small.en.bin` download on demand from [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp). The model card identifies them as MIT licensed. They are not included in this repository or installer. Their downloads are integrity-checked against pinned hashes.

Optional Ollama, OpenAI, and Anthropic services have their own terms and privacy policies. Selecting cloud cleanup sends transcript text to the selected provider with a user-supplied key; it does not send recorded audio.
