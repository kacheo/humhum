#!/usr/bin/env bash
# Verify the signed/notarized Wipzi build.
#
# Asserts that the built .app is signed with a Developer ID Application identity
# (hardened runtime) and that the .dmg is notarized + stapled — the conditions
# that make the TCC designated requirement stable across version bumps, so a
# single Accessibility / Input Monitoring grant survives updates.
#
# Exits non-zero on the first failed check. Run after `tauri build` (the local
# build script calls this automatically for signed builds; CI runs it before
# publishing).
#
# SKIP_NOTARIZE=1 verifies only the .app signature + designated requirement and
# skips the .dmg notarization/staple checks — for fast local signed builds where
# the stable identity (not notarization) is what makes TCC grants persist.
#
# Usage: scripts/verify-signing.sh [app-path] [dmg-path]
#   With no args it auto-discovers the most recent .app and .dmg under
#   src-tauri/target/*/release/bundle (and the non-target path too).
set -euo pipefail
cd "$(dirname "$0")/.."

fail() { echo "verify-signing: FAIL: $*" >&2; exit 1; }

app="${1:-}"
dmg="${2:-}"

if [[ -z "$app" ]]; then
  app=$(find src-tauri/target -path '*/release/bundle/macos/*.app' -maxdepth 6 2>/dev/null \
        | head -1)
fi
if [[ -z "$dmg" ]]; then
  dmg=$(find src-tauri/target -path '*/release/bundle/dmg/*.dmg' -maxdepth 6 2>/dev/null \
        | head -1)
fi

# Tauri deletes the standalone .app after packing it into the .dmg, so when no
# loose .app exists we inspect the copy inside the dmg by mounting it read-only.
mountpoint=""
cleanup() { [[ -n "$mountpoint" ]] && hdiutil detach "$mountpoint" -quiet 2>/dev/null || true; }
trap cleanup EXIT

if [[ -z "$app" || ! -d "$app" ]]; then
  [[ -n "$dmg" && -f "$dmg" ]] || fail "no .app and no .dmg found (build first, or pass paths as args)"
  echo "verify-signing: standalone .app not found — inspecting the .app inside the dmg"
  mountpoint=$(mktemp -d /tmp/wipzi-verify.XXXXXX)
  hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mountpoint" -quiet \
    || fail "could not mount $dmg"
  app=$(find "$mountpoint" -maxdepth 2 -name '*.app' 2>/dev/null | head -1)
fi

[[ -n "$app" && -d "$app" ]] || fail "no .app found (pass it as arg 1)"
echo "verify-signing: app = $app"
if [[ -z "${SKIP_NOTARIZE:-}" ]]; then
  [[ -n "$dmg" && -f "$dmg" ]] || fail "no .dmg found (pass it as arg 2)"
  echo "verify-signing: dmg = $dmg"
fi

# 1. The .app is signed by Developer ID Application and carries a Team ID.
echo "== codesign -dvvv (signing identity) =="
sig=$(codesign -dvvv --verbose=4 "$app" 2>&1) || fail "codesign -dvvv failed"
echo "$sig"
echo "$sig" | grep -q "Authority=Developer ID Application:" \
  || fail "not signed by 'Developer ID Application' (ad-hoc/Development build?)"
echo "$sig" | grep -q "^TeamIdentifier=[A-Z0-9]\{10\}" \
  || fail "missing 10-char TeamIdentifier"

# 2. Signature is intact and complete.
echo "== codesign --verify --deep --strict =="
codesign --verify --deep --strict --verbose=2 "$app" \
  || fail "code signature verification failed"

# 3. Print the designated requirement — this is the stable string TCC keys on.
#    Identical output across versions == grants persist.
echo "== designated requirement (must be identical across versions) =="
codesign -d -r- "$app" 2>&1 || fail "could not read designated requirement"

if [[ -n "${SKIP_NOTARIZE:-}" ]]; then
  echo "verify-signing: OK — Developer ID signed (notarization checks skipped via SKIP_NOTARIZE)."
  exit 0
fi

# 4. Gatekeeper accepts the dmg as a notarized Developer ID artifact.
echo "== spctl -a -vvv -t install (Gatekeeper / notarization) =="
spctl_out=$(spctl -a -vvv -t install "$dmg" 2>&1) || fail "spctl rejected the dmg: $spctl_out"
echo "$spctl_out"
echo "$spctl_out" | grep -q "source=Notarized Developer ID" \
  || fail "dmg is not 'Notarized Developer ID' (notarization missing?)"

# 5. The notarization ticket is stapled to the dmg.
echo "== stapler validate =="
xcrun stapler validate "$dmg" || fail "notarization ticket not stapled to dmg"

echo "verify-signing: OK — Developer ID signed, notarized, and stapled."
