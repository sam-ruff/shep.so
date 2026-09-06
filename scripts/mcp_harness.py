#!/usr/bin/env python3
"""Shep native desktop MCP server. Standard-library-only, newline-delimited JSON-RPC.

Real X11 mouse/keyboard input; no application actions are invoked through test hooks.
The inspection file is a read-only oracle, enabled only in a test-support build.
"""
import atexit
import base64
import json
import os
from pathlib import Path
import re
import shutil
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
        atexit.register(self.stop)

    def stop(self):
        for process in (self.clipboard, self.app, self.xvfb):
            if process and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
        self.app = self.xvfb = None
        self.clipboard = None
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

    def start(self, width=1440, height=920, empty_calendars=False, conversation_mail=False, readonly_calendars=False, pending_transfer=False, outgoing_mail=False, google_permissions=None, long_folders=False, mail_actions=None, background_sync=False, sync_failure_once=False):
        self.stop()
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
        self.env.pop("WAYLAND_DISPLAY", None)
        self.log = (self.directory / "app.log").open("w")
        self.app = subprocess.Popen(
            [str(binary), "--demo", "--test-state", str(self.directory / "state.json"), *(["--empty-calendars"] if empty_calendars else []), *(["--conversation-mail"] if conversation_mail else []), *(["--readonly-calendars"] if readonly_calendars else []), *(["--pending-transfer"] if pending_transfer else []), *(["--outgoing-mail"] if outgoing_mail else []), *(["--long-folders"] if long_folders else []), *(["--background-sync"] if background_sync else []), *(["--sync-failure-once"] if sync_failure_once else []), *(["--mail-actions=" + mail_actions] if mail_actions in ("slow", "fail") else []), *(["--google-permissions=" + google_permissions] if google_permissions else [])],
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
        return json.loads((self.directory / "state.json").read_text())

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
        self.command("xdotool", "windowfocus", window)
        time.sleep(.15)
        if path is None:
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "Escape")
        else:
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+l")
            time.sleep(.1)
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+a")
            # Paste in one native input operation. GTK path completion can alter
            # partially typed paths; the clipboard belongs only to our Xvfb.
            if self.clipboard and self.clipboard.poll() is None:
                self.clipboard.terminate()
                self.clipboard.wait(timeout=3)
            self.clipboard = subprocess.Popen(["xclip", "-selection", "clipboard", "-quiet"],
                env=self.env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=self.log)
            self.clipboard.stdin.write(str(path).encode())
            self.clipboard.stdin.close()
            time.sleep(.05)
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "ctrl+v")
            time.sleep(.15)
            self.command("import", "-window", window, "-quality", "90", str(self.directory / "native-file-picker.webp"))
            self.command("xdotool", "key", "--clearmodifiers", "--delay", "1", "Return")
        deadline = time.monotonic() + 3
        while True:
            try:
                visible = self.command("xdotool", "search", "--onlyvisible", "--class", "zenity").splitlines()
            except subprocess.CalledProcessError:
                visible = []
            if window not in visible:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("The native file picker did not accept the selected file.")
            time.sleep(.05)
        # Xvfb has no window manager to return focus after closing a native dialog.
        self.command("xdotool", "windowfocus", self.window)
        return {"selected": str(path) if path else None}

    def assertion(self, action):
        value = self.state()
        for key in action["path"].split("."):
            try:
                value = value[key] if isinstance(value, dict) else value[int(key)]
            except (KeyError, IndexError, TypeError) as error:
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
     "inputSchema": {"type": "object", "properties": {"empty_calendars": {"type": "boolean", "default": False}, "conversation_mail": {"type": "boolean", "default": False}, "readonly_calendars": {"type": "boolean", "default": False}, "pending_transfer": {"type": "boolean", "default": False}, "outgoing_mail": {"type": "boolean", "default": False}, "long_folders": {"type": "boolean", "default": False}, "mail_actions": {"type": "string", "enum": ["slow", "fail"]}, "background_sync": {"type": "boolean", "default": False}, "sync_failure_once": {"type": "boolean", "default": False}, "google_permissions": {"type": "string", "enum": ["drive", "calendar", "read-only"]}, "width": {"type": "integer", "default": 1440}, "height": {"type": "integer", "default": 920}}}},
    {"name": "desktop.batch", "description": "Run 1–100 real mouse/keyboard actions in order, including short waits, state assertions and WebP screenshots. Stops at first failure and captures evidence. Prefer batches to one call per action.",
     "inputSchema": {"type": "object", "required": ["actions"], "properties": {"actions": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["type"], "properties": {"type": {"enum": ["click", "double_click", "hover", "resize", "drag", "type", "key", "choose_file", "scroll", "wait", "assert", "wait_for", "screenshot", "state"]}, "x": {"type": "integer"}, "y": {"type": "integer"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "button": {"type": "integer", "enum": [1, 2, 3]}, "modifiers": {"type": "array", "items": {"type": "string", "enum": ["ctrl", "shift", "alt", "super"]}}, "end_x": {"type": "integer"}, "end_y": {"type": "integer"}, "duration_ms": {"type": "integer", "maximum": 2000}, "text": {"type": "string"}, "key": {"type": "string"}, "ms": {"type": "integer", "maximum": 2000}, "path": {"type": "string"}, "op": {"enum": ["eq", "ne", "contains", "gte", "lte"]}, "value": {}, "name": {"type": "string"}, "amount": {"type": "integer"}, "timeout_ms": {"type": "integer", "maximum": 5000}}}}}}},
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
