"""Release packaging contracts with fictional binaries and isolated installers."""
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import tomllib

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module}), patch.object(sys, "path", [str(ROOT / "scripts"), *sys.path]):
        spec.loader.exec_module(module)
    return module


release = load("release")
verify = load("verify_release")
SOURCE = "a" * 40
LINUX = "x86_64-unknown-linux-gnu"
WINDOWS = "x86_64-pc-windows-msvc"


def executable(target):
    header = bytearray(128)
    if target == LINUX:
        header[:6] = b"\x7fELF\x02\x01"
        header[18:20] = b"\x3e\x00"
    else:
        header[:2] = b"MZ"
        header[60:64] = (64).to_bytes(4, "little")
        header[64:70] = b"PE\0\0\x64\x86"
    return bytes(header) + b"fictional release binary"


class ReleasePackaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shep package ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in (*release.CONTENTS, *release.VERSION_FILES):
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        shutil.copyfile(ROOT / "scripts/verify_release.py", self.root / "scripts/verify_release.py")
        shutil.copyfile(ROOT / "scripts/release.py", self.root / "scripts/release.py")

    def package(self, target=LINUX, version="1.2.3", source=SOURCE):
        binary = self.root / ("fixture.exe" if target == WINDOWS else "fixture")
        binary.write_bytes(executable(target))
        return release.package(version, target, source, 1234567890, binary, self.root)

    def rewrite(self, archive, change):
        with tarfile.open(archive, "r:gz") as original:
            members = [(member, original.extractfile(member).read()) for member in original]
        replacement = archive.with_suffix(".replacement")
        with tarfile.open(replacement, "w:gz") as output:
            change(members)
            for member, data in members:
                member.size = len(data)
                output.addfile(member, io.BytesIO(data))
        replacement.replace(archive)
        (archive.parent / "SHA256SUMS").write_text(f"{release.sha256(archive)}  {archive.name}\n")

    def test_stamps_desktop_shared_manifests_and_all_consuming_locks_only(self):
        release.stamp("2.3.4-rc.1", self.root)
        release.verify_versions("2.3.4-rc.1", self.root)
        for name in release.MANIFESTS:
            self.assertEqual(tomllib.loads((self.root / name).read_text())["package"]["version"], "2.3.4-rc.1")
        for name in release.LOCKS:
            original = tomllib.loads((ROOT / name).read_text())["package"]
            updated = tomllib.loads((self.root / name).read_text())["package"]
            for before, after in zip(original, updated, strict=True):
                if before["name"] in release.MANIFESTS.values() and "source" not in before:
                    self.assertEqual(after["version"], "2.3.4-rc.1")
                else:
                    self.assertEqual(before, after)

    def test_invalid_versions_or_incomplete_lock_never_partially_stamp(self):
        for version in ("01.2.3", "1.2", "1.2.3-", "1.2.3-01", "1.2.3+metadata", "1.2.3/../../x"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.stamp(version, self.root)
        original = (self.root / "Cargo.toml").read_bytes()
        (self.root / "backend/Cargo.lock").write_text("version = 4\n")
        with self.assertRaises(ValueError):
            release.stamp("2.3.4", self.root)
        self.assertEqual((self.root / "Cargo.toml").read_bytes(), original)

    def test_cargo_selects_explicit_locked_production_binary_and_configured_target_directory(self):
        binary = self.root / "custom target" / LINUX / "release/shep"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(executable(LINUX))
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 0, json.dumps({"reason": "compiler-artifact", "target": {"name": "shep", "kind": ["bin"]}, "features": [], "profile": {"test": False}, "executable": str(binary)}))

        self.assertEqual(release.build(self.root, LINUX, self.root / "custom target", run), binary)
        command = commands[0]
        self.assertNotIn("--config", command)
        for argument in ("--locked", "--release", "--no-default-features"):
            self.assertIn(argument, command)
        for argument, value in (("--package", "shep"), ("--bin", "shep"), ("--target", LINUX), ("--jobs", "4"), ("--target-dir", str(self.root / "custom target"))):
            self.assertEqual(command[command.index(argument) + 1], value)

        release.build(self.root, WINDOWS, run=run)
        command = commands[1]
        self.assertEqual(command[command.index("--target") + 1], WINDOWS)
        self.assertEqual(command[command.index("--config") + 1],
                         'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]')

    def test_windows_imports_include_delay_load_and_reject_unbundled_runtimes(self):
        prefix = "File Type: EXECUTABLE IMAGE\n  Image has the following dependencies:\n\n    KERNEL32.dll\n"
        valid = prefix + "  Image has the following delay load dependencies:\n    D3D11.dll\n    api-ms-win-core-synch-l1-2-0.dll\n  Summary\n"
        self.assertEqual(verify.windows_dependencies(valid), ["api-ms-win-core-synch-l1-2-0.dll", "d3d11.dll", "kernel32.dll"])
        for dependency in ("VCRUNTIME140.dll", "MSVCP140.dll", "ucrtbase.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "libcrypto-3-x64.dll", "unexpected.dll"):
            with self.subTest(dependency=dependency), self.assertRaisesRegex(ValueError, "unbundled or unapproved"):
                verify.windows_dependencies(prefix + f"  Image has the following delay load dependencies:\n    {dependency}\n  Summary\n")
        for output in ("", prefix, "  Summary\n", prefix + "    C:/outside.dll\n  Summary\n"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                verify.windows_dependencies(output)

    def test_dumpbin_discovery_uses_visual_studio_default_toolset(self):
        locator = self.root / "vswhere.exe"
        locator.touch()
        installation = self.root / "Visual Studio"
        version_file = installation / "VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"
        version_file.parent.mkdir(parents=True)
        version_file.write_text("14.44.35207\n")
        dumpbin = installation / "VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64/dumpbin.exe"
        dumpbin.parent.mkdir(parents=True)
        dumpbin.touch()

        def run(command, **kwargs):
            self.assertEqual(command[0], str(locator))
            self.assertIn("Microsoft.VisualStudio.Component.VC.Tools.x86.x64", command)
            self.assertTrue(kwargs["check"])
            return subprocess.CompletedProcess(command, 0, str(installation) + "\n")

        with patch.object(verify.shutil, "which", side_effect=lambda name: str(locator) if name == "vswhere.exe" else None):
            self.assertEqual(verify.find_dumpbin(run), dumpbin)
            dumpbin.unlink()
            with self.assertRaisesRegex(ValueError, "dumpbin is missing"):
                verify.find_dumpbin(run)
        with patch.object(verify.shutil, "which", return_value=str(dumpbin)):
            self.assertEqual(verify.find_dumpbin(lambda *args, **kwargs: self.fail("PATH discovery should not run vswhere")), dumpbin)

    def test_native_windows_archive_verification_checks_exact_binary_imports(self):
        archive = self.package(WINDOWS)
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            self.assertEqual(command[:3], ["dumpbin.exe", "/NOLOGO", "/DEPENDENTS"])
            self.assertEqual(Path(command[3]).read_bytes(), executable(WINDOWS))
            self.assertEqual(kwargs["env"]["VSLANG"], "1033")
            return subprocess.CompletedProcess(command, 0, "Image has the following dependencies:\nKERNEL32.dll\nSummary\n")

        def failed_tool(*args, **kwargs):
            raise subprocess.CalledProcessError(1, "dumpbin")

        with patch.object(verify.sys, "platform", "win32"), patch.object(verify.shutil, "which", return_value="dumpbin.exe"):
            verify.verify(archive, run=run)
            self.assertEqual(len(commands), 1)
            with self.assertRaises(subprocess.CalledProcessError):
                verify.verify(archive, run=failed_tool)
            with self.assertRaises(ValueError):
                verify.verify(archive, run=lambda command, **kwargs: subprocess.CompletedProcess(command, 0, "Image has the following dependencies:\nVCRUNTIME140.dll\nSummary\n"))

    def test_missing_or_preview_cargo_artifact_fails(self):
        for item in ({"reason": "build-finished"}, {"reason": "compiler-artifact", "target": {"name": "shep", "kind": ["bin"]}, "features": ["test-support"], "executable": "fixture"}):
            with self.subTest(item=item), self.assertRaises(ValueError):
                release.build(self.root, LINUX, run=lambda command, item=item, **kwargs: subprocess.CompletedProcess(command, 0, json.dumps(item)))

    def test_source_mismatch_fails_before_stamping_or_building(self):
        original = (self.root / "Cargo.toml").read_bytes()
        with self.assertRaises(ValueError):
            release.prepare("2.3.4", source="b" * 40, root=self.root,
                            run=lambda command, **kwargs: subprocess.CompletedProcess(command, 0, SOURCE))
        self.assertEqual((self.root / "Cargo.toml").read_bytes(), original)

    def test_stamp_only_needs_no_rust_and_no_stamp_refuses_stale_versions(self):
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            self.assertEqual(command[0], "git")
            output = SOURCE if command[1] == "rev-parse" else "1234567890" if command[1] == "show" else ""
            return subprocess.CompletedProcess(command, 0, output)

        with self.assertRaises(ValueError):
            release.prepare("2.3.4", source=SOURCE, target=LINUX, no_stamp=True, root=self.root, run=run)
        release.prepare("2.3.4", source=SOURCE, stamp_only=True, root=self.root, run=run)
        release.verify_versions("2.3.4", self.root)
        self.assertTrue(commands)

    def test_changed_or_untracked_source_is_not_attributed_to_clean_git_commit(self):
        for tracked, untracked in (("src/main.rs\n", ""), ("", "src/new_module.rs\n")):
            def run(command, tracked=tracked, untracked=untracked, **kwargs):
                output = tracked if command[1] == "diff" else untracked
                return subprocess.CompletedProcess(command, 0, output)

            with self.subTest(tracked=tracked, untracked=untracked), self.assertRaises(ValueError):
                release.verify_source_tree(self.root, run)

    def test_verifier_rejects_wrong_executable_architecture_even_with_fresh_checksums(self):
        binary = self.root / "wrong-platform"
        binary.write_bytes(executable(WINDOWS))
        archive = release.package("1.2.3", LINUX, SOURCE, 1234567890, binary, self.root)
        with self.assertRaises(ValueError):
            verify.verify(archive, skip_install=True)

    def test_only_owned_versions_may_differ_from_source_manifests(self):
        original = (self.root / "Cargo.toml").read_text()
        release.stamp("2.3.4", self.root)

        def run(command, **kwargs):
            output = "Cargo.toml\n" if command[1] == "diff" else original if command[1] == "show" else ""
            return subprocess.CompletedProcess(command, 0, output)

        release.verify_source_tree(self.root, run)
        manifest = self.root / "Cargo.toml"
        manifest.write_text(manifest.read_text() + "\n[package.metadata.release_fixture]\nchanged = true\n")
        with self.assertRaises(ValueError):
            release.verify_source_tree(self.root, run)

    def test_archives_are_repeatable_and_preserve_both_platform_checksums(self):
        linux = self.package()
        digest = release.sha256(linux)
        for path in self.root.rglob("*"):
            if path.is_file():
                os.utime(path, (1999999999, 1999999999))
        self.assertEqual(release.sha256(self.package()), digest)
        windows = self.package(WINDOWS)
        text = (linux.parent / "SHA256SUMS").read_text()
        self.assertIn(f"{digest}  {linux.name}", text)
        self.assertIn(f"{release.sha256(windows)}  {windows.name}", text)
        for archive, target in ((linux, LINUX), (windows, WINDOWS)):
            self.assertEqual(verify.verify(archive, version="1.2.3", source=SOURCE, target=target, skip_install=True)["target"], target)
            self.assertEqual((archive.parent / (archive.name + ".sha256")).read_text(), f"{release.sha256(archive)}  {archive.name}\n")

    @unittest.skipUnless(sys.platform == "linux", "Runs the bundled Linux installer")
    def test_real_bundled_installer_installs_exact_binary_and_launcher_without_rust(self):
        verify.verify(self.package(), version="1.2.3", source=SOURCE, target=LINUX)

    def test_refuses_to_mix_versions_or_source_revisions(self):
        self.package()
        for version, source in (("1.2.4", SOURCE), ("1.2.3", "b" * 40)):
            with self.subTest(version=version, source=source), self.assertRaises(ValueError):
                self.package(WINDOWS, version, source)

    def test_checksums_and_expected_source_fail_closed(self):
        archive = self.package()
        with self.assertRaises(ValueError):
            verify.verify(archive, source="b" * 40, skip_install=True)
        sums = archive.parent / "SHA256SUMS"
        sums.write_text(sums.read_text() * 2)
        with self.assertRaises(ValueError):
            verify.verify(archive, skip_install=True)
        sums.write_text(f"{'0' * 64}  {archive.name}\n")
        with self.assertRaises(ValueError):
            verify.verify(archive, skip_install=True)

    def test_rejects_duplicate_link_traversal_missing_and_tampered_members(self):
        mutations = (
            lambda members: members.append(members[0]),
            lambda members: setattr(members[0][0], "type", tarfile.SYMTYPE),
            lambda members: setattr(members[0][0], "name", "../escaped"),
            lambda members: members.pop(0),
            lambda members: members.__setitem__(next(index for index, (member, _) in enumerate(members) if member.name == "shep"), (next(member for member, _ in members if member.name == "shep"), executable(LINUX) + b"modified")),
        )
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                shutil.rmtree(self.root / "dist", ignore_errors=True)
                archive = self.package()
                self.rewrite(archive, mutate)
                with self.assertRaises(ValueError):
                    verify.verify(archive, skip_install=True)

    def test_assembly_requires_both_platforms_and_checks_each_original_sidecar(self):
        self.package()
        with self.assertRaises(ValueError):
            release.assemble("1.2.3", SOURCE, self.root)
        windows = self.package(WINDOWS)
        release.assemble("1.2.3", SOURCE, self.root)
        (windows.parent / (windows.name + ".sha256")).write_text(f"{'0' * 64}  {windows.name}\n")
        with self.assertRaises(subprocess.CalledProcessError):
            release.assemble("1.2.3", SOURCE, self.root,
                             run=lambda command, check=True, **kwargs: subprocess.run(command, check=check, **kwargs, capture_output=True))


if __name__ == "__main__":
    unittest.main()
