# Releasing Wipzi

Wipzi distributes source and a signed, notarized macOS `.dmg` from the same GitHub tag. The bundle identifier is `com.kacheo.wipzi`; keep it and the Apple Team ID stable so macOS permissions persist across Wipzi updates. Public beta installers target Apple Silicon only.

## One-time private repository setup

Create the GitHub Actions environment `release` and configure `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID` as secrets. Use environment secrets when the GitHub plan supports them for private repositories; repository secrets also work with the workflow. Review the exact commit before pushing a release tag. Enable `v*` tag rules and environment approvals if the repository plan offers them. GitHub's required environment reviewers are unavailable for private repositories on Free, Pro, and Team plans. Never commit a certificate or password. Validate CI and a private test release before making the repository public.

For the two certificate secrets, export the existing **Developer ID Application** certificate together with its private key from Keychain Access → login → My Certificates as a password-protected `.p12`. Encode the file with `openssl base64 -A -in /path/to/certificate.p12 -out /path/to/certificate-base64.txt`. Enter the encoded text as `APPLE_CERTIFICATE` and the export password as `APPLE_CERTIFICATE_PASSWORD`. Delete both temporary files after the secrets are stored. The release workflow imports this certificate into a temporary runner Keychain.

## Tag a release

1. Update the same version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`; update both lockfiles.
2. Run `npm ci`, `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml --locked`, and the [manual smoke test](manual-test-checklist.md).
3. Commit the reviewed source. Tag it `v<version>` and push the tag.
4. The Release workflow calls `scripts/build-release.sh` to sign, notarize, staple, and verify the `.dmg`, then creates the GitHub Release with checksums and a link to that exact source tag. Existing release assets and human-written notes are preserved on reruns.
5. Download the `.dmg` from the private Release, check Gatekeeper and the app manually, and approve public visibility only after source and license review.

For a local signing check, copy `.env.example` to ignored `.env.local`, fill in Apple credentials, and run `scripts/build-release.sh aarch64-apple-darwin`. `scripts/verify-signing.sh` can verify an existing build. An unsigned local build is for development only.

## Public beta preparation

Keep v0.1.3 unless an audit requires a shipped-artifact correction. The current preparation audit found affected networking dependencies and missing packaged notices, so the checkout prepares v0.1.4. Do not overwrite v0.1.3 or publish the corrected installer until the signed-build and manual checks pass. See [the readiness record](public-readiness.md).

Before a distributable build, regenerate notices against the exact lockfiles:

```sh
npm ci
cargo install cargo-about --version 0.9.2 --locked
cargo about generate --manifest-path src-tauri/Cargo.toml --config about.toml --locked --format json --output-file /tmp/wipzi-licenses.json
python3 scripts/generate-notices.py /tmp/wipzi-licenses.json
```

Review changes to the inventory and license texts, including vendored native code. Licenses are bundled in `Wipzi.app/Contents/Resources/licenses`. MPL-covered dependency source must remain available; release notes must link to the exact source tag and the corresponding dependency sources.

CI release builds explicitly target `aarch64-apple-darwin`, require all signing credentials, prohibit `SKIP_NOTARIZE`, and archive previous bundles before building and select only the newly built requested target's installer. Four version declarations (tag, npm, Cargo, Tauri) must match. Release automation tests run with `npm run test:release`.

Notice regeneration records lockfile and policy hashes; `node scripts/check-notices.mjs` runs in CI and releases to prevent stale notices. Release reruns verify the original uploaded installer and repair missing checksums from its bytes, preserving existing installer assets and notes. A mismatched existing checksum fails without overwriting it.
