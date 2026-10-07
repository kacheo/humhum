# Public repository settings

These settings are prepared for `kacheo/wipzi`; they have not been applied. The repository remains private. GitHub Free blocks these rulesets for the current private repository. Apply them immediately after an explicitly authorized visibility change, before announcing the beta.

1. Create or update the three rulesets from the adjacent JSON files using GitHub's repository rulesets API or Settings → Rules → Rulesets. For an existing ruleset with the same name, update its ID instead of creating a duplicate.
2. Main requires a PR, resolved review threads, an up-to-date branch, and checks `test`, `secrets`, and `dependencies`. Administrators have an explicit emergency bypass. The audited repository has one administrator; review this bypass when adding admins. A second reviewer is not required for solo maintenance.
3. Only administrators may create `v*` tags. A separate ruleset blocks updating/deleting those tags with no bypass actors.
4. In Actions → General, keep default token permissions read-only and PR approval disabled; require full-SHA Action pins where available. Public PR workflows must never receive release secrets or use persistent self-hosted runners.
5. In the `release` environment, scope deployment to `v*` tags. Require owner approval with self-approval permitted for solo maintenance, where the plan supports reviewers. Keep signing secrets in that environment; do not add contributors as reviewers or grant secret access without review.
6. Enable private vulnerability reporting, available secret scanning/push protection, and dependency alerts. `SECURITY.md` does not enable reporting by itself.
7. Confirm anonymous access to README, exact release source tag, installer, and checksums. Inspect settings again through the API and record the result in the readiness record.

Ruleset payloads:

```sh
gh api --method POST repos/kacheo/wipzi/rulesets --input docs/github/main-ruleset.json
gh api --method POST repos/kacheo/wipzi/rulesets --input docs/github/tag-creation-ruleset.json
gh api --method POST repos/kacheo/wipzi/rulesets --input docs/github/tag-immutability-ruleset.json
gh api --method PUT repos/kacheo/wipzi/private-vulnerability-reporting
```

Do not run these commands before confirming visibility and checking existing rule IDs. Do not automatically change visibility as part of these commands.
