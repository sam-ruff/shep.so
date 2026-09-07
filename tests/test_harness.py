import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("harness", ROOT / "scripts/mcp_harness.py")
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


class HarnessTests(unittest.TestCase):
    def test_reading_mail_fixture_is_explicit_and_validated_before_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in ("true", 1, None):
                with self.assertRaisesRegex(ValueError, "Reading mail fixture"):
                    desktop.start(reading_mail=value)
            launch.assert_not_called()
        tool = next(tool for tool in harness.TOOLS if tool["name"] == "desktop.start")
        self.assertEqual(tool["inputSchema"]["properties"]["reading_mail"], {"type":"boolean", "default":False})

    def test_move_recovery_fixture_rejects_unrecognized_modes_before_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (0, 1, None, "live", "host", "unknown", []):
                with self.assertRaisesRegex(ValueError, "move recovery"):
                    desktop.start(move_recovery=value)
            launch.assert_not_called()

    def test_notification_delivery_fixture_is_explicit_and_validated_before_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (True, 42, "native", "host", "unknown"):
                with self.assertRaisesRegex(ValueError, "notification delivery fixture"):
                    desktop.start(notification_delivery=value)
            launch.assert_not_called()
        start = next(tool for tool in harness.TOOLS if tool["name"] == "desktop.start")
        self.assertEqual(start["inputSchema"]["properties"]["notification_delivery"]["enum"], ["slow", "fail-once"])

    def test_pixel_measurement_validates_current_resized_window_before_input(self):
        from scripts import native_pixels
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        desktop.window, desktop.launch_size = "123", (1440, 920)
        desktop.env["DISPLAY"] = ":owned"
        desktop.command, desktop.screenshot = Mock(), Mock()
        probe = Mock()
        probe.dimensions.return_value = (900, 640)
        points = [[10, 20, 0, 0, 0]]*8
        with patch.dict(sys.modules, {"native_pixels": native_pixels}), patch.object(native_pixels, "Window", return_value=probe):
            with self.assertRaisesRegex(RuntimeError, "inside the owned window"):
                desktop.batch([{"type": "measure_pixels", "x": 1000, "y": 400, "points": points}])
            desktop.command.assert_not_called()
            probe.click_until_visible.assert_not_called()
            probe.close.assert_called_once()
        desktop.app = None

    def test_clipboard_paste_requires_owned_display_and_preserves_unicode_whitespace(self):
        desktop = harness.Desktop()
        desktop.command = Mock()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (None, 1, "x"*10001):
                with self.assertRaises(ValueError):
                    desktop.paste_text(value)
            with self.assertRaisesRegex(RuntimeError, "owned fixture"):
                desktop.paste_text("日本語")
            launch.assert_not_called()
        desktop.app, desktop.xvfb, desktop.clipboard = Mock(), Mock(), Mock()
        desktop.app.poll.return_value = desktop.xvfb.poll.return_value = desktop.clipboard.poll.return_value = None
        previous = desktop.clipboard
        clipboard = Mock()
        clipboard.poll.return_value = None
        desktop.env["DISPLAY"] = ":owned"
        text = " 日本語\n"
        try:
            with patch.object(harness.subprocess, "Popen", return_value=clipboard) as launch, patch.object(harness.subprocess, "run", return_value=Mock(stdout=text.encode())) as read:
                desktop.paste_text(text)
                previous.terminate.assert_called_once()
                previous.wait.assert_called_once()
                self.assertEqual(launch.call_args.kwargs["env"]["DISPLAY"], ":owned")
                self.assertEqual(read.call_args.kwargs["env"]["DISPLAY"], ":owned")
                clipboard.stdin.write.assert_called_once_with(text.encode())
                clipboard.stdin.close.assert_called_once()
                desktop.command.assert_called_once_with("xdotool","key","--clearmodifiers","--delay","1","ctrl+v")
            desktop.stop()
            clipboard.terminate.assert_called_once()
        finally:
            desktop.app = desktop.xvfb = desktop.clipboard = None
        batch = next(tool for tool in harness.TOOLS if tool["name"]=="desktop.batch")
        self.assertIn("paste", batch["inputSchema"]["properties"]["actions"]["items"]["properties"]["type"]["enum"])

    def test_clipboard_failure_never_pastes_stale_content_and_remains_owned_for_cleanup(self):
        desktop = harness.Desktop()
        desktop.app, desktop.xvfb = Mock(), Mock()
        desktop.app.poll.return_value = desktop.xvfb.poll.return_value = None
        desktop.command = Mock()
        clipboard = Mock()
        clipboard.poll.return_value = None
        try:
            with patch.object(harness.subprocess, "Popen", return_value=clipboard), patch.object(harness.subprocess, "run", return_value=Mock(stdout=b"stale")), patch.object(harness.time, "monotonic", side_effect=[0,4]):
                with self.assertRaisesRegex(RuntimeError, "did not accept"):
                    desktop.paste_text("日本語")
            desktop.command.assert_not_called()
            desktop.stop()
            clipboard.terminate.assert_called_once()
        finally:
            desktop.app = desktop.xvfb = desktop.clipboard = None

    def test_held_mouse_requires_an_owned_app_and_cleanup_releases_only_its_display(self):
        desktop = harness.Desktop()
        desktop.command = Mock()
        with self.assertRaisesRegex(RuntimeError, "owned fixture"):
            desktop.mouse_button(True)
        desktop.command.assert_not_called()
        desktop.app, desktop.xvfb = Mock(), Mock()
        desktop.app.poll.return_value = desktop.xvfb.poll.return_value = None
        desktop.mouse_button(True)
        self.assertTrue(desktop.mouse_held)
        with self.assertRaisesRegex(RuntimeError, "already held"):
            desktop.mouse_button(True)
        desktop.mouse_button(False)
        self.assertFalse(desktop.mouse_held)
        with self.assertRaisesRegex(RuntimeError, "not held"):
            desktop.mouse_button(False)
        desktop.mouse_button(True)
        desktop.stop()
        self.assertFalse(desktop.mouse_held)
        self.assertEqual(desktop.command.call_args_list[-1].args, ("xdotool", "mouseup", "1"))

    def test_persistent_fixture_and_crash_mode_require_booleans(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (0, 1, "1", None):
                with self.assertRaisesRegex(ValueError, "Persistent fixture"):
                    desktop.start(persistent=value)
                with self.assertRaisesRegex(ValueError, "Crash restart"):
                    desktop.close_app(crash=value)
            with self.assertRaisesRegex(RuntimeError, "owned persistent fixture"):
                desktop.restart()
            launch.assert_not_called()

    def test_graceful_close_timeout_keeps_the_owned_process_and_never_relaunches(self):
        desktop = harness.Desktop()
        desktop.persistent, desktop.launch_args = True, ["owned-fixture"]
        desktop.app, desktop.xvfb = Mock(), Mock()
        desktop.app.poll.return_value = desktop.xvfb.poll.return_value = None
        desktop.app.wait.side_effect = subprocess.TimeoutExpired("fixture", 10)
        desktop.env["DISPLAY"], desktop.window = ":owned", "123"
        desktop.launch_app = Mock()
        try:
            with patch.object(harness, "request_window_close") as close:
                with self.assertRaisesRegex(RuntimeError, "No replacement was launched"):
                    desktop.restart()
                close.assert_called_once_with(":owned", "123")
            desktop.launch_app.assert_not_called()
            desktop.app.kill.assert_not_called()
            desktop.app.terminate.assert_not_called()
        finally:
            desktop.app = desktop.xvfb = None

    def test_crash_restart_retains_cache_archives_old_observations_and_targets_only_owned_app(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            database = desktop.directory / "fixture.sqlite"
            database.write_bytes(b"fixture cache")
            (desktop.directory / "state.json").write_text('{"ready":true}')
            desktop.persistent, desktop.launch_args = True, ["owned-fixture"]
            desktop.app, desktop.xvfb = Mock(pid=123,returncode=-9), Mock()
            desktop.app.poll.return_value = desktop.xvfb.poll.return_value = None
            def launch():
                self.assertFalse((desktop.directory / "state.json").exists())
                self.assertTrue((desktop.directory / "state-before-restart-1.json").exists())
                self.assertEqual(database.read_bytes(),b"fixture cache")
                return {"pid":456}
            desktop.launch_app = Mock(side_effect=launch)
            try:
                with patch.object(harness, "request_window_close") as close:
                    result = desktop.restart(crash=True)
                    close.assert_not_called()
                self.assertEqual(result["previous_process"]["pid"],123)
                self.assertEqual(result["pid"],456)
                desktop.app.kill.assert_called_once()
                desktop.xvfb.kill.assert_not_called()
                desktop.xvfb.terminate.assert_not_called()
            finally:
                desktop.app = desktop.xvfb = None

    def test_nested_folder_fixture_is_validated_and_declared(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess,"Popen") as launch:
            for value in (0,1,"yes",None):
                with self.assertRaisesRegex(ValueError,"Nested folder fixture"):
                    desktop.start(nested_folders=value)
            launch.assert_not_called()
        schema=next(t for t in harness.TOOLS if t["name"]=="desktop.start")["inputSchema"]["properties"]
        self.assertEqual(schema["nested_folders"],{"type":"boolean","default":False})

    def test_idle_navigation_fixture_is_validated_and_declared(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (0, 1, "yes", None):
                with self.assertRaisesRegex(ValueError, "Idle navigation fixture"):
                    desktop.start(idle_navigation=value)
            launch.assert_not_called()
        schema = next(t for t in harness.TOOLS if t["name"] == "desktop.start")["inputSchema"]["properties"]
        self.assertEqual(schema["idle_navigation"], {"type": "boolean", "default": False})

    def test_badge_fixture_requires_a_boolean_before_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (0, 1, "1", None):
                with self.assertRaisesRegex(ValueError, "Desktop badge fixture"):
                    desktop.start(desktop_badges=value)
            launch.assert_not_called()

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux launcher protocol")
    def test_badge_observer_reads_real_signals_on_an_owned_bus(self):
        desktop = harness.Desktop()
        with tempfile.TemporaryDirectory() as directory:
            desktop.directory = Path(directory)
            try:
                desktop.start_badge_bus()
                address = desktop.env["DBUS_SESSION_BUS_ADDRESS"]
                self.assertEqual(address, f"unix:path={directory}/badge-bus")
                activatable = json.loads(desktop.command("busctl", f"--address={address}", "--json=short", "call",
                    "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListActivatableNames"))
                self.assertEqual(activatable["data"], [["org.freedesktop.DBus"]])
                self.assertEqual(desktop.env["XDG_RUNTIME_DIR"], str(Path(directory)/"runtime"))
                for count in (2, 0):
                    desktop.command("busctl", f"--address={address}", "emit", "/so/shep/Shep/Launcher",
                                    "com.canonical.Unity.LauncherEntry", "Update", "sa{sv}",
                                    "application://so.shep.Shep.desktop", "2", "count", "x", str(count),
                                    "count-visible", "b", "true" if count else "false")
                    deadline = time.monotonic() + 2
                    while time.monotonic() < deadline:
                        observed = desktop.badge_state()
                        if observed and observed["count"] == count:
                            break
                        time.sleep(0.02)
                    self.assertEqual(observed["count"], count)
                    self.assertEqual(observed["visible"], bool(count))
                bus, monitor = desktop.badge_bus, desktop.badge_monitor
            finally:
                desktop.stop()
            self.assertIsNotNone(bus.poll())
            self.assertIsNotNone(monitor.poll())

    def test_html_failure_fixture_rejects_non_boolean_values(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for value in (0, 1, "1", None):
                with self.assertRaisesRegex(ValueError, "HTML failure fixture"):
                    desktop.start(html_failure_once=value)
            launch.assert_not_called()

    def test_image_delay_is_bounded_and_invalid_values_never_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for delay in (-1, 5001, True, "500", 1.5):
                with self.assertRaisesRegex(ValueError, "Image fixture delay"):
                    desktop.start(image_delay_ms=delay)
            launch.assert_not_called()

    def test_html_delay_is_bounded_and_invalid_values_never_launch(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            for delay in (-1, 2001, True, "500", 1.5):
                with self.assertRaisesRegex(ValueError, "HTML fixture delay"):
                    desktop.start(html_delay_ms=delay)
            launch.assert_not_called()

    def test_print_fixture_is_explicit_and_schema_matches_batch_actions(self):
        desktop = harness.Desktop()
        with patch.object(harness.subprocess, "Popen") as launch:
            with self.assertRaisesRegex(ValueError, "Unknown print browser"):
                desktop.start(print_browser="personal")
            launch.assert_not_called()
        start = next(t for t in harness.TOOLS if t["name"] == "desktop.start")
        self.assertEqual(start["inputSchema"]["properties"]["print_browser"]["enum"], ["pdf", "dialog", "fail"])
        batch = next(t for t in harness.TOOLS if t["name"] == "desktop.batch")
        actions = batch["inputSchema"]["properties"]["actions"]["items"]["properties"]["type"]["enum"]
        for action in ("print_output", "cancel_print", "focus_app"):
            self.assertIn(action, actions)

    def test_print_failure_launcher_cannot_fall_back_to_a_personal_browser(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            with patch.object(harness.subprocess, "Popen") as launch:
                desktop.start_print_browser("fail")
                launch.assert_not_called()
            launcher = desktop.env["SHEP_TEST_PRINT_BROWSER"]
            self.assertEqual(Path(launcher).parent, desktop.directory)
            self.assertEqual(subprocess.run([launcher, "http://127.0.0.1:1/fixture"]).returncode, 1)
            desktop.stop()
            self.assertNotIn("SHEP_TEST_PRINT_BROWSER", desktop.env)

    def test_print_output_rejects_paths_and_invalid_counts(self):
        desktop = harness.Desktop()
        for arguments in ({"count":0}, {"pages":0}, {"name":"../private"}):
            with self.assertRaisesRegex(ValueError, "Invalid print output"):
                desktop.print_output(**arguments)

    def test_print_browser_uses_owned_x11_profile_and_waits_before_app_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            desktop.env["DISPLAY"] = ":321"
            browser = Mock(pid=12345)
            browser.poll.return_value = 0  # Cleanup must never signal a real PID, even if this test fails.
            desktop.command = Mock(return_value="456")
            with patch.object(harness.shutil, "which", return_value="/bin/true"), patch.object(harness.subprocess, "Popen", return_value=browser) as launch:
                desktop.start_print_browser("pdf")
                args = launch.call_args.args[0]
                self.assertIn("--ozone-platform=x11", args)
                self.assertIn(f"--user-data-dir={desktop.directory / 'print-profile'}", args)
                self.assertIn("--kiosk-printing", args)
                self.assertEqual(launch.call_args.kwargs["env"]["DISPLAY"], ":321")
                self.assertTrue(launch.call_args.kwargs["start_new_session"])
                desktop.command.assert_called_with("xdotool", "search", "--onlyvisible", "--pid", "12345")
            # The process is a mock; cleanup must never signal a real PID.
            desktop.browser = None
            desktop.stop()

    def test_invalid_mail_action_fault_mode_is_rejected_before_launch(self):
        desktop = harness.Desktop()
        with self.assertRaisesRegex(ValueError, "Unknown mail actions"):
            desktop.start(mail_actions="live")

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
            clipboard_reads = iter(["previous path", str(fixture), str(fixture)])
            desktop.command = Mock(side_effect=lambda *args: next(windows) if args[1] == "search" else next(clipboard_reads) if args[0] == "xclip" else "")
            with patch.object(harness.time, "sleep"), patch.object(harness.subprocess, "Popen") as clipboard:
                clipboard.return_value.poll.return_value = 0
                self.assertEqual(desktop.choose_file(str(fixture)), {"selected": str(fixture)})
                commands = [call.args for call in desktop.command.call_args_list]
                self.assertIn(("xdotool", "windowfocus", "--sync", "123"), commands)
                self.assertEqual(commands.count(("xclip", "-selection", "clipboard", "-out")), 3)
                self.assertLess(next(i for i, command in enumerate(commands) if command[0] == "xclip"), commands.index(("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+v")))
                self.assertIn(("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+v"), commands)
                clipboard.return_value.stdin.write.assert_called_once_with(str(fixture).encode())
                desktop.choose_file()
                self.assertIn(("xdotool", "key", "--clearmodifiers", "--delay", "1", "Escape"), [call.args for call in desktop.command.call_args_list])
                self.assertEqual(desktop.command.call_args.args, ("xdotool", "windowfocus", "--sync", "main"))
            desktop.command.reset_mock()
            with self.assertRaises(ValueError):
                desktop.choose_file(str(ROOT / "Cargo.toml"))
            desktop.command.assert_not_called()

    def test_picker_retries_ignored_input_and_requires_gtk_clipboard_ownership(self):
        desktop = harness.Desktop()
        fixture = Path("/isolated/fixture.txt")
        desktop.command = Mock(return_value=str(fixture))
        pending, copied = Mock(), Mock()
        pending.poll.return_value = None
        copied.poll.return_value = 0
        tick = [0.0]
        def advance(seconds):
            tick[0] += seconds
        with patch.object(harness.time, "monotonic", side_effect=lambda: tick[0]), \
             patch.object(harness.time, "sleep", side_effect=advance), \
             patch.object(harness.subprocess, "Popen", side_effect=[pending, copied]):
            desktop.enter_picker_path("picker", fixture)
        pending.terminate.assert_called_once()
        commands = [call.args for call in desktop.command.call_args_list]
        self.assertEqual(commands.count(("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+l")), 2)
        self.assertNotIn(("xdotool", "key", "--clearmodifiers", "--delay", "1", "Return"), commands)
        desktop.clipboard = None

    def test_picker_never_submits_a_path_that_gtk_did_not_accept(self):
        desktop = harness.Desktop()
        fixture = Path("/isolated/fixture.txt")
        desktop.command = Mock(return_value=str(fixture))
        owner = Mock()
        owner.poll.return_value = None
        tick = [0.0]
        def advance(seconds):
            tick[0] += seconds
        with patch.object(harness.time, "monotonic", side_effect=lambda: tick[0]), \
             patch.object(harness.time, "sleep", side_effect=advance), \
             patch.object(harness.subprocess, "Popen", return_value=owner):
            with self.assertRaisesRegex(RuntimeError, "location field"):
                desktop.enter_picker_path("picker", fixture)
        self.assertGreaterEqual(tick[0], 3)
        desktop.clipboard = None

    def test_picker_confirmation_retries_only_its_window_after_filename_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = harness.Desktop()
            desktop.directory = Path(directory)
            fixture = desktop.directory / "fixture.txt"
            fixture.write_text("Fixture")
            desktop.enter_picker_path = Mock()
            desktop.window = "main"
            windows = iter(["123"] * 9 + [""])
            desktop.command = Mock(side_effect=lambda *args: next(windows) if args[1] == "search" else "")
            tick = [0.0]
            def advance(seconds):
                tick[0] += seconds
            with patch.object(harness.time, "monotonic", side_effect=lambda: tick[0]), \
                 patch.object(harness.time, "sleep", side_effect=advance):
                desktop.choose_file(fixture)
            desktop.enter_picker_path.assert_called_once_with("123", fixture)
            confirms = [call.args for call in desktop.command.call_args_list if call.args[-1] == "Return"]
            self.assertEqual(confirms, [("xdotool", "key", "--window", "123", "--clearmodifiers", "--delay", "1", "Return")] * 2)
            self.assertEqual(desktop.command.call_args.args, ("xdotool", "windowfocus", "--sync", "main"))

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
            desktop.state = Mock(side_effect=[{"removal": None}, {"removal": {"transfers": 1}}, {}])
            with patch.object(harness.time, "sleep"):
                result = desktop.batch([{"type": "wait_for", "path": "removal.transfers", "value": 1}])
            self.assertEqual(result["actions"][0]["result"], 1)
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
