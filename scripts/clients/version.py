#!/usr/bin/env python3
"""Stamp one future release version across clients; never build or publish."""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]


def cargo_version(text, version):
    updated, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{version}"', text, count=1)
    if count != 1:
        raise ValueError('Cargo package version is missing')
    return updated


def lock_versions(text, version):
    blocks = text.split('[[package]]')
    for i, block in enumerate(blocks[1:], 1):
        name = re.search(r'(?m)^name = "([^"]+)"$', block)
        if name and name[1] in ('shep-mail-core', 'shep-mail-content', 'shep-profile-core', 'shep-beta-server', 'shep_mobile_native') and '\nsource = ' not in block:
            blocks[i] = cargo_version(block, version)
    return '[[package]]'.join(blocks)


def stamp(version, build, root=ROOT):
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', version) or build < 1:
        raise ValueError('Use a semantic version and a positive mobile build number')
    paths = ['flutter/pubspec.yaml', 'web/package.json', 'web/package-lock.json',
             'website/package.json', 'website/package-lock.json', 'backend/Cargo.toml',
             'shared/mail-core/Cargo.toml', 'flutter/rust/Cargo.toml', 'shared/mail-content/Cargo.toml', 'shared/profile-core/Cargo.toml',
             'backend/Cargo.lock', 'Cargo.lock', 'flutter/rust/Cargo.lock']
    original = {p: (root / p).read_text() for p in paths}
    # Prepare and validate every edit before changing a manifest.
    updated = {}
    flutter, count = re.subn(r'(?m)^version: .*$', f'version: {version}+{build}', original[paths[0]], count=1)
    if count != 1:
        raise ValueError('Flutter version is missing')
    updated[paths[0]] = flutter
    for p in paths[1:5]:
        data = json.loads(original[p])
        data['version'] = version
        if p.endswith('package-lock.json'):
            data['packages']['']['version'] = version
        updated[p] = json.dumps(data, indent=2) + '\n'
    for p in paths[5:10]:
        updated[p] = cargo_version(original[p], version)
    for p in paths[10:]:
        updated[p] = lock_versions(original[p], version)
    for p, text in updated.items():
        (root / p).write_text(text)
    print('Client versions stamped. Root scripts/release.py owns the desktop version; coordinated signed packaging remains a release prerequisite.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('version')
    parser.add_argument('--build', type=int, required=True)
    args = parser.parse_args()
    stamp(args.version, args.build)
