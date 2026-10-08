# CI

Public pull requests and pushes to `main` run frontend and Rust checks on GitHub-hosted macOS runners with read-only repository access and no signing secrets. The E2E workflow runs on trusted `main` pushes or manual dispatch, also on a GitHub-hosted runner. Public pull requests never execute on a persistent self-hosted runner.

The API-key E2E case remains excluded from the default suite until Keychain behavior is verified in a normal runner session. Its Rust Keychain round-trip test and manual release check remain required.

The release workflow runs only for version tags and uses the `release` environment. Add Apple signing secrets and review the tagged commit before each release. Use environment approvals and tag rules when the GitHub plan supports them; see [the release guide](release.md). The release job checks that package, Tauri, and tag versions match, builds and notarizes the installer, verifies it, then uploads the `.dmg` with an exact source-tag link.

## Security checks

The Security workflow runs on PRs, main pushes, weekly, and on demand. Gitleaks scans full Git history with redacted output. Production npm advisories and Rust vulnerabilities/unsoundness fail the dependency check. Development-only npm findings are reported as an artifact for review; known unresolved upstream tooling findings are recorded in [the readiness record](public-readiness.md).

Two narrowly scoped Rust advisory exceptions cover crates not built in the default Apple Silicon production/build graph: optional `rkyv` archive support and the Linux-only `glib` backend. The workflow verifies both crates stay absent before auditing. Enabling either crate, another target, or additional production features requires reassessing these exceptions. Unmaintained upstream crates remain visible as warnings.

All external Actions are pinned to full commit SHAs. Dependabot proposes weekly npm, Cargo, and Actions updates; automatic merging is not configured. CI also executes the release safeguard tests without signing credentials.
