"""Release/staging controls must fail closed before any publication."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / 'clients' / f'{name}.py')
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


class ClientRelease(unittest.TestCase):
    def test_readiness_requires_every_explicit_boolean(self):
        ready = module('release_ready')
        for value in [{}, {'schema': 1}, {'schema': 1, **{k: 'true' for k in ready.REQUIRED}}, {'schema': 1, **{k: True for k in ready.REQUIRED[:-1]}}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                ready.validate(value)
        ready.validate({'schema': 1, **{k: True for k in ready.REQUIRED}})

    def test_staging_keeps_protected_assets_outside_public_root_and_rejects_preview(self):
        stage = module('assemble_site')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for folder in ['promo', 'app']:
                (root / folder).mkdir()
                (root / folder / 'index.html').write_text(folder)
            stage.assemble(root / 'promo', root / 'app', root / 'stage')
            self.assertEqual((root / 'stage/web/index.html').read_text(), 'app')
            self.assertFalse((root / 'stage/website/app').exists())
            with self.assertRaises(ValueError):
                stage.assemble(root / 'promo', root / 'app', root / 'stage')
            (root / 'app/preview.html').write_text('fictional')
            with self.assertRaises(ValueError):
                stage.assemble(root / 'promo', root / 'app', root / 'preview-stage')
            self.assertFalse((root / 'preview-stage').exists())

    def test_staging_rejects_symlinks_and_source_overlap(self):
        stage = module('assemble_site')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for folder in ['promo', 'app']:
                (root / folder).mkdir()
                (root / folder / 'index.html').touch()
            with self.assertRaises(ValueError):
                stage.assemble(root / 'promo', root / 'app', root / 'app/stage')
            try:
                (root / 'app/linked').symlink_to(root / 'promo/index.html')
            except OSError:
                self.skipTest('Host cannot create unprivileged symlinks')
            with self.assertRaises(ValueError):
                stage.assemble(root / 'promo', root / 'app', root / 'stage')

    def test_manifest_and_path_package_lock_versions_move_together(self):
        version = module('version')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for folder in ['flutter', 'web', 'website', 'backend', 'shared/mail-core', 'shared/mail-content', 'flutter/rust']:
                (root / folder).mkdir(parents=True)
            (root / 'flutter/pubspec.yaml').write_text('name: shep\nversion: 0.1.0+1\n')
            for folder in ['web', 'website']:
                (root / folder / 'package.json').write_text('{"version":"0.1.0"}')
                (root / folder / 'package-lock.json').write_text('{"version":"0.1.0","packages":{"":{"version":"0.1.0"}}}')
            for folder in ['backend', 'shared/mail-core', 'shared/mail-content', 'flutter/rust']:
                (root / folder / 'Cargo.toml').write_text('[package]\nname = "fixture"\nversion = "0.1.0"\n')
            lock = 'version = 4\n[[package]]\nname = "shep-mail-core"\nversion = "0.1.0"\n[[package]]\nname = "dependency"\nversion = "1.2.3"\nsource = "registry+fixture"\n'
            for p in ['Cargo.lock', 'backend/Cargo.lock', 'flutter/rust/Cargo.lock']:
                (root / p).write_text(lock + '\n[[package]]\nname = "shep-mail-content"\nversion = "0.1.0"\n')
            mobile_lock = root / 'flutter/rust/Cargo.lock'
            mobile_lock.write_text(mobile_lock.read_text() + '\n[[package]]\nname = "shep_mobile_native"\nversion = "0.1.0"\n')
            version.stamp('2.3.4', 29, root)
            self.assertIn('name = "shep_mobile_native"\nversion = "2.3.4"', mobile_lock.read_text())
            self.assertIn('version = "2.3.4"', (root / 'flutter/rust/Cargo.toml').read_text())
            self.assertIn('version = "2.3.4"', (root / 'shared/mail-content/Cargo.toml').read_text())
            self.assertIn('version: 2.3.4+29', (root / 'flutter/pubspec.yaml').read_text())
            for p in ['Cargo.lock', 'backend/Cargo.lock', 'flutter/rust/Cargo.lock']:
                text = (root / p).read_text()
                self.assertIn('name = "shep-mail-core"\nversion = "2.3.4"', text)
                self.assertIn('name = "shep-mail-content"\nversion = "2.3.4"', text)
                self.assertIn('name = "dependency"\nversion = "1.2.3"', text)

    def test_android_apk_requires_native_libraries_permissions_and_fixture_exclusion(self):
        import zipfile
        verify = module('verify_android')
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'app.apk'
            def build(*, missing=False, preview=False, fixture=False, permission=True):
                with zipfile.ZipFile(path, 'w') as archive:
                    package = 'so.shep.shep_mobile' + ('.preview' if preview else '')
                    manifest = package + (' android.permission.INTERNET' if permission else '')
                    archive.writestr('AndroidManifest.xml', manifest.encode('utf-16le'))
                    for abi in verify.ABIS:
                        if not missing or abi != 'arm64-v8a':
                            archive.writestr(f'lib/{abi}/libshep_mobile_native.so', b'production Rust')
                    archive.writestr('assets/flutter_assets/kernel_blob.bin', b'Native durable draft' if fixture else b'production Dart')
            build()
            self.assertEqual(len(verify.inspect(path)['native_libraries']), 3)
            for option in ['missing', 'preview', 'fixture', 'permission']:
                with self.subTest(option=option):
                    build(**{option: option != 'permission'})
                    with self.assertRaises(ValueError):
                        verify.inspect(path)

    def test_android_fixture_scanning_covers_chunk_boundaries(self):
        from io import BytesIO
        verify = module('verify_android')
        self.assertTrue(verify.contains(BytesIO(b'x' * (64 * 1024 - 4) + verify.MARKERS[0]), verify.MARKERS))
        self.assertFalse(verify.contains(BytesIO(b'x' * (128 * 1024)), verify.MARKERS))
