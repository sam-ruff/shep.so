#!/usr/bin/env python3
"""Regenerate the V2 interoperability fixture using libargon2 + Python AESGCM.

Optional developer utility; needs libargon2 and Python cryptography. Never uses
account data. Rust tests consume the committed bytes without Python dependencies.
"""
import ctypes
import ctypes.util
import json
from pathlib import Path
from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def vector():
    salt = bytes(range(16))
    nonce = bytes(range(16, 23))
    password = b"a format fixture passphrase"
    key = ctypes.create_string_buffer(32)
    library = ctypes.CDLL(ctypes.util.find_library("argon2"))
    derive = library.argon2id_hash_raw
    derive.argtypes = [ctypes.c_uint32] * 3 + [ctypes.c_void_p, ctypes.c_size_t] * 3
    derive.restype = ctypes.c_int
    if derive(2, 19 * 1024, 1, password, len(password), salt, len(salt), key, 32):
        raise RuntimeError("Argon2 fixture derivation failed")
    header = b"SHEPBK02" + b"\x02" + bytes(7) + salt + nonce + b"\x00"
    payload = json.dumps(dict(version=1, created_at=1, messages=[], accounts=[],
                              calendars=[], preferences={}, credentials=[]), separators=(",", ":")).encode()
    prefix = (0x80000000 | (len(payload) + 16)).to_bytes(4, "big")
    ciphertext = AESGCM(key.raw).encrypt(nonce + bytes(4) + b"\x01", payload, header + prefix)
    return header + prefix + ciphertext


if __name__ == "__main__":
    path = Path(__file__).resolve().parent.parent / "tests/fixtures/backup-format-v2.hex"
    path.write_text(vector().hex() + "\n")
