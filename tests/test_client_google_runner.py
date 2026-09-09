"""An interrupted native driver must not turn a partial run into a pass."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'scripts/clients/android_e2e.py'
SPEC = importlib.util.spec_from_file_location('shep_android_google_runner', SOURCE)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class GoogleRunnerTests(unittest.TestCase):
    def test_zero_exit_with_missing_or_partial_report_is_not_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / 'artifacts/flutter/native/integration-google-result.json'
            report.parent.mkdir(parents=True)
            complete = ['consent-cancel-retry-cleanup', 'pending-browse-changed-choices']
            # A previous successful run cannot satisfy a new interrupted run.
            report.write_text(json.dumps({'google_controls': complete}))
            with patch.object(runner, 'ROOT', root), patch.object(runner, 'run'):
                with self.assertRaisesRegex(RuntimeError, 'did not report completion'):
                    runner.google('emulator-5554', 'flutter', {})
                self.assertFalse(report.exists())
            for stages, succeeds in [(complete[:1], False), (complete, True)]:
                def finish(*args, **kwargs):
                    report.write_text(json.dumps({'google_controls': stages}))
                with patch.object(runner, 'ROOT', root), patch.object(runner, 'run', side_effect=finish):
                    if succeeds:
                        runner.google('emulator-5554', 'flutter', {})
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'did not report completion'):
                            runner.google('emulator-5554', 'flutter', {})


if __name__ == '__main__':
    unittest.main()
