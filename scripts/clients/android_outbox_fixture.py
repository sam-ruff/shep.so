#!/usr/bin/env python3
"""Hand an isolated Outbox fixture over before the native profile opens."""
import argparse
from email.message import EmailMessage
from email.policy import SMTP
import json
from pathlib import Path
import re
import shlex
import subprocess
import sqlite3
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST = 'files/shep-outbox-request'
FIXTURE = 'files/shep-outbox-fixture.sqlite'
CHECKED = 'files/shep-outbox-lock-checked'


def prepare(source=None):
    source = source or ROOT / 'artifacts/flutter/native/outbox/fixture.sqlite'
    source.parent.mkdir(parents=True, exist_ok=True)
    source.unlink(missing_ok=True)
    with sqlite3.connect(source) as db:
        db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
        account = dict(id='fixture', name='Native Outbox fixture', email='alex@example.test',
                       protocol='Pop3', host='mail.example.test', port=995,
                       username='fixture', smtp_host='smtp.example.test', smtp_port=465)
        db.execute('INSERT INTO accounts VALUES(?,?)', ('fixture', json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)', ('fixture', '["INBOX","Sent","Archive"]'))
        imap = dict(account, id='imap-fixture', name='Native Sent fixture', protocol='Imap', port=993,
                    sent_copy='Automatic', sent_folder='Sent Mail')
        db.execute('INSERT INTO accounts VALUES(?,?)', ('imap-fixture', json.dumps(imap)))
        db.execute('INSERT INTO folders VALUES(?,?)', ('imap-fixture', '["INBOX","Sent","Sent Mail"]'))
        remote = EmailMessage(policy=SMTP)
        remote['From'] = 'Robin <robin@example.test>'
        remote['To'] = 'alex@example.test'
        remote['Subject'] = 'Native IMAP credential fixture'
        remote.set_content('Cached server mail remains readable without credentials.')
        db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                   ('imap-fixture:Sent:local-sent-provider', 'imap-fixture', '91.5', 'INBOX',
                    str(remote['From']), str(remote['To']), str(remote['Subject']), 'Cached server mail',
                    1788688800, 0, 0, 0, remote.get_content(), remote.as_bytes()))
        entries = [('copy-uncertain', 'delivered'), ('copy-saved', 'delivered'), ('copy-handover', 'delivered'), ('delivered', 'delivered'), ('rejected', 'rejected'),
                   ('mark', 'uncertain'), ('return', 'submitting')]
        for name, state in entries:
            identity = f'outbox-{name}'
            subject = f'Native Outbox {name}'
            owner = 'imap-fixture' if name.startswith('copy-') else 'fixture'
            draft = dict(id=identity, account_id=owner, to='robin@example.test',
                         cc='', bcc='hidden@example.test', subject=subject,
                         body=f'Original native {name} body.', revision=1, attachments=[])
            message = EmailMessage(policy=SMTP)
            message['From'] = 'Alex <alex@example.test>'
            message['To'] = draft['to']
            message['Subject'] = subject
            message['Message-ID'] = f'<{identity}@example.test>'
            message['Date'] = 'Sun, 06 Sep 2026 10:00:00 +0000'
            message.set_content(draft['body'])
            if name == 'return':
                part = dict(id='original-outbox-file', name='review.bin',
                            media_type='application/octet-stream', size=3)
                draft['attachments'] = [part]
                message.add_attachment(bytes([0, 255, 1]), maintype='application',
                                       subtype='octet-stream', filename='review.bin')
            db.execute('INSERT INTO drafts VALUES(?,?,?)', (identity, 1, json.dumps(draft)))
            if name == 'return':
                db.execute('INSERT INTO draft_files VALUES(?,?,?,?,?)',
                           (part['id'], identity, part['name'], part['media_type'], bytes([0, 255, 1])))
            db.execute('INSERT INTO outgoing VALUES(?,?,?,?,?,?,?)',
                       (identity, identity, state, owner, message['Message-ID'], message.as_bytes(), json.dumps(draft)))
            db.execute('INSERT INTO outgoing_meta(id,created,from_address) VALUES(?,?,?)',
                       (identity, 1788688800, 'alex@example.test'))
            if name.startswith('copy-'):
                db.execute('INSERT INTO outgoing_sent(id,account,state,folder,receipt) VALUES(?,?,?,?,?)',
                           (identity, json.dumps(imap), 'saved' if name in ('copy-saved','copy-handover') else 'appending',
                            'Sent Mail', json.dumps(dict(folder='Sent Mail', remote_id='91.4' if name == 'copy-handover' else None)) if name in ('copy-saved','copy-handover') else None))
            if name == 'copy-handover':
                db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                           (f'{owner}:Sent Mail:91.4', owner, '91.4', 'Sent Mail', str(message['From']), str(message['To']), subject, draft['body'], 1788688800, 0, 0, 0, draft['body'], message.as_bytes()))
    return source.read_bytes()


