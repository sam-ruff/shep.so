"""Keep failed required evidence separate from optional runner profiling."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHA = "a" * 40


@unittest.skipUnless(sys.platform == "linux", "Linux performance wrapper")
class PerformanceWrapperTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="shep timing checkout ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/ci-performance.sh", self.root / "scripts/ci-performance.sh")
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.env = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"],
                        TIMING_TEST_CALLS=str(self.calls), TIMING_REQUIRED_EXIT="0", TIMING_DIAGNOSTIC_EXIT="0")
        for name in ("RUST_LOG", "SHEP_E2E_ARTIFACTS"):
            self.env.pop(name, None)
        body = """import json, os, pathlib, sys
mode = os.environ['SHEP_PERFORMANCE_MODE']
args = sys.argv[1:]
report = (pathlib.Path(os.environ['SHEP_BENCH_REPORT_DIR']) / 'backend-progress.json'
          if pathlib.Path(sys.argv[0]).name == 'cargo' else pathlib.Path(args[args.index('--output') + 1]))
record = {'args': args, 'mode': mode, 'source': os.environ['SHEP_PERFORMANCE_SOURCE'],
          'report': str(report), 'trace': os.environ.get('RUST_LOG'),
          'screenshots': os.environ.get('SHEP_E2E_ARTIFACTS')}
with pathlib.Path(os.environ['TIMING_TEST_CALLS']).open('a') as output:
    output.write(json.dumps(record) + '\\n')
report.parent.mkdir(parents=True, exist_ok=True)
report.write_text(json.dumps(record))
if record['screenshots']:
    captures = pathlib.Path(record['screenshots'])
    captures.mkdir(parents=True, exist_ok=True)
    (captures / 'frame.webp').write_bytes(b'fixture')
print(mode + ' replay output')
sys.exit(int(os.environ['TIMING_REQUIRED_EXIT' if mode == 'required' else 'TIMING_DIAGNOSTIC_EXIT']))
"""
        for name in ("cargo", "python3"):
            path = self.tools / name
            path.write_text(f"#!{sys.executable}\n{body}")
            path.chmod(0o755)

    def run_script(self, phase, source=SHA):
        return subprocess.run(["bash", str(self.root / "scripts/ci-performance.sh"), phase, source],
                              env=self.env, capture_output=True, text=True, check=False)

    def test_required_success_never_runs_diagnostics(self):
        for phase in ("backend", "action"):
            with self.subTest(phase=phase):
                self.calls.unlink(missing_ok=True)
                result = self.run_script(phase)
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
                self.assertEqual(len(calls), 1)
                self.assertEqual(calls[0]["mode"], "required")
                self.assertEqual(calls[0]["source"], SHA)

    def test_backend_failure_preserves_original_status_and_samples(self):
        self.env["TIMING_REQUIRED_EXIT"] = "101"
        for diagnostic_exit in ("0", "7"):
            with self.subTest(diagnostic_exit=diagnostic_exit):
                self.calls.unlink(missing_ok=True)
                self.env["TIMING_DIAGNOSTIC_EXIT"] = diagnostic_exit
                result = self.run_script("backend")
                self.assertEqual(result.returncode, 101)
                required, diagnostic = [json.loads(line) for line in self.calls.read_text().splitlines()]
                self.assertEqual(required["args"], ["bench", "--locked", "--bench", "responsiveness"])
                self.assertEqual(diagnostic["args"], ["bench", "--locked", "--features", "test-support", "--bench", "responsiveness"])
                self.assertEqual(required["report"], "artifacts/performance/backend-progress.json")
                self.assertEqual(diagnostic["report"], "artifacts/diagnostics/backend/backend-progress.json")
                self.assertEqual(json.loads((self.root / required["report"]).read_text()), required)
                self.assertEqual(diagnostic["mode"], "diagnostic")
                self.assertEqual(diagnostic["source"], SHA)
                self.assertEqual(diagnostic["trace"], "shep::query_timing=debug")
                self.assertIn("diagnostic replay output", (self.root / "artifacts/diagnostics/backend/run.log").read_text())

    def test_action_failure_preserves_original_report_and_separate_captures(self):
        self.env["TIMING_REQUIRED_EXIT"] = "1"
        for diagnostic_exit in ("0", "9"):
            with self.subTest(diagnostic_exit=diagnostic_exit):
                self.calls.unlink(missing_ok=True)
                self.env["TIMING_DIAGNOSTIC_EXIT"] = diagnostic_exit
                result = self.run_script("action")
                self.assertEqual(result.returncode, 1)
                required, diagnostic = [json.loads(line) for line in self.calls.read_text().splitlines()]
                for call in (required, diagnostic):
                    self.assertEqual(call["args"][:3], ["scripts/action_latency.py", "--samples", "20"])
                    self.assertEqual(call["source"], SHA)
                self.assertEqual(required["report"], "artifacts/performance/actions.json")
                self.assertEqual(json.loads((self.root / required["report"]).read_text()), required)
                self.assertEqual(diagnostic["report"], "artifacts/diagnostics/action/actions.json")
                self.assertEqual(diagnostic["mode"], "diagnostic")
                self.assertEqual(diagnostic["screenshots"], str(self.root / "artifacts/diagnostics/action/e2e"))
                self.assertTrue((Path(diagnostic["screenshots"]) / "frame.webp").exists())

    def test_runner_cpu_samples_stop_with_the_wrapper_and_stay_out_of_reports(self):
        for phase, status in (("backend", "0"), ("action", "1")):
            with self.subTest(phase=phase):
                self.env["TIMING_REQUIRED_EXIT"] = status
                result = self.run_script(phase)
                self.assertEqual(result.returncode, int(status), result.stderr)
                log = self.root / f"artifacts/logs/runner-cpu-{phase}.log"
                samples = log.read_text().splitlines()
                # The start and exit samples bound the whole phase even between periodic ones.
                self.assertGreaterEqual(len(samples), 2)
                for sample in (samples[0], samples[-1]):
                    self.assertRegex(sample, r"^\d+ cpu +\d+( \d+){6,} \| [0-9.]+ [0-9.]+ [0-9.]+ ")
                time.sleep(2.5)
                self.assertEqual(log.read_text().splitlines(), samples)
                reports = [path for path in (self.root / "artifacts").rglob("*runner-cpu*") if path.parent.name != "logs"]
                self.assertEqual(reports, [])

    def test_invalid_source_or_phase_never_starts_measurement(self):
        for phase, source in (("backend", "main"), ("unknown", SHA)):
            self.assertEqual(self.run_script(phase, source).returncode, 2)
            self.assertFalse(self.calls.exists())

    def test_failed_diagnostic_directory_keeps_required_failure_and_evidence(self):
        (self.root / "artifacts").mkdir()
        (self.root / "artifacts/diagnostics").write_text("obstructed diagnostic directory")
        for phase, status in (("backend", 101), ("action", 3)):
            with self.subTest(phase=phase):
                self.calls.unlink(missing_ok=True)
                self.env["TIMING_REQUIRED_EXIT"] = str(status)
                result = self.run_script(phase)
                self.assertEqual(result.returncode, status, result.stderr)
                calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
                self.assertEqual(len(calls), 1)
                required = calls[0]
                self.assertEqual(json.loads((self.root / required["report"]).read_text()), required)
                self.assertIn("diagnostic replay exited 1", result.stderr)
