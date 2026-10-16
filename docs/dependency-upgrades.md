# Dependency upgrade execution record

Approved sequence: toolchains and baseline; Whisper 0.16; coordinated Tauri; frontend/test tooling; reqwest 0.13; CPAL 0.18; Rubato 5. Source updates only, with Apple Silicon macOS 13+ support and unchanged Base/Small English model hashes.

## Real-model regression measurements

`tests/transcription-corpus/manifest.json` identifies fixed headerless signed little-endian PCM16 mono 16 kHz audio, with hashes and reference transcripts. The synthetic corpus uses original prompts generated with macOS `say` (Samantha) and ffmpeg conversion. Committed bytes, not a later voice regeneration, define the inputs. It contains no personal audio. Synthetic results supplement real microphone acceptance checks.

Build the release **test** executable with `cargo +1.88.0 test --manifest-path src-tauri/Cargo.toml --release --lib --no-run --locked --message-format=json`. Obtain its executable path from the `compiler-artifact` JSON record with `profile.test=true`. The ignored benchmark is test-only and uses production context loading, inference, chunking, cleanup and overlap joining; it is absent from distributed builds.

Run `RUSTUP_TOOLCHAIN=1.88.0 python3 scripts/transcription-benchmark.py --binary <test-executable> --models <directory-with-ggml-base.en.bin-and-ggml-small.en.bin> --engine 0.12.0 --output /tmp/baseline.json`. After upgrading, repeat with `--engine 0.16.0 --output /tmp/candidate.json --baseline /tmp/baseline.json`. Models and corpus are hash-verified before inference; model files are not committed.

Each clip/model starts in a fresh process: model load is timed separately, followed by one cold inference and five warm inferences reusing the context. Peak RSS is process-wide (bytes on macOS). Compare on the same machine/OS/toolchain, idle and on mains power, with identical backend/features. Keep raw JSON reports with the PR evidence. The comparison rejects different corpus/model hashes, machine, backend or build profile. Inspect raw long-clip outputs for newly repeated boundary sentences.

WER normalization lowercases, removes punctuation and collapses whitespace. Per-model corpus WER may increase by at most two absolute percentage points; median warm latency by at most ten percent; peak RSS by at most twenty percent. New blank speech or new output on previously blank nonspeech fails. Gates are not relaxed automatically; investigate a failure and keep an upgrade unmerged if unresolved.

## Deferred compatibility work

WebdriverIO 10 remains deferred while the Tauri test service declares a WebdriverIO 9 peer. Do not force peer resolution, loosen audit checks or overwrite model hashes. Mixed bot PRs are closed only after all contents map to merged replacements or explicit deferred work.
