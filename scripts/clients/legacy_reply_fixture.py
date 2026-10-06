"""Prepare fictional reply bytes for the separately loaded older native writer."""
import argparse
import json
from pathlib import Path
import sqlite3

ROOT = Path(__file__).resolve().parents[2]


def prepare(path, version, groups=False):
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
        if groups:
            db.execute('INSERT INTO accounts VALUES(?,?)', ('fixture', json.dumps({
                'id': 'fixture', 'name': 'Owned group fixture', 'email': 'owner@example.test',
                'protocol': 'Pop3', 'host': 'mail.example.test', 'port': 995,
                'username': 'fixture', 'smtp_host': 'mail.example.test', 'smtp_port': 465,
            })))
            db.execute("INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,"
                       "preview,timestamp,unread,starred,attachment_count,body,raw) "
                       "VALUES('fixture:INBOX:owned','fixture','owned','INBOX','Fixture','Owner',"
                       "'Owned source','Cached source',1,1,0,0,'Exact body',?)", (b'Exact MIME bytes',))
            lineage = db.execute("SELECT token FROM mail_lineage WHERE id='fixture:INBOX:owned'").fetchone()[0]
            fields = '{"unread":false}'
            physical = json.dumps({'account': 'fixture', 'folder': 'INBOX', 'remote_id': 'owned',
                                   'unread': True, 'starred': False, 'lineage': lineage})
            db.execute("INSERT INTO group_jobs(id,action,fields,state,scope,created,approved,total) "
                       "VALUES('owned-group','{\"kind\":\"read\"}',?,'paused','{}',1,1,1)", (fields,))
            db.execute("INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,"
                       "starred,state,fields,attempt,lineage) VALUES('owned-group',0,"
                       "'fixture:INBOX:owned','fixture','INBOX','owned',1,0,'repair',?,"
                       "'owned-attempt',?)", (fields, lineage))
            db.execute("INSERT INTO individual_mail_actions(id,mail,account,fields,accepted_fields,"
                       "physical,intent_revision,status,created,group_job,group_position,group_inverse) "
                       "VALUES('owned-attempt','fixture:INBOX:owned','fixture',?,?,?,1,'repair',"
                       "1,'owned-group',0,0)", (fields, fields, physical))
            db.execute("INSERT INTO individual_mail_action_receipts VALUES('owned-attempt',?)",
                       ('{ "kind" : "flags" }',))
            db.execute("INSERT INTO mail_intents(mail,field,revision) VALUES('fixture:INBOX:owned','unread',1)")
        db.execute(f'PRAGMA user_version={version}')


def inspect(path):
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as db:
        return dict(version=db.execute('PRAGMA user_version').fetchone()[0],
                    drafts=dict(db.execute('SELECT id,content FROM drafts')),
                    files=[list(row) for row in db.execute(
                        'SELECT id,draft_id,name,media_type,hex(bytes) FROM draft_files')],
                    groups={table: list(db.execute(f'SELECT * FROM {table}')) for table in (
                        'group_jobs', 'group_items', 'individual_mail_actions',
                        'individual_mail_action_receipts', 'mail_intents')})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('path', type=Path)
    parser.add_argument('--prepare', type=int, choices=[25, 26, 27])
    parser.add_argument('--groups', action='store_true')
    args = parser.parse_args()
    if args.prepare is not None:
        prepare(args.path, args.prepare, args.groups)
    print(json.dumps(inspect(args.path)))
