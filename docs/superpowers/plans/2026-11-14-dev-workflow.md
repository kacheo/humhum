# Humhum Dev Workflow Implementation Plan

**Goal:** One stable signed Dev installation, isolated from official Humhum, with recoverable updates and controlled cleanup.
**Architecture:** Explicit Tauri overlay and Rust dev-app feature; a Python standard-library CLI owns signing verification, installation, inventory, and archival. Shell entrypoint loads existing local signing configuration.
**Approved design:** Conversation plan approved by “Implement the plan.”
**Constraints:** Production identity com.kacheo.humhum and credential service/accounts stay stable. Dev is com.kacheo.humhum.dev, humhum-dev storage, humhum-api-dev service. No automatic imports or permission resets. Local Whisper and key/preference semantics unchanged.

- [x] Task 1: Add isolated Dev identity/storage, feature/config validation, and distinguishable app resources. Test default/Dev paths and mismatch failures.
- [x] Task 2: Add dev:install/open/doctor/clean commands. Test signer validation, conflict detection, rollback, install locking, archival/protected paths, and paths with spaces.
- [ ] Task 3: Document signed testing versus isolated hot reload and credential E2E. Run suites, build/install signed app, and obtain independent review.

## Review focus
- A failed installation must leave the previous installed app usable.
- Official and unknown app copies/data must never be removed by cleanup.
- Wrong signer/config must fail before replacement or app state creation.
- Running copies and concurrent operations must prevent conflicting updates.
- Permission retention requires a real two-build interactive test; signing checks alone cannot establish it.

## Execution ledger
- Baseline: main clean at 54c7f54; created feat/dev-workflow.
- Ruling: Work in a new branch in the existing clean checkout rather than a second worktree, retaining the native caches and existing local signing configuration. The approved plan requires a new branch; no concurrent implementation modifies this checkout.
- Task 1: profile tests failed before implementation; default and Dev suites passed (391 unit + 2 integration each). Initial parallel baseline compilation overlapped the new failing tests, so the full post-implementation suite is the Rust verification record.
- Task 2: lifecycle tests failed before the CLI existed; 17 lifecycle tests now pass; complete release suite 43 tests passed.
- Ruling: Correct the existing E2E workflow’s missing capability stage/unstage when updating its documented commands, ensuring the documented harness build and CI use the same required setup.
- Signed build A installed successfully at ~/Applications/Humhum Dev.app. Official Humhum remains running; interactive permission/dictation checks are pending.
