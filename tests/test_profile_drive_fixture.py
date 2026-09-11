"""Actual loopback uploads for deterministic native profile scenarios."""
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from urllib.parse import quote
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("harness", ROOT / "scripts/mcp_harness.py")
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def upload(fixture, identity):
    metadata = json.dumps({"id": identity}).encode()
    data = (b'--fixture\r\nContent-Type: application/json\r\n\r\n' + metadata +
            b'\r\n--fixture\r\nContent-Type: application/json\r\n\r\n{}\r\n--fixture--\r\n')
    request = Request(fixture.url + "upload/drive/v3/files", data=data,
                      headers={"Authorization": "Bearer fixture-profile-token",
                               "Content-Type": "multipart/related; boundary=fixture"})
    with urlopen(request, timeout=5) as response:
        return response.status, json.load(response)


class ProfileDriveFixtureTests(unittest.TestCase):
    def test_held_upload_requires_explicit_owned_release_and_later_uploads_finish(self):
        fixture = harness._profile_fixture.ProfileDriveFixture("held-upload")
        try:
            with ThreadPoolExecutor(max_workers=1) as worker, tempfile.TemporaryDirectory() as directory:
                pending = worker.submit(upload, fixture, "first")
                self.assertTrue(fixture.upload_held.wait(2))
                self.assertFalse(pending.done())
                self.assertIn("first", fixture.files)
                desktop = harness.Desktop()
                desktop.directory = Path(directory)
                desktop.app = Mock()
                desktop.app.poll.return_value = None
                desktop.profile_drive = fixture
                with patch.object(desktop, "state", return_value={}), patch.object(desktop, "screenshot"):
                    desktop.batch([{"type": "release_profile_upload"}])
                    with self.assertRaisesRegex(RuntimeError, "No owned profile upload"):
                        desktop.batch([{"type": "release_profile_upload"}])
                self.assertEqual(pending.result(timeout=2)[0], 201)
                self.assertEqual(upload(fixture, "second")[0], 201)
                self.assertEqual(set(fixture.files), {"first", "second"})
        finally:
            fixture.close()

    def test_fixture_shutdown_releases_a_pending_upload_and_joins_its_server(self):
        fixture = harness._profile_fixture.ProfileDriveFixture("held-upload")
        with ThreadPoolExecutor(max_workers=1) as worker:
            pending = worker.submit(upload, fixture, "closing")
            try:
                self.assertTrue(fixture.upload_held.wait(2))
            finally:
                fixture.close()
            self.assertFalse(fixture.thread.is_alive())
            self.assertEqual(pending.result(timeout=2)[0], 201)

    def test_credential_files_are_listed_apart_from_profiles_and_can_be_removed(self):
        fixture = harness._profile_fixture.ProfileDriveFixture("existing-passwords")
        try:
            auth = {"Authorization": "Bearer fixture-profile-token"}
            scope = ("appProperties has { key='shepProfile' and value='10000000-0000-4000-8000-000000000001' } "
                     "and appProperties has { key='shepGeneration' and value='20000000-0000-4000-8000-000000000001' }")
            query = "(appProperties has { key='shepType' and value='credential-key' } or appProperties has { key='shepType' and value='credential-vault' }) and " + scope
            with urlopen(Request(fixture.url + "drive/v3/files?q=" + quote(query), headers=auth), timeout=5) as response:
                listed = json.load(response)["files"]
            self.assertEqual(sorted(f["appProperties"]["shepType"] for f in listed), ["credential-key", "credential-vault"])
            # Profile discovery never sees them.
            with urlopen(Request(fixture.url + "drive/v3/files?q=" + quote("appProperties has { key='shepType' and value='profile' }"), headers=auth), timeout=5) as response:
                self.assertTrue(all(f["appProperties"]["shepType"] == "profile" for f in json.load(response)["files"]))
            self.assertEqual(fixture.credential_state(), {"keys": 1, "vaults": 1, "plaintext": False})
            metadata = json.dumps({"id": "new-vault", "name": "shep-credential-vault-x.json",
                                   "appProperties": {"shepType": "credential-vault"}}).encode()
            data = (b'--fixture\r\nContent-Type: application/json\r\n\r\n' + metadata +
                    b'\r\n--fixture\r\nContent-Type: application/json\r\n\r\n{"secret":"fixture-cloud-smtp"}\r\n--fixture--\r\n')
            with urlopen(Request(fixture.url + "upload/drive/v3/files", data=data,
                                 headers={**auth, "Content-Type": "multipart/related; boundary=fixture"}), timeout=5) as response:
                self.assertEqual(response.status, 200)
            # The oracle reports plaintext passwords stored by a faulty client.
            self.assertTrue(fixture.credential_state()["plaintext"])
            with urlopen(Request(fixture.url + "drive/v3/files/new-vault", headers=auth, method="DELETE"), timeout=5) as response:
                self.assertEqual(response.status, 204)
            self.assertEqual(fixture.credential_state(), {"keys": 1, "vaults": 1, "plaintext": False})
            self.assertNotIn("new-vault", fixture.files)
        finally:
            fixture.close()

    def test_release_action_cannot_control_an_unrelated_profile_fixture(self):
        desktop = harness.Desktop()
        desktop.app = Mock()
        desktop.app.poll.return_value = None
        with patch.object(desktop, "screenshot"):
            with self.assertRaisesRegex(RuntimeError, "No owned profile upload"):
                desktop.batch([{"type": "release_profile_upload"}])
