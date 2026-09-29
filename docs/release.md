# Releasing Wipzi

Wipzi distributes source and a signed, notarized macOS `.dmg` from the same GitHub tag. The bundle identifier is `com.kacheo.wipzi`; keep it and the Apple Team ID stable so macOS permissions persist across Wipzi updates. The old app has a different identifier and is not migrated.

## One-time private repository setup

Create the GitHub Actions environment `release` and configure `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID` as secrets. Use environment secrets when the GitHub plan supports them for private repositories; repository secrets also work with the workflow. Review the exact commit before pushing a release tag. Enable `v*` tag rules and environment approvals if the repository plan offers them. GitHub's required environment reviewers are unavailable for private repositories on Free, Pro, and Team plans. Never commit a certificate or password. Validate CI and a private test release before making the repository public.

## Tag a release

1. Update the same version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`; update both lockfiles.
2. Run `npm ci`, `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml --locked`, and the [manual smoke test](manual-test-checklist.md).
3. Commit the reviewed source. Tag it `v<version>` and push the tag.
4. The Release workflow calls `scripts/build-release.sh` to sign, notarize, staple, and verify the `.dmg`, then creates the GitHub Release with a link to that exact source tag.
5. Download the `.dmg` from the private Release, check Gatekeeper and the app manually, and approve public visibility only after source and license review.

For a local signing check, copy `.env.example` to ignored `.env.local`, fill in Apple credentials, and run `scripts/build-release.sh`. `scripts/verify-signing.sh` can verify an existing build. An unsigned local build is for development only.
