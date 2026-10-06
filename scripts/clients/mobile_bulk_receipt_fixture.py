"""Inject a controlled receipt/cache gap into an isolated native test profile."""
import argparse
import json
import sqlite3


def change(path, release):
    with sqlite3.connect(path) as db:
        fixture = db.execute(
            "SELECT COUNT(*) FROM accounts WHERE id='fixture' "
            "AND json_extract(settings,'$.email')='owner@example.test'"
        ).fetchone()[0]
        if fixture != 1 or db.execute('SELECT COUNT(*) FROM accounts').fetchone()[0] != 1:
            raise ValueError('Only the isolated incoming fixture is supported')
        if release:
            db.execute('DROP TRIGGER fixture_group_cache_failure')
            return
        row = db.execute(
            "SELECT i.mail,i.account,i.lineage,j.approved,j.fields FROM group_items i "
            "JOIN group_jobs j ON j.id=i.job WHERE i.job='ffi-repair' AND i.position=0"
        ).fetchone()
        if row is None or row[0] != 'fixture:INBOX:files':
            raise ValueError('Prepare the exact one-message fixture review first')
        mail, account, lineage, revision, fields = row
        folder, uid, unread, starred = db.execute(
            'SELECT folder,remote_id,unread,starred FROM mail WHERE id=?', (mail,)
        ).fetchone()
        physical = json.dumps({'account': account, 'folder': folder, 'remote_id': uid,
                               'unread': bool(unread), 'starred': bool(starred), 'lineage': lineage})
        db.execute(
            "INSERT INTO individual_mail_actions(id,mail,account,fields,accepted_fields,physical,"
            "intent_revision,status,created,group_job,group_position,group_inverse) "
            "VALUES('ffi-repair-attempt',?,?,?,?,?,?,'repair',1,'ffi-repair',0,0)",
            (mail, account, fields, fields, physical, revision),
        )
        db.execute("INSERT INTO individual_mail_action_receipts VALUES('ffi-repair-attempt',?)",
                   (json.dumps({'kind': 'flags'}),))
        db.execute('INSERT INTO mail_intents(mail,field,revision) VALUES(?,?,?)',
                   (mail, 'unread', revision))
        db.execute("UPDATE group_items SET state='repair',attempt='ffi-repair-attempt',"
                   "reason='The provider acknowledgement is saved. Retry to save it locally.' "
                   "WHERE job='ffi-repair'")
        db.execute("UPDATE group_jobs SET state='paused' WHERE id='ffi-repair'")
        db.execute("CREATE TRIGGER fixture_group_cache_failure BEFORE UPDATE OF unread ON mail "
                   "BEGIN SELECT RAISE(FAIL,'Synthetic persistent cache failure'); END")


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('path')
    parser.add_argument('--release', action='store_true')
    args = parser.parse_args()
    change(args.path, args.release)
