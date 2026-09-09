#!/usr/bin/env python3
"""Shep native desktop MCP server. Standard-library-only, newline-delimited JSON-RPC.

Real X11 mouse/keyboard input; no application actions are invoked through test hooks.
The inspection file is a read-only oracle, enabled only in a test-support build.
"""
import atexit
import base64
import html
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "artifacts" / "e2e"


class Desktop:
    def __init__(self):
        self.app = None
        self.clipboard = None
        self.xvfb = None
        self.window = None
        self.env = os.environ.copy()
        self.directory = None
        self.log = None
        self.xvfb_log = None
        self.browser = None
        self.browser_log = None
        self.badge_bus = None
        self.badge_monitor = None
        self.badge_log = None
        self.badge_events = None
        atexit.register(self.stop)

    def stop(self):
        if self.browser and self.browser.poll() is None:
            os.killpg(self.browser.pid, signal.SIGTERM)
            try:
                self.browser.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(self.browser.pid, signal.SIGKILL)
                self.browser.wait(timeout=3)
        self.browser = None
        if self.browser_log:
            self.browser_log.close()
            self.browser_log = None
        self.env.pop("SHEP_TEST_PRINT_BROWSER", None)
        for process in (self.clipboard, self.app, self.badge_monitor, self.badge_bus, self.xvfb):
            if process and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
        self.app = self.xvfb = None
        self.clipboard = None
        self.badge_bus = self.badge_monitor = None
        for stream in (self.badge_log, self.badge_events):
            if stream:
                stream.close()
        self.badge_log = self.badge_events = None
        if self.log:
            self.log.close()
            self.log = None
        if self.xvfb_log:
            self.xvfb_log.close()
            self.xvfb_log = None
        return {"stopped": True}

    def command(self, *args):
        return subprocess.run(args, env=self.env, capture_output=True, text=True,
                              check=True, timeout=10).stdout.strip()

    def start(self, width=1440, height=920, empty_calendars=False, conversation_mail=False, readonly_calendars=False, pending_transfer=False, outgoing_mail=False, google_permissions=None, long_folders=False, mail_actions=None, background_sync=False, sync_failure_once=False, search_mail=False, html_mail=False, discard_failure_once=False, undo_failure_once=False, print_browser=None, html_delay_ms=0, image_delay_ms=0, html_failure_once=False, desktop_badges=False, profile_discovery=False, profile_pages=False, profile_sync=False):
        self.stop()
        if type(profile_sync) is not bool:
            raise ValueError("profile_sync must be a boolean")
        if profile_sync:
            profile_discovery = True
        if type(profile_pages) is not bool:
            raise ValueError("Profile page fixture must be a boolean.")
        if type(profile_discovery) is not bool:
            raise ValueError("Profile fixture must be a boolean.")
        if profile_pages:
            profile_discovery = True
        if type(desktop_badges) is not bool:
            raise ValueError("Desktop badge fixture must be a boolean.")
        if type(html_delay_ms) is not int or not 0 <= html_delay_ms <= 2000:
            raise ValueError("HTML fixture delay must be 0–2000 milliseconds.")
        if type(html_failure_once) is not bool:
            raise ValueError("HTML failure fixture must be a boolean.")
        if type(image_delay_ms) is not int or not 0 <= image_delay_ms <= 5000:
            raise ValueError("Image fixture delay must be 0–5000 milliseconds.")
        if print_browser not in (None, "pdf", "dialog", "fail"):
            raise ValueError("Unknown print browser fixture.")
        if mail_actions not in (None, "slow", "fail"):
            raise ValueError("Unknown mail actions fixture.")
        if google_permissions not in (None, "drive", "calendar", "read-only"):
            raise ValueError("Unknown Google permissions fixture.")
        if not 900 <= width <= 2560 or not 640 <= height <= 1600:
            raise ValueError("Test window must be 900–2560 × 640–1600.")
        for tool in ("Xvfb", "xdotool", "import", "zenity", "xclip"):
            if not shutil.which(tool):
                raise RuntimeError(f"Install {tool}; this native harness currently supports Linux/X11.")
        binary = ROOT / "target" / "test-ui" / "shep"
        if not binary.exists():
            raise RuntimeError("Run cargo build --profile test-ui --features test-support before starting the harness.")
        self.directory = ARTIFACTS / uuid.uuid4().hex[:12]
        self.directory.mkdir(parents=True)
        self.xvfb_log = (self.directory / "xvfb.log").open("w")
        read_fd, write_fd = os.pipe()
        self.xvfb = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", f"{width}x{height}x24", "-nolisten", "tcp", "-noreset"],
            pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=self.xvfb_log)
        os.close(write_fd)
        with os.fdopen(read_fd) as display:
            number = display.readline().strip()
        if not number.isdigit():
            raise RuntimeError("Xvfb did not allocate a display.")
        self.env.update(DISPLAY=f":{number}", WINIT_UNIX_BACKEND="x11")
        self.wait_display()
        # Keep native file selection on our display, never the user's portal.
        # rfd falls back to the real GTK/Zenity picker when no portal is available.
        self.env.update(DBUS_SESSION_BUS_ADDRESS=f"unix:path={self.directory}/no-session-bus",
                        GDK_BACKEND="x11", GTK_USE_PORTAL="0", GSETTINGS_BACKEND="memory",
                        XDG_DATA_HOME=str(self.directory / "data"),
                        XDG_CONFIG_HOME=str(self.directory / "config"),
                        XDG_CACHE_HOME=str(self.directory / "cache"))
        self.env["SHEP_TEST_HTML_DELAY_MS"] = str(html_delay_ms)
        self.env["SHEP_TEST_IMAGE_DELAY_MS"] = str(image_delay_ms)
        self.env["SHEP_TEST_HTML_FAILURE_ONCE"] = "1" if html_failure_once else "0"
        self.env.pop("WAYLAND_DISPLAY", None)
        if desktop_badges:
            self.start_badge_bus()
        if print_browser:
            self.start_print_browser(print_browser)
        self.log = (self.directory / "app.log").open("w")
        self.app = subprocess.Popen(
            [str(binary), *(["--profile-sync"] if profile_sync else []), *(["--profile-discovery"] if profile_discovery else []), *(["--profile-pages"] if profile_pages else []), "--demo", "--test-state", str(self.directory / "state.json"), *(["--empty-calendars"] if empty_calendars else []), *(["--conversation-mail"] if conversation_mail else []), *(["--readonly-calendars"] if readonly_calendars else []), *(["--pending-transfer"] if pending_transfer else []), *(["--outgoing-mail"] if outgoing_mail else []), *(["--long-folders"] if long_folders else []), *(["--discard-failure-once"] if discard_failure_once else []), *(["--undo-failure-once"] if undo_failure_once else []), *(["--search-mail"] if search_mail else []), *(["--html-mail"] if html_mail else []), *(["--background-sync"] if background_sync else []), *(["--sync-failure-once"] if sync_failure_once else []), *(["--mail-actions=" + mail_actions] if mail_actions in ("slow", "fail") else []), *(["--google-permissions=" + google_permissions] if google_permissions else [])],
            cwd=ROOT, env=self.env, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.app.poll() is not None:
                raise RuntimeError(f"Shep exited. See {self.directory / 'app.log'}")
            try:
                windows = self.command("xdotool", "search", "--name", "Shep.*Mail")
                if windows:
                    self.window = windows.splitlines()[-1]
                    self.command("xdotool", "windowsize", self.window, str(width), str(height))
                    self.command("xdotool", "windowmove", self.window, "0", "0")
                    self.command("xdotool", "windowfocus", self.window)
                    state = self.state()
                    if state.get("ready") and state.get("total", 0) > 0:
                        return {"window": self.window, "size": [width, height], "state": state,
                                "artifacts": str(self.directory)}
            except (subprocess.SubprocessError, FileNotFoundError, json.JSONDecodeError):
                pass
            time.sleep(0.05)
        raise RuntimeError("Shep was not ready within 20 seconds; check the app log and test-support feature.")

    def start_badge_bus(self):
        for tool in ("dbus-daemon", "busctl"):
            if not shutil.which(tool):
                raise RuntimeError(f"Install {tool} for isolated desktop badge tests.")
        address = f"unix:path={self.directory}/badge-bus"
        self.env["DBUS_SESSION_BUS_ADDRESS"] = address
        runtime = self.directory / "runtime"
        runtime.mkdir(mode=0o700, exist_ok=True)
        self.env["XDG_RUNTIME_DIR"] = str(runtime)
        # A normal --session config loads the host's service directories and can
        # activate portals/keyrings. This fixture bus has no activatable services.
        config = self.directory / "badge-bus.conf"
        config.write_text(f'''<busconfig>
  <type>session</type><listen>{html.escape(address)}</listen><auth>EXTERNAL</auth>
  <policy user="{os.getuid()}">
    <allow own="*"/><allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
  </policy>
</busconfig>''')
        self.badge_log = (self.directory / "badge-bus.log").open("w")
        self.badge_events = (self.directory / "badge-events.jsonl").open("w")
        self.badge_bus = subprocess.Popen(
            ["dbus-daemon", f"--config-file={config}", "--nofork", "--nopidfile"],
            env=self.env, stdout=subprocess.DEVNULL, stderr=self.badge_log)
        deadline = time.monotonic() + 5
        while not (self.directory / "badge-bus").exists():
            if self.badge_bus.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Isolated badge bus did not start.")
            time.sleep(0.02)
        self.badge_monitor = subprocess.Popen(
            ["busctl", f"--address={address}", "--json=short",
             "--match=type='signal',interface='com.canonical.Unity.LauncherEntry'", "monitor"],
            env=self.env, stdout=self.badge_events, stderr=self.badge_log)
        while "became a monitor" not in (self.directory / "badge-bus.log").read_text():
            if self.badge_monitor.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Isolated badge observer did not start: " + (self.directory / "badge-bus.log").read_text())
            time.sleep(0.02)

    def badge_state(self):
        if self.badge_monitor is None:
            return None
        latest = None
        history = []
        for line in (self.directory / "badge-events.jsonl").read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue  # The monitor may still be writing its final line.
            if event.get("member") != "Update":
                continue
            payload = event.get("payload", {})
            if payload.get("type") != "sa{sv}":
                continue
            uri, properties = payload["data"]
            if uri != "application://so.shep.Shep.desktop":
                continue
            latest = {"count": properties["count"]["data"],
                      "visible": properties["count-visible"]["data"],
                      "sender": event["sender"], "uri": uri}
            history.append(latest["count"])
        if latest:
            latest["history"] = history[-128:]
        return latest

    def start_print_browser(self, mode):
        """A fresh browser profile on the owned display; never the personal browser."""
        launcher = self.directory / "print-browser.py"
        if mode == "fail":
            launcher.write_text(f"#!{sys.executable}\nraise SystemExit(1)\n")
        else:
            chrome = next((path for path in (shutil.which("google-chrome"), shutil.which("chromium"), "/opt/google/chrome/chrome") if path and Path(path).is_file()), None)
            if not chrome or not all(shutil.which(tool) for tool in ("pdftotext", "pdfinfo", "pdftoppm", "convert")):
                raise RuntimeError("Print E2E needs Chrome/Chromium, Poppler tools and ImageMagick.")
            profile = self.directory / "print-profile"
            (profile / "Default").mkdir(parents=True)
            output = self.directory / "printed"
            output.mkdir()
            settings = {"version": 2, "recentDestinations": [{"id": "Save as PDF", "origin": "local", "account": ""}],
                        "selectedDestinationId": "Save as PDF", "isHeaderFooterEnabled": False, "isCssBackgroundEnabled": True}
            (profile / "Default" / "Preferences").write_text(json.dumps({
                "printing": {"print_preview_sticky_settings": {"appState": json.dumps(settings)}},
                "savefile": {"default_directory": str(output)}, "download": {"default_directory": str(output)}}))
            common = [chrome, f"--user-data-dir={profile}", "--ozone-platform=x11", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync"]
            flags = ["--kiosk-printing"] if mode == "pdf" else []
            self.browser_log = (self.directory / "browser.log").open("w")
            self.browser = subprocess.Popen([*common, *flags, "about:blank"], env=self.env, stdout=self.browser_log, stderr=self.browser_log, start_new_session=True)
            deadline = time.monotonic() + 10
            while True:
                try:
                    self.command("xdotool", "search", "--onlyvisible", "--pid", str(self.browser.pid))
                    break
                except subprocess.SubprocessError:
                    if self.browser.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError(f"The isolated X11 print browser did not open. See {self.directory / 'browser.log'}")
                    time.sleep(.05)
            launcher.write_text(f"#!{sys.executable}\nimport subprocess, sys\nwith open({str(self.directory / 'browser.log')!r}, 'a') as log:\n    result = subprocess.run({common!r} + ['--new-window', sys.argv[1]], stdout=log, stderr=log, timeout=10)\nraise SystemExit(result.returncode)\n")
        launcher.chmod(0o700)
        self.env["SHEP_TEST_PRINT_BROWSER"] = str(launcher)

    def print_output(self, count=1, text="", pages=1, name="printed-message"):
        """Read actual browser-produced PDF output and capture its first page."""
        if not 1 <= int(count) <= 20 or not 1 <= int(pages) <= 1000 or not re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", name):
            raise ValueError("Invalid print output assertion.")
        deadline = time.monotonic() + 5
        while True:
            files = sorted((self.directory / "printed").glob("*.pdf"), key=lambda p: p.stat().st_mtime_ns)
            try:
                if len(files) < count:
                    raise AssertionError(f"Expected {count} PDF(s), found {len(files)}")
                path = files[count - 1]
                info = self.command("pdfinfo", str(path))
                actual_pages = int(re.search(r"^Pages:\s+(\d+)", info, re.M)[1])
                content = self.command("pdftotext", "-layout", str(path), "-")
                if actual_pages < pages or text not in content:
                    raise AssertionError(f"PDF must contain {text!r} and at least {pages} pages; got {actual_pages} pages")
                break
            except (AssertionError, subprocess.SubprocessError):
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.05)
        raster = self.directory / name
        self.command("pdftoppm", "-f", "1", "-singlefile", "-scale-to", "1200", "-png", str(path), str(raster))
        self.command("convert", str(raster.with_suffix(".png")), "-quality", "90", str(raster.with_suffix(".webp")))
        raster.with_suffix(".png").unlink()
        self.command("xdotool", "windowraise", self.window)
        self.command("xdotool", "windowfocus", self.window)
        return {"pdf": str(path), "pages": actual_pages, "text": content, "screenshot": str(raster.with_suffix(".webp"))}

    def wait_display(self):
        deadline = time.monotonic() + 5
        while self.xvfb.poll() is None:
            try:
                self.command("xdotool", "getdisplaygeometry")
                return
            except subprocess.SubprocessError:
                if time.monotonic() >= deadline:
                    break
                time.sleep(.05)
        raise RuntimeError(f"Isolated X display is unavailable. See {self.directory / 'xvfb.log'}")

    def state(self):
        if not self.directory:
            raise RuntimeError("Call desktop.start first.")
        state = json.loads((self.directory / "state.json").read_text())
        state["desktop_badge"] = self.badge_state()
        return state

    def screenshot(self, name="screenshot"):
        if not re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", name):
            raise ValueError("Screenshot name must use letters, numbers, hyphens or underscores.")
        # State is emitted before presentation; allow the compositor one short settling interval.
        time.sleep(0.15)
        path = self.directory / f"{name}.webp"
        self.command("import", "-window", self.window, "-quality", "90", str(path))
        return path

    def choose_file(self, path=None):
        """Select an isolated fixture through the actual native picker, or cancel it."""
        if path is not None:
            path = Path(path).resolve(strict=True)
            if not path.is_relative_to(self.directory.resolve()) or not path.is_file():
                raise ValueError("Choose a fixture file inside this run's artifact directory.")
        deadline = time.monotonic() + 5
        while True:
            try:
                windows = self.command("xdotool", "search", "--onlyvisible", "--class", "zenity")
                if windows:
                    window = windows.splitlines()[-1]
                    break
            except subprocess.CalledProcessError:
                pass
            if time.monotonic() >= deadline:
                raise RuntimeError("The native file picker did not open on the isolated display.")
            time.sleep(.05)
        self.command("xdotool", "windowfocus", "--sync", window)
        time.sleep(.15)
        if path is None:
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "Escape")
        else:
            self.enter_picker_path(window, path)
            self.command("import", "-window", "root", "-quality", "90", str(self.directory / "native-file-picker.webp"))
        deadline = time.monotonic() + 3
        confirm_after = time.monotonic()
        while True:
            try:
                visible = self.command("xdotool", "search", "--onlyvisible", "--class", "zenity").splitlines()
            except subprocess.CalledProcessError:
                visible = []
            if window not in visible:
                break
            if path is not None and time.monotonic() >= confirm_after:
                # GTK validates the entered file asynchronously. Return may
                # initially complete its path without accepting the dialog.
                # Target only this picker so a close cannot send a key to Shep.
                try:
                    self.command("xdotool", "key", "--window", window, "--clearmodifiers", "--delay", "1", "Return")
                except subprocess.CalledProcessError:
                    pass  # It may have closed after the visibility check.
                confirm_after = time.monotonic() + .3
            if time.monotonic() >= deadline:
                self.command("import", "-window", "root", "-quality", "90", str(self.directory / "native-file-picker-failed.webp"))
                raise RuntimeError("The native file picker did not accept the selected file.")
            time.sleep(.05)
        # Xvfb has no window manager to return focus after closing a native dialog.
        self.command("xdotool", "windowfocus", "--sync", self.window)
        return {"selected": str(path) if path else None}

    def enter_picker_path(self, window, path):
        # A mapped GTK picker can still ignore its first location shortcut.
        # Verify text copied back by GTK before Return; the old clipboard owner
        # must exit, otherwise reading our own pasted value proves nothing.
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            self.command("xdotool", "windowfocus", "--sync", window)
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+l")
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+a")
            if self.clipboard and self.clipboard.poll() is None:
                self.clipboard.terminate()
                self.clipboard.wait(timeout=3)
            self.clipboard = subprocess.Popen(["xclip", "-selection", "clipboard", "-quiet"],
                env=self.env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=self.log)
            self.clipboard.stdin.write(str(path).encode())
            self.clipboard.stdin.close()
            while True:
                try:
                    if self.command("xclip", "-selection", "clipboard", "-out") == str(path):
                        break
                except subprocess.CalledProcessError:
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("The isolated clipboard did not accept the fixture path.")
                time.sleep(.02)
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+v")
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+a", "ctrl+c")
            copied_deadline = min(deadline, time.monotonic() + .3)
            while time.monotonic() < copied_deadline:
                if self.clipboard.poll() is not None:
                    try:
                        if self.command("xclip", "-selection", "clipboard", "-out") == str(path):
                            return
                    except subprocess.CalledProcessError:
                        pass
                    break
                time.sleep(.02)
        raise RuntimeError("The native file picker's location field did not accept the fixture path.")

    def assertion(self, action):
        value = self.state()
        for key in action["path"].split("."):
            try:
                value = value[key] if isinstance(value, dict) else value[int(key)]
            except (KeyError, IndexError, TypeError, ValueError) as error:
                raise AssertionError(f"{action['path']}: not present in the current state") from error
        expected = action.get("value")
        op = action.get("op", "eq")
        passed = {"eq": lambda: value == expected,
                  "ne": lambda: value != expected,
                  "contains": lambda: value is not None and expected in value,
                  "gte": lambda: value is not None and value >= expected,
                  "lte": lambda: value is not None and value <= expected}[op]()
        if not passed:
            raise AssertionError(f"{action['path']}: expected {op} {expected!r}; got {value!r}")
        return value

    def batch(self, actions):
        if not self.app or self.app.poll() is not None:
            raise RuntimeError("Call desktop.start first.")
        if not 1 <= len(actions) <= 100:
            raise ValueError("Batch size must be 1–100 actions.")
        waits = sum(a.get("ms", 0) for a in actions if a.get("type") == "wait")
        if waits > 10000:
            raise ValueError("Explicit waits are limited to 10 seconds per batch.")
        results = []
        for index, action in enumerate(actions):
            started = time.monotonic()
            kind = action["type"]
            try:
                result = None
                if kind in ("click", "double_click"):
                    modifiers = action.get("modifiers", [])
                    if not isinstance(modifiers, list) or any(m not in ("ctrl", "shift", "alt", "super") for m in modifiers):
                        raise ValueError("Click modifiers must be ctrl, shift, alt or super.")
                    self.command("xdotool", "mousemove", "--window", self.window,
                                 str(int(action["x"])), str(int(action["y"])))
                    try:
                        for modifier in modifiers:
                            self.command("xdotool", "keydown", modifier)
                        if modifiers:
                            time.sleep(.04)
                        self.command("xdotool", "click", "--repeat", "2" if kind == "double_click" else "1", "--delay", "90", str(action.get("button", 1)))
                        if modifiers:
                            time.sleep(.04)
                    finally:
                        for modifier in reversed(modifiers):
                            self.command("xdotool", "keyup", modifier)
                elif kind == "resize":
                    width, height = int(action["width"]), int(action["height"])
                    if not 900 <= width <= 2560 or not 640 <= height <= 1600:
                        raise ValueError("Test window must be 900–2560 × 640–1600.")
                    self.command("xdotool", "windowsize", self.window, str(width), str(height))
                elif kind == "hover":
                    self.command("xdotool", "mousemove", "--window", self.window, str(int(action["x"])), str(int(action["y"])))
                elif kind == "drag":
                    duration = int(action.get("duration_ms", 200))
                    if not 0 <= duration <= 2000:
                        raise ValueError("Drag duration must be 0–2,000 ms")
                    x, y = int(action["x"]), int(action["y"])
                    dx, dy = int(action["end_x"]) - x, int(action["end_y"]) - y
                    self.command("xdotool", "mousemove", "--window", self.window, str(x), str(y))
                    time.sleep(0.04)
                    self.command("xdotool", "mousedown", "1")
                    time.sleep(0.04)
                    try:
                        for step in range(1, 11):
                            self.command("xdotool", "mousemove", "--window", self.window,
                                         str(round(x + dx * step / 10)), str(round(y + dy * step / 10)))
                            time.sleep(duration / 10000)
                    finally:
                        self.command("xdotool", "mouseup", "1")
                elif kind == "type":
                    if len(action["text"]) > 10000:
                        raise ValueError("Text is limited to 10,000 characters per action.")
                    self.command("xdotool", "type", "--clearmodifiers", "--delay", "1", "--", action["text"])
                elif kind == "key":
                    self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "--", action["key"])
                elif kind == "choose_file":
                    result = self.choose_file(action.get("path"))
                elif kind == "print_output":
                    result = self.print_output(action.get("count", 1), action.get("text", ""), action.get("pages", 1), action.get("name", "printed-message"))
                elif kind == "focus_app":
                    self.command("xdotool", "windowraise", self.window)
                    self.command("xdotool", "windowfocus", self.window)
                elif kind == "cancel_print":
                    if not self.browser or self.browser.poll() is not None:
                        raise RuntimeError("No owned print browser is running.")
                    # Native Escape cancels the browser dialog; no app state is injected.
                    windows = self.command("xdotool", "search", "--onlyvisible", "--class", "[Cc]hrom")
                    self.command("xdotool", "windowfocus", windows.splitlines()[-1])
                    self.command("xdotool", "key", "--clearmodifiers", "Escape")
                    self.command("xdotool", "windowraise", self.window)
                    self.command("xdotool", "windowfocus", self.window)
                elif kind == "browser_screenshot":
                    if not self.browser or self.browser.poll() is not None:
                        raise RuntimeError("No owned print browser is running.")
                    name = action.get("name", "browser")
                    if not re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", name):
                        raise ValueError("Invalid browser screenshot name.")
                    path = self.directory / f"{name}.webp"
                    self.command("import", "-window", "root", "-quality", "90", str(path))
                    result = str(path)
                elif kind == "scroll":
                    amount = int(action.get("amount", 3))
                    if not 1 <= abs(amount) <= 30:
                        raise ValueError("Scroll amount must be between 1 and 30 in either direction.")
                    self.command("xdotool", "click", "--repeat", str(abs(amount)), "--delay", "20", "5" if amount > 0 else "4")
                elif kind == "wait":
                    ms = int(action["ms"])
                    if not 0 <= ms <= 2000:
                        raise ValueError("Individual waits must be between 0 and 2,000 ms.")
                    time.sleep(ms / 1000)
                elif kind in ("assert", "wait_for"):
                    deadline = time.monotonic() + (min(action.get("timeout_ms", 3000), 5000) / 1000 if kind == "wait_for" else 0)
                    while True:
                        try:
                            result = self.assertion(action)
                            break
                        except (AssertionError, FileNotFoundError, json.JSONDecodeError):
                            if time.monotonic() >= deadline:
                                raise
                            time.sleep(0.005)
                elif kind == "screenshot":
                    result = str(self.screenshot(action.get("name", f"step-{index}")))
                elif kind == "state":
                    result = self.state()
                else:
                    raise ValueError(f"Unknown action type: {kind}")
                results.append({"index": index, "type": kind, "elapsed_ms": round((time.monotonic() - started) * 1000, 2), "result": result})
            except Exception as error:
                try:
                    self.screenshot(f"failure-{index}")
                except Exception:
                    pass
                raise RuntimeError(f"Batch stopped at action {index} ({kind}): {error}") from error
        report = {"actions": results, "state": self.state()}
        (self.directory / f"batch-{time.time_ns()}.json").write_text(json.dumps(report, indent=2))
        return report


