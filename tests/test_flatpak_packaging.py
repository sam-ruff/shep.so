"""Flatpak manifest, metainfo and offline Cargo source contracts."""
import configparser
import importlib.util
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ElementTree

ROOT = Path(__file__).resolve().parents[1]
PACKAGING = ROOT / "packaging/flatpak"
MANIFEST = PACKAGING / "so.shep.Shep.json"
METAINFO = PACKAGING / "so.shep.Shep.metainfo.xml"
DESKTOP = PACKAGING / "so.shep.Shep.desktop"
INSTALL = re.compile(r"^install -Dm(?P<mode>\d+) (?P<source>\S+) (?P<target>\S+)$")

# Each permission is justified in docs/agents/flatpak.md; adding one needs a review there too.
ALLOWED_FINISH_ARGS = {
    "--share=ipc",
    "--share=network",
    "--socket=wayland",
    "--socket=fallback-x11",
    "--talk-name=org.freedesktop.Notifications",
    "--talk-name=org.kde.StatusNotifierWatcher",
    "--talk-name=com.canonical.Unity",
    "--talk-name=org.freedesktop.secrets",
}


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module}), patch.object(sys, "path", [str(path.parent), *sys.path]):
        spec.loader.exec_module(module)
    return module


installer = load("install_linux", ROOT / "scripts/install_linux.py")
release = load("release", ROOT / "scripts/release.py")
cargo_sources = load("cargo_sources", PACKAGING / "cargo_sources.py")


def manifest():
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def shep_module():
    modules = [module for module in manifest()["modules"] if isinstance(module, dict) and module["name"] == "shep"]
    if len(modules) != 1:
        raise AssertionError("Expected exactly one shep module")
    return modules[0]


def installs():
    """Map each installed /app path to its checkout source."""
    result = {}
    for command in shep_module()["build-commands"]:
        match = INSTALL.match(command)
        if match:
            result[match["target"]] = match["source"]
    return result


def desktop_entry(text):
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    parser.read_string(text)
    return dict(parser["Desktop Entry"])


def png_size(path):
    header = path.read_bytes()[:24]
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError(f"{path.name} is not a PNG")
    return struct.unpack(">II", header[16:24])


