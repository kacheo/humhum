#!/usr/bin/env bash
# Rust unit-test coverage gate for the src-tauri crate.
#
# Excludes native/FFI/runtime code that can't meaningfully run under a unit
# test: macOS-native modules (objc2/coreaudio-sys, matched by the `macos.rs`
# filename convention), the Tauri app entrypoint (lib.rs sets up the tray/
# window runtime; main.rs is a one-line bootstrap). The remaining pure-logic
# surface is gated at 95% line coverage.
#
# Must run on macOS so the #[cfg(target_os = "macos")] dispatch arms (not
# their non-macOS fallbacks) are the ones measured — matching CI and the
# actual shipped app.
#
# Usage: scripts/coverage-rust.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo llvm-cov \
  --manifest-path src-tauri/Cargo.toml \
  --ignore-filename-regex '(macos\.rs|/lib\.rs|/main\.rs)$' \
  --fail-under-lines 95 \
  --lcov --output-path src-tauri/lcov.info \
  --summary-only
