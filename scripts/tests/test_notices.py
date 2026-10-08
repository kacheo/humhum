import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


class NoticeTests(unittest.TestCase):
    def check(self, changed=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "scripts").mkdir()
            (root / "src-tauri").mkdir()
            (root / "docs/licenses").mkdir(parents=True)
            shutil.copy(ROOT / "scripts/check-notices.mjs", root / "scripts")
            paths = ["package-lock.json", "src-tauri/Cargo.lock", "about.toml"]
            for path in paths:
                (root / path).write_bytes(b"")
            (root / "docs/licenses/lockfiles.json").write_text(json.dumps({path: EMPTY_SHA256 for path in paths}))
            if changed:
                (root / "src-tauri/Cargo.lock").write_bytes(b"new dependency version")
            return subprocess.run(["node", "scripts/check-notices.mjs"], cwd=root, capture_output=True, text=True)

    def test_reviewed_inputs_are_accepted(self):
        self.assertEqual(self.check().returncode, 0)

    def test_changed_lockfile_requires_regeneration(self):
        self.assertNotEqual(self.check(changed=True).returncode, 0)
