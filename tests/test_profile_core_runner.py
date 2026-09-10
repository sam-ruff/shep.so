"""Shared crate testing runs the workspace member under the root lock and never another source."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "profile_core_runner", Path(__file__).resolve().parents[1] / "scripts/test_profile_core.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ProfileCoreRunnerTests(unittest.TestCase):
    def test_workspace_member_path_is_only_accepted_when_cargo_lists_it_as_a_member(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            crate = root / "shared/profile-core"
            crate.mkdir(parents=True)
            manifest = {"dependencies": {"shep-profile-core": {"path": "shared/profile-core"}}}
            package = {"name": "shep-profile-core", "id": "path+file://member#0.1.0",
                       "manifest_path": str(crate / "Cargo.toml"), "source": None}
            metadata = {"packages": [package], "workspace_members": [package["id"]]}
            self.assertEqual(runner.workspace_member(manifest, metadata, root), crate)
            with self.assertRaises(ValueError):
                runner.workspace_member(manifest, {"packages": [package], "workspace_members": []}, root)
            with self.assertRaises(ValueError):
                runner.workspace_member(manifest, {**metadata, "packages": [
                    {**package, "manifest_path": str(root / "elsewhere/Cargo.toml")}]}, root)
            with self.assertRaises(ValueError):
                runner.workspace_member(manifest, {**metadata, "packages": [package, package]}, root)

    def test_git_pins_and_sibling_checkouts_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            revision = "a" * 40
            package = {"name": "shep-profile-core", "id": "git+example#0.1.0",
                       "manifest_path": str(root / "shared/profile-core/Cargo.toml"),
                       "source": f"git+https://example.test/repository?rev={revision}#{revision}"}
            metadata = {"packages": [package], "workspace_members": [package["id"]]}
            for dependency in (
                {"git": "https://example.test/repository", "rev": revision},
                {"path": "shared/profile-core", "git": "https://example.test/repository"},
                {"path": "../other-worktree/shared/profile-core"},
            ):
                with self.assertRaises(ValueError):
                    runner.workspace_member({"dependencies": {"shep-profile-core": dependency}}, metadata, root)


if __name__ == "__main__":
    unittest.main()
