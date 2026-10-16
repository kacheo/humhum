# Dependency upgrade execution record

Approved sequence: toolchains and baseline; evaluate Whisper 0.16; coordinated Tauri; frontend/test tooling; reqwest 0.13; CPAL 0.18; Rubato 5. Whisper is deferred after its manual quality gate failed. Source updates only, with Apple Silicon macOS 13+ support and unchanged Base/Small English model hashes.

## Real-model regression measurements

`tests/transcription-corpus/manifest.json` identifies fixed headerless signed little-endian PCM16 mono 16 kHz audio, with hashes and reference transcripts. The synthetic corpus uses original prompts generated with macOS `say` (Samantha) and ffmpeg conversion. Committed bytes, not a later voice regeneration, define the inputs. It contains no personal audio. Synthetic results supplement real microphone acceptance checks.

Build the release **test** executable with `cargo +1.90.0 test --manifest-path src-tauri/Cargo.toml --release --lib --no-run --locked --message-format=json`. Obtain its executable path from the `compiler-artifact` JSON record with `profile.test=true`. The ignored benchmark is test-only and uses production context loading, inference, chunking, cleanup and overlap joining; it is absent from distributed builds.

Run `RUSTUP_TOOLCHAIN=1.90.0 python3 scripts/transcription-benchmark.py --binary <test-executable> --models <directory-with-ggml-base.en.bin-and-ggml-small.en.bin> --engine 0.12.0 --output /tmp/baseline.json`. After upgrading, repeat with `--engine 0.16.0 --output /tmp/candidate.json --baseline /tmp/baseline.json`. Models and corpus are hash-verified before inference; model files are not committed.

Each clip/model starts in a fresh process: model load is timed separately, followed by one cold inference and five warm inferences reusing the context. Peak RSS is process-wide (bytes on macOS). Compare on the same machine/OS/toolchain, idle and on mains power, with identical backend/features. Keep raw JSON reports with the PR evidence. The comparison rejects different corpus/model hashes, machine, backend or build profile. Inspect raw long-clip outputs for newly repeated boundary sentences.

WER normalization lowercases, removes punctuation and collapses whitespace. Per-model corpus WER may increase by at most two absolute percentage points; median warm latency by at most ten percent; peak RSS by at most twenty percent. New blank speech or new output on previously blank nonspeech fails. Gates are not relaxed automatically; investigate a failure and keep an upgrade unmerged if unresolved.

## Deferred compatibility work

WebdriverIO 10 remains deferred while the Tauri test service declares a WebdriverIO 9 peer. Do not force peer resolution, loosen audit checks or overwrite model hashes. Mixed bot PRs are closed only after all contents map to merged replacements or explicit deferred work.

For a saturated local machine, dispatch **Transcription benchmark** with reviewed full baseline/candidate commit SHAs. The read-only workflow builds both release test executables before measuring, then runs the same corpus/models on one dedicated macOS runner and uploads synthetic JSON evidence. It has no signing credentials and does not publish installers.

## Toolchain compatibility

The coordinated Tauri2.12 stack requires Rust1.90. The declared minimum, contributor instructions and minimum-toolchain CI job move together. An isolated Whisper comparison built with1.88 remains evidence for that candidate; comparisons including the new Tauri stack must build both baseline and candidate with1.90.

The WDIO9 family remains compatible with Tauri test service1.5. Its development-only dependency graph currently reports21high audit findings (20before the service update), including the service's exposure to the existing WDIO advisories. The suggested automatic fix requires WDIO10 and is deferred with that incompatible major upgrade; production npm audit remains clean. Keep the existing security reporting and audit artifact.


## Whisper deferral

Whisper0.16 passes the numerical gates in [paired run38846208689](https://github.com/kacheo/humhum/actions/runs/38846208689), but Small English introduces two repeated chunk-boundary fragments in the long speech-10 clip across all six runs. Manual inspection therefore rejects the upgrade. A text-only inexact matcher was also rejected because it can collapse distinct valid instructions. Retain Whisper0.12 and the existing transcription/joining behavior while the other dependency upgrades proceed. A future migration needs timing-aware overlap alignment and fresh real-model gates; PR#41 remains draft and Dependabot#36 stays open.