TOOLS = [
    {"name": "desktop.start", "description": "Launch an isolated Shep fixture workspace on Xvfb. Requires cargo build --profile test-ui --features test-support. No real credentials or network writes.",
     "inputSchema": {"type": "object", "properties": {"profile_sync": {"type":"boolean","default":False,"description":"Isolated remote preference change after enrollment; enables profile discovery."}, "profile_pages": {"type":"boolean", "default":False, "description":"51 synthetic profiles and 75 accounts for real paging controls; enables profile discovery fixture."}, "profile_discovery": {"type": "boolean", "default": False}, "desktop_badges": {"type": "boolean", "default": False}, "html_failure_once": {"type": "boolean", "default": False}, "image_delay_ms": {"type": "integer", "minimum": 0, "maximum": 5000, "default": 0}, "html_delay_ms": {"type": "integer", "minimum": 0, "maximum": 2000, "default": 0}, "print_browser": {"type": "string", "enum": ["pdf", "dialog", "fail"]}, "empty_calendars": {"type": "boolean", "default": False}, "conversation_mail": {"type": "boolean", "default": False}, "readonly_calendars": {"type": "boolean", "default": False}, "pending_transfer": {"type": "boolean", "default": False}, "outgoing_mail": {"type": "boolean", "default": False}, "long_folders": {"type": "boolean", "default": False}, "mail_actions": {"type": "string", "enum": ["slow", "fail"]}, "search_mail": {"type": "boolean", "default": False}, "html_mail": {"type": "boolean", "default": False}, "background_sync": {"type": "boolean", "default": False}, "sync_failure_once": {"type": "boolean", "default": False}, "undo_failure_once": {"type": "boolean", "default": False}, "discard_failure_once": {"type": "boolean", "default": False}, "google_permissions": {"type": "string", "enum": ["drive", "calendar", "read-only"]}, "width": {"type": "integer", "default": 1440}, "height": {"type": "integer", "default": 920}}}},
    {"name": "desktop.batch", "description": "Run 1–100 real mouse/keyboard actions in order, including short waits, state assertions and WebP screenshots. Stops at first failure and captures evidence. Prefer batches to one call per action.",
     "inputSchema": {"type": "object", "required": ["actions"], "properties": {"actions": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["type"], "properties": {"type": {"enum": ["click", "double_click", "hover", "resize", "drag", "type", "key", "choose_file", "print_output", "cancel_print", "focus_app", "browser_screenshot", "scroll", "wait", "assert", "wait_for", "screenshot", "state"]}, "count": {"type": "integer"}, "pages": {"type": "integer"}, "x": {"type": "integer"}, "y": {"type": "integer"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "button": {"type": "integer", "enum": [1, 2, 3]}, "modifiers": {"type": "array", "items": {"type": "string", "enum": ["ctrl", "shift", "alt", "super"]}}, "end_x": {"type": "integer"}, "end_y": {"type": "integer"}, "duration_ms": {"type": "integer", "maximum": 2000}, "text": {"type": "string"}, "key": {"type": "string"}, "ms": {"type": "integer", "maximum": 2000}, "path": {"type": "string"}, "op": {"enum": ["eq", "ne", "contains", "gte", "lte"]}, "value": {}, "name": {"type": "string"}, "amount": {"type": "integer"}, "timeout_ms": {"type": "integer", "maximum": 5000}}}}}}},
    {"name": "desktop.state", "description": "Read observed UI state, cache counts, shortcuts and handler timings; does not change app state.", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "desktop.screenshot", "description": "Capture the actual iced window as WebP. Returns image and artifact path.", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}}},
    {"name": "desktop.stop", "description": "Stop only the isolated app and Xvfb processes created by this harness.", "inputSchema": {"type": "object", "properties": {}}},
]


