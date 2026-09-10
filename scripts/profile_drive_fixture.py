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

MODES = ("empty", "fail-once", "hold-list", "slow-upload", "held-upload", "invalid-local", "existing", "existing-unsupported", "existing-incomplete", "existing-legacy", "existing-single", "existing-matching", "existing-many", "existing-conflict", "existing-connections", "existing-removal", "existing-link", "existing-updates", "existing-update-failure", "existing-upload-failure")


class ProfileDriveFixture:
    def __init__(self, mode):
        if mode not in MODES:
            raise ValueError("Unknown profile Drive fixture.")
        self.mode = mode
        self.files = {}
        self.next_id = 0
        self.changes = []
        self.failed = False
        self.scoped_lists = 0
        self.requests = {"lists": 0, "scoped_lists": 0, "metadata": 0, "media": 0}
        self.updated = False
        self.release = threading.Event()
        self.upload_held = threading.Event()
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
                    q = query.get("q", [""])[0]
                    owner.requests["lists"] += 1
                    if "shepProfile" in q:
                        owner.requests["scoped_lists"] += 1
                    if "shepProfile" in q and owner.mode in ("existing-conflict", "existing-connections", "existing-removal", "existing-link", "existing-updates", "existing-update-failure", "existing-upload-failure"):
                        owner.scoped_lists += 1
                        if owner.scoped_lists >= 2 and not owner.updated:
                            if owner.mode == "existing-update-failure" and not owner.failed:
                                owner.failed = True
                                return self.reply(503, {"error":"Fixture is offline during a continuous check."})
                            if owner.mode == "existing-connections":
                                owner.seed_connections()
                            elif owner.mode == "existing-removal":
                                owner.seed_removal()
                            elif owner.mode == "existing-link":
                                owner.seed_link()
                            elif owner.mode == "existing-conflict":
                                owner.seed_conflict()
                            else:
                                owner.seed_update()
                    rows = [entry[0] for entry in owner.files.values()]
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
                owner.requests["media" if query.get("alt") == ["media"] else "metadata"] += 1
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
                if owner.mode == "existing-upload-failure":
                    return self.reply(503, {"error":"Fixture upload is unavailable."})
                if identity in owner.files:
                    return self.reply(409, {})
                metadata.update(ownedByMe=True, trashed=False, spaces=["appDataFolder"],
                                mimeType="application/json", size=str(len(record)),
                                sha256Checksum=hashlib.sha256(record).hexdigest())
                owner.files[identity] = (metadata, record)
                owner.changes.append(identity)
                if owner.mode == "held-upload" and not owner.release.is_set():
                    owner.upload_held.set()
                    owner.release.wait()
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
        if self.mode in ("existing-matching", "existing-many"):
            account.update(email="alex@studio.example", username="alex@studio.example", smtp_username="alex@studio.example",
                host="imap.example", smtp_host="smtp.example", smtp_port=465, smtp_security="Tls",
                smtp_auth="Automatic", smtp_separate_password=False, sent_folder="")
        if self.mode == "existing-unsupported":
            account["future_tls_requirement"] = True
        names = ("Home",) if self.mode in ("existing-single", "existing-matching", "existing-many", "existing-conflict", "existing-connections", "existing-removal", "existing-link", "existing-updates", "existing-update-failure", "existing-upload-failure") else ("Home", "Work")
        for number, name in enumerate(names, start=1):
            operation = dict(original)
            for field, prefix in (("profile","1"),("generation","2"),("device","3"),("operation","4")):
                operation[field] = f"{prefix}0000000-0000-4000-8000-{number:012d}"
            operation["namespace"] = "so.shep"
            operation["parents"] = []
            operation["changes"] = [{"kind":"profile_name", "name":name},
                {"kind":"setting", "key":"appearance", "value":"Dark" if number == 1 and self.mode != "existing-link" else "Light"}]
            if number == 1:
                operation["changes"] += [{"kind":"account_connection", "account":account},
                    {"kind":"account_name", "id":account["id"], "name":"Cloud account"}]
            if self.mode == "existing-many" and number == 1:
                for extra in range(2,13):
                    copied=dict(account,id=f"50000000-0000-4000-8000-{extra:012d}",email=f"account{extra}@example.test")
                    operation["changes"] += [{"kind":"account_connection","account":copied},
                        {"kind":"account_name","id":copied["id"],"name":f"Shared account {extra:02d}"}]
            if number == 1:
                operation["changes"].append({"kind":"setting", "key":"tooltips", "value":False})
            if self.mode == "existing-legacy" and number == 1:
                records = [("", operation)]
            else:
                operation["requires"] = [*operation["requires"], "initialization-v1"]
                start = dict(operation, operation=f"60000000-0000-4000-8000-{number:012d}",
                             parents=[], changes=[{"kind":"profile_setup", "complete":False}])
                operation["parents"] = [start["operation"]]
                end = dict(operation, operation=f"70000000-0000-4000-8000-{number:012d}",
                           parents=[operation["operation"]], changes=[{"kind":"profile_setup", "complete":True}])
                records = [("-start", start), ("", operation)]
                if not (self.mode == "existing-incomplete" and number == 1):
                    records.append(("-complete", end))
            for suffix, record in records:
                raw = json.dumps(record, ensure_ascii=False).encode()
                identity = f"existing-profile-{number}{suffix}"
                digest = hashlib.sha256(raw).hexdigest()
                metadata = {"id":identity, "name":f"shep-profile-{record['operation']}.json", "ownedByMe":True,
                    "trashed":False,"spaces":["appDataFolder"],"mimeType":"application/json","size":str(len(raw)),
                    "sha256Checksum":digest, "appProperties":{"shepType":"profile","shepFormat":"operation-v1",
                        "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(),"shepProfile":record["profile"],
                        "shepGeneration":record["generation"],"shepOperation":record["operation"],"shepSha256":digest}}
                self.files[identity] = (metadata, raw)
                self.changes.append(identity)

    def seed_conflict(self):
        original = json.loads(self.files["existing-profile-1"][1])
        for number, value in enumerate(("Light", "System"), start=1):
            operation = f"50000000-0000-4000-8000-{number:012d}"
            record = dict(original, device=f"90000000-0000-4000-8000-{number:012d}",
                operation=operation, parents=["70000000-0000-4000-8000-000000000001"],
                changes=[{"kind":"setting", "key":"appearance", "value":value, "peer_hint":value}])
            raw = json.dumps(record).encode()
            digest = hashlib.sha256(raw).hexdigest()
            identity = f"existing-profile-1-conflict-{number}"
            metadata = {"id":identity, "name":f"shep-profile-{operation}.json", "ownedByMe":True,
                "trashed":False, "spaces":["appDataFolder"], "mimeType":"application/json", "size":str(len(raw)),
                "sha256Checksum":digest, "appProperties":{"shepType":"profile", "shepFormat":"operation-v1",
                    "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(), "shepProfile":record["profile"],
                    "shepGeneration":record["generation"], "shepOperation":operation, "shepSha256":digest}}
            self.files[identity] = (metadata, raw)
            self.changes.append(identity)
        self.updated = True

    def seed_removal(self):
        original = json.loads(self.files["existing-profile-1"][1])
        account = next(c["account"] for c in original["changes"] if c["kind"] == "account_connection")
        operation = "50000000-0000-4000-8000-000000000001"
        record = dict(original, device="90000000-0000-4000-8000-000000000001", operation=operation,
            parents=["70000000-0000-4000-8000-000000000001"],
            changes=[{"kind":"account_removed", "id":account["id"]}])
        raw = json.dumps(record).encode()
        digest = hashlib.sha256(raw).hexdigest()
        identity = "existing-profile-1-removal"
        metadata = {"id":identity, "name":f"shep-profile-{operation}.json", "ownedByMe":True,
            "trashed":False, "spaces":["appDataFolder"], "mimeType":"application/json", "size":str(len(raw)),
            "sha256Checksum":digest, "appProperties":{"shepType":"profile", "shepFormat":"operation-v1",
                "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(), "shepProfile":record["profile"],
                "shepGeneration":record["generation"], "shepOperation":operation, "shepSha256":digest}}
        self.files[identity] = (metadata, raw)
        self.changes.append(identity)
        self.updated = True

    def seed_connections(self):
        original = json.loads(self.files["existing-profile-1"][1])
        account = next(c["account"] for c in original["changes"] if c["kind"] == "account_connection").copy()
        for number, host in enumerate(("incoming-new.example.test", "incoming-other.example.test"), start=1):
            shared = dict(account, host=host, smtp_host="outgoing-new.example.test")
            operation = f"50000000-0000-4000-8000-{number:012d}"
            record = dict(original, device=f"90000000-0000-4000-8000-{number:012d}", operation=operation,
                parents=["70000000-0000-4000-8000-000000000001"],
                changes=[{"kind":"account_connection", "account":shared, "peer_hint":host}])
            raw = json.dumps(record).encode()
            digest = hashlib.sha256(raw).hexdigest()
            identity = f"existing-profile-1-connection-{number}"
            metadata = {"id":identity, "name":f"shep-profile-{operation}.json", "ownedByMe":True,
                "trashed":False, "spaces":["appDataFolder"], "mimeType":"application/json", "size":str(len(raw)),
                "sha256Checksum":digest, "appProperties":{"shepType":"profile", "shepFormat":"operation-v1",
                    "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(), "shepProfile":record["profile"],
                    "shepGeneration":record["generation"], "shepOperation":operation, "shepSha256":digest}}
            self.files[identity] = (metadata, raw)
            self.changes.append(identity)
        self.updated = True

    def seed_link(self):
        """Two later definitions from another device: one exactly matches the
        preview Design studio account, one shares Personal's address only."""
        original = json.loads(self.files["existing-profile-1"][1])
        account = next(c["account"] for c in original["changes"] if c["kind"] == "account_connection").copy()
        studio = dict(account, id="80000000-0000-4000-8000-000000000002", email="alex@studio.example",
            username="alex@studio.example", smtp_username="alex@studio.example", host="imap.example", port=993,
            smtp_host="smtp.example", smtp_port=465, smtp_security="Tls", smtp_auth="Automatic",
            smtp_separate_password=False, sent_copy="Automatic", sent_folder="")
        personal = dict(studio, id="80000000-0000-4000-8000-000000000003", email="alex@example.com",
            username="alex@example.com", smtp_username="alex@example.com", host="mail.other.example",
            smtp_host="smtp.other.example")
        record = dict(original, device="90000000-0000-4000-8000-000000000001",
            operation="50000000-0000-4000-8000-000000000002",
            parents=["70000000-0000-4000-8000-000000000001"],
            changes=[{"kind":"account_connection", "account":studio, "peer_hint":"studio"},
                {"kind":"account_name", "id":studio["id"], "name":"Studio (shared)"},
                {"kind":"account_connection", "account":personal},
                {"kind":"account_name", "id":personal["id"], "name":"Personal (other server)"}])
        raw = json.dumps(record, ensure_ascii=False).encode()
        digest = hashlib.sha256(raw).hexdigest()
        identity = "existing-profile-1-link"
        metadata = {"id":identity,"name":f"shep-profile-{record['operation']}.json","ownedByMe":True,
            "trashed":False,"spaces":["appDataFolder"],"mimeType":"application/json","size":str(len(raw)),
            "sha256Checksum":digest,"appProperties":{"shepType":"profile","shepFormat":"operation-v1",
                "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(),"shepProfile":record["profile"],
                "shepGeneration":record["generation"],"shepOperation":record["operation"],"shepSha256":digest}}
        self.files[identity] = (metadata,raw)
        self.changes.append(identity)
        self.updated = True

    def seed_update(self):
        original = json.loads(self.files["existing-profile-1"][1])
        account = next(c["account"] for c in original["changes"] if c["kind"] == "account_connection").copy()
        account["id"] = "80000000-0000-4000-8000-000000000001"
        account["email"] = account["username"] = account["smtp_username"] = "second@example.test"
        record = dict(original, device="90000000-0000-4000-8000-000000000001",
            operation="50000000-0000-4000-8000-000000000001",
            parents=["70000000-0000-4000-8000-000000000001"],
            changes=[{"kind":"setting", "key":"tooltips", "value":True},
                {"kind":"account_connection", "account":account},
                {"kind":"account_name", "id":account["id"], "name":"Second cloud account"}])
        raw = json.dumps(record, ensure_ascii=False).encode()
        digest = hashlib.sha256(raw).hexdigest()
        identity = "existing-profile-1-update"
        metadata = {"id":identity,"name":f"shep-profile-{record['operation']}.json","ownedByMe":True,
            "trashed":False,"spaces":["appDataFolder"],"mimeType":"application/json","size":str(len(raw)),
            "sha256Checksum":digest,"appProperties":{"shepType":"profile","shepFormat":"operation-v1",
                "shepNamespace":hashlib.sha256(b"so.shep").hexdigest(),"shepProfile":record["profile"],
                "shepGeneration":record["generation"],"shepOperation":record["operation"],"shepSha256":digest}}
        self.files[identity] = (metadata,raw)
        self.changes.append(identity)
        self.updated = True

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
