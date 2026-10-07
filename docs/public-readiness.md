# Public beta readiness record

Reviewed on 2026-10-07. Target: Apple Silicon, macOS 13+, English transcription. Repository visibility remains **private**. Preparation branch: `chore/public-readiness`.

## Release decision

The plan was to retain v0.1.3 unless a shipped-artifact correction proved necessary. The audit found that v0.1.3 omitted packaged license/attribution files and included h2 0.4.15, affected by RUSTSEC-2026-0258. The source now prepares **v0.1.4**, with notices bundled and h2 upgraded to 0.4.20. Compatible lockfile updates also address quick-xml, rustls, anyhow, and event-listener advisories; not every cross-platform lockfile component is built for macOS.

The old installer has not been overwritten. Its signature, Gatekeeper acceptance, and stapled notarization ticket passed verification. Its SHA-256 is `79833ab1efdb81d4e53c687847167c0e2760138dacf6d3bff2ce4e16f4ab0605`.

The corrected local candidate passed Developer ID signature verification, Gatekeeper acceptance, and stapled notarization validation. Its version is 0.1.4, and all five bundled license/notice files match the final checkout. Its SHA-256 is `9189e7dc2a8e66d0c02915028d5598577e670f48e99b76c13d508767c5d10053`. It has not been tagged or published. Do not announce the public beta until its manual release checks pass.

## Completed preparation and checks

- Gitleaks 8.30.1: all 10 pre-preparation commits scanned, no leaks found. Tools were downloaded from official releases and checked against published SHA-256 values.
- Retained Actions logs: 24 completed CI/E2E/Release logs inspected, no credential-pattern matches. This is a pattern scan, not a claim that every kind of sensitive content can be detected.
- Historical commit identity review: one author identity, no personal-domain email addresses identified. No history rewrite was needed.
- Static inspection of application cleanup/recording logging found no deliberate API-key, transcript, or provider-response-body logging. Local signing preflight no longer prints the Apple account email.
- Current repository access: one admin collaborator, no deploy keys. Repository settings list Claude, Continue, and Vercel as installed GitHub Apps; detailed permission configuration requires GitHub reauthentication and remains a manual access-review gate. Actions default token permissions are read-only and PR approval is disabled. All external workflow Actions now use full commit SHA pins.
- README, troubleshooting, contributor setup, roadmap, issue forms, PR template, and synthetic-state UI screenshots are prepared.
- Reviewed Apple Silicon dependency inventory and full notices are generated, including vendored whisper.cpp/ggml and exact-source upstream attribution where crate packages omit license files.
- Production npm audit: zero advisories. Vitest/coverage updated to 4.1.11 and compatible development dependencies refreshed; no critical npm findings remain.
- Rust audit: zero vulnerabilities and zero unsoundness findings after updates and reviewed target exceptions. Six unmaintained upstream crates remain warnings.
- Frontend: 153 tests passed. Coverage: statements 95.62%, branches 90.07%, functions 95.02%, lines 97.73%; configured thresholds passed.
- A clean source archive of the preparation commit passed `npm ci`, all 153 frontend tests, all 16 release tests, the production frontend build, and the dependency notice hash check. The full Git history including the preparation commit passed Gitleaks (11 commits).
- Rust: 368 tests passed, 5 intentionally ignored; binary/doc tests also passed. Sandboxed tests initially failed because of loopback/Keychain restrictions; a normal unsandboxed macOS run passed.
- Default E2E: onboarding, model download, and settings persistence across cold restart passed. The known API-key case remains excluded.
- Sixteen release safeguard/version/notice tests passed, including stale same-target bundle prevention, interrupted checksum-upload recovery, unsigned CI prevention, CI notarization enforcement, target-specific artifact selection, and mismatched versions.
- Notice regeneration now records reviewed lockfile/policy hashes; CI and releases reject stale notices after input changes.
- Independent review found two release edge cases (same-target stale DMGs and interrupted checksum uploads); both were reproduced and fixed with regression tests.
- Workflow YAML passed actionlint 1.7.12. Public branch/tag rule payloads and settings instructions are prepared in `docs/github`.

## Reviewed advisory exceptions and tooling findings

`.cargo/audit.toml` excepts RUSTSEC-2026-0235 (`rkyv` optional archive support) and RUSTSEC-2024-0429 (`glib`, Linux backend). Neither crate is in the default Apple Silicon production/build graph. Security CI verifies that absence before applying the exceptions. New platforms/features require reassessment.

The full npm audit still reports **20 high findings, all in development tooling**. The direct affected upstream components are `basic-ftp`, `braces`, `deepmerge-ts`, `extract-zip`, and `serialize-javascript`, pulled through WebdriverIO/Mocha and its browser tooling. This preparation does not force incompatible transitive majors or downgrade WebdriverIO merely to silence the audit. Default E2E runs against a trusted build and controlled local fixture/server; public PR CI has no signing secrets or persistent runner. These findings remain visible in weekly/PR audit artifacts and require follow-up upstream fixes or a separately tested tooling migration.

Unmaintained Rust warnings cover `proc-macro-error` and the five `unic-*` crates used upstream. Keep them visible and monitor framework updates; no blanket suppression is configured.

## Remaining launch gates

- Complete the [manual checklist](manual-test-checklist.md) against the corrected downloaded/signed installer: fresh permissions, live microphone capture, hotkeys, pasting/clipboard fallback, both real Whisper model downloads, recording limit, and upgrade permission retention.
- Test actual Ollama cleanup and user-supplied OpenAI/Anthropic API-key storage/use/removal in a normal interactive macOS session. Automated fixture tests do not prove real provider connectivity or fresh-install behavior.
- Verify offline raw transcription after downloading a model.
- Verify exact source-tag availability and bundled notices for the installer selected for publication.
- Complete detailed scope review for the installed Claude, Continue, and Vercel GitHub Apps. The repository installation list was inspected, but opening detailed configuration requires GitHub reauthentication. No apps or access grants were changed.
- After explicit publication authorization, apply the prepared rulesets, release-environment protections, private vulnerability reporting, and available secret-scanning/push-protection settings. The private repo's current release environment has no protection rules; ruleset inspection is plan-blocked by GitHub while private.
- Confirm anonymous README/source/installer/checksum access after changing visibility, before announcing the beta.

Fresh-install, microphone, cloud-provider, and permission-retention checks are **not marked passed** by this record. The prepared settings do not change repository visibility.
