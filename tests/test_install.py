import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("install_linux", ROOT / "scripts/install_linux.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


@unittest.skipUnless(sys.platform == "linux", "Linux desktop installer")
class Installation(unittest.TestCase):
    def test_checkout_wrapper_from_another_directory_honours_cargo_artifact(self):
        with tempfile.TemporaryDirectory(prefix="shep source install ") as temporary:
            root = Path(temporary)
            checkout = root / "checkout"
            shutil.copytree(ROOT / "scripts", checkout / "scripts", ignore=shutil.ignore_patterns("__pycache__"))
            shutil.copytree(ROOT / "assets", checkout / "assets")
            (checkout / "Cargo.toml").write_text('[package]\nname="shep"\nversion="0.0.0"\n')
            tools = root / "tools"
            tools.mkdir()
            capture = root / "cargo-args.json"
            cargo = tools / "cargo"
            cargo.write_text(f"#!{sys.executable}\n" + '''import json,os,pathlib,sys
pathlib.Path(os.environ["FIXTURE_CAPTURE"]).write_text(json.dumps({"args":sys.argv[1:],"cwd":os.getcwd()}))
target=pathlib.Path(os.environ["CARGO_TARGET_DIR"])/"x86_64-unknown-linux-gnu/release/shep"
target.parent.mkdir(parents=True,exist_ok=True)
target.write_bytes(b"fictional configured-target build")
print(json.dumps({"reason":"compiler-artifact","target":{"name":"shep","kind":["bin"]},"executable":str(target)}))
''')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                       CARGO_TARGET_DIR=str(root / "custom target"), FIXTURE_CAPTURE=str(capture))
            wrapper = ["bash", str(checkout / "scripts/install-linux.sh")]
            for arguments, success in ((["--help"], True), (["--not-an-option"], False), (["--prefix"], False)):
                result = subprocess.run(wrapper + arguments, cwd=root, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, success, result.stderr)
                self.assertFalse(capture.exists(), "help and invalid arguments must not build")
            options = ["--prefix", str(root / "installed"), "--data-dir", str(root / "data")]
            subprocess.run(wrapper + options, cwd=root, env=env, check=True, capture_output=True)
            invocation = json.loads(capture.read_text())
            self.assertEqual(invocation["cwd"], str(checkout))
            self.assertIn("--locked", invocation["args"])
            self.assertIn("--no-default-features", invocation["args"])
            self.assertEqual(invocation["args"][invocation["args"].index("--jobs") + 1], "4")
            self.assertEqual((root / "installed/bin/shep").read_bytes(), b"fictional configured-target build")
            capture.unlink()
            replacement = root / "explicit binary"
            replacement.write_bytes(b"explicit binary")
            subprocess.run(wrapper + [f"--binary={replacement}"] + options, cwd=root, env=env, check=True, capture_output=True)
            self.assertFalse(capture.exists())
            self.assertEqual((root / "installed/bin/shep").read_bytes(), b"explicit binary")
            mail = root / "data/mail.sqlite"
            mail.write_bytes(b"owned fixture mail")
            subprocess.run(wrapper + ["--uninstall"] + options, cwd=root, env=env, check=True, capture_output=True)
            self.assertFalse(capture.exists())
            self.assertFalse((root / "installed/bin/shep").exists())
            self.assertEqual(mail.read_bytes(), b"owned fixture mail")

    def test_install_update_uninstall_preserves_data(self):
        with tempfile.TemporaryDirectory(prefix="shep install ") as temporary:
            root = Path(temporary)
            prefix, data = root / "prefix", root / "data"
            binary = root / "release-binary"
            binary.write_bytes(b"first production build")
            with patch.object(installer.shutil, "which", return_value=None):
                executable, desktop, icon = installer.install(binary, prefix, data)
                self.assertEqual(executable.read_bytes(), binary.read_bytes())
                self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
                content = desktop.read_text()
                self.assertIn(f'Exec="{executable}"', content)
                self.assertIn("StartupWMClass=so.shep.Shep", content)
                self.assertIn("StartupNotify=false", content)
                self.assertIn("Icon=so.shep.Shep", content)
                self.assertTrue(icon.is_file())
                symbolic = data / "icons/hicolor/scalable/apps/so.shep.Shep-symbolic.svg"
                self.assertTrue(symbolic.is_file())
                self.assertIn("Icon=so.shep.Shep\n", content)
                scalable = symbolic.with_name("so.shep.Shep.svg")
                tray = symbolic.with_name("so.shep.Shep-tray.svg")
                self.assertEqual(scalable.read_bytes(), (ROOT / "assets/shepherd-light.svg").read_bytes())
                self.assertEqual(tray.read_bytes(), (ROOT / "assets/shepherd-tray.svg").read_bytes())
                binary.write_bytes(b"updated release")
                installer.install(binary, prefix, data)
                self.assertEqual(executable.read_bytes(), b"updated release")
                vault = data / "mail.sqlite"
                vault.write_bytes(b"personal mail")
                installer.install(None, prefix, data, uninstall=True)
                self.assertFalse(executable.exists())
                self.assertFalse(desktop.exists())
                self.assertFalse(icon.exists())
                self.assertFalse(symbolic.exists())
                self.assertFalse(scalable.exists())
                self.assertFalse(tray.exists())
                self.assertEqual(vault.read_bytes(), b"personal mail")

    def test_desktop_exec_escapes_reserved_characters(self):
        encoded = installer.exec_value('/tmp/a% "b$`\\c/shep')
        self.assertTrue(encoded.startswith('"') and encoded.endswith('"'))
        self.assertIn('%%', encoded)
        self.assertIn('\\\\$', encoded)
        self.assertIn('\\\\`', encoded)
        self.assertIn('\\\\"', encoded)
        with self.assertRaises(ValueError):
            installer.exec_value('/tmp/a\nExec=evil')

    def test_pin_preserves_existing_favorites_and_is_idempotent(self):
        import subprocess
        original = "['org.gnome.Nautilus.desktop', 'so.shep.Shep.desktop']"
        with patch.object(installer.shutil, "which", return_value="gsettings"), \
                patch.object(installer.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, original)) as run:
            installer.pin_gnome()
            self.assertEqual(run.call_args.args[0][-1], original)
