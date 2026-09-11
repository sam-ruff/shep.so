import importlib.util
from pathlib import Path
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
