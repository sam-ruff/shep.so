#!/usr/bin/env python3
"""Synthetic complete-source forwards, handed over before the native cache opens."""
import argparse
import base64
import json
from pathlib import Path
import sqlite3
import subprocess
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST = 'files/shep-forward-request'
FIXTURE = 'files/shep-forward-fixture.sqlite'


def prepare(path):
    path = Path(path)
    if path.exists():
        raise ValueError('Fixture destination must be new')
    case = json.loads((ROOT / 'shared/forward-fixtures.json').read_text())[0]
    image = (ROOT / 'assets/logo-light.webp').read_bytes()
    raw = case['raw'].replace('aW5saW5lIGZpeHR1cmU=', base64.b64encode(image).decode()).replace('Content-Type: image/png', 'Content-Type: image/webp')
    broken = raw.replace(base64.b64encode(image).decode(), 'PRIVATE-invalid***')
    long = 'Complete source. ' * 2500 + '\nEND OF COMPLETE ORIGINAL'
    rows = [
        ('source', 'Café project', 'Complete café.', raw),
        ('long', 'Long forward source', 'Short cached preview.', f'Subject: Long forward source\r\n\r\n{long}'),
        ('broken', 'Damaged forward', 'Readable damaged source.', broken),
        ('other', 'Other letter', 'Another message.', 'Subject: Other letter\r\n\r\nAnother message.'),
    ]
    with sqlite3.connect(path) as db:
        db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
        account = dict(id='fixture', name='Forward fixture', email='owner@example.test', protocol='Pop3', host='mail.example.test', port=995, username='fixture', smtp_host='mail.example.test', smtp_port=465)
        db.execute('INSERT INTO accounts VALUES(?,?)', ('fixture', json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)', ('fixture', '["INBOX","Archive"]'))
        for index, (id, subject, body, raw) in enumerate(rows):
            db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                       (f'fixture:INBOX:{id}', 'fixture', id, 'INBOX', 'Sender <sender@example.test>', 'owner@example.test', subject, body, 1788692400 - index, 0, 0, 2 if id == 'source' else 0, body, raw.encode()))


class ForwardDriver(AndroidPicker):
    def handover(self):
        output = ROOT / 'artifacts/flutter/native/forward'
        output.mkdir(parents=True, exist_ok=True)
        path = output / 'fixture.sqlite'
        path.unlink(missing_ok=True)
        prepare(path)
        self.adb('shell', 'run-as', PACKAGE, 'rm', '-f', REQUEST, FIXTURE, check=False)
        deadline = time.monotonic() + 600
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'waiting-before-profile-open':
            if time.monotonic() > deadline:
                raise TimeoutError('Forward fixture was not requested')
            time.sleep(.2)
        self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'cat > {FIXTURE}.tmp'", data=path.read_bytes())
        self.adb('shell', 'run-as', PACKAGE, 'mv', FIXTURE + '.tmp', FIXTURE)
        while time.monotonic() < deadline:
            state = self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).decode().strip()
            if state == 'hide-keyboard':
                self.adb('shell', 'input', 'keyevent', '4')
                self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'echo keyboard-hidden > {REQUEST}'")
            elif state.startswith('capture:'):
                name = state.removeprefix('capture:')
                if name not in ('native-forward-light', 'native-forward-dark', 'native-forward-retry', 'native-forward-independent', 'complete'):
                    raise ValueError('Unexpected capture name')
                if name != 'complete':
                    png = output / f'{name}.png'
                    png.write_bytes(self.adb('exec-out', 'screencap', '-p'))
                    subprocess.run(['convert', str(png), str(png.with_suffix('.webp'))], check=True)
                    png.unlink()
                self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'echo captured:{name} > {REQUEST}'")
                if name == 'complete':
                    return
            time.sleep(.1)
        raise TimeoutError('Native Forward controls did not finish')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--device')
    parser.add_argument('--prepare', type=Path)
    args = parser.parse_args()
    if args.prepare:
        prepare(args.prepare)
    elif args.device:
        ForwardDriver(args.device).handover()
    else:
        parser.error('Choose --prepare or --device')
