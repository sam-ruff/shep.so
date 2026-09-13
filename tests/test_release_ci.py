"""Only tested main artifacts with the planned version can be published."""
import copy
import importlib.util
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("release_ci", ROOT / "scripts/release_ci.py")
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)
SOURCE = "a" * 40


class ReleaseEligibility(unittest.TestCase):
    def setUp(self):
        self.event = {"workflow_run": {
            "name": "Desktop quality and release artifacts", "conclusion": "success",
            "event": "push", "head_branch": "main", "head_sha": SOURCE,
            "head_repository": {"full_name": "owner/mail"},
        }}
        self.plan = {"schema": 1, "source": SOURCE, "version": "1.2.3", "scope": "desktop",
                     "targets": ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]}

    def test_successful_current_main_is_required(self):
        self.assertEqual(RELEASE.validate_run(self.event, "owner/mail", SOURCE), SOURCE)
        for key, value in [("conclusion", "failure"), ("event", "pull_request"),
                           ("event", "workflow_dispatch"), ("head_branch", "feature"),
                           ("head_repository", {"full_name": "fork/mail"}),
                           ("name", "Different workflow"), ("head_sha", "bad")]:
            with self.subTest(key=key, value=value):
                event = copy.deepcopy(self.event)
                event["workflow_run"][key] = value
                with self.assertRaises(ValueError):
                    RELEASE.validate_run(event, "owner/mail", SOURCE)
        with self.assertRaises(ValueError):
            RELEASE.validate_run(self.event, "owner/mail", "b" * 40)

    def test_plan_is_bound_to_source_scope_and_both_platforms(self):
        self.assertEqual(RELEASE.validate_plan(self.plan, SOURCE), "1.2.3")
        for key, value in [("schema", 2), ("source", "b" * 40), ("scope", "all-clients"),
                           ("targets", ["x86_64-unknown-linux-gnu"]),
                           ("version", "1.2.3\nrelease=yes"), ("version", 123)]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                RELEASE.validate_plan({**self.plan, key: value}, SOURCE)
        for key in self.plan:
            with self.subTest(missing=key), self.assertRaises(ValueError):
                RELEASE.validate_plan({k: v for k, v in self.plan.items() if k != key}, SOURCE)

    def test_no_release_is_explicit_and_changed_versions_fail(self):
        self.assertIsNone(RELEASE.validate_plan({**self.plan, "version": None}, SOURCE))
        RELEASE.validate_version("1.2.3", "1.2.3")
        for version, expected in [("1.2.4", "1.2.3"), ("1.2.3", ""), ("unsafe", "unsafe")]:
            with self.subTest(version=version, expected=expected), self.assertRaises(ValueError):
                RELEASE.validate_version(version, expected)

    def test_release_analysis_cannot_publish(self):
        subprocess.run(["node", "--test", "scripts/release_plan.test.mjs"], cwd=ROOT, check=True)
