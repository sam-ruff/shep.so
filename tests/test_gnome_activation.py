import json
import shutil
import sys
import tempfile
import subprocess
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from gnome_activation import (cleanup_all, desktop_entry, fixture_processes, notifier_items, require_stable,
                              runtime_processes, start_system_bus, stop_process, stop_runtime_processes)


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

    def test_runtime_sweep_matches_only_this_runs_exact_runtime_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for pid, environment in {
                21: ["DISPLAY=:0", "XDG_RUNTIME_DIR=/tmp/shep-notification-runtime-owned"],
                22: ["XDG_RUNTIME_DIR=/run/user/1000"],
                23: ["XDG_RUNTIME_DIR=/tmp/shep-notification-runtime-owned-other"],
                24: ["HOME=/tmp/shep-notification-runtime-owned"],
            }.items():
                process = root / str(pid)
                process.mkdir()
                (process / "environ").write_bytes("\0".join(environment).encode() + b"\0")
            (root / "self").mkdir()
            self.assertEqual(runtime_processes("/tmp/shep-notification-runtime-owned", root), [21])

    def test_runtime_sweep_stops_an_actual_escaped_process(self):
        with tempfile.TemporaryDirectory(prefix="shep-sweep-runtime-") as runtime:
            # A new session escapes process-group cleanup, like the daemonised input method.
            escaped = subprocess.Popen(["sleep", "60"], env={"XDG_RUNTIME_DIR": runtime, "PATH": "/usr/bin:/bin"},
                                       start_new_session=True)
            try:
                self.assertEqual(runtime_processes(runtime), [escaped.pid])
                stop_runtime_processes(runtime)
                self.assertIsNotNone(escaped.wait(timeout=5))
                self.assertEqual(runtime_processes(runtime), [])
            finally:
                if escaped.poll() is None:
                    escaped.kill()

    @unittest.skipUnless(shutil.which("dbus-daemon") and shutil.which("busctl"), "dbus-daemon and busctl are required")
    def test_owned_system_bus_has_no_services_and_marks_the_lock_notice_shown(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "run").mkdir()
            desktop = Mock(directory=root, env={"XDG_RUNTIME_DIR": str(root / "run"),
                                                "XDG_DATA_HOME": str(root / "data")})
            bus = start_system_bus(desktop)
            try:
                address = desktop.env["DBUS_SYSTEM_BUS_ADDRESS"]
                self.assertEqual(address, f"unix:path={root / 'run' / 'system-bus'}")
                self.assertIn("<type>system</type>", (root / "system-bus.conf").read_text())
                names = json.loads(subprocess.check_output(
                    ["busctl", f"--address={address}", "--json=short", "call", "org.freedesktop.DBus",
                     "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListActivatableNames"]))
                self.assertEqual(names["data"], [["org.freedesktop.DBus"]])
                self.assertTrue((root / "data/gnome-shell/lock-warning-shown").exists())
            finally:
                stop_process(bus)
            self.assertIsNotNone(bus.poll())
