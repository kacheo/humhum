# Releasing Humhum

Humhum distributes source releases and opt-in Apple Silicon installer betas. Signed, notarized macOS `.dmg` installers link to the exact source tag. The bundle identifier is `com.kacheo.humhum`; keep it and the Apple Team ID stable so macOS permissions persist across Humhum updates. Public beta installers target Apple Silicon only.

## One-time private repository setup

Create the GitHub Actions environment `release` and configure `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID` as secrets. Use environment secrets when the GitHub plan supports them for private repositories; repository secrets also work with the workflow. Review the exact commit before pushing a release tag. Enable `v*` tag rules and environment approvals if the repository plan offers them. GitHub's required environment reviewers are unavailable for private repositories on Free, Pro, and Team plans. Never commit a certificate or password. Validate CI and a private test release before making the repository public.

For the two certificate secrets, export the existing **Developer ID Application** certificate together with its private key from Keychain Access → login → My Certificates as a password-protected `.p12`. Encode the file with `openssl base64 -A -in /path/to/certificate.p12 -out /path/to/certificate-base64.txt`. Enter the encoded text as `APPLE_CERTIFICATE` and the export password as `APPLE_CERTIFICATE_PASSWORD`. Delete both temporary files after the secrets are stored. The release workflow imports this certificate into a temporary runner Keychain.

## Historical source-only v0.1.5 release

Humhum v0.1.5 publishes source only. Existing Wipzi installer releases remain drafts, and v0.1.4 is retained as a historical source tag. Never rewrite those tags or advertise those installers as Humhum builds.

1. Update matching npm, Cargo, and Tauri versions and lockfiles; regenerate notices.
2. Run frontend, Rust, release-automation, build, notice, secret, and dependency checks; merge the reviewed PR.
3. Tag the reviewed commit and push the tag. Tag pushes do **not** run the installer workflow.
4. Create a GitHub Release with source-only notes and no installer assets. GitHub supplies source archives for the tag.

## Installer beta release (manual only)

Complete the [manual smoke test](manual-test-checklist.md) on the exact signed build before advertising an installer. From Actions → Release → Run workflow on `main`, enter an existing reviewed `v<major>.<minor>.<patch>` tag. The workflow validates and checks out that tag, verifies matching versions and notices, then signs, notarizes, staples, and verifies the Apple Silicon `.dmg` before publishing checksums. An explicit manual run can add an installer to an existing source release; preserve and update its source-only notes deliberately once manual installer acceptance is complete.

The release environment must require maintainer review on workflow dispatches from `main`. Restrict approved installer tags with the prepared tag rulesets. Keep the Apple Team ID stable for future Humhum updates. Wipzi grants are not transferred to the new bundle ID.

For a local signing check, copy `.env.example` to ignored `.env.local`, fill in Apple credentials, and run `scripts/build-release.sh aarch64-apple-darwin`. `scripts/verify-signing.sh` can verify an existing build. An unsigned local build is for development only.

## Dependency notices

Before a distributable build, regenerate notices against the exact lockfiles:

```sh
npm ci
cargo install cargo-about --version 0.9.2 --locked --features cli
cargo about generate --manifest-path src-tauri/Cargo.toml --config about.toml --locked --format json --output-file /tmp/humhum-licenses.json
python3 scripts/generate-notices.py /tmp/humhum-licenses.json
```

Review changes to the inventory and license texts, including vendored native code. Licenses are bundled in `Humhum.app/Contents/Resources/licenses`. MPL-covered dependency source must remain available; release notes must link to the exact source tag and the corresponding dependency sources.

CI release builds explicitly target `aarch64-apple-darwin`, require all signing credentials, prohibit `SKIP_NOTARIZE`, and archive previous bundles before building and select only the newly built requested target's installer. Four version declarations (tag, npm, Cargo, Tauri) must match. Release automation tests run with `npm run test:release`.

Notice regeneration records lockfile, Cargo manifest, and policy hashes; `node scripts/check-notices.mjs` runs in CI and releases to prevent stale notices. Release reruns verify the original uploaded installer and repair missing checksums from its bytes, preserving existing installer assets and notes. A mismatched existing checksum fails without overwriting it.