class Manifest(unittest.TestCase):
    def test_app_identity_matches_the_linux_installer(self):
        data = manifest()
        self.assertEqual(data["id"], installer.APP_ID)
        self.assertEqual(MANIFEST.name, f"{installer.APP_ID}.json")
        self.assertEqual(data["command"], "shep")
        with (ROOT / "Cargo.toml").open("rb") as cargo:
            self.assertEqual(tomllib.load(cargo)["package"]["name"], data["command"])
        self.assertEqual(installs()["/app/bin/shep"], "target/release/shep")

    def test_uses_the_freedesktop_runtime_and_rust_extension(self):
        data = manifest()
        self.assertEqual(data["runtime"], "org.freedesktop.Platform")
        self.assertEqual(data["sdk"], "org.freedesktop.Sdk")
        self.assertRegex(data["runtime-version"], r"^\d{2}\.08$")
        self.assertEqual(data["sdk-extensions"], ["org.freedesktop.Sdk.Extension.rust-stable"])
        self.assertIn("/usr/lib/sdk/rust-stable/bin", data["build-options"]["append-path"])

    def test_cargo_builds_offline_from_generated_locked_sources(self):
        module = shep_module()
        build = module["build-commands"][0]
        self.assertIn("--offline", build)
        self.assertIn("--locked", build)
        self.assertIn("--release", build)
        self.assertNotIn("--features", build)
        self.assertEqual(manifest()["build-options"]["env"]["CARGO_HOME"], f"/run/build/{module['name']}/cargo")
        self.assertIn("cargo-sources.json", module["sources"])
        checkout = [source for source in module["sources"] if isinstance(source, dict)]
        self.assertEqual([source["type"] for source in checkout], ["dir"])
        self.assertEqual((MANIFEST.parent / checkout[0]["path"]).resolve(), ROOT)
        self.assertIn("target", checkout[0]["skip"])
        self.assertNotIn("build-args", manifest()["build-options"], "builds must not request network access")

    def test_finish_args_are_minimal_and_never_grant_broad_filesystem_access(self):
        arguments = manifest()["finish-args"]
        self.assertEqual(len(arguments), len(set(arguments)))
        self.assertEqual(set(arguments), ALLOWED_FINISH_ARGS)
        for argument in arguments:
            self.assertFalse(argument.startswith(("--filesystem", "--persist", "--device")), argument)
            self.assertNotIn("*", argument)
        self.assertNotIn("--socket=x11", arguments)
        self.assertFalse({"--socket=session-bus", "--socket=system-bus"} & set(arguments))

    def test_installs_the_same_launcher_and_icons_as_the_linux_installer(self):
        with tempfile.TemporaryDirectory(prefix="shep flatpak ") as temporary:
            root = Path(temporary)
            binary = root / "shep"
            binary.write_bytes(b"#!/bin/sh\n")
            with patch.object(shutil, "which", return_value=None), redirect_stdout(io.StringIO()):
                installer.install(binary, root / "prefix", root / "share", process_root=root / "proc")
            expected = {f"/app/share/{path.relative_to(root / 'share')}" for path in (root / "share").rglob("*")
                        if path.is_file()}
            installed_launcher = (root / "share/applications" / f"{installer.APP_ID}.desktop").read_text()
        shared = {target for target in installs() if target.startswith("/app/share/") and "/licenses/" not in target
                  and "/metainfo/" not in target}
        self.assertEqual(shared, expected)
        for target, source in installs().items():
            self.assertTrue((ROOT / source).is_file() or source.startswith("target/"), source)
            if "/icons/" in target and not target.endswith(".png"):
                self.assertTrue(source.endswith(".svg"), source)
        ours = desktop_entry(DESKTOP.read_text(encoding="utf-8"))
        theirs = desktop_entry(installed_launcher)
        self.assertEqual(ours.pop("Exec"), manifest()["command"])
        theirs.pop("Exec")
        self.assertEqual(ours, theirs)
        self.assertEqual(ours["StartupWMClass"], installer.APP_ID)

    def test_installs_every_release_licence_notice(self):
        notices = {path for path in release.CONTENTS if "LICENSE" in path or path.startswith("licenses/")
                   or path.endswith((".patch", "README.shep.md", "UPSTREAM.md"))}
        loop = next(command for command in shep_module()["build-commands"] if command.startswith("for notice in"))
        listed = set(loop.removeprefix("for notice in ").split("; do", 1)[0].split())
        self.assertEqual(listed, notices)
        self.assertIn('"/app/share/licenses/so.shep.Shep/$notice"', loop)
        for notice in listed:
            self.assertTrue((ROOT / notice).is_file(), notice)


