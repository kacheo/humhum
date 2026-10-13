#!/usr/bin/env bash
# Release build for Humhum (macOS).
#
# Tauri-native signing + notarization: when the APPLE_* vars below are set,
# `tauri build` signs the .app (hardened runtime + Entitlements.plist), notarizes
# it via notarytool, and staples the ticket. Tauri does NOT notarize the final
# .dmg itself (tauri-apps/tauri#7533), so this script submits + staples the .dmg
# as a second step below — all in one pass from the caller's perspective. With
# the APPLE_* vars unset it produces an UNSIGNED (ad-hoc) .dmg, useful only for
# local smoke testing.
#
# Required for a distributable build (provide as env / CI secrets — never commit):
#   APPLE_CERTIFICATE           base64 of the Developer ID Application .p12
#   APPLE_CERTIFICATE_PASSWORD  password for that .p12
#   APPLE_SIGNING_IDENTITY      e.g. "Developer ID Application: Name (TEAMID)"
#   APPLE_ID                    Apple account email
#   APPLE_PASSWORD              app-specific password (for notarization)
#   APPLE_TEAM_ID               10-character team id
#
# Usage: scripts/build-release.sh [tauri-target]
#   default target = host arch (aarch64 on Apple Silicon).
#   pass "universal-apple-darwin" for a fat binary
#   (prereq: rustup target add aarch64-apple-darwin x86_64-apple-darwin).
set -euo pipefail
cd "$(dirname "$0")/.."

# Timestamped logger so each phase (esp. the silent notarization wait) is visible.
log() { printf '[build-release %s] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# Load local secrets if present (gitignored; copy from .env.example). CI provides
# the APPLE_* values via environment/secrets instead, so this is skipped there.
if [[ -z "${CI:-}" && -f .env.local ]]; then
  set -a; . ./.env.local; set +a
fi

target="${1:-}"
case "$target" in
  ""|aarch64-apple-darwin|x86_64-apple-darwin|universal-apple-darwin) ;;
  *) echo "build-release: unsupported macOS target: $target" >&2; exit 1 ;;
esac
bundle_root="src-tauri/target${target:+/$target}/release/bundle"
if [[ -n "${CI:-}" ]]; then
  [[ -z "${SKIP_NOTARIZE:-}" ]] || { echo "build-release: CI cannot skip notarization" >&2; exit 1; }
  for credential in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD; do
    [[ -n "${!credential:-}" ]] || { echo "build-release: missing $credential in CI" >&2; exit 1; }
  done
fi
# Start the argv array with `build` so it is never empty: under macOS system bash
# (3.2) `"${arr[@]}"` on an empty array trips `set -u` and aborts before tauri runs.
args=(build)
[[ -n "$target" ]] && args+=(--target "$target")
# VERBOSE=1 → stream Tauri's bundler + notarytool detail (submission id, polling).
[[ -n "${VERBOSE:-}" ]] && args+=(--verbose)

notarize=1
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "warning: APPLE_SIGNING_IDENTITY unset — producing an UNSIGNED build." >&2
  notarize=0
elif [[ -n "${SKIP_NOTARIZE:-}" ]]; then
  # Fast local build: SIGN but do NOT notarize. The stable Developer ID identity
  # (not notarization) is what keeps TCC Accessibility/Input Monitoring grants
  # across rebuilds, so this gives quick iteration without the multi-minute Apple
  # wait. Gatekeeper warns on first open (right-click → Open). NOT for distribution.
  log "SKIP_NOTARIZE set — signing WITHOUT notarization (fast local build)."
  log "signing identity: ${APPLE_SIGNING_IDENTITY}"
  # Tauri notarizes only when APPLE_ID + APPLE_PASSWORD are present; drop them.
  unset APPLE_ID APPLE_PASSWORD
  notarize=0
else
  # Notary preflight: surface config (never the password) and fail fast on bad
  # credentials / no network, instead of hanging deep inside `tauri build`.
  log "signing identity: ${APPLE_SIGNING_IDENTITY}"
  for credential in APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD; do
    [[ -n "${!credential:-}" ]] || { echo "build-release: missing $credential" >&2; exit 1; }
  done
  log "notary preflight: checking credentials + reachability..."
  xcrun notarytool history --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_PASSWORD" >/dev/null \
    || { echo "build-release: notary preflight FAILED (bad creds or no network)" >&2; exit 1; }
  log "notary preflight OK"
fi

if [[ "$notarize" == 1 ]]; then
  log "starting tauri build — signing + notarization happen here. The notarize step"
  log "can block several minutes with NO output while Apple processes it (not hung)."
  log "watch it from another shell: xcrun notarytool history --apple-id ... --team-id ... --password ..."
else
  log "starting tauri build..."
fi
# Keep previous bundles recoverable, but make this invocation start with an
# empty bundle directory. A successful build that emits no DMG must fail.
if [[ -d "$bundle_root" ]]; then
  previous_bundle=$(mktemp -d "$(dirname "$bundle_root")/bundle.previous.XXXXXX")
  mv "$bundle_root" "$previous_bundle/bundle"
fi
npx tauri "${args[@]}"
log "tauri build finished"

shopt -s nullglob
installers=("$bundle_root"/dmg/*.dmg)
[[ ${#installers[@]} -eq 1 ]] || { echo "build-release: expected exactly one installer in $bundle_root/dmg" >&2; exit 1; }
dmg="${installers[0]}"
echo "DMG artifact: $dmg"

# Tauri notarizes + staples the .app but never submits the final .dmg itself
# (upstream gap: https://github.com/tauri-apps/tauri/issues/7533), so bundle_dmg.sh
# ships a signed-but-unnotarized dmg. Gatekeeper's install-time check runs against
# whatever file the user double-clicks — the dmg — so it needs its own ticket too.
if [[ "$notarize" == 1 && -n "$dmg" ]]; then
  log "notarizing the .dmg itself (tauri only notarizes the .app — see tauri-apps/tauri#7533)..."
  xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_PASSWORD" --wait \
    || { echo "build-release: dmg notarization FAILED" >&2; exit 1; }
  log "stapling notarization ticket to the dmg..."
  xcrun stapler staple "$dmg" \
    || { echo "build-release: stapling the dmg FAILED" >&2; exit 1; }
fi

# Self-check signed builds: assert Developer ID signature + notarization + staple,
# so a mis-signed bundle is caught here instead of by users (re-granting TCC).
# Skipped for the unsigned fallback path.
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  scripts/verify-signing.sh "$bundle_root/macos/Humhum.app" "$dmg"
fi
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'dmg=%s\n' "$dmg" >> "$GITHUB_OUTPUT"
fi
