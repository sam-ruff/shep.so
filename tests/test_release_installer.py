import contextlib
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_installer", ROOT / "scripts/install_release.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class ReleaseFixture:
    def __init__(self):
        self.files = {}
        self.requests = []
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                fixture.requests.append(self.path)
                body = fixture.files.get(self.path)
                self.send_response(200 if body is not None else 404)
                self.end_headers()
                if body is not None:
                    self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.worker = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.worker.start()
        self.seed()

    def seed(self, binary=b"fictional release one", entries=None, checksum=None):
        self.name = "shep-1.2.3-linux-x86_64.tar.gz"
        contents = entries if entries is not None else [
            ("shep", binary),
            ("scripts/install_linux.py", (ROOT / "scripts/install_linux.py").read_bytes()),
            ("assets/launcher.png", (ROOT / "assets/launcher.png").read_bytes()),
            ("assets/shepherd-symbolic.svg", (ROOT / "assets/shepherd-symbolic.svg").read_bytes()),
        ]
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for name, value in contents:
                entry = tarfile.TarInfo(name)
                if isinstance(value, tuple):
                    entry.type, entry.linkname = value
                    archive.addfile(entry)
                else:
                    entry.size = len(value)
                    archive.addfile(entry, io.BytesIO(value))
        payload = buffer.getvalue()
        self.files[f"/sam-ruff/shep.so/releases/download/v1.2.3/{self.name}"] = payload
        self.files["/sam-ruff/shep.so/releases/download/v1.2.3/SHA256SUMS"] = (
            f"{checksum or hashlib.sha256(payload).hexdigest()}  {self.name}\n").encode()
        release = {"tag_name": "v1.2.3", "draft": False, "assets": [
            {"name": name, "browser_download_url": installer.DOWNLOADS + "v1.2.3/" + name}
            for name in [self.name, "SHA256SUMS"]]}
        self.files["/repos/sam-ruff/shep.so/releases/latest"] = json.dumps(release).encode()
        self.files["/repos/sam-ruff/shep.so/releases/tags/v1.2.3"] = json.dumps(release).encode()

    def fetch(self, url):
        # Explicit fixture transport: production URLs are checked before being
        # routed to this owned loopback HTTP server; no internet or credentials.
        path = urllib.parse.urlsplit(url).path
        return urllib.request.urlopen(f"http://127.0.0.1:{self.server.server_port}{path}", timeout=3)

    def seed_source(self, entries=None):
        commit = "abc12345" * 5
        self.files["/repos/sam-ruff/shep.so/commits/main"] = json.dumps({"sha": commit}).encode()
        files = entries if entries is not None else [
            (name, (ROOT / name).read_bytes()) for name in (
                "Cargo.toml", "Cargo.lock", "scripts/install_linux.py", "assets/launcher.png",
                "assets/shepherd-light.svg", "assets/shepherd-tray.svg", "assets/shepherd-symbolic.svg")]
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for name, contents in files:
                entry = tarfile.TarInfo(f"shep.so-{commit}/{name}")
                entry.size = len(contents)
                archive.addfile(entry, io.BytesIO(contents))
        self.files[f"/sam-ruff/shep.so/tar.gz/{commit}"] = buffer.getvalue()
        return commit

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join()


