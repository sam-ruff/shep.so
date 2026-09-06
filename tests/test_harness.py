import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("harness", ROOT / "scripts/mcp_harness.py")
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


class HarnessTests(unittest.TestCase):
    def test_display_readiness_retries_only_before_launch_and_reports_failure(self):
        desktop = harness.Desktop()
        desktop.directory = Path("/tmp/isolated")
        desktop.xvfb = Mock()
        desktop.xvfb.poll.return_value = None
        desktop.command = Mock(side_effect=[subprocess.CalledProcessError(1, "xdotool"), "1440 920"])
        with patch.object(harness.time, "sleep"):
            desktop.wait_display()
        self.assertEqual(desktop.command.call_count, 2)
        desktop.xvfb.poll.return_value = 1
        with self.assertRaisesRegex(RuntimeError, "xvfb.log"):
            desktop.wait_display()
        desktop.xvfb = None

    def test_google_permission_fixtures_reject_unknown_values_before_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            with self.assertRaisesRegex(ValueError, "Unknown Google permissions"):
                desktop.start(google_permissions="unsupported")
            launch.assert_not_called()

    def test_native_file_picker_uses_real_input_and_restricts_files_to_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            fixture = desktop.directory / "a file.txt"
            fixture.write_text("Fixture")
            windows = iter(["123", "", "123", ""])
            desktop.window = "main"
            desktop.command = Mock(side_effect=lambda *args: next(windows) if args[1] == "search" else "")
            with patch.object(harness.time, "sleep"), patch.object(harness.subprocess, "Popen") as clipboard:
                self.assertEqual(desktop.choose_file(str(fixture)), {"selected": str(fixture)})
                commands = [call.args for call in desktop.command.call_args_list]
                self.assertIn(("xdotool", "windowfocus", "123"), commands)
                self.assertIn(("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+v"), commands)
                clipboard.return_value.stdin.write.assert_called_once_with(str(fixture).encode())
                desktop.choose_file()
                self.assertIn(("xdotool", "key", "--clearmodifiers", "--delay", "1", "Escape"), [call.args for call in desktop.command.call_args_list])
                self.assertEqual(desktop.command.call_args.args, ("xdotool", "windowfocus", "main"))
            desktop.command.reset_mock()
            with self.assertRaises(ValueError):
                desktop.choose_file(str(ROOT / "Cargo.toml"))
            desktop.command.assert_not_called()

    def test_mcp_initialize_discovery_and_unknown_tool(self):
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "does-not-exist"}},
        ]
        result = subprocess.run([sys.executable, str(ROOT / "scripts/mcp_harness.py")],
                                input="\n".join(json.dumps(m) for m in messages) + "\n",
                                text=True, capture_output=True, timeout=10, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(responses), 3)
        self.assertEqual(responses[0]["result"]["protocolVersion"], "2025-11-25")
        names = {t["name"] for t in responses[1]["result"]["tools"]}
        self.assertIn("desktop.batch", names)
        self.assertTrue(responses[2]["result"]["isError"])

    def test_assertions_observe_state_and_fail_on_wrong_value(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            (desktop.directory / "state.json").write_text(json.dumps({"dialog": "Move", "cache": {"entries": 3}}))
            self.assertEqual(desktop.assertion({"path": "cache.entries", "op": "gte", "value": 2}), 3)
            (desktop.directory / "state.json").write_text(json.dumps({"selection": None}))
            with self.assertRaises(AssertionError):
                desktop.assertion({"path": "selection", "op": "contains", "value": "text"})
            with self.assertRaises(AssertionError):
                desktop.assertion({"path": "dialog", "value": "Compose"})
            with self.assertRaises(ValueError):
                desktop.screenshot("../../outside")

    def test_drag_releases_mouse_after_a_failed_move_and_batch_stops(self):
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        desktop.window = "123"
        desktop.screenshot = Mock()
        commands = []
        def command(*args):
            commands.append(args)
            if args[1] == "mousemove" and len(commands) > 2:
                raise RuntimeError("test input failure")
        desktop.command = command
        with patch.object(harness.time, "sleep"), self.assertRaises(RuntimeError):
            desktop.batch([{"type": "drag", "x": 1, "y": 2, "end_x": 4, "end_y": 2},
                           {"type": "click", "x": 10, "y": 10}])
        self.assertEqual(commands[-1], ("xdotool", "mouseup", "1"))
        self.assertEqual(len(commands), 4)
        desktop.app = None

    def test_wait_for_retries_until_async_list_entries_exist(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            desktop.app = Mock()
            desktop.app.poll.return_value = None
            desktop.state = Mock(side_effect=[{}, {"rows": []}, {"rows": [{"folder": "Projects"}]}, {}])
            with patch.object(harness.time, "sleep"):
                result = desktop.batch([{"type": "wait_for", "path": "rows.0.folder", "value": "Projects"}])
            self.assertEqual(result["actions"][0]["result"], "Projects")
            desktop.state = Mock(return_value={"rows": []})
            with self.assertRaises(AssertionError):
                desktop.assertion({"path": "rows.0.folder", "value": "Projects"})
            desktop.app = None

    def test_batch_limits_reject_excessive_waits_and_actions(self):
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        with self.assertRaises(ValueError):
            desktop.batch([{"type": "wait", "ms": 2000}] * 6)
        with self.assertRaises(ValueError):
            desktop.batch([{"type": "state"}] * 101)
        desktop.app = None

    def test_invalid_requests_stay_valid_json_rpc(self):
        result = subprocess.run([sys.executable, str(ROOT / "scripts/mcp_harness.py")],
                                input='bad json\n{"jsonrpc":"2.0","id":2,"method":"unknown"}\n',
                                text=True, capture_output=True, timeout=10, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(responses[0]["error"]["code"], -32600)
        self.assertEqual(responses[1]["error"]["code"], -32601)

    def test_modified_click_releases_keys_after_failed_native_input(self):
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        desktop.window = "fixture-window"
        desktop.screenshot = Mock(return_value={})
        def command(*args):
            if args[1] == "click":
                raise RuntimeError("fixture click failure")
            return ""
        desktop.command = Mock(side_effect=command)
        with patch.object(harness.time, "sleep"):
            with self.assertRaisesRegex(Exception, "fixture click failure"):
                desktop.batch([{"type": "click", "x": 20, "y": 30, "modifiers": ["ctrl", "shift"]}])
        calls = [call.args for call in desktop.command.call_args_list]
        self.assertEqual(calls[-2:], [("xdotool", "keyup", "shift"), ("xdotool", "keyup", "ctrl")])

    def test_resize_rejects_invalid_dimensions_before_native_input(self):
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        desktop.screenshot = Mock()
        desktop.command = Mock()
        for size in [(899, 640), (900, 639), (2561, 800)]:
            with self.assertRaises(RuntimeError):
                desktop.batch([{"type": "resize", "width": size[0], "height": size[1]}])
        desktop.command.assert_not_called()
        desktop.app = None


if __name__ == "__main__":
    unittest.main()
