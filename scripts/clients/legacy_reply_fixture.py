"""Prepare fictional reply bytes for the separately loaded older native writer."""
import argparse
import json
from pathlib import Path
import sqlite3

ROOT = Path(__file__).resolve().parents[2]


def prepare(path, version):
    with sqlite3.connect(path) as db:
        db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
        db.executescript((ROOT / 'flutter/rust/src/folders/schema.sql').read_text())
        draft = dict(id='modern-reply', account_id='', to='recipient@example.test',
                     subject='Reply compatibility', body='Typed answer', revision=1,
                     bcc='private@example.test', references=['<original@example.test>'],
                     in_reply_to='<original@example.test>', reply_context=dict(
                         account_id='fixture', mail_id='cached-source',
                         quote='\n\nOriginal retained exactly.\n> Earlier thread',
                         include_quote=True))
        db.execute('INSERT INTO drafts VALUES(?,?,?)',
                   (draft['id'], draft['revision'], json.dumps(draft, indent=2)))
        db.execute('INSERT INTO draft_files VALUES(?,?,?,?,?)',
                   ('exact-file', draft['id'], 'proof.bin', 'application/octet-stream', b'\x00\xff\r\n'))
        db.execute(f'PRAGMA user_version={version}')


def inspect(path):
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as db:
        return dict(version=db.execute('PRAGMA user_version').fetchone()[0],
                    drafts=dict(db.execute('SELECT id,content FROM drafts')),
                    files=[list(row) for row in db.execute(
                        'SELECT id,draft_id,name,media_type,hex(bytes) FROM draft_files')])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('path', type=Path)
    parser.add_argument('--prepare', type=int, choices=[25, 26])
    args = parser.parse_args()
    if args.prepare is not None:
        prepare(args.path, args.prepare)
    print(json.dumps(inspect(args.path)))
