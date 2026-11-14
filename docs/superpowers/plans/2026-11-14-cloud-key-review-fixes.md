# Cloud key review fixes

**Goal:** Address all seven findings from the PR #66 review and close the meaningful test gaps.

**Architecture:** Keep successfully applied preferences in a dedicated mutation mutex. Credential commits use that snapshot and the known credential mutation result, with a short write-lock swap for recording. Confirm native credential absence before reporting removal success. Filter unsafe SDK logs at the shared logging boundary. Reconcile asynchronous UI invalidation, serialize preference saves, and gate onboarding navigation during key actions.

## Backend
- [x] Reproduce contention with an injected credential callback that attempts a recording-config read; move blocking work outside the recording-facing lock.
- [x] Replace disk/Keychain reload after key mutation with the committed credential and canonical preferences. Exercise inactive-provider changes, persistence failures, saved-mode preservation, and the actual next request's Authorization header.
- [x] Inject deletion and absence lookup results to cover native false-success and post-delete read failure; require confirmed absence.
- [x] Capture actual SDK error logging through the production logging filter; assert sentinel key/transcript values never reach the sink or serialized errors.

## Frontend
- [x] Add deferred invalidation regressions, implement reconciliation after every outcome, and clear obsolete success feedback.
- [x] Add edits-during-save and overlapping-save regressions; serialize saves and compare current drafts to the last successful snapshot.
- [x] Add a full-wizard delayed verification failure test; disable navigation/provider/mode changes during key actions and retain failed drafts.
- [x] Cover removal failure, safe secret-bearing errors, and immediate credential mutations followed by Discard.

## Validation
- [x] Run affected tests through red/green cycles, then `npm run test:coverage` and `npm run build`.
- [x] Run `cargo test --manifest-path src-tauri/Cargo.toml --locked`, notice/release checks, and targeted Rust formatting.
- [x] Resolve the dependency audit's rustix advisory with a compatible lockfile update and rerun its check.
- [x] Build the opt-in harness and retry `npm run test:e2e -- apiKeyTest`; report an environmental blocker accurately if it persists.
- [x] Obtain independent review of the final changes and inspect the final diff.
- [x] Rebuild the native app and harness after dependency/notice regeneration.
- [x] Diagnose scratch-HOME Keychain lookup, fix isolated setup, and verify credential save/replace/cold-restart/removal.
- [x] Reproduce setup-failure restoration regression and verify the cleanup fix with native Keychain checks.
- [ ] Perform separate manual acceptance and live-provider dictation in a suitable session.

## Handoff status

All seven code fixes are implemented, uncommitted, on `feat/cloud-cleanup-keys` (HEAD `42eab43`, draft PR #66). Final Rust run: 389 unit tests and 2 integration tests passed; 6 ignored. After `npm ci` corrected stale installed dependencies, Vitest 5.0.4 passed all 204 tests, with 91.7% branch coverage; frontend build and production npm audit passed. Rust audit passes after rustix 1.1.4 → 1.1.6; notices regenerated and hash check passed. Release-script tests: 26 passed.

The final native app and opt-in harness both rebuilt successfully after `npm ci` and notice regeneration. A fresh diff-aware independent review inspected all tracked changes and untracked `logging.rs`/this plan, finding no blockers in the application fixes; its 76 focused frontend tests passed. Fresh full verification again passed: 204 frontend tests (91.7% branch coverage), 389 Rust unit tests + 2 integration tests (6 ignored), 26 release tests, Rust audit (5 existing allowed warnings), production npm audit (zero vulnerabilities), notice check, and `git diff --check HEAD`.

The credential E2E blocker was traced to the runner's isolated HOME, rather than a session-bootstrap restriction: a native `SecKeychainCopyDomainDefault(kSecPreferencesDomainUser)` probe returned 0 with normal HOME and -25307 with scratch HOME. Setting the default and search list under the app's scratch HOME to the same scratch Keychain fixes native lookup while keeping the app's search list restricted to that Keychain. `createScratchKeychain` now configures both homes, and the runner no longer skips the credential flow. The targeted credential run passed verification/save, rejected replacement with saved-key preservation, successful replacement, real process restart, restored status, and removal against real macOS Keychain with mock provider responses. After the setup-failure cleanup fix, the complete `npm run test:e2e` run passed all six spec files across onboarding, model download, credentials, and settings persistence. The normal app bundle is rebuilt after E2E so the final build artifact has no harness plugin.

Review of the runner addition found a setup-failure cleanup gap: rejection before the helper returned left no handle for normal teardown. A native failure-injection check using an invalid isolated-home path failed before the fix, then passed after setup was wrapped with restore-and-rethrow. The same check confirms native lookup/search resolve only the scratch Keychain and successful teardown restores the original real-user default/search list. The check scripts/logs are in `/tmp/humhum-keychain-check.mts`, `/tmp/humhum-keychain-probe.c`, and `/tmp/humhum-keychain-check-red.log` for this session.

Separate manual acceptance and live-provider dictation have not been performed. Whisper remains local. Follow-up fixes are prepared for PR #66 on `feat/cloud-cleanup-keys`, based on `42eab43`. See the branch history for the review-fix commit.