def main(device):
    android = AndroidPicker(device)
    android.adb('shell', 'run-as', PACKAGE, 'rm', '-f', REQUEST, FIXTURE, CHECKED, check=False)
    fixture = prepare()
    deadline = time.monotonic() + 900
    while android.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'waiting-before-profile-open':
        if time.monotonic() > deadline:
            raise TimeoutError('Native Outbox test did not request its fixture')
        time.sleep(.5)
    android.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'cat > {FIXTURE}.tmp'", data=fixture)
    android.adb('shell', 'run-as', PACKAGE, 'mv', f'{FIXTURE}.tmp', FIXTURE)
    print('Synthetic Outbox handed over before NativeRepository opens', flush=True)
    deadline = time.monotonic() + 45
    while True:
        state = android.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).decode().strip()
        if state.startswith('profile-open:'):
            path = state.removeprefix('profile-open:')
            break
        if time.monotonic() > deadline:
            raise TimeoutError('Native profile did not open before lock verification')
        time.sleep(.2)
    pattern = rf'/data/(?:user/0|data)/{re.escape(PACKAGE)}/(?:cache|code_cache)/shep-outbox-fixture-[A-Za-z0-9_-]+/mail\.sqlite3'
    if not re.fullmatch(pattern, path):
        raise ValueError(f'Unexpected synthetic Outbox fixture path: {path!r}')
    def lock(target):
        command = shlex.quote(f'toybox flock -n 9 9<> {shlex.quote(target)}')
        return subprocess.run(android.prefix + ['shell','run-as',PACKAGE,'sh','-c',command],capture_output=True,timeout=15)
    control = lock('files/shep-outbox-control.owner-lock')
    if control.returncode != 0:
        raise RuntimeError('Android lock control failed')
    held = lock(path + '.owner-lock')
    # Toybox returns exactly 1 with no diagnostic for nonblocking EAGAIN;
    # other lock/open errors print a diagnostic. The independent control above
    # proves the same shell, descriptor and syscall path can acquire a lock.
    if held.returncode != 1 or held.stderr.strip() or held.stdout.strip():
        raise RuntimeError(f'Expected a conflicting native profile lock, got exit {held.returncode}: {held.stderr.decode()}')
    android.adb('shell','run-as',PACKAGE,'sh','-c',shlex.quote(f'cat > {CHECKED}'),data=b'PROFILE_LOCK_REFUSED')
    android.adb('shell','run-as',PACKAGE,'rm','-f','files/shep-outbox-control.owner-lock')
    print('Android process lock refused a second owner; independent lock control passed', flush=True)



if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument('--device')
    target.add_argument('--prepare', type=Path, help='Prepare the same isolated fixture for host bridge tests')
    args = parser.parse_args()
    if args.prepare:
        prepare(args.prepare)
    else:
        main(args.device)
