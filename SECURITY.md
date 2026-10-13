# Security

Please report suspected vulnerabilities through GitHub's **Report a vulnerability** flow for `kacheo/humhum`. Do not include real API keys, recordings, or transcripts in a report. For issues that are not sensitive, open a regular GitHub issue.

Humhum v0.1.6 provides an Apple Silicon installer beta. Download it from the official GitHub Release and verify its checksum. Published installers require Developer ID signing, notarization, and checksums linked to the corresponding source tag.

## Supported versions and scope

During beta, security fixes target the latest supported release. Include the app version, macOS version, reproduction steps with synthetic data, and the affected permission/network path. Avoid posting sensitive findings in public issues. There is no guaranteed response-time SLA.

Private reporting must be enabled in repository settings when the repository becomes public; this file alone does not enable the reporting flow. See [the beta verification record](docs/v0.1.6-readiness.md) for launch status and the reviewed dependency findings.
