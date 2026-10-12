import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('android_calendar_harness', ROOT / 'scripts/clients/android_e2e.py')
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class FinishedProcess:
    returncode = 0

    def poll(self):
        return 0

    def wait(self, timeout=None):
        return 0


class AndroidReportGuards(unittest.TestCase):
    def run_scenario(self, name, payload):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            logs = root / 'logs'
            logs.mkdir()
            reports = root / 'artifacts/flutter/native'
            reports.mkdir(parents=True)

            def write_report():
                if payload is not None:
                    (reports / f'integration-{name}-result.json').write_text(json.dumps(payload))

            def start(args, **kwargs):
                if 'integration_test/bulk_android_test.dart' in args:
                    write_report()
                return FinishedProcess()

            def run(*args, **kwargs):
                write_report()

            with patch.object(HARNESS, 'ROOT', root), patch.object(HARNESS, 'LOGS', logs), \
                    patch.object(HARNESS.subprocess, 'Popen', side_effect=start), patch.object(HARNESS, 'run', side_effect=run):
                getattr(HARNESS, name)('emulator-5554', 'flutter', {})

    def test_calendar_accepts_only_its_complete_native_report(self):
        self.run_scenario('calendar', {'calendar_native': [
            'timed-dark', 'all-day-dark', 'timed-light', 'all-day-light', 'native-reopen'], 'bulk_native': []})

    def test_calendar_rejects_successful_driver_without_complete_controls(self):
        for payload in [None, {}, {'calendar_native': ['timed-dark']}]:
            with self.subTest(payload=payload), self.assertRaisesRegex(RuntimeError, 'Calendar controls'):
                self.run_scenario('calendar', payload)

    def test_bulk_keeps_its_independent_completion_guard(self):
        self.run_scenario('bulk', {'bulk_native': [
            'bulk-controls-light', 'bulk-controls-dark', 'bulk-native-journal-restart'], 'calendar_native': []})

    def test_bulk_rejects_successful_driver_without_complete_controls(self):
        for payload in [None, {}, {'bulk_native': ['bulk-controls-light']}]:
            with self.subTest(payload=payload), self.assertRaisesRegex(RuntimeError, 'Group action native'):
                self.run_scenario('bulk', payload)


if __name__ == '__main__':
    unittest.main()
