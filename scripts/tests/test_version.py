import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class VersionTests(unittest.TestCase):
    def check(self, cargo_version="0.1.3", tag="v0.1.3", release_tag=None):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "scripts").mkdir()
            (root / "src-tauri").mkdir()
            script = ROOT / "scripts/check-release-version.mjs"
            if not script.exists():
                self.fail("release version checker has not been implemented")
            shutil.copy(script, root / "scripts")
            (root / "package.json").write_text(json.dumps({"version": "0.1.3"}))
            (root / "src-tauri/tauri.conf.json").write_text(json.dumps({"version": "0.1.3"}))
            (root / "src-tauri/Cargo.toml").write_text(f'[package]\nversion = "{cargo_version}"\n[dependencies]\nother = "9"\n')
            return subprocess.run(["node", "scripts/check-release-version.mjs"], cwd=root, env=os.environ | {"GITHUB_REF_NAME": tag} | ({"RELEASE_TAG": release_tag} if release_tag is not None else {}), capture_output=True, text=True)

    def test_matching_versions(self):
        self.assertEqual(self.check().returncode, 0)

    def test_manual_dispatch_uses_requested_tag_instead_of_workflow_branch(self):
        self.assertEqual(self.check(tag="main", release_tag="v0.1.3").returncode, 0)

    def test_cargo_mismatch(self):
        self.assertNotEqual(self.check(cargo_version="0.1.2").returncode, 0)

    def test_tag_mismatch(self):
        self.assertNotEqual(self.check(tag="v0.1.2").returncode, 0)


if __name__ == "__main__":
    unittest.main()
