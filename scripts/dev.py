#!/usr/bin/env python3
"""Local macOS Dev lifecycle. No production data or Keychain mutations."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time

DEV_ID = 'com.kacheo.humhum.dev'
PROD_ID = 'com.kacheo.humhum'


class DevError(RuntimeError):
    pass


class Workflow:
    def __init__(self, root, home, env=None):
        self.root = Path(root).resolve()
        self.home = Path(home).resolve()
        self.env = dict(os.environ if env is None else env)
        self.install_path = self.home / 'Applications/Humhum Dev.app'
        self.target_root = self.root / 'src-tauri/target/dev-app'
        self.backup_root = self.home / 'Library/Application Support/humhum-dev/build-backups'

    def command(self, argv, capture=True, env=None):
        result = subprocess.run(argv, cwd=self.root, env=self.env if env is None else env,
                                text=True, stdout=subprocess.PIPE if capture else None,
                                stderr=subprocess.STDOUT)
        if result.returncode:
            # Never echo inherited signing/notarization credentials.
            raise DevError(f'{Path(argv[0]).name} failed (exit {result.returncode})' +
                           (f': {result.stdout.strip()}' if capture and result.stdout else ''))
        return result.stdout or ''

    @contextmanager
    def lock(self):
        self.backup_root.mkdir(parents=True, exist_ok=True)
        with (self.backup_root / 'operation.lock').open('a') as handle:
            try:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise DevError('Another Dev operation is running; wait for it to finish.') from None
            try:
                yield
            finally:
                fcntl.flock(handle, fcntl.LOCK_UN)

    def running_apps(self):
        # NSWorkspace identifies real bundles and also exposes unbundled Tauri
        # executables. No process-name-only matching and no Apple Events to apps.
        script = '''ObjC.import('AppKit');
function value(f) { try { return ObjC.unwrap(f()) || null; } catch(e) { return null; } }
var apps = $.NSWorkspace.sharedWorkspace.runningApplications;
var result = [];
for (var i=0; i<apps.count; i++) {
  var app = apps.objectAtIndex(i);
  result.push({bundleId:value(function(){return app.bundleIdentifier;}),
    path:value(function(){return app.bundleURL.path;}),
    executable:value(function(){return app.executableURL.path;}), pid:app.processIdentifier});
}
JSON.stringify(result);'''
        return json.loads(self.command(['/usr/bin/osascript', '-l', 'JavaScript', '-e', script]))

    @staticmethod
    def is_humhum(app):
        executable = app.get('executable') or ''
        return app.get('bundleId') in (PROD_ID, DEV_ID) or bool(re.search(
            r'/src-tauri/target/(?:[^/]+/)*(?:debug|release)/humhum$', executable))

    def require_no_apps(self, dev_only=False):
        conflicts = [app for app in self.running_apps() if self.is_humhum(app) and
                     (not dev_only or app.get('bundleId') == DEV_ID or
                      not app.get('bundleId'))]
        if conflicts:
            locations = ', '.join(str(app.get('path') or app.get('executable')) for app in conflicts)
            raise DevError(f'Quit the running Humhum app(s) first, then retry: {locations}')

    @staticmethod
    def identifier(app):
        try:
            with (app / 'Contents/Info.plist').open('rb') as handle:
                return plistlib.load(handle).get('CFBundleIdentifier')
        except (OSError, ValueError, plistlib.InvalidFileException):
            return None

    def signature(self, app):
        if app.is_symlink() or self.identifier(app) != DEV_ID:
            raise DevError(f'Unexpected app identifier or symlink: {app}')
        self.command(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)])
        details = self.command(['/usr/bin/codesign', '-dvvv', str(app)])
        if not re.search(r'^Authority=Developer ID Application:', details, re.M) or not re.search(
                r'^TeamIdentifier=[A-Z0-9]{10}$', details, re.M) or '(runtime)' not in details:
            raise DevError('Dev app must have an intact Developer ID signature and hardened runtime.')
        if f'Identifier={DEV_ID}\n' not in details:
            raise DevError('Code-signing identifier does not match the Dev bundle identifier.')
        requirement = self.command(['/usr/bin/codesign', '-d', '-r-', str(app)])
        lines = [line for line in requirement.splitlines() if line.startswith('designated => ')]
        if len(lines) != 1 or 'cdhash ' in lines[0]:
            raise DevError('Missing stable designated requirement; ad-hoc signing is unsupported.')
        return details, lines[0]

    def require_signer(self):
        identity = self.env.get('APPLE_SIGNING_IDENTITY', '')
        if not identity.startswith('Developer ID Application:'):
            raise DevError('Set APPLE_SIGNING_IDENTITY to an available Developer ID Application identity in .env.local.')
        identities = self.command(['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning'])
        if f'"{identity}"' not in identities:
            raise DevError('Configured signing identity is not available in the Keychain.')
        return identity

    def require_unstaged_harness(self):
        if (self.root / 'src-tauri/capabilities/e2e.json').exists():
            raise DevError('E2E capability is staged; finish the harness build and unstage it before using Dev.')

    def build(self):
        self.require_unstaged_harness()
        env = dict(self.env)
        for key in ('APPLE_ID', 'APPLE_PASSWORD', 'APPLE_API_KEY', 'APPLE_API_ISSUER',
                    'APPLE_API_KEY_PATH', 'APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD'):
            env.pop(key, None)
        env['CARGO_TARGET_DIR'] = str(self.target_root)
        env.pop('CARGO_BUILD_TARGET', None)
        app = self.target_root / 'release/bundle/macos/Humhum Dev.app'
        # A successful compiler invocation must emit a fresh bundle.
        if app.exists():
            self.archive(app, self.backup_root / 'last-build.tar.gz')
            shutil.rmtree(app)
        self.command(['npx', 'tauri', 'build', '--config', 'src-tauri/tauri.dev.conf.json',
                      '--bundles', 'app'], capture=False, env=env)
        if not app.is_dir():
            raise DevError(f'Build emitted no Dev app: {app}')
        return app

    @staticmethod
    def archive(app, destination):
        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=destination.parent, suffix='.tar.gz', delete=False) as f:
            temporary = Path(f.name)
        try:
            with tarfile.open(temporary, 'w:gz') as archive:
                archive.add(app, arcname=app.name)
            # Confirm the archive can be read before removing any source app.
            with tarfile.open(temporary, 'r:gz') as archive:
                for member in archive:
                    if member.isfile():
                        with archive.extractfile(member) as contents:
                            while contents.read(1024 * 1024):
                                pass
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)

    def install(self):
        with self.lock():
            signer = self.require_signer()
            self.require_no_apps(dev_only=True)
            if self.install_path.is_symlink():
                raise DevError('Installed Dev path is a symlink; refusing replacement.')
            old_signature = self.signature(self.install_path) if self.install_path.exists() else None
            built = self.build()
            details, requirement = self.signature(built)
            if f'Authority={signer}\n' not in details:
                raise DevError('Built app does not use the configured signing identity.')
            if old_signature and old_signature[1] != requirement:
                raise DevError('Signing requirement changed; installed Dev app preserved. Restore the original signing identity.')
            self.install_path.parent.mkdir(parents=True, exist_ok=True)
            staging = Path(tempfile.mkdtemp(prefix='.humhum-dev-', dir=self.install_path.parent))
            incoming = staging / 'Humhum Dev.app'
            previous = staging / 'previous.app'
            succeeded = False
            try:
                # copytree preserves symlinks, executable modes, and extended attributes.
                shutil.copytree(built, incoming, symlinks=True)
                self.signature(incoming)
                self.require_no_apps(dev_only=True)
                if old_signature:
                    self.archive(self.install_path, self.backup_root / 'previous.tar.gz')
                try:
                    # Include the first rename in the rollback boundary: Ctrl-C
                    # immediately after it must not discard the previous app.
                    if old_signature:
                        os.replace(self.install_path, previous)
                    os.replace(incoming, self.install_path)
                    self.signature(self.install_path)
                except BaseException:
                    try:
                        if previous.exists():
                            if self.install_path.exists():
                                shutil.rmtree(self.install_path)
                            os.replace(previous, self.install_path)
                        elif not old_signature and not incoming.exists() and self.install_path.exists():
                            shutil.rmtree(self.install_path)
                    except BaseException as rollback_error:
                        raise DevError(f'Rollback failed; recovery copy retained at {previous}; '
                                       f'archive: {self.backup_root / "previous.tar.gz"}') from rollback_error
                    raise
                succeeded = True
            finally:
                # Never discard the saved app if restoring it failed.
                if succeeded or not previous.exists():
                    shutil.rmtree(staging)
            shutil.rmtree(built)
            print(f'Installed: {self.install_path}\n{requirement}\nUse npm run dev:open to launch.')

    def open(self):
        with self.lock():
            self.require_no_apps()
            self.signature(self.install_path)
            self.command(['/usr/bin/open', str(self.install_path)])
            # LaunchServices returns before the app necessarily registers. Keep
            # installers blocked until their running-app checks can see it.
            deadline = time.monotonic() + 10
            while not any(app.get('bundleId') == DEV_ID and app.get('path') and
                          Path(app['path']).resolve() == self.install_path
                          for app in self.running_apps()):
                if time.monotonic() >= deadline:
                    raise DevError('Dev did not start within 10 seconds; check dev:doctor and the Dev logs.')
                time.sleep(0.1)

    def ui(self):
        with self.lock():
            self.require_no_apps()
            self.require_unstaged_harness()
            env = dict(self.env)
            env['CARGO_TARGET_DIR'] = str(self.target_root)
            env.pop('CARGO_BUILD_TARGET', None)
            self.command(['npx', 'tauri', 'dev', '--config', 'src-tauri/tauri.dev.conf.json'], capture=False, env=env)

    def known_copies(self):
        yield self.home / 'Applications/Humhum PR66.app'
        legacy = self.home / 'Library/Application Support/humhum/dev-build-backups'
        yield from legacy.glob('*/Humhum.app')
        target = self.root / 'src-tauri/target'
        for pattern in ('*/bundle/macos/*.app', '*/*/bundle/macos/*.app',
                        '*/bundle.previous.*/bundle/macos/*.app', '*/*/bundle.previous.*/bundle/macos/*.app'):
            yield from target.glob(pattern)

    def clean(self, apply=False):
        if not apply:
            self._clean(False)
            return
        with self.lock():
            self._clean(True)

    def _clean(self, apply):
        candidates = []
        for app in sorted(set(self.known_copies())):
            if not app.exists() or app == self.install_path:
                continue
            # Refuse symlinked ancestors as well as symlinked bundles.
            if any(p.is_symlink() for p in (app, *app.parents)) or self.identifier(app) not in (PROD_ID, DEV_ID):
                print(f'Skipped unrecognized or symlinked copy: {app}')
                continue
            candidates.append(app)
        for app in candidates:
            print(f'{"Archive" if apply else "Would archive"}: {app}')
        if not apply:
            print('Preview only. Use npm run dev:clean -- --apply to archive these copies.')
            return
        running = self.running_apps()
        for app in candidates:
            if any((item.get('path') and Path(item['path']).resolve() == app.resolve()) or
                   (item.get('executable') and app.resolve() in Path(item['executable']).resolve().parents)
                   for item in running):
                raise DevError(f'Cannot clean a running app: {app}')
        for app in candidates:
            name = hashlib.sha256(str(app).encode()).hexdigest()[:16] + '.tar.gz'
            destination = self.backup_root / 'legacy' / name
            self.archive(app, destination)
            shutil.rmtree(app)
            print(f'Archived to: {destination}')

    def doctor(self):
        print(f'Canonical Dev app: {self.install_path}')
        print(f'Dev data: {self.home / "Library/Application Support/humhum-dev"}')
        print('Dev Keychain service: humhum-api-dev (key values are never inspected)')
        print(f'Dev logs: {self.home / "Library/Logs" / DEV_ID}')
        print(f'Signing identity: {self.env.get("APPLE_SIGNING_IDENTITY", "not configured")}')
        if self.install_path.exists():
            _, requirement = self.signature(self.install_path)
            print(requirement)
        print('Running Humhum processes:')
        for app in self.running_apps():
            if self.is_humhum(app):
                print(f'  PID {app["pid"]}: {app.get("path") or app.get("executable")}')
        print('Indexed app copies (inventory only):')
        print(self.command(['/usr/bin/mdfind', f'kMDItemCFBundleIdentifier == "{PROD_ID}" || kMDItemCFBundleIdentifier == "{DEV_ID}"']).strip())
        self.clean(False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['install', 'open', 'doctor', 'clean', 'ui'])
    parser.add_argument('--apply', action='store_true', help='Archive known redundant copies (clean only)')
    args = parser.parse_args()
    if args.apply and args.action != 'clean':
        parser.error('--apply is only supported for clean')
    if sys.platform != 'darwin':
        parser.error('The Dev workflow requires macOS.')
    workflow = Workflow(Path(__file__).resolve().parents[1], Path.home())
    try:
        if args.action == 'clean':
            workflow.clean(args.apply)
        else:
            getattr(workflow, args.action)()
    except (DevError, OSError, ValueError) as error:
        print(f'humhum-dev: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
