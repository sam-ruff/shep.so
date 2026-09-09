"""Owned loopback Drive fixture for native MCP tests. No real Google access."""
import email
import email.policy
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import re
import threading
import time
from urllib.parse import parse_qs, urlparse

MODES = ("empty", "fail-once", "hold-list", "slow-upload", "invalid-local", "existing", "existing-unsupported")


class ProfileDriveFixture:
    def __init__(self, mode):
        if mode not in MODES:
            raise ValueError("Unknown profile Drive fixture.")
        self.mode = mode
        self.files = {}
        self.next_id = 0
        self.changes = []
        self.failed = False
        self.release = threading.Event()
        if mode.startswith("existing"):
            self.seed_existing()
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
                if url.path == "/drive/v3/changes/startPageToken":
                    return self.reply(200, {"startPageToken": f"fixture-change-{len(owner.changes)}"})
                if url.path == "/drive/v3/changes":
                    token = query.get("pageToken", [""])[0]
                    if not re.fullmatch(r"fixture-change-\d+", token):
                        return self.reply(400, {})
                    start = int(token.rsplit("-", 1)[1])
                    entries = owner.changes[start:start + 50]
                    end = start + len(entries)
                    reply = {"changes": [{"fileId":identity, "removed":False, "changeType":"file", "file":owner.files[identity][0]} for identity in entries]}
                    reply["nextPageToken" if end < len(owner.changes) else "newStartPageToken"] = f"fixture-change-{end}"
                    return self.reply(200, reply)
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
                    start = int(query.get("pageToken", ["0"])[0])
                    count = int(query.get("pageSize", ["50"])[0])
                    reply = {"files":rows[start:start + count], "incompleteSearch":False}
                    if start + count < len(rows):
                        reply["nextPageToken"] = str(start + count)
                    return self.reply(200, reply)
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
                owner.changes.append(identity)
                if owner.mode == "slow-upload":
                    time.sleep(1)
                self.reply(201, metadata)

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def seed_existing(self):
        original = json.loads((Path(__file__).resolve().parents[1] / "tests/support/profile-operation.json").read_text())
        account = original["changes"][0]["account"]
        account = {key:value for key,value in account.items() if not key.startswith("x-")}
        account["email"] = account["username"] = account["smtp_username"] = "cloud@example.test"
        if self.mode == "existing-unsupported":
            account["future_tls_requirement"] = True
        for number, name in enumerate(("Home", "Work"), start=1):
            operation = dict(original)
            for field, prefix in (("profile","1"),("generation","2"),("device","3"),("operation","4")):
                operation[field] = f"{prefix}0000000-0000-4000-8000-{number:012d}"
            operation["namespace"] = "so.shep"
            operation["parents"] = []
            operation["changes"] = [{"kind":"profile_name", "name":name},
                {"kind":"setting", "key":"appearance", "value":"Dark" if number == 1 else "Light"}]
            if number == 1:
                operation["changes"] += [{"kind":"account_connection", "account":account},
                    {"kind":"account_name", "id":account["id"], "name":"Cloud account"}]
            raw = json.dumps(operation, ensure_ascii=False).encode()
            identity = f"existing-profile-{number}"
            digest = hashlib.sha256(raw).hexdigest()
            metadata = {"id":identity, "name":f"shep-profile-{operation['operation']}.json", "ownedByMe":True,
                "trashed":False,"spaces":["appDataFolder"],"mimeType":"application/json","size":str(len(raw)),
                "sha256Checksum":digest, "appProperties":{"shepType":"profile","shepFormat":"operation-v1",
                    "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(),"shepProfile":operation["profile"],
                    "shepGeneration":operation["generation"],"shepOperation":operation["operation"],"shepSha256":digest}}
            self.files[identity] = (metadata, raw)
            self.changes.append(identity)

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
