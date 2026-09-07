#!/usr/bin/env python3
"""Inspect the production APK before any distribution; no device or network use."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ABIS = ('arm64-v8a', 'armeabi-v7a', 'x86_64')
MARKERS = (b'OBSOLETE-MIME-ALTERNATIVE', b'Find fixture sentinel.', b'ConnectionFixtureRepository', b'Synthetic activation failure',
           b'Synthetic lost credential save acknowledgment', b'A little room for good ideas', b'Native durable draft',
           b'fixture-only-not-a-real-password', b'test/support/preview_repository.dart',
           b'shep-e2e-first.txt', b'Pending text survives removing a file.',
           b'Sent handover fixture', b'Native Outbox return', b'Native IMAP credential fixture', b'Native Sent fixture', b'Original native copy-saved body.', b'Original native return body.')


def contains(stream, markers):
    tail = b''
    overlap = max(map(len, markers)) - 1
    while chunk := stream.read(64 * 1024):
        data = tail + chunk
        if any(marker in data for marker in markers):
            return True
        tail = data[-overlap:]
    return False


def inspect(apk):
    apk = Path(apk)
    expected = {f'lib/{abi}/libshep_mobile_native.so' for abi in ABIS}
    with zipfile.ZipFile(apk) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError('APK contains duplicate entries')
        if not expected.issubset(names):
            raise ValueError('Production APK must contain Rust libraries for all three Android architectures')
        manifest = archive.read('AndroidManifest.xml')
        encoded = lambda text: (text.encode(), text.encode('utf-16le'))
        if not any(value in manifest for value in encoded('android.permission.INTERNET')):
            raise ValueError('Production manifest lacks Internet permission')
        if any(value in manifest for value in encoded('so.shep.shep_mobile.preview')):
            raise ValueError('Preview packages cannot be distributed as production')
        if not any(value in manifest for value in encoded('so.shep.shep_mobile')):
            raise ValueError('Unexpected application identity')
        binaries = [name for name in names if name.endswith('/kernel_blob.bin') or
                    name.endswith('/libapp.so') or name in expected]
        if len(binaries) <= len(expected):
            raise ValueError('APK has no Flutter entrypoint binary')
        for name in binaries:
            with archive.open(name) as stream:
                if contains(stream, MARKERS):
                    raise ValueError(f'Fictional test data found in {name}')
    with apk.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'apk': str(apk), 'sha256': digest, 'native_libraries': sorted(expected),
            'fixture_markers_absent': True, 'internet_permission': True,
            'signing_verification': 'Separate distribution signing verification is required'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('apk', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    result = json.dumps(inspect(args.apk), indent=2) + '\n'
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(result)
    print(result, end='')
