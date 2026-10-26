# Memory lifecycle changes and measurement

The 15 GB mostly-idle incident remains unexplained. The app was not running
when inspected, and the isolated permission-poll experiment plateaued around
16 MB with and without autorelease pools. These changes bound known resource
lifetimes; they do not establish the incident's cause.

## Implementation

One process-wide, persistent native worker admits one recording at a time.
Dropping its caller (including the pipeline timeout) signals a job-owned atomic
cancellation flag. The worker checks before/after model loading and inference;
Whisper also receives a raw abort callback borrowing that flag for the duration
of its synchronous native call. Callback storage outlives all native calls.

Admission belongs to the job until execution exits, including errors and Rust
unwinding. Retry recordings are rejected rather than retained in a queue. A
native call that ignores cancellation keeps admission; the error explains that
Humhum needs restarting if it stays unavailable. No replacement worker is
started. Abandoned output never returns to the pipeline; cancelled jobs release
their context when native work exits. Native process aborts cannot be recovered.

The worker owns one context, drops the old context before switching models, and
unloads it after five minutes idle. Idle time starts when a job finishes, so
inference cannot race eviction. A later recording reloads the selected model.

Latest main (52cfb67) upgraded Whisper to 0.16 and removed application chunking.
This change preserves its whole-recording inference, decoding parameters,
non-speech filtering, silence handling, and per-recording state destruction.
No chunk copies or overlap joining are reintroduced.

Capture converts interleaved input directly into a bounded mono buffer, accepting
at most `device sample rate × 330` samples, with checked arithmetic. Existing
time-based stop behavior still applies. Permission checks and native worker
operations run in synchronous autorelease pools; none span an `await`.
IPC and preferences are unchanged.

## Opt-in measurements

For the rebuilt app, launch its executable with a **new** JSONL output path:

```sh
HUMHUM_MEMORY_DIAGNOSTICS=/tmp/humhum-memory.jsonl \
  /Applications/Humhum.app/Contents/MacOS/humhum
```

The first record contains build revision, dependency-lock hash, compiler, optimization/debug
settings, target/features, requested GPU setting, and Whisper system info. Every 30 seconds,
it records current kernel RSS/physical footprint, recording phase, active jobs,
cached model, and capture/inference sample counts. Output stops at 5 MB. It does
not contain audio, transcript text, or keys. Counters are metadata sampled
independently and can briefly differ during transitions. `gpu_requested` is a
request, not proof of the backend actually selected: retain native model-init
logs to identify CPU/Metal/BLAS use. Dependency versions are pinned by Cargo.lock.

Capture main process and explicitly attributed WebKit helpers separately:

```sh
python3 scripts/memory-diagnostics.py --pid MAIN_PID \
  --binary /Applications/Humhum.app/Contents/MacOS/humhum \
  --helper-pid ATTRIBUTED_WEBKIT_PID --output /tmp/humhum-memory-run
```

Omit `--helper-pid` if no helper has been attributed to this app; repeat it for
multiple helpers. Attribute helpers with Activity Monitor / process inspection,
not name matching across every WebKit process. The collector fingerprints the
binary and current source/lockfile separately (they need not match), records
macOS/kernel identity, and detects process exit/PID reuse. It samples every
30 seconds for eight hours by default. At five-minute warm-up and subsequent
256 MiB footprint increases it captures `vmmap -summary` and `heap -s` output
(up to three growth snapshots per process). All collector files share a 5 MB
budget. Tool permission failures/exit codes are recorded; missing snapshots
are not evidence of absent allocations. Snapshots may contain VM paths and
allocation class names; inspect before sharing.

An old installed binary cannot emit new in-app diagnostics. Use the external
collector for a pre-change baseline, keeping its binary hash distinct. No
pre-change idle measurement was possible in this session because Humhum was
not running. Observed installed SHA256 before syncing main:
`1ad37b79c13d2b1ead6a71f38ccdad4b025da1a0a8a3d1dbe0bb25b68f080342`;
macOS 27.0.1 (26A434); source then 26f5e6d. Installed inference backend unknown.

## Deferred validation

Tests and benchmark execution are deferred at the user's request. Added
regressions cover worker timeout/drop admission, native error/panic recovery,
context replacement and failed load, controllable idle eviction/reload, callback
cancellation, capture limits, bounded diagnostic output, and late pipeline
output/paste suppression. Before merging/releasing, run:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
npm test -- --exclude '.worktrees/**'
npm run build
python3 -m unittest discover -s scripts/tests -v
```

Use `npm test -- --exclude '.worktrees/**'` when this checkout contains other worktrees: the bare
Vitest command currently discovers their tests too and can load incompatible
React instances (a path filter of `src` also matches those nested paths).
The early frontend attempts hit those worktree discovery errors; early Rust
attempts stopped on tests referring to not-yet-implemented helpers. Neither
attempt validates the final diff. No final test pass is claimed for this PR.

Then compare equivalent builds and actual backends through cold idle, idle after
one dictation, Preferences visible/hidden, and sleep/wake. Run 100 dictations in
one process alternating models, followed by eight hours idle and sleep/wake.
Track main/helper footprints separately and worker/context counts throughout.
After warm-up require retained footprint growth below 64 MiB, idle growth below
5 MiB/hour, active worker count ≤1 and cached context count ≤1. Preserve existing
corpus accuracy checks and paired benchmark gates: warm latency ≤10% regression
and peak memory ≤20% regression. The existing benchmark deliberately uses fresh
processes per clip and cannot establish long-lived idle behavior. Do not mark the
15 GB incident resolved without reproduction or allocation attribution.
