"""Publication recovery tests against a local fake GitHub asset store."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.store = self.root / "assets"
        self.store.mkdir()
        self.dmg = self.root / "Humhum_0.1.4_aarch64.dmg"
        self.dmg.write_bytes(b"new build, different signing timestamp")
        self.original = b"original published installer"
        (self.store / self.dmg.name).write_bytes(self.original)
        self.fake = self.root / "gh"
        self.fake.write_text('''#!/usr/bin/env python3
import json,os,pathlib,shutil,sys
a=sys.argv[1:]; store=pathlib.Path(os.environ["ASSET_STORE"])
if a[:2]==["release","view"]:
 if os.environ.get("MISSING_RELEASE"):
  print("release not found",file=sys.stderr); sys.exit(1)
 if os.environ.get("LOOKUP_ERROR"):
  print("error connecting to api.github.com",file=sys.stderr); sys.exit(1)
 print(json.dumps({"assets":[{"name":p.name} for p in store.iterdir()],"body":"Keep these human release notes"}))
elif a[:2]==["release","download"]:
 name=a[a.index("--pattern")+1]; dest=pathlib.Path(a[a.index("--dir")+1]); shutil.copyfile(store/name,dest/name)
elif a[:2]==["release","upload"]:
 for name in a[3:a.index("--repo")]:
  path=pathlib.Path(name)
  if (store/path.name).exists(): sys.exit(2)
  shutil.copyfile(path,store/path.name)
elif a[:2]==["release","create"]:
 for name in a[3:a.index("--repo")]:
  path=pathlib.Path(name); shutil.copyfile(path,store/path.name)
 pathlib.Path(os.environ["ASSET_STORE"]).parent.joinpath("notes").write_text(pathlib.Path(a[a.index("--notes-file")+1]).read_text())
else: sys.exit(3)
''')
        self.fake.chmod(0o755)

    def publish(self, **extra_env):
        script = ROOT / "scripts/publish-release.mjs"
        if not script.exists():
            self.fail("release publisher has not been implemented")
        env = os.environ | {"PATH": str(self.root) + os.pathsep + os.environ["PATH"], "ASSET_STORE": str(self.store), "HUMHUM_DMG": str(self.dmg), "GITHUB_REF_NAME": "v0.1.4", "RELEASE_TAG": "v0.1.4", "GITHUB_REPOSITORY": "example/humhum"}
        env.update(extra_env)
        return subprocess.run(["node", str(script)], cwd=self.root, env=env, text=True, capture_output=True)

    def checksum(self):
        return hashlib.sha256(self.original).hexdigest() + "  " + self.dmg.name + "\n"

    def test_repairs_checksum_from_original_not_rebuilt_bytes(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.store / "SHA256SUMS.txt").read_text(), self.checksum())
        self.assertEqual((self.store / self.dmg.name).read_bytes(), self.original)

    def test_existing_valid_checksum_is_preserved(self):
        (self.store / "SHA256SUMS.txt").write_text(self.checksum())
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.store / self.dmg.name).read_bytes(), self.original)

    def test_incorrect_existing_checksum_fails_without_overwrite(self):
        (self.store / "SHA256SUMS.txt").write_text("0" * 64 + "  " + self.dmg.name + "\n")
        result = self.publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.store / self.dmg.name).read_bytes(), self.original)

    def test_new_release_includes_installer_checksum_and_source_tag(self):
        (self.store / self.dmg.name).unlink()
        result = self.publish(MISSING_RELEASE="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        self.assertEqual((self.store / "SHA256SUMS.txt").read_text(), digest + "  " + self.dmg.name + "\n")
        self.assertIn("https://github.com/example/humhum/tree/v0.1.4", (self.root / "notes").read_text())

    def test_failed_lookup_does_not_attempt_publication(self):
        result = self.publish(LOOKUP_ERROR="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(self.store.iterdir()), [self.store / self.dmg.name])


if __name__ == "__main__":
    unittest.main()
