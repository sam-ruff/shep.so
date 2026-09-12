import json
import sys
import tempfile
import subprocess
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from gnome_activation import cleanup_all, desktop_entry, fixture_processes, notifier_items, require_stable, stop_process


class GnomeActivationTests(unittest.TestCase):
    def test_cleanup_continues_after_failed_receipt_and_escalates_hung_child(self):
        receipt = Mock(side_effect=OSError("fixture disk full"))
        process = Mock()
        process.poll.return_value = None
        process.wait.side_effect = [subprocess.TimeoutExpired("owned child", 3), 0]
        desktop = Mock()
        runtime = Mock()
        with self.assertRaises(ExceptionGroup) as error:
            cleanup_all([receipt, lambda: stop_process(process), desktop, runtime])
        self.assertEqual(len(error.exception.exceptions), 1)
        process.terminate.assert_called_once()
        process.kill.assert_called_once()
        self.assertEqual(process.wait.call_count, 2)
        desktop.assert_called_once()
        runtime.assert_called_once()

    def test_stable_check_rejects_a_duplicate_appearing_after_initial_success(self):
        probe = Mock(side_effect=[None, None, AssertionError("late secondary")])
        with patch("gnome_activation.time.sleep"), patch("gnome_activation.time.monotonic", side_effect=[0, 0, .1]):
            with self.assertRaisesRegex(AssertionError, "late secondary"):
                require_stable(probe)
        self.assertEqual(probe.call_count, 3)

    def test_launcher_retains_owned_fixture_arguments_with_spaces(self):
        entry = desktop_entry(["/tmp/owned demo/shep", "--demo", "--persist-demo",
                               "--test-state", "/tmp/owned demo/state.json"])
        self.assertIn('Exec="/tmp/owned demo/shep" "--demo" "--persist-demo"', entry)
        self.assertIn('"--test-state" "/tmp/owned demo/state.json"', entry)
        self.assertIn("StartupWMClass=so.shep.Shep", entry)
        self.assertIn("StartupNotify=false", entry)

    def test_full_watcher_list_preserves_duplicate_process_registrations(self):
        items = [":1.8/StatusNotifierItem", ":1.9/StatusNotifierItem"]
        self.assertEqual(notifier_items(json.dumps({"type": "as", "data": items})), items)
        self.assertEqual(notifier_items('{"type":"as","data":[]}'), [])

    def test_wrong_watcher_payload_cannot_pass_as_one_registration(self):
        for payload in [{"type": "s", "data": ":1.8"}, {"type": "as", "data": [None]}]:
            with self.assertRaises(ValueError):
                notifier_items(json.dumps(payload))

    def test_cleanup_cannot_target_personal_or_other_fixture_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for pid, arguments in {
                11: ["/owned/shep", "--demo", "--test-state", "/owned/state.json"],
                12: ["/owned/shep"],
                13: ["/owned/shep", "--demo", "--test-state", "/other/state.json"],
                14: ["/other/shep", "--demo", "--test-state", "/owned/state.json"],
                15: ["/owned/shep", "--test-state", "/owned/state.json"],
            }.items():
                process = root / str(pid)
                process.mkdir()
                (process / "cmdline").write_bytes("\0".join(arguments).encode() + b"\0")
            self.assertEqual(fixture_processes("/owned/shep", "/owned/state.json", root), [11])
