"""Exercise documented commands in owned checkouts with a fictional toolchain."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(sys.platform == "linux", "Linux shell command contracts")
class ReadmeScripts(unittest.TestCase):
    def test_quickstart_commands_clone_run_and_install_from_clean_directory(self):
        with tempfile.TemporaryDirectory(prefix="shep readme ") as directory:
            root = Path(directory)
            tools = root / "tools"
            tools.mkdir()
            git = tools / "git"
            git.write_text(f"#!{sys.executable}\n" + f'''import pathlib,shutil,sys
assert sys.argv[1:]==["clone","https://github.com/sam-ruff/shep.so.git"]
root=pathlib.Path("shep.so")
shutil.copytree({str(ROOT / "scripts")!r},root/"scripts")
shutil.copytree({str(ROOT / "assets")!r},root/"assets")
(root/"Cargo.toml").write_text('[package]\\nname="shep"\\nversion="0.0.0"\\n')
''')
            cargo = tools / "cargo"
            cargo.write_text(f"#!{sys.executable}\n" + '''import json,os,pathlib,sys
with open(os.environ["FIXTURE_CARGO_LOG"],"a") as log: log.write(json.dumps(sys.argv[1:])+"\\n")
assert "--release" in sys.argv and "--locked" in sys.argv and "--no-default-features" in sys.argv
if sys.argv[1]=="build":
 target=pathlib.Path("target/release/shep").resolve()
 target.parent.mkdir(parents=True)
 target.write_bytes(b"fictional quickstart build")
 print(json.dumps({"reason":"compiler-artifact","target":{"name":"shep","kind":["bin"]},"executable":str(target)}))
else: assert sys.argv[1]=="run"
''')
            git.chmod(0o755)
            cargo.chmod(0o755)
            home = root / "home"
            log = root / "cargo.jsonl"
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"], HOME=str(home),
                       XDG_DATA_HOME=str(home / ".local/share"), FIXTURE_CARGO_LOG=str(log))
            section = (ROOT / "README.md").read_text().split("## Get started", 1)[1]
            blocks = [part.split("```", 1)[0] for part in section.split("```sh\n")[1:]]
            self.assertEqual(len(blocks), 2)
            result = subprocess.run(["bash", "-e", "-c", "\n".join(blocks)], cwd=root, env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual([json.loads(line)[0] for line in log.read_text().splitlines()], ["run", "build"])
            self.assertEqual((home / ".local/bin/shep").read_bytes(), b"fictional quickstart build")
            self.assertTrue((home / ".local/share/applications/so.shep.Shep.desktop").is_file())

    def test_documented_hook_installer_works_from_another_directory(self):
        with tempfile.TemporaryDirectory(prefix="shep hooks ") as directory:
            root = Path(directory)
            checkout = root / "checkout"
            shutil.copytree(ROOT / ".githooks", checkout / ".githooks")
            (checkout / "scripts").mkdir()
            shutil.copyfile(ROOT / "scripts/install-hooks.sh", checkout / "scripts/install-hooks.sh")
            subprocess.run(["git", "init", "--quiet", str(checkout)], check=True)
            subprocess.run(["bash", str(checkout / "scripts/install-hooks.sh")], cwd=root, check=True,
                           capture_output=True)
            result = subprocess.run(["git", "-C", str(checkout), "config", "--local", "core.hooksPath"],
                                    check=True, capture_output=True, text=True)
            self.assertEqual(result.stdout.strip(), ".githooks")
            for name in ("pre-commit", "commit-msg"):
                self.assertTrue(os.access(checkout / ".githooks" / name, os.X_OK))

    def test_documented_checks_keep_full_native_gate_and_stop_on_failure(self):
        with tempfile.TemporaryDirectory(prefix="shep check ") as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copyfile(ROOT / "scripts/check.sh", root / "scripts/check.sh")
            tools = root / "tools"
            tools.mkdir()
            for name in ("cargo", "python3"):
                executable = tools / name
                executable.write_text(f"#!{sys.executable}\n" + '''import json,os,pathlib,sys
with open(os.environ["FIXTURE_CHECK_LOG"],"a") as log: log.write(json.dumps([pathlib.Path(sys.argv[0]).name,*sys.argv[1:]])+"\\n")
if os.environ.get("FIXTURE_CHECK_FAIL"): sys.exit(1)
''')
                executable.chmod(0o755)
            log = root / "checks.jsonl"
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                       SHEP_SKIP_E2E="0", FIXTURE_CHECK_LOG=str(log))
            subprocess.run(["bash", str(root / "scripts/check.sh")], cwd=tools, env=env, check=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertIn(["cargo", "test", "--all-features"], calls)
            self.assertIn(["python3", "scripts/e2e.py"], calls)
            self.assertEqual(calls[-1], ["python3", "scripts/performance_gate.py"])
            log.unlink()
            env["FIXTURE_CHECK_FAIL"] = "1"
            result = subprocess.run(["bash", str(root / "scripts/check.sh")], cwd=tools, env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(len(log.read_text().splitlines()), 1)
