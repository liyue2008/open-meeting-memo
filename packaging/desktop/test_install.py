"""Private synthetic bundle tests; signature/resource verification is injected, not claimed."""
import contextlib
import hashlib
import io
import json
import os
import pathlib
import runpy
import shutil
import sys
import tempfile
import types
import unittest
from unittest import mock

INSTALL = pathlib.Path(__file__).with_name('install.py')


class DesktopInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir(mode=0o700)
        self.source = self.root / 'source' / 'Open Meeting Memo.app'
        (self.source / 'Contents/Resources').mkdir(parents=True)
        (self.source / 'Contents/MacOS').mkdir()
        (self.source / 'Contents/MacOS/omm-desktop').write_bytes(b'synthetic executable')
        (self.source / 'Contents/Resources/bundle-manifest.json').write_text(json.dumps({
            'version': '0.1.0', 'sourceRevision': 'a' * 40,
        }))
        self.target = self.home / 'Applications/Open Meeting Memo.app'
        self.resources = types.ModuleType('resources')
        self.resources.__dict__['sha'] = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
        def verify(app):
            self.assertFalse(app.is_symlink())
            self.assertEqual(app.resolve(), app)
            self.assertEqual((app / 'Contents/MacOS/omm-desktop').read_bytes(), b'synthetic executable')
        self.resources.__dict__['verify'] = verify

    def run_install(self, check=False):
        def copy(argv, check):
            self.assertEqual(argv[0], 'ditto')
            shutil.copytree(argv[1], argv[2])
        output = io.StringIO()
        with mock.patch.dict(os.environ, {'HOME': str(self.home)}), \
             mock.patch.dict(sys.modules, {'resources': self.resources}), \
             mock.patch.object(sys, 'argv', [str(INSTALL), str(self.source), str(check).lower()]), \
             mock.patch('subprocess.run', side_effect=copy), contextlib.redirect_stdout(output):
            runpy.run_path(str(INSTALL), run_name='__main__')
        return json.loads(output.getvalue())

    def test_default_name_and_revision_receipt_are_idempotent(self):
        result = self.run_install()
        self.assertEqual(result['target'], str(self.target))
        receipt = self.home / '.local/state/omm-desktop-installs' / ('a' * 40 + '.json')
        self.assertTrue(receipt.is_file())
        before = receipt.read_bytes()
        self.assertTrue(self.run_install()['installed'])
        self.assertEqual(receipt.read_bytes(), before)

    def test_check_is_read_only(self):
        result = self.run_install(check=True)
        self.assertEqual(result['target'], str(self.target))
        self.assertFalse(self.target.parent.exists())
        self.assertFalse((self.home / '.local').exists())

    def test_foreign_target_is_retained(self):
        self.target.mkdir(parents=True)
        marker = self.target / 'foreign'
        marker.write_bytes(b'keep')
        with self.assertRaises((AssertionError, FileNotFoundError)):
            self.run_install()
        self.assertEqual(marker.read_bytes(), b'keep')

    def test_different_verified_package_is_not_overwritten(self):
        self.target.parent.mkdir()
        shutil.copytree(self.source, self.target)
        manifest = self.target / 'Contents/Resources/bundle-manifest.json'
        manifest.write_text(json.dumps({'version': '0.1.0', 'sourceRevision': 'b' * 40}))
        before = manifest.read_bytes()
        with self.assertRaises(AssertionError):
            self.run_install()
        self.assertEqual(manifest.read_bytes(), before)

    def test_symlink_targets_are_refused_even_when_dangling(self):
        self.target.parent.mkdir()
        for destination in [self.source, self.root / 'absent']:
            with self.subTest(destination=destination):
                self.target.symlink_to(destination)
                try:
                    with self.assertRaises(AssertionError):
                        self.run_install()
                    self.assertTrue(self.target.is_symlink())
                finally:
                    self.target.unlink()


if __name__ == '__main__':
    unittest.main()
