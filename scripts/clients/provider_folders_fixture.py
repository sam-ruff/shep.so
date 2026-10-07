"""Isolated TLS IMAP destination fixture for the real Flutter native bridge."""
import argparse
import json
import os
import sqlite3
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CERTIFICATE = ROOT / 'shared/mail-core/tests/fixtures/tls-cert.pem'


def prepare(path: str, port: int) -> None:
    destination = Path(path)
    if destination.exists():
        raise ValueError('Fixture destination must be new')
    account = {
        'id': 'fixture', 'name': 'Destination fixture', 'email': 'owner@example.test',
        'protocol': 'Imap', 'host': '127.0.0.1', 'port': port, 'username': 'fixture',
        'smtp_host': '127.0.0.1', 'smtp_port': port,
    }
    raw = b'Subject: Destination fixture\r\n\r\nSynthetic cached message.\r\n'
    with sqlite3.connect(destination) as db:
        db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
        db.execute('INSERT INTO accounts VALUES(?,?)', ('fixture', json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)', ('fixture', '["INBOX"]'))
        db.execute(
            'INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
            ('fixture:INBOX:1.1', 'fixture', '1.1', 'INBOX', 'Fixture <sender@example.test>',
             'owner@example.test', 'Destination fixture', 'Synthetic cached message.',
             1788692400, 1, 0, 0, 'Synthetic cached message.', raw),
        )


def run() -> int:
    environment = dict(os.environ, SSL_CERT_FILE=str(CERTIFICATE), CARGO_BUILD_JOBS='4')
    return subprocess.run(
        ['flutter', 'test', '--no-pub', 'test/native_logical_mail_actions_test.dart', '--reporter', 'expanded'],
        cwd=ROOT / 'flutter', env=environment, check=False,
    ).returncode


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--prepare')
    parser.add_argument('--port', type=int)
    parser.add_argument('--run', action='store_true')
    arguments = parser.parse_args()
    if arguments.prepare and arguments.port:
        prepare(arguments.prepare, arguments.port)
    elif arguments.run:
        raise SystemExit(run())
    else:
        parser.error('Choose --run or --prepare PATH --port PORT')
