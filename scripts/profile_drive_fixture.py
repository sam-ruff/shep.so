"""Owned loopback Drive fixture for native MCP tests. No real Google access."""
import email
import email.policy
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import re
import threading
import time
from urllib.parse import parse_qs, urlparse

MODES = ("empty", "fail-once", "hold-list", "slow-upload")


class ProfileDriveFixture:
    def __init__(self, mode):
        if mode not in MODES:
            raise ValueError("Unknown profile Drive fixture.")
        self.mode = mode
        self.files = {}
        self.next_id = 0
        self.failed = False
        self.release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, status, value):
                raw = value if isinstance(value, bytes) else json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                try:
                    self.wfile.write(raw)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def authorized(self):
                if self.headers.get("Authorization") != "Bearer fixture-profile-token":
                    self.reply(401, {})
                    return False
                return True

            def do_GET(self):
                if not self.authorized():
                    return
                url = urlparse(self.path)
                query = parse_qs(url.query)
                if url.path == "/drive/v3/about":
                    return self.reply(200, {"user": {"permissionId": "fixture"}})
                if url.path == "/drive/v3/files/generateIds":
                    owner.next_id += 1
                    return self.reply(200, {"ids": [f"fixture-profile-{owner.next_id}"], "space": "appDataFolder"})
                if url.path == "/drive/v3/files":
                    if owner.mode == "hold-list":
                        owner.release.wait()
                    if owner.mode == "fail-once" and not owner.failed:
                        owner.failed = True
                        return self.reply(503, {"error": "Fixture offline; retry discovery."})
                    rows = [entry[0] for entry in owner.files.values()]
                    q = query.get("q", [""])[0]
                    for key in ("shepProfile", "shepGeneration"):
                        match = re.search("key='" + key + r"' and value='([^']+)'", q)
                        if match:
                            rows = [r for r in rows if r["appProperties"].get(key) == match[1]]
                    return self.reply(200, {"files": rows, "incompleteSearch": False})
                identity = url.path.removeprefix("/drive/v3/files/")
                if identity not in owner.files:
                    return self.reply(404, {})
                metadata, raw = owner.files[identity]
                return self.reply(200, raw if query.get("alt") == ["media"] else metadata)

            def do_POST(self):
                if not self.authorized():
                    return
                if urlparse(self.path).path != "/upload/drive/v3/files":
                    return self.reply(404, {})
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 2 * 1024 * 1024:
                    return self.reply(413, {})
                raw = self.rfile.read(length)
                message = email.message_from_bytes(
                    b"Content-Type: " + self.headers["Content-Type"].encode() + b"\r\n\r\n" + raw,
                    policy=email.policy.default)
                parts = list(message.iter_parts())
                if len(parts) != 2:
                    return self.reply(400, {})
                metadata = json.loads(parts[0].get_payload(decode=True))
                record = parts[1].get_payload(decode=True)
                json.loads(record)
                identity = metadata["id"]
                if identity in owner.files:
                    return self.reply(409, {})
                metadata.update(ownedByMe=True, trashed=False, spaces=["appDataFolder"],
                                mimeType="application/json", size=str(len(record)),
                                sha256Checksum=hashlib.sha256(record).hexdigest())
                owner.files[identity] = (metadata, record)
                if owner.mode == "slow-upload":
                    time.sleep(1)
                self.reply(201, metadata)

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
