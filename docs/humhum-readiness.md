# Humhum source-release readiness

Prepared on 2026-10-13. Target: Apple Silicon, macOS 13+, English transcription. Version: 0.1.5. This release publishes source only; no signed Humhum installer is advertised.

## Rebrand

The app, packages, Rust library/binary, repository links, custom environment variables, data paths, Keychain service, and macOS identity use Humhum. The new icon pairs rounded sound waves using the existing plum/gold palette; the interface remains unchanged. README screenshots show synthetic state.

Humhum uses `com.kacheo.humhum`, `~/Library/Application Support/humhum`, and Keychain service `humhum-api`. It starts fresh and does not import or remove Wipzi data, downloaded models, keys, app, login registration, or permission grants. Quit Wipzi and disable its Launch at Login setting before using Humhum. Grant Humhum permissions, download models, and configure cleanup again.

## Release policy

Historical Wipzi tags remain unchanged. Existing v0.1.2/v0.1.3 installer releases are drafts. The v0.1.4 installer workflow was canceled before signing/publication. Source-tag pushes cannot publish an installer: the Release workflow requires an explicit manual dispatch naming a reviewed version tag.

The historical [Wipzi beta readiness record](public-readiness.md) is retained as evidence about Wipzi builds, not Humhum installer acceptance. Fresh signed-installer, real microphone/provider, offline, and permission-retention checks remain gates for any future binary launch.

## Verification and publication

The rebrand passed 153 frontend tests, 371 Rust unit tests plus 2 pipeline integration tests (5 intentionally ignored), 20 release-automation tests, frontend production build, native debug app build, and the default native E2E flows for onboarding, model download, and settings persistence across restart. Fresh-identity tests verify separation from Wipzi settings, models, and isolated legacy test keys. Workflow validation passed actionlint 1.7.12. Notices were regenerated with cargo-about 0.9.2 against the final lockfiles and Cargo manifest.

Production npm audit reported zero vulnerabilities. The full npm audit still reports 20 high development-tool findings. The dependency graph retains the historical Rust advisory exceptions and unmaintained warnings; Security CI audits the exact release commit. Gitleaks 8.30.1 checked the retained Git history with no leaks found; it also checks the rebrand commit in Security CI.

Preparation and review were performed while private. Publication must verify the renamed repository, branch/tag rules, read-only Actions permissions, protected manual release environment, private vulnerability reporting, and available secret scanning/push protection. Current settings and visibility can be checked through GitHub Settings/API. Source release notes must state that no installer is provided.

Development-tool npm advisories and the six unmaintained Rust dependency warnings remain tracked in the historical readiness record and Security CI. They must not be described as a fully clean dependency audit.
