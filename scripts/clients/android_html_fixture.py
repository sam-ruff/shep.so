#!/usr/bin/env python3
"""Synthetic native HTML cache, handed over before the profile opens."""
import argparse
import json
from pathlib import Path
import sqlite3
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST='files/shep-html-request'
FIXTURE='files/shep-html-fixture.sqlite'

def prepare(path):
    path=Path(path)
    if path.exists(): raise ValueError('Fixture destination must be new')
    raw=(ROOT/'shared/html-reader-fixture.eml').read_bytes()
    plain='Plain alternative: verification needed.\n\nAlpha in plain text.\n> Alpha in plain quoted history.'
    broken=b'Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>Readable damaged message.</p><img src="cid:bad">\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <bad>\r\nContent-Transfer-Encoding: base64\r\n\r\n%%%invalid%%%\r\n--x--\r\n'
    with sqlite3.connect(path) as db:
        db.executescript((ROOT/'flutter/rust/src/schema.sql').read_text())
        account=dict(id='fixture',name='HTML fixture',email='owner@example.test',protocol='Pop3',host='mail.example.test',port=995,username='fixture',smtp_host='mail.example.test',smtp_port=465)
        db.execute('INSERT INTO accounts VALUES(?,?)',('fixture',json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)',('fixture','["INBOX","Archive"]'))
        for id,subject,body,data in [('html','Shep formatted reader fixture',plain,raw),('plain','Plain message fixture','Another plain message.',b'Content-Type: text/plain\r\n\r\nAnother plain message.'),('broken','Damaged HTML fixture','Readable damaged message.',broken)]:
            db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',(f'fixture:INBOX:{id}','fixture',id,'INBOX','Shep preview <preview@example.test>','owner@example.test',subject,body[:120],1788692400,0,0,0,body,data))

class HTMLDriver(AndroidPicker):
    def handover(self):
        output=ROOT/'artifacts/flutter/native/html';output.mkdir(parents=True,exist_ok=True)
        path=output/'fixture.sqlite';path.unlink(missing_ok=True);prepare(path)
        deadline=time.monotonic()+600
        self.adb('shell','run-as',PACKAGE,'rm','-f',REQUEST,FIXTURE,check=False)
        while b'waiting-before-profile-open' not in self.adb('shell','run-as',PACKAGE,'cat',REQUEST,check=False):
            if time.monotonic()>deadline: raise TimeoutError('HTML fixture was not requested')
            time.sleep(.2)
        self.adb('shell','run-as',PACKAGE,'sh','-c',f"'cat > {FIXTURE}.tmp'",data=path.read_bytes())
        self.adb('shell','run-as',PACKAGE,'mv',FIXTURE+'.tmp',FIXTURE)
        while time.monotonic()<deadline:
            state=self.adb('shell','run-as',PACKAGE,'cat',REQUEST,check=False).decode().strip()
            if state.startswith('capture:'):
                name=state.removeprefix('capture:')
                if name not in ('native-formatted-light','native-formatted-find-tail','native-formatted-retry','complete'):
                    raise ValueError('Unexpected capture name')
                if name!='complete': self.capture(name)
                self.adb('shell','run-as',PACKAGE,'sh','-c',f"'echo captured:{name} > {REQUEST}'")
                if name=='complete': return
            time.sleep(.1)
        raise TimeoutError('HTML native controls did not finish')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--device');parser.add_argument('--prepare',type=Path);args=parser.parse_args()
    if args.prepare: prepare(args.prepare)
    elif args.device: HTMLDriver(args.device).handover()
    else: parser.error('Choose --prepare or --device')
