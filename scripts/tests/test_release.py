"""Exercise release safeguards without Apple credentials or a real build."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "bin").mkdir()
        shutil.copy(ROOT / "scripts/build-release.sh", self.root / "scripts")
        self.fake("npx", 'touch "$PWD/build-called"\nmkdir -p src-tauri/target/aarch64-apple-darwin/release/bundle/dmg\nprintf new > src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/current.dmg')
        self.fake("xcrun", 'printf "%s\\n" "$*" >> "$PWD/notary-calls"')
        verify = self.root / "scripts/verify-signing.sh"
        verify.write_text('#!/bin/bash\nprintf "%s\\n" "$*" > "$PWD/verified-paths"\n')
        verify.chmod(0o755)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("APPLE_") and k not in ("CI", "SKIP_NOTARIZE", "GITHUB_OUTPUT")}
        self.env["PATH"] = str(self.root / "bin") + os.pathsep + self.env["PATH"]

    def fake(self, name, body):
        path = self.root / "bin" / name
        path.write_text("#!/bin/bash\nset -eu\n" + body + "\n")
        path.chmod(0o755)

    def run_build(self, **env):
        return subprocess.run(["bash", "scripts/build-release.sh", "aarch64-apple-darwin"], cwd=self.root, env=self.env | env, text=True, capture_output=True)

    def signed_env(self):
        return dict(APPLE_SIGNING_IDENTITY="Developer ID Application: Test", APPLE_ID="private@example.invalid", APPLE_TEAM_ID="TEAM123456", APPLE_PASSWORD="test-only-placeholder")

    def test_ci_missing_credentials_stops_before_build(self):
        result = self.run_build(CI="true")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "build-called").exists())

    def test_ci_cannot_skip_notarization(self):
        result = self.run_build(CI="true", SKIP_NOTARIZE="1", **self.signed_env())
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "build-called").exists())

    def test_ci_with_all_credentials_builds_and_verifies(self):
        result = self.run_build(CI="true", APPLE_CERTIFICATE="test-only-certificate", APPLE_CERTIFICATE_PASSWORD="test-only-password", **self.signed_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "verified-paths").exists())

    def test_verifies_and_notarizes_requested_target_only(self):
        stale = self.root / "src-tauri/target/release/bundle/dmg/stale.dmg"
        stale.parent.mkdir(parents=True)
        stale.write_text("old")
        output = self.root / "github-output"
        result = self.run_build(GITHUB_OUTPUT=str(output), **self.signed_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = "src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/current.dmg"
        self.assertIn(expected, (self.root / "verified-paths").read_text())
        self.assertIn(expected, (self.root / "notary-calls").read_text())
        self.assertNotIn("stale.dmg", (self.root / "notary-calls").read_text())
        self.assertEqual(output.read_text(), f"dmg={expected}\n")
        self.assertNotIn("private@example.invalid", result.stderr + result.stdout)

    def test_missing_target_artifact_does_not_use_stale_installer(self):
        self.fake("npx", "true")
        stale = self.root / "src-tauri/target/release/bundle/dmg/stale.dmg"
        stale.parent.mkdir(parents=True)
        stale.write_text("old")
        result = self.run_build(**self.signed_env())
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("notarytool submit", (self.root / "notary-calls").read_text())

    def test_successful_build_cannot_reuse_same_target_installer(self):
        self.fake("npx", "true")
        stale = self.root / "src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/old-version.dmg"
        stale.parent.mkdir(parents=True)
        stale.write_text("old")
        result = self.run_build(**self.signed_env())
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("notarytool submit", (self.root / "notary-calls").read_text())


if __name__ == "__main__":
    unittest.main()