def main():
    desktop = Desktop()
    for line in sys.stdin:
        request = None
        try:
            request = json.loads(line)
            method = request.get("method")
            if "id" not in request:
                continue
            if method == "initialize":
                result = {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
                          "serverInfo": {"name": "shep-native-e2e", "version": "1.0.0"}}
            elif method == "ping":
                result = {}
            elif method == "tools/list":
                result = {"tools": TOOLS}
            elif method == "tools/call":
                params = request["params"]
                name = params["name"]
                arguments = params.get("arguments", {})
                handlers = {"desktop.start": desktop.start, "desktop.batch": desktop.batch,
                            "desktop.state": desktop.state, "desktop.screenshot": desktop.screenshot,
                            "desktop.stop": desktop.stop}
                try:
                    if name not in handlers:
                        raise ValueError(f"Unknown tool: {name}")
                    value = handlers[name](**arguments)
                    if isinstance(value, Path):
                        result = {"content": [{"type": "text", "text": str(value)},
                                              {"type": "image", "mimeType": "image/webp", "data": base64.b64encode(value.read_bytes()).decode()}]}
                    else:
                        result = {"content": [{"type": "text", "text": json.dumps(value)}], "structuredContent": value}
                except Exception as error:
                    result = {"isError": True, "content": [{"type": "text", "text": str(error)}]}
            else:
                response = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Method not found"}}
                print(json.dumps(response), flush=True)
                continue
            response = {"jsonrpc": "2.0", "id": request["id"], "result": result}
        except Exception as error:
            response = {"jsonrpc": "2.0", "id": request.get("id") if isinstance(request, dict) else None,
                        "error": {"code": -32600, "message": str(error)}}
        print(json.dumps(response), flush=True)
    desktop.stop()


if __name__ == "__main__":
    main()