@unittest.skipUnless(sys.platform == "linux", "Linux native release installer")
class ReleaseInstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="shep raw installer ")
        self.root = Path(self.directory.name)
        self.fixture = ReleaseFixture()
        self.stages = []
        self.args = installer.parser().parse_args(["--yes", "--prefix", str(self.root / "prefix"),
            "--data-dir", str(self.root / "data")])

    def tearDown(self):
        uncleaned = [stage for stage in self.stages if stage.exists()]
        self.fixture.close()
        self.directory.cleanup()
        self.assertFalse(uncleaned, "installer staging must be removed on success/failure/cancellation")

    def install(self, **kwargs):
        temporary = tempfile.TemporaryDirectory
        def stage(*args, **options):
            result = temporary(*args, dir=self.root, **options)
            self.stages.append(Path(result.name))
            return result
        fetch = kwargs.pop("fetch", self.fixture.fetch)
        with contextlib.redirect_stdout(io.StringIO()), patch.object(installer.tempfile, "TemporaryDirectory", side_effect=stage):
            return installer.install(self.args, fetch=fetch, machine="x86_64", **kwargs)

    def test_download_verified_archive_installs_and_atomically_updates_native_launcher(self):
        self.install()
        binary = self.root / "prefix/bin/shep"
        self.assertEqual(binary.read_bytes(), b"fictional release one")
        self.assertTrue(os.access(binary, os.X_OK))
        launcher = self.root / "data/applications/so.shep.Shep.desktop"
        self.assertIn('StartupWMClass=so.shep.Shep', launcher.read_text())
        self.assertIn(str(binary), launcher.read_text())
        with binary.open("rb") as old_process:
            self.fixture.seed(binary=b"fictional release two")
            self.args.version = "v1.2.3"
            self.install()
            self.assertEqual(old_process.read(), b"fictional release one")
        self.assertEqual(binary.read_bytes(), b"fictional release two")
        self.assertIn("/repos/sam-ruff/shep.so/releases/tags/v1.2.3", self.fixture.requests)
        self.assertTrue((self.root / "data/icons/hicolor/128x128/apps/so.shep.Shep.png").is_file())

    def test_checksum_failure_retains_previous_install_and_cleans_stage(self):
        self.install()
        self.fixture.seed(checksum="0" * 64)
        with self.assertRaisesRegex(installer.InstallError, "checksum mismatch"):
            self.install()
        self.assertEqual((self.root / "prefix/bin/shep").read_bytes(), b"fictional release one")

    def test_unsafe_archives_never_write_outside_staging_or_install(self):
        cases = [
            [("../escaped", b"bad")], [(str(self.root / "outside"), b"bad")],
            [("pivot", (tarfile.SYMTYPE, "..")), ("pivot/escaped", b"bad")],
            [("hard", (tarfile.LNKTYPE, "../escaped"))],
            [("duplicate", b"one"), ("duplicate", b"two")], [("a\\b", b"bad")],
        ]
        for entries in cases:
            with self.subTest(entries=entries):
                self.fixture.seed(entries=entries)
                with self.assertRaisesRegex(installer.InstallError, "unsafe or duplicate"):
                    self.install()
                self.assertFalse((self.root / "prefix").exists())
                self.assertFalse((self.root / "escaped").exists())
                self.assertFalse((self.root / "outside").exists())

    def test_missing_asset_checksum_and_archive_files_are_explicit(self):
        with self.assertRaisesRegex(installer.InstallError, "no linux/aarch64"):
            installer.release_asset(None, "linux", "aarch64", self.fixture.fetch)
        release = json.loads(self.fixture.files["/repos/sam-ruff/shep.so/releases/latest"])
        release["assets"] = release["assets"][:1]
        self.fixture.files["/repos/sam-ruff/shep.so/releases/latest"] = json.dumps(release).encode()
        with self.assertRaisesRegex(installer.InstallError, "no SHA256SUMS"):
            self.install()
        self.fixture.seed(entries=[("shep", b"incomplete")])
        with self.assertRaisesRegex(installer.InstallError, "missing scripts/install_linux.py"):
            self.install()
        self.assertFalse((self.root / "prefix").exists())

    def test_unpublished_release_and_invalid_version_do_not_install(self):
        self.fixture.files.pop("/repos/sam-ruff/shep.so/releases/latest")
        self.args.release_only = True
        with self.assertRaisesRegex(installer.InstallError, "No published"):
            self.install()
        with self.assertRaisesRegex(installer.InstallError, "Use a version"):
            installer.release_asset("../../main", "linux", "x86_64", self.fixture.fetch)
        self.assertFalse((self.root / "prefix").exists())

    def source_run(self, command, **options):
        if command[0] == "cargo":
            self.assertEqual(command[1], "build")
            for flag in ("--release", "--locked", "--no-default-features"):
                self.assertIn(flag, command)
            self.assertEqual(command[command.index("--jobs") + 1], "4")
            binary = Path(command[command.index("--target-dir") + 1]) / "x86_64-unknown-linux-gnu/release/shep"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"fictional source build")
            self.assertEqual(Path(command[command.index("--manifest-path") + 1]).parent, options["cwd"])
            artifact = {"reason": "compiler-artifact", "target": {"name": "shep", "kind": ["bin"]},
                        "executable": str(binary)}
            return subprocess.CompletedProcess(command, 0, json.dumps(artifact))
        return subprocess.run(command, **options)

    def test_missing_latest_builds_pinned_source_and_installs_real_artifacts(self):
        commit = self.fixture.seed_source()
        self.fixture.files.pop("/repos/sam-ruff/shep.so/releases/latest")
        with patch.object(installer.shutil, "which", return_value="cargo"):
            self.install(run=self.source_run)
        self.assertIn(f"/sam-ruff/shep.so/tar.gz/{commit}", self.fixture.requests)
        self.assertEqual((self.root / "prefix/bin/shep").read_bytes(), b"fictional source build")
        for name in ("so.shep.Shep.svg", "so.shep.Shep-tray.svg", "so.shep.Shep-symbolic.svg"):
            self.assertTrue((self.root / "data/icons/hicolor/scalable/apps" / name).is_file())

    def test_explicit_source_skips_release_and_build_failure_preserves_install(self):
        self.install()
        self.fixture.seed_source()
        self.fixture.requests.clear()
        self.args.source = True
        run = Mock(side_effect=subprocess.CalledProcessError(1, "cargo"))
        with patch.object(installer.shutil, "which", return_value="cargo"):
            with self.assertRaises(subprocess.CalledProcessError):
                self.install(run=run)
        self.assertFalse(any("/releases/" in request for request in self.fixture.requests))
        self.assertEqual(run.call_count, 1)
        self.assertEqual((self.root / "prefix/bin/shep").read_bytes(), b"fictional release one")

    def test_version_and_checksum_failure_never_fall_back_to_source(self):
        self.fixture.seed_source()
        self.args.version = "9.9.9"
        run = Mock()
        with self.assertRaises(installer.NoPublishedRelease):
            self.install(run=run)
        self.args.version = None
        self.fixture.seed(checksum="0" * 64)
        with self.assertRaisesRegex(installer.InstallError, "checksum mismatch"):
            self.install(run=run)
        run.assert_not_called()
        self.assertNotIn("/repos/sam-ruff/shep.so/commits/main", self.fixture.requests)

    def test_invalid_source_revision_and_archive_fail_before_build(self):
        self.args.source = True
        run = Mock()
        self.fixture.seed_source()
        with patch.object(installer.shutil, "which", return_value="cargo"):
            self.fixture.files["/repos/sam-ruff/shep.so/commits/main"] = b'{"sha":"../../main"}'
            with self.assertRaisesRegex(installer.InstallError, "exact source revision"):
                self.install(run=run)
            self.fixture.seed_source(entries=[("../../escaped", b"bad")])
            with self.assertRaisesRegex(installer.InstallError, "unsafe or duplicate"):
                self.install(run=run)
        run.assert_not_called()
        self.assertFalse((self.root / "escaped").exists())

    def test_source_all_user_builds_unprivileged_and_elevates_only_final_copy(self):
        self.fixture.seed_source()
        self.args = installer.parser().parse_args(["--source", "--system"])
        commands = []
        def run(command, **options):
            commands.append(command)
            if command[0] == "cargo":
                return self.source_run(command, **options)
            return subprocess.CompletedProcess(command, 0)
        with patch.object(installer.os, "geteuid", return_value=1000), \
                patch.object(installer.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}"):
            self.install(run=run)
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[0][0], "cargo")
        self.assertEqual(commands[1][:3], ["/usr/bin/sudo", "--", sys.executable])

    def test_invalid_system_scope_fails_before_network_or_build(self):
        args = installer.parser().parse_args(["--source", "--system", "--pin"])
        fetch, run = Mock(), Mock()
        with self.assertRaisesRegex(installer.InstallError, "cannot combine"):
            installer.install(args, fetch=fetch, run=run)
        fetch.assert_not_called()
        run.assert_not_called()

    def test_readme_pipeline_installs_source_outside_checkout(self):
        commit = self.fixture.seed_source()
        self.fixture.files.pop("/repos/sam-ruff/shep.so/releases/latest")
        tools = self.root / "tools"
        tools.mkdir()
        temporary = self.root / "temporary"
        temporary.mkdir()
        adapter = self.root / "fixture-transport.py"
        adapter.write_text(f'''import importlib.util,sys,urllib.parse,urllib.request
spec=importlib.util.spec_from_file_location("installer",{str(ROOT / "scripts/install_release.py")!r})
installer=importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
def fetch(url):
 return urllib.request.urlopen("http://127.0.0.1:{self.fixture.server.server_port}"+urllib.parse.urlsplit(url).path,timeout=3)
installer.install(installer.parser().parse_args(),fetch=fetch,machine="x86_64")
''')
        curl = tools / "curl"
        curl.write_text(f"#!{sys.executable}\n" + f'''import pathlib,sys
url=next(value for value in sys.argv if value.startswith("https://"))
if url=="https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-linux.sh":
 sys.stdout.buffer.write(pathlib.Path({str(ROOT / "scripts/install-release-linux.sh")!r}).read_bytes())
else:
 assert url=="https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install_release.py"
 assert "=https" in sys.argv
 pathlib.Path(sys.argv[sys.argv.index("--output")+1]).write_bytes(pathlib.Path({str(adapter)!r}).read_bytes())
''')
        cargo = tools / "cargo"
        cargo.write_text(f"#!{sys.executable}\n" + '''import json,pathlib,sys
assert "--locked" in sys.argv and "--no-default-features" in sys.argv
assert sys.argv[sys.argv.index("--jobs")+1]=="4"
target=pathlib.Path(sys.argv[sys.argv.index("--target-dir")+1])/"configured-target/release/shep"
target.parent.mkdir(parents=True)
target.write_bytes(b"fictional pipeline source build")
print(json.dumps({"reason":"compiler-artifact","target":{"name":"shep","kind":["bin"]},"executable":str(target)}))
''')
        curl.chmod(0o755)
        cargo.chmod(0o755)
        home = self.root / "home"
        env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"], HOME=str(home),
                   XDG_DATA_HOME=str(home / ".local/share"), TMPDIR=str(temporary))
        readme = (ROOT / "README.md").read_text()
        command = readme.split("```sh\n", 1)[1].split("```", 1)[0].strip()
        result = subprocess.run(["bash", "-o", "pipefail", "-c", command], cwd=self.root,
                                env=env, capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        binary = home / ".local/bin/shep"
        self.assertEqual(binary.read_bytes(), b"fictional pipeline source build")
        self.assertTrue(os.access(binary, os.X_OK))
        launcher = home / ".local/share/applications/so.shep.Shep.desktop"
        self.assertIn('StartupNotify=false', launcher.read_text())
        self.assertIn(str(binary), launcher.read_text())
        self.assertIn(f"/sam-ruff/shep.so/tar.gz/{commit}", self.fixture.requests)
        self.assertEqual(list(temporary.iterdir()), [])

    def test_scope_defaults_to_user_and_cancel_happens_before_network(self):
        args = installer.parser().parse_args([])
        self.assertFalse(installer.installation_scope(args, lambda: ""))
        self.assertTrue(installer.installation_scope(args, lambda: "all"))
        fetch = Mock()
        with self.assertRaisesRegex(installer.InstallError, "cancelled"):
            installer.install(args, fetch=fetch, choose=lambda: "c", machine="x86_64")
        fetch.assert_not_called()

    def test_all_user_elevation_is_explicit_and_cancellation_never_claims_success(self):
        self.args = installer.parser().parse_args(["--system"])
        run = Mock(side_effect=subprocess.CalledProcessError(1, "sudo"))
        with patch.object(installer.os, "geteuid", return_value=1000), patch.object(installer.shutil, "which", return_value="/usr/bin/sudo"):
            with self.assertRaises(subprocess.CalledProcessError):
                self.install(run=run)
        command = run.call_args.args[0]
        self.assertEqual(command[:3], ["/usr/bin/sudo", "--", sys.executable])
        self.assertEqual(command[-4:], ["--prefix", "/usr/local", "--data-dir", "/usr/local/share"])
        self.assertFalse((self.root / "prefix").exists())

    def test_interrupted_download_never_changes_existing_install(self):
        self.install()
        fetch = self.fixture.fetch

        class Interrupted(io.BytesIO):
            def read(self, size=-1):
                raise KeyboardInterrupt()

        def interrupt(url):
            return Interrupted() if url.endswith(".tar.gz") else fetch(url)

        with self.assertRaises(KeyboardInterrupt):
            self.install(fetch=interrupt)
        self.assertEqual((self.root / "prefix/bin/shep").read_bytes(), b"fictional release one")

    def test_raw_bash_wrapper_quotes_arguments_and_cleans_download_on_failure(self):
        tools = self.root / "tools"
        tools.mkdir()
        temporary = self.root / "temporary"
        temporary.mkdir()
        capture = self.root / "args.json"
        curl = tools / "curl"
        curl.write_text(f"#!{sys.executable}\n" + '''import json, os, pathlib, sys
if os.environ.get("SHEP_FIXTURE_CURL_FAIL"): sys.exit(22)
assert "--proto" in sys.argv and "=https" in sys.argv
assert "https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install_release.py" in sys.argv
path = pathlib.Path(sys.argv[sys.argv.index("--output") + 1])
path.write_text("import json,os,sys; open(os.environ['SHEP_FIXTURE_ARGS'],'w').write(json.dumps(sys.argv[1:]))")
''')
        curl.chmod(0o755)
        env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"], TMPDIR=str(temporary), SHEP_FIXTURE_ARGS=str(capture))
        literal = str(self.root / 'spaces;$(touch should-not-exist)')
        wrapper = ["bash", str(ROOT / "scripts/install-release-linux.sh"), "--prefix", literal, "--yes"]
        subprocess.run(wrapper, env=env, check=True)
        self.assertEqual(json.loads(capture.read_text()), ["--platform", "linux", "--prefix", literal, "--yes"])
        self.assertEqual(list(temporary.iterdir()), [])
        capture.unlink()
        env["SHEP_FIXTURE_CURL_FAIL"] = "1"
        self.assertNotEqual(subprocess.run(wrapper, env=env).returncode, 0)
        self.assertFalse(capture.exists())
        self.assertEqual(list(temporary.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
