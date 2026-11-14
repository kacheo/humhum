"""Exercise Dev installs with real files and fake macOS/build boundaries."""
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class DevTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('humhum_dev', ROOT / 'scripts/dev.py')
        self.dev = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.dev)
        self.tmp = tempfile.TemporaryDirectory(prefix='humhum test space ')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'repo'
        self.root.mkdir()
        self.home = Path(self.tmp.name) / 'home'
        self.home.mkdir()
        self.env = {'APPLE_SIGNING_IDENTITY': 'Developer ID Application: Test (TEAM123456)'}
        self.flow = self.dev.Workflow(self.root, self.home, self.env)
        self.app = self.make_app(self.root / 'built/Humhum Dev.app')
        self.calls = []
        self.signature = 'designated => identifier "com.kacheo.humhum.dev" and anchor apple generic and certificate leaf[subject.OU] = TEAM123456'
        self.apps = []
        self.runner = patch.object(self.flow, 'command', side_effect=self.command)
        self.runner.start()
        self.addCleanup(self.runner.stop)

    def make_app(self, path, identifier='com.kacheo.humhum.dev'):
        (path / 'Contents/MacOS').mkdir(parents=True)
        (path / 'Contents/MacOS/humhum').write_text('new')
        with (path / 'Contents/Info.plist').open('wb') as f:
            plistlib.dump({'CFBundleIdentifier': identifier, 'CFBundleExecutable': 'humhum'}, f)
        return path

    def command(self, argv, **kwargs):
        self.calls.append((argv, kwargs))
        if argv[0] == '/usr/bin/osascript':
            return json.dumps(self.apps)
        if argv[0] == '/usr/bin/security':
            return '1) HASH "Developer ID Application: Test (TEAM123456)"\n1 valid identities found'
        if argv[:2] == ['/usr/bin/codesign', '-dvvv']:
            return 'Identifier=com.kacheo.humhum.dev\nAuthority=Developer ID Application: Test (TEAM123456)\nTeamIdentifier=TEAM123456\nflags=0x10000(runtime)'
        if argv[:3] == ['/usr/bin/codesign', '-d', '-r-']:
            return self.signature
        if argv[0] == 'npx':
            return ''
        return ''

    def install(self):
        with patch.object(self.flow, 'build', return_value=self.app):
            self.flow.install()

    def test_installs_verified_app_with_spaces_without_leaving_build_copy(self):
        self.install()
        self.assertEqual((self.home / 'Applications/Humhum Dev.app/Contents/MacOS/humhum').read_text(), 'new')
        self.assertFalse(self.app.exists())

    def test_missing_signer_stops_before_build(self):
        self.flow.env.clear()
        with self.assertRaisesRegex(self.dev.DevError, 'APPLE_SIGNING_IDENTITY'):
            self.flow.install()
        self.assertFalse(self.flow.install_path.exists())
        self.assertFalse(any(c[0][0] == 'npx' for c in self.calls))

    def test_unavailable_signer_stops_before_build(self):
        with patch.object(self.flow, 'command', return_value='0 valid identities found'):
            with self.assertRaisesRegex(self.dev.DevError, 'available'):
                self.flow.install()
        self.assertFalse(self.flow.install_path.exists())

    def test_wrong_bundle_id_cannot_replace_existing_app(self):
        old = self.make_app(self.flow.install_path)
        (old / 'Contents/MacOS/humhum').write_text('old')
        with (self.app / 'Contents/Info.plist').open('wb') as f:
            plistlib.dump({'CFBundleIdentifier':'com.kacheo.humhum'}, f)
        with self.assertRaisesRegex(self.dev.DevError, 'identifier'):
            self.install()
        self.assertEqual((old / 'Contents/MacOS/humhum').read_text(), 'old')

    def test_invalid_signature_prevents_install(self):
        original = self.command
        def broken(argv, **kwargs):
            if '--verify' in argv:
                raise self.dev.DevError('invalid signature')
            return original(argv, **kwargs)
        with patch.object(self.flow, 'command', side_effect=broken):
            with self.assertRaisesRegex(self.dev.DevError, 'invalid signature'):
                self.install()
        self.assertFalse(self.flow.install_path.exists())

    def test_changed_requirement_preserves_installed_app(self):
        self.make_app(self.flow.install_path)
        original = self.command
        def changed(argv, **kwargs):
            if argv[:3] == ['/usr/bin/codesign','-d','-r-'] and argv[-1] == str(self.flow.install_path):
                return 'designated => old identity'
            return original(argv, **kwargs)
        with patch.object(self.flow, 'command', side_effect=changed):
            with self.assertRaisesRegex(self.dev.DevError, 'requirement'):
                self.install()
        self.assertTrue(self.flow.install_path.exists())

    def test_running_dev_blocks_replacement(self):
        self.apps = [{'bundleId':'com.kacheo.humhum.dev','path':str(self.flow.install_path),'executable':str(self.flow.install_path / 'Contents/MacOS/humhum'),'pid':123}]
        with self.assertRaisesRegex(self.dev.DevError, 'Quit'):
            self.install()
        self.assertFalse(self.flow.install_path.exists())

    def test_official_running_blocks_open_without_quitting_it(self):
        self.make_app(self.flow.install_path)
        self.apps = [{'bundleId':'com.kacheo.humhum','path':'/Applications/Humhum.app','executable':'/Applications/Humhum.app/Contents/MacOS/humhum','pid':123}]
        with self.assertRaisesRegex(self.dev.DevError, 'Quit'):
            self.flow.open()
        self.assertFalse(any(c[0][0] == '/usr/bin/open' for c in self.calls))

    def test_unbundled_hot_reload_blocks_open(self):
        self.make_app(self.flow.install_path)
        self.apps = [{'bundleId':None,'path':None,'executable':'/some checkout/src-tauri/target/debug/humhum','pid':321}]
        with self.assertRaisesRegex(self.dev.DevError, 'Quit'):
            self.flow.open()

    def test_other_unrelated_process_is_not_a_conflict(self):
        self.make_app(self.flow.install_path)
        self.apps = [{'bundleId':'example.other','path':'/Applications/Other.app','executable':'/Applications/Other.app/Contents/MacOS/humhum','pid':123}]
        self.flow.open()
        self.assertIn((['/usr/bin/open', str(self.flow.install_path)], {}), self.calls)

    def test_lock_blocks_concurrent_install(self):
        with self.flow.lock():
            with self.assertRaisesRegex(self.dev.DevError, 'operation'):
                self.install()
        self.assertFalse(self.flow.install_path.exists())

    def test_failed_swap_restores_previous_app(self):
        old = self.make_app(self.flow.install_path)
        (old / 'Contents/MacOS/humhum').write_text('old')
        original = os.replace
        def broken(src, dst):
            if Path(src).name == 'Humhum Dev.app' and Path(src) != old and Path(dst) == old:
                raise OSError('disk failure')
            return original(src, dst)
        with patch.object(self.dev.os, 'replace', side_effect=broken):
            with self.assertRaisesRegex(OSError, 'disk failure'):
                self.install()
        self.assertEqual((old / 'Contents/MacOS/humhum').read_text(), 'old')
        self.assertTrue(self.app.exists())

    def test_replacement_archives_previous_app(self):
        import tarfile
        old = self.make_app(self.flow.install_path)
        (old / 'Contents/MacOS/humhum').write_text('old')
        self.install()
        with tarfile.open(self.flow.backup_root / 'previous.tar.gz') as archive:
            self.assertEqual(archive.extractfile('Humhum Dev.app/Contents/MacOS/humhum').read(), b'old')
        self.assertEqual((old / 'Contents/MacOS/humhum').read_text(), 'new')

    def test_cleanup_preview_is_read_only_and_apply_archives_known_copy(self):
        stale = self.make_app(self.home / 'Applications/Humhum PR66.app', 'com.kacheo.humhum')
        unknown = self.make_app(self.home / 'Applications/Other.app', 'com.kacheo.humhum')
        installed = self.make_app(self.flow.install_path)
        self.flow.clean(False)
        self.assertTrue(stale.exists())
        self.flow.clean(True)
        self.assertFalse(stale.exists())
        self.assertTrue(unknown.exists())
        self.assertTrue(installed.exists())
        self.assertEqual(len(list((self.flow.backup_root / 'legacy').glob('*.tar.gz'))), 1)

    def test_cleanup_rejects_symlink_and_unexpected_identity(self):
        official = self.make_app(self.home / 'official/Humhum.app', 'com.kacheo.humhum')
        stale = self.home / 'Applications/Humhum PR66.app'
        stale.parent.mkdir()
        stale.symlink_to(official, target_is_directory=True)
        self.flow.clean(True)
        self.assertTrue(official.exists())
        self.assertTrue(stale.is_symlink())
        stale.unlink()
        self.make_app(stale, 'example.unknown')
        self.flow.clean(True)
        self.assertTrue(stale.exists())

    def test_cleanup_does_not_archive_running_copy(self):
        stale = self.make_app(self.home / 'Applications/Humhum PR66.app', 'com.kacheo.humhum')
        self.apps = [{'bundleId':'com.kacheo.humhum','path':str(stale),'executable':str(stale / 'Contents/MacOS/humhum'),'pid':123}]
        with self.assertRaisesRegex(self.dev.DevError, 'running'):
            self.flow.clean(True)
        self.assertTrue(stale.exists())

    def test_build_excludes_notarization_credentials_and_uses_separate_target(self):
        self.flow.env.update(APPLE_ID='private', APPLE_PASSWORD='private')
        # The fake builder emits the expected output, without running a compiler.
        expected = self.flow.target_root / 'release/bundle/macos/Humhum Dev.app'
        original = self.command
        def builder(argv, **kwargs):
            if argv[0] == 'npx':
                self.make_app(expected)
            return original(argv, **kwargs)
        with patch.object(self.flow, 'command', side_effect=builder):
            self.assertEqual(self.flow.build(), expected)
        args, options = next(c for c in self.calls if c[0][0] == 'npx')
        self.assertIn('--config', args)
        self.assertIn('--bundles', args)
        self.assertEqual(args[args.index('--bundles') + 1], 'app')
        self.assertNotIn('APPLE_ID', options['env'])
        self.assertNotIn('APPLE_PASSWORD', options['env'])
        self.assertEqual(options['env']['CARGO_TARGET_DIR'], str(self.flow.target_root))


if __name__ == '__main__':
    unittest.main()
