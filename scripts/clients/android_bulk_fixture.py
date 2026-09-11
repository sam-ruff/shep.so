#!/usr/bin/env python3
"""Isolated 130-message POP3 profile for the native group action journal."""
import argparse
import json
from pathlib import Path
import sqlite3
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST = 'files/shep-bulk-request'
FIXTURE = 'files/shep-bulk-fixture.sqlite'
MESSAGES = 130


def prepare(path):
    path = Path(path)
    if path.exists():
        raise ValueError('Fixture destination must be new')
    with sqlite3.connect(path) as db:
        db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
        account = dict(id='fixture', name='Bulk fixture', email='owner@example.test', protocol='Pop3', host='mail.example.test', port=995, username='fixture', smtp_host='mail.example.test', smtp_port=465)
        db.execute('INSERT INTO accounts VALUES(?,?)', ('fixture', json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)', ('fixture', '["INBOX","Archive","Trash"]'))
        base = 1788692400
        for n in range(MESSAGES):
            subject = f'Bulk message {n + 1}'
            body = f'Fictional message {n + 1} for the native group journal.'
            raw = f'From: Robin Field <robin@example.test>\r\nTo: owner@example.test\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\n{body}'.encode()
            db.execute(
                'INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                (f'fixture:INBOX:bulk-{n:03d}', 'fixture', f'bulk-{n:03d}', 'INBOX', 'Robin Field <robin@example.test>', 'owner@example.test', subject, body, base - n * 60, int(n % 2 == 0), int(n % 5 == 0), 0, body, raw),
            )


class BulkFixture(AndroidPicker):
    def run(self):
        output = ROOT / 'artifacts/flutter/native/bulk'
        output.mkdir(parents=True, exist_ok=True)
        source = output / 'fixture.sqlite'
        source.unlink(missing_ok=True)
        prepare(source)
        self.adb('shell', 'run-as', PACKAGE, 'rm', '-f', REQUEST, FIXTURE, check=False)
        deadline = time.monotonic() + 600
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'waiting-before-profile-open':
            if time.monotonic() > deadline:
                raise TimeoutError('Bulk test did not request its isolated fixture')
            time.sleep(.3)
        self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'cat > {FIXTURE}.tmp'", data=source.read_bytes())
        self.adb('shell', 'run-as', PACKAGE, 'mv', f'{FIXTURE}.tmp', FIXTURE)
        deadline = time.monotonic() + 600
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'done':
            if time.monotonic() > deadline:
                raise TimeoutError('Bulk test did not finish with the isolated fixture')
            time.sleep(.5)
        print('Handed over the isolated group journal fixture', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--device')
    parser.add_argument('--prepare', help='Write the fixture SQLite profile to this new path and exit')
    args = parser.parse_args()
    if args.prepare:
        prepare(args.prepare)
    else:
        if not args.device:
            parser.error('--device is required')
        BulkFixture(args.device).run()
