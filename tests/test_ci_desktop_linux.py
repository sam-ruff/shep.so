"""Exercise container orchestration without Docker or a personal profile."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHA = "a" * 40


@unittest.skipUnless(sys.platform == "linux", "Linux container orchestration")
class DesktopContainerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shep ci checkout ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / ".github").mkdir()
        for name in ("scripts/ci-desktop-linux.sh", ".github/desktop-linux.Dockerfile", ".github/desktop-linux-seccomp.json"):
            shutil.copyfile(ROOT / name, self.root / name)
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.tool("id", "import sys\nprint('1001' if sys.argv[1] == '-u' else '1002')")
        self.tool("docker", """import json, os, pathlib, sys
with pathlib.Path(os.environ['CI_TEST_CALLS']).open('a') as output:
    output.write(json.dumps({'args': sys.argv[1:], 'stdin': sys.stdin.read() if sys.argv[1] == 'build' else ''}) + '\\n')
if sys.argv[1] == 'build' and os.environ.get('CI_TEST_BUILD_FAIL'):
    sys.exit(7)
""")
        self.env = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"],
                        CI_TEST_CALLS=str(self.calls), SHEP_GOOGLE_CLIENT_ID="fixture-client",
                        SHEP_GOOGLE_CLIENT_SECRET="fixture-desktop-config", GITHUB_TOKEN="fixture-never-forward")
        self.env.pop("SHEP_DESKTOP_CONTAINER", None)

    def tool(self, name, body):
        path = self.tools / name
        path.write_text(f"#!{sys.executable}\n{body}\n")
        path.chmod(0o755)

    def run_script(self, *arguments):
        return subprocess.run(["bash", str(self.root / "scripts/ci-desktop-linux.sh"), *arguments],
                              env=self.env, capture_output=True, text=True, check=False)

    def test_container_owns_profile_and_forwards_only_explicit_configuration(self):
        result = self.run_script("1.2.3", SHA)
        self.assertEqual(result.returncode, 0, result.stderr)
        build, run = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(build["args"][-1], "-")
        self.assertEqual(build["stdin"], (ROOT / ".github/desktop-linux.Dockerfile").read_text())
        self.assertIn("SHEP_UID=1001", build["args"])
        self.assertIn("SHEP_GID=1002", build["args"])
        args = run["args"]
        self.assertEqual(args[args.index("--user") + 1], "1001:1002")
        self.assertIn("--shm-size=2g", args)
        self.assertIn("no-new-privileges", args)
        self.assertIn(f"seccomp={self.root}/.github/desktop-linux-seccomp.json", args)
        self.assertEqual([args[i + 1] for i, value in enumerate(args) if value == "--mount"],
                         [f"type=bind,source={self.root},target=/workspace"])
        self.assertEqual([args[i + 1] for i, value in enumerate(args) if value == "--env"],
                         ["SHEP_DESKTOP_CONTAINER=1", "SHEP_GOOGLE_CLIENT_ID", "SHEP_GOOGLE_CLIENT_SECRET"])
        self.assertEqual(args[-4:], ["bash", "scripts/ci-desktop-linux.sh", "1.2.3", SHA])
        self.assertNotIn("fixture-never-forward", json.dumps(run))
        self.assertNotIn("--privileged", args)

    def test_failed_build_never_runs_container(self):
        self.env["CI_TEST_BUILD_FAIL"] = "1"
        result = self.run_script("", SHA)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(len(self.calls.read_text().splitlines()), 1)

    def test_invalid_context_and_root_user_fail_before_docker(self):
        for version, source in (("v1.2.3", SHA), ("1.2.3", "main")):
            with self.subTest(version=version, source=source):
                self.assertEqual(self.run_script(version, source).returncode, 2)
                self.assertFalse(self.calls.exists())
        self.tool("id", "print('0')")
        self.assertIn("non-root", self.run_script("", SHA).stderr)
        self.assertFalse(self.calls.exists())

    def test_seccomp_keeps_default_denial_and_clone_fallback(self):
        policy = json.loads((ROOT / ".github/desktop-linux-seccomp.json").read_text())
        self.assertEqual(policy["defaultAction"], "SCMP_ACT_ERRNO")
        clone3 = [rule for rule in policy["syscalls"] if "clone3" in rule["names"]]
        fallback = [rule for rule in clone3 if rule["action"] == "SCMP_ACT_ERRNO"]
        self.assertEqual(len(fallback), 1)
        self.assertEqual(fallback[0]["errnoRet"], 38)
        self.assertEqual(fallback[0]["excludes"]["caps"], ["CAP_SYS_ADMIN"])
        unrestricted = {name for rule in policy["syscalls"]
                        if rule["action"] == "SCMP_ACT_ALLOW" and not rule.get("includes") and not rule.get("args")
                        for name in rule["names"]}
        self.assertTrue({"openat2", "clone", "setns", "unshare"}.issubset(unrestricted))
        self.assertTrue({"mount", "fsmount", "fsconfig", "bpf"}.isdisjoint(unrestricted))
        self.assertIn("61eaf32614c7c71b60bd8927d3e6a4ffc8ff1f31", policy["comment"])
        self.assertIn("Apache License", policy["comment"])