class Metainfo(unittest.TestCase):
    def setUp(self):
        self.root = ElementTree.parse(METAINFO).getroot()

    def test_identity_launchable_and_binary_match_the_manifest(self):
        self.assertEqual(self.root.get("type"), "desktop-application")
        self.assertEqual(self.root.findtext("id"), installer.APP_ID)
        self.assertEqual(self.root.findtext("launchable[@type='desktop-id']"), DESKTOP.name)
        self.assertEqual(self.root.findtext("provides/binary"), manifest()["command"])
        with (ROOT / "Cargo.toml").open("rb") as cargo:
            self.assertEqual(self.root.findtext("project_license"), tomllib.load(cargo)["package"]["license"])
        self.assertEqual(installs()[f"/app/share/metainfo/{METAINFO.name}"], f"packaging/flatpak/{METAINFO.name}")

    def test_store_metadata_is_complete(self):
        self.assertLessEqual(len(self.root.findtext("summary")), 35)
        self.assertTrue(self.root.findtext("developer/name"))
        for kind in ("homepage", "bugtracker", "vcs-browser", "help"):
            self.assertTrue(self.root.findtext(f"url[@type='{kind}']", "").startswith("https://"), kind)
        colours = {colour.get("scheme_preference"): colour.text for colour in self.root.findall("branding/color")}
        self.assertEqual(set(colours), {"light", "dark"})
        for colour in colours.values():
            self.assertRegex(colour, r"^#[0-9a-f]{6}$")
        self.assertEqual(self.root.find("content_rating").get("type"), "oars-1.1")
        releases = self.root.findall("releases/release")
        self.assertTrue(releases)
        self.assertRegex(releases[0].get("date"), r"^\d{4}-\d{2}-\d{2}$")

    def test_screenshots_are_committed_fixture_pngs_with_declared_sizes(self):
        screenshots = self.root.findall("screenshots/screenshot")
        self.assertGreaterEqual(len(screenshots), 3)
        self.assertEqual([shot.get("type") for shot in screenshots].count("default"), 1)
        prefix = "https://raw.githubusercontent.com/sam-ruff/shep.so/main/packaging/flatpak/"
        for screenshot in screenshots:
            self.assertTrue(screenshot.findtext("caption"))
            image = screenshot.find("image")
            self.assertTrue(image.text.startswith(prefix), image.text)
            local = PACKAGING / image.text.removeprefix(prefix)
            self.assertEqual(png_size(local), (int(image.get("width")), int(image.get("height"))))

    @unittest.skipUnless(shutil.which("appstreamcli"), "appstreamcli is not installed")
    def test_appstreamcli_accepts_the_metainfo(self):
        result = subprocess.run(["appstreamcli", "validate", "--no-net", str(METAINFO)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipUnless(shutil.which("desktop-file-validate"), "desktop-file-validate is not installed")
    def test_desktop_file_validate_accepts_the_launcher(self):
        result = subprocess.run(["desktop-file-validate", str(DESKTOP)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class CargoSources(unittest.TestCase):
    def test_every_registry_package_in_the_lock_is_vendored_by_checksum(self):
        with (ROOT / "Cargo.lock").open("rb") as lock:
            parsed = tomllib.load(lock)
        sources = cargo_sources.crate_sources(parsed)
        archives = {source["dest"]: source for source in sources if source["type"] == "archive"}
        registry = [package for package in parsed["package"] if "source" in package]
        self.assertEqual(len(archives), len(registry))
        for package in registry:
            archive = archives[f"cargo/vendor/{package['name']}-{package['version']}"]
            self.assertEqual(archive["sha256"], package["checksum"])
            self.assertTrue(archive["url"].startswith("https://static.crates.io/crates/"))
        checksums = [source for source in sources if source.get("dest-filename") == ".cargo-checksum.json"]
        self.assertEqual(len(checksums), len(registry))
        config = sources[-1]
        self.assertEqual((config["dest"], config["dest-filename"]), ("cargo", "config.toml"))
        self.assertEqual(tomllib.loads(config["contents"])["source"]["crates-io"]["replace-with"], "vendored-sources")

    def test_path_members_are_built_from_the_checkout(self):
        lock = {"package": [{"name": "shep", "version": "0.1.0"}]}
        self.assertEqual(cargo_sources.crate_sources(lock), [cargo_sources.crate_sources({})[-1]])

    def test_git_and_unchecked_sources_are_rejected(self):
        git = {"name": "x", "version": "1.0.0", "source": "git+https://example.test/x#abc"}
        unchecked = {"name": "y", "version": "1.0.0", "source": cargo_sources.CRATES_IO}
        for package in (git, unchecked):
            with self.subTest(package=package["name"]), self.assertRaises(ValueError):
                cargo_sources.crate_sources({"package": [package]})

    def test_command_line_writes_json_sources(self):
        with tempfile.TemporaryDirectory(prefix="shep cargo sources ") as temporary:
            lock = Path(temporary) / "Cargo.lock"
            lock.write_text('version = 4\n[[package]]\nname = "a"\nversion = "1.2.3"\n'
                            f'source = "{cargo_sources.CRATES_IO}"\nchecksum = "{"0" * 64}"\n')
            output = Path(temporary) / "cargo-sources.json"
            subprocess.run([sys.executable, str(PACKAGING / "cargo_sources.py"), "--lock", str(lock),
                            "--output", str(output)], check=True, capture_output=True)
            written = json.loads(output.read_text())
        self.assertEqual(written[0]["url"], "https://static.crates.io/crates/a/a-1.2.3.crate")
        self.assertEqual(len(written), 3)


if __name__ == "__main__":
    unittest.main()
