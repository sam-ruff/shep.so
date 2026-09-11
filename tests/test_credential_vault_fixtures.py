"""Check the shared credential vault fixtures with an independent AES-GCM."""
import base64
import hashlib
import json
from pathlib import Path
import unittest

try:
    from cryptography.exceptions import InvalidTag
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
except ImportError:  # pragma: no cover - optional dependency
    AESGCM = None

FIXTURES = Path(__file__).resolve().parents[1] / "shared/credential-vault-fixtures.json"


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


@unittest.skipIf(AESGCM is None, "python3-cryptography is not installed")
class CredentialVaultFixtureTests(unittest.TestCase):
    def setUp(self):
        self.f = json.loads(FIXTURES.read_text(encoding="utf-8"))
        self.keys = {name: bytes.fromhex(key["material_hex"]) for name, key in self.f["keys"].items()}

    def aad(self, key, account, field, revision, endpoint, profile=None):
        return "\n".join(["so.shep.credential-vault", "1", profile or self.f["profile"], self.f["generation"],
                          self.f["keys"][key]["id"], account, field, str(revision), endpoint])

    def test_key_files_are_compact_json_in_documented_field_order(self):
        for name, key in self.f["keys"].items():
            expected = compact({"format": "so.shep.credential-key", "major": 1, "minor": 0, "algorithm": "A256GCM",
                                "profile": self.f["profile"], "generation": self.f["generation"], "key": key["id"],
                                "sequence": key["sequence"], "material": base64.b64encode(self.keys[name]).decode()})
            self.assertEqual(key["file"], expected)

    def test_endpoints_and_seals_match_an_independent_implementation(self):
        for name in ("incoming", "smtp"):
            text = self.f["endpoints"][f"{name}_text"]
            self.assertEqual(hashlib.sha256(text.encode()).hexdigest(), self.f["endpoints"][name])
        connection = self.f["connection"]
        self.assertIn(f"\n{connection['host'].lower()}\n", self.f["endpoints"]["incoming_text"])
        for case in self.f["seals"]:
            aad = self.aad(case["key"], case["account"], case["field"], case["revision"], case["endpoint"])
            self.assertEqual(case["aad"], aad)
            nonce = bytes.fromhex(case["nonce_hex"])
            body = AESGCM(self.keys[case["key"]]).encrypt(nonce, case["secret"].encode(), aad.encode())
            self.assertEqual(case["sealed"], base64.b64encode(bytes([1]) + nonce + body).decode())

    def test_rejections_fail_authentication_or_version_checks(self):
        seals = {case["name"]: case for case in self.f["seals"]}
        for case in self.f["rejections"]:
            base = seals[case["seal"]]
            envelope = base64.b64decode(case.get("sealed", base["sealed"]))
            aad = self.aad(case.get("key", base["key"]), case.get("account", base["account"]),
                           case.get("field", base["field"]), case.get("revision", base["revision"]),
                           case.get("endpoint", base["endpoint"]), case.get("profile"))
            if case["error"] == "upgrade":
                self.assertNotEqual(envelope[0], 1, case["name"])
                continue
            if case["error"] == "invalid":
                self.assertLessEqual(len(envelope), 29, case["name"])
                continue
            with self.assertRaises(InvalidTag, msg=case["name"]):
                AESGCM(self.keys[case.get("key", base["key"])]).decrypt(envelope[1:13], envelope[13:], aad.encode())

    def test_vault_files_open_with_their_keys(self):
        vault = json.loads(self.f["vault"]["file"])
        self.assertEqual(compact(vault), self.f["vault"]["file"])
        native = self.f["native_fixture"]
        for document, secrets in ((vault, None), (json.loads(native["vault"]), native)):
            key = self.keys["primary"]
            for entry in document["entries"]:
                if entry.get("removed"):
                    self.assertNotIn("sealed", entry)
                    continue
                envelope = base64.b64decode(entry["sealed"])
                aad = self.aad("primary", entry["account"], entry["field"], entry["revision"], entry["endpoint"])
                secret = AESGCM(key).decrypt(envelope[1:13], envelope[13:], aad.encode()).decode()
                if secrets:
                    self.assertEqual(secret, secrets[entry["field"]])


if __name__ == "__main__":
    unittest.main()
