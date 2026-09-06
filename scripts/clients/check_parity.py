#!/usr/bin/env python3
"""Require a recorded client-parity review with desktop changes, not fake parity."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]

def check(base=None):
    scenarios = json.loads((ROOT / 'shared/client-scenarios.json').read_text())
    ids = [row['id'] for row in scenarios['scenarios']]
    if len(ids) != len(set(ids)) or not ids:
        raise ValueError('Client scenario IDs must be nonempty and unique')
    for row in scenarios['scenarios']:
        if not all(row.get(k) for k in ('id', 'contract', 'mobile', 'browser')):
            raise ValueError('Each contract needs mobile and browser evidence or an explicit OPEN gap')
    if base:
        if not re.fullmatch(r'[0-9a-fA-F]{7,40}|HEAD(?:~\d+)?', base):
            raise ValueError('Expected a commit SHA or HEAD revision')
        changed = set(subprocess.check_output(['git', 'diff', '--name-only', base], cwd=ROOT, text=True).splitlines())
        changed.update(subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard'], cwd=ROOT, text=True).splitlines())
        if any(p.startswith(('src/', 'shared/mail-core/', 'shared/mail-content/')) for p in changed) and not changed.intersection(
                {'docs/CLIENT_PARITY.md', 'shared/client-scenarios.json'}):
            raise ValueError('Desktop changed: update docs/CLIENT_PARITY.md and record client behavior/evidence or a TODO gap')
    print(f'Parity review structure valid: {len(ids)} contracts; open gaps are not completion evidence.')

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--base')
    check(parser.parse_args().base)
