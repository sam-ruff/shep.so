"""Pinned dependency testing never modifies a checkout or chooses another source."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "profile_core_runner", Path(__file__).resolve().parents[1] / "scripts/test_profile_core.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ProfileCoreRunnerTests(unittest.TestCase):
    def test_resolves_exact_published_pin_and_rejects_local_or_other_git_versions(self):
        revision = "a" * 40
        manifest = {"dependencies": {"shep-profile-core": {
            "git": "https://example.test/repository", "rev": revision}}}
        package = {"name": "shep-profile-core", "manifest_path": "/fixture/shared/profile-core/Cargo.toml",
                   "source": f"git+https://example.test/repository?rev={revision}#{revision}"}
        self.assertEqual(runner.source_package(manifest, {"packages": [package]}),
                         Path("/fixture/shared/profile-core"))
        for source in [None, package["source"].replace("#" + revision, "#" + "b" * 40)]:
            with self.assertRaises(ValueError):
                runner.source_package(manifest, {"packages": [{**package, "source": source}]})
        with self.assertRaises(ValueError):
            runner.source_package(manifest, {"packages": [package, package]})
        manifest["dependencies"]["shep-profile-core"]["rev"] = "branch-name"
        with self.assertRaises(ValueError):
            runner.source_package(manifest, {"packages": [package]})

    def test_copy_preserves_exact_fixture_bytes_and_source_and_uses_locked_workspace(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "checkout/shared/profile-core"
            source.mkdir(parents=True)
            manifest = b'[package]\nname="shep-profile-core"\nversion="0.1.0"\n'
            (source / "Cargo.toml").write_bytes(manifest)
            (source / "target").mkdir()
            (source / "target/leave-original").write_text("original")
            (source / "Cargo.lock").write_text("checkout lock")
            fixture = 'Unicode é 🌙\r\n'.encode()
            for name in runner.FIXTURES:
                (source.parent / name).write_bytes(fixture)
            lock = root / "reviewed.lock"
            lock.write_bytes(b"reviewed lock\n")
            copied = runner.prepare(source, root / "isolated", lock)
            self.assertIn("[workspace]", copied.read_text())
            self.assertEqual((source / "Cargo.toml").read_bytes(), manifest)
            self.assertFalse((copied.parent / "target").exists())
            self.assertEqual((copied.parent / "Cargo.lock").read_bytes(), lock.read_bytes())
            self.assertEqual((source / "Cargo.lock").read_text(), "checkout lock")
            for name in runner.FIXTURES:
                self.assertEqual((copied.parent.parent / name).read_bytes(), fixture)
                self.assertEqual((source.parent / name).read_bytes(), fixture)


if __name__ == "__main__":
    unittest.main()
