# Dependency upgrade execution record

Approved sequence: toolchains and baseline; evaluate Whisper 0.16; coordinated Tauri; frontend/test tooling; reqwest 0.13; CPAL 0.18; Rubato 5. The initial Whisper candidate was deferred after its manual quality gate failed; the whole-recording follow-up is described below. Source updates only, with Apple Silicon macOS 13+ support and unchanged Base/Small English model hashes.

## Real-model regression measurements

`tests/transcription-corpus/manifest.json` identifies fixed headerless signed little-endian PCM16 mono 16 kHz audio, with hashes and reference transcripts. The synthetic corpus uses original prompts generated with macOS `say` (Samantha) and ffmpeg conversion. Committed bytes, not a later voice regeneration, define the inputs. It contains no personal audio. Synthetic results supplement real microphone acceptance checks.

Build the release **test** executable with `cargo +1.90.0 test --manifest-path src-tauri/Cargo.toml --release --lib --no-run --locked --message-format=json`. Obtain its executable path from the `compiler-artifact` JSON record with `profile.test=true`. The ignored benchmark is test-only and uses production context loading, whole-recording inference and transcript normalization; it is absent from distributed builds.

Run `RUSTUP_TOOLCHAIN=1.90.0 python3 scripts/transcription-benchmark.py --binary <test-executable> --models <directory-with-ggml-base.en.bin-and-ggml-small.en.bin> --engine 0.12.0 --output /tmp/baseline.json`. After upgrading, repeat with `--engine 0.16.0 --output /tmp/candidate.json --baseline /tmp/baseline.json`. Models and corpus are hash-verified before inference; model files are not committed.

Each clip/model starts in a fresh process: model load is timed separately, followed by one cold inference and five warm inferences reusing the context. Peak RSS is process-wide (bytes on macOS). Compare on the same machine/OS/toolchain, idle and on mains power, with identical backend/features. Keep raw JSON reports with the PR evidence. The comparison rejects different corpus/model hashes, machine, backend or build profile. Inspect raw long-clip outputs for newly repeated boundary sentences.

WER normalization lowercases, removes punctuation and collapses whitespace. Per-model corpus WER may increase by at most two absolute percentage points; median warm latency by at most ten percent; peak RSS by at most twenty percent. New blank speech or new output on previously blank nonspeech fails. Gates are not relaxed automatically; investigate a failure and keep an upgrade unmerged if unresolved.

## Deferred compatibility work

WebdriverIO 10 remains deferred while the Tauri test service declares a WebdriverIO 9 peer. Do not force peer resolution, loosen audit checks or overwrite model hashes. Mixed bot PRs are closed only after all contents map to merged replacements or explicit deferred work.

For a saturated local machine, dispatch **Transcription benchmark** with reviewed full baseline/candidate commit SHAs. The read-only workflow builds both release test executables before measuring, then runs the same corpus/models on one dedicated macOS runner and uploads synthetic JSON evidence. It has no signing credentials and does not publish installers.

## Toolchain compatibility

The coordinated Tauri2.12 stack requires Rust1.90. The declared minimum, contributor instructions and minimum-toolchain CI job move together. An isolated Whisper comparison built with1.88 remains evidence for that candidate; comparisons including the new Tauri stack must build both baseline and candidate with1.90.

The WDIO9 family remains compatible with Tauri test service1.5. Its development-only dependency graph currently reports21high audit findings (20before the service update), including the service's exposure to the existing WDIO advisories. The suggested automatic fix requires WDIO10 and is deferred with that incompatible major upgrade; production npm audit remains clean. Keep the existing security reporting and audit artifact.


## Whisper 0.16: whole-recording inference

The initial chunked candidate passed numerical gates in [paired run 38846208689](https://github.com/kacheo/humhum/actions/runs/38846208689), but Small English repeated fragments at the application’s overlapping chunk boundaries. That candidate was deferred. A text-only inexact matcher was rejected because it could erase distinct valid instructions.

The follow-up upgrades `whisper-rs` to 0.16.0 (`whisper-rs-sys` 0.15.0) and passes each complete 16 kHz mono recording to Whisper once. Whisper performs its own internal windowing; Humhum no longer copies 30-second overlapping chunks or deduplicates transcript text. The model cache, fresh state per recording, English greedy decoding, `no_context=true`, Base/Small model hashes, public transcription interfaces and recording/time limits remain unchanged. Exact digital silence returns `empty_result` before inference because Base hallucinated “you” on it with both the old and new engines; nonzero audio is decoded without a loudness threshold.

### Focused local checks

Build the release test executable as above, then run:

```sh
python3 scripts/transcription-benchmark.py --binary <test-executable> --models <model-directory> --engine 0.16.0 --smoke --output /tmp/whisper-smoke.json
```

Smoke mode runs `speech-01`, `speech-07`, `speech-09`, `speech-10`, silence and noise once per model, verifies model/corpus hashes and fails on blank speech or nonempty nonspeech output. Inspect raw text for missing sentences, repeated fragments and complete endings. `--model small_en --clips <IDs>` selects a smaller check; `--corpus <directory>` supports a scratch fixture with the same manifest format. Smoke reports are labelled and cannot serve as paired benchmark evidence. The default six-run benchmark remains optional and unchanged; no hosted benchmark campaign is required for this migration.

Local single-pass checks passed on both models: `speech-10` matches the reference after case/punctuation normalization, without repeated boundary fragments; silence and noise are blank. A scratch 300-second Small English fixture preserved all four deliberately repeated sections and the final sentence (fixture SHA-256 `e49fe62faa2d3328d56a2dd3589185e951efc7f330f9396f94aedd8c41482d21`). The native debug app builds and launches, but a live microphone-to-paste check remains manual: macOS reports microphone, Accessibility and Input Monitoring denied for this test build. PR #41 stays draft until that check is completed. These checks establish functional behavior on the tested clips, not performance equivalence or general recognition accuracy.


## Security patch updates for the memory lifecycle PR

The dependency audit on PR #56 reported inherited advisories in Tokio 1.53.2
and rustls-webpki 0.103.15. Update the lockfile to Tokio 1.53.3 and
rustls-webpki 0.103.16, and set the direct production/test Tokio minimum to
1.53.3. These are targeted patch updates; other dependency versions and audit
exceptions remain unchanged. Regenerate the production dependency inventory,
notices, and lockfile fingerprints from the updated graph.

The patched ranges are documented in
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html)
(Tokio AsyncFd I/O safety) and
[RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html)
(certificate verification with distrusted intermediates). Keep the existing
`cargo audit --file src-tauri/Cargo.lock --deny unsound` gate; no advisory is
silenced for these updates.
