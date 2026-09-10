#!/usr/bin/env python3
"""Isolated cached incoming files and real DocumentsUI Save/Cancel controls."""
import argparse
import json
from pathlib import Path
import sqlite3
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST='files/shep-incoming-request'
FIXTURE='files/shep-incoming-fixture.sqlite'
DESTINATION='/sdcard/Download/shep-e2e-saved-binary.bin'

def prepare(path):
    path=Path(path)
    if path.exists(): raise ValueError('Fixture destination must be new')
    case=json.loads((ROOT/'shared/attachment-fixtures.json').read_text())[0]
    body=json.loads((ROOT/'shared/find-preview.json').read_text())['body']
    raw=case['raw'].replace('Cached incoming files.',body.replace('\n','\r\n'))
    with sqlite3.connect(path) as db:
        db.executescript((ROOT/'flutter/rust/src/schema.sql').read_text())
        account=dict(id='fixture',name='Incoming fixture',email='owner@example.test',protocol='Pop3',host='mail.example.test',port=995,username='fixture',smtp_host='mail.example.test',smtp_port=465)
        db.execute('INSERT INTO accounts VALUES(?,?)',('fixture',json.dumps(account)))
        db.execute('INSERT INTO folders VALUES(?,?)',('fixture','["INBOX","Archive"]'))
        db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',('fixture:INBOX:files','fixture','files','INBOX','Files <files@example.test>','owner@example.test','Incoming files fixture','Cached incoming files.',1788692400,1,0,3,body,raw.encode()))
        draft=dict(id='removal-draft',account_id='fixture',to='recipient@example.test',cc='',bcc='',subject='Account removal draft',body='Synthetic draft retained until removal.',revision=1)
        db.execute('INSERT INTO drafts VALUES(?,?,?)',(draft['id'],1,json.dumps(draft)))


class IncomingPicker(AndroidPicker):
    def run(self):
        output=ROOT/'artifacts/flutter/native/incoming';output.mkdir(parents=True,exist_ok=True)
        source=output/'fixture.sqlite';source.unlink(missing_ok=True);prepare(source)
        self.adb('shell','run-as',PACKAGE,'rm','-f',REQUEST,FIXTURE,check=False)
        self.adb('shell','rm','-f',DESTINATION)
        deadline=time.monotonic()+600
        while self.adb('exec-out','run-as',PACKAGE,'cat',REQUEST,check=False).strip()!=b'waiting-before-profile-open':
            if time.monotonic()>deadline:raise TimeoutError('Incoming test did not request its isolated fixture')
            time.sleep(.3)
        self.adb('shell','run-as',PACKAGE,'sh','-c',f"'cat > {FIXTURE}.tmp'",data=source.read_bytes())
        self.adb('shell','run-as',PACKAGE,'mv',f'{FIXTURE}.tmp',FIXTURE)
        phase='await-cancel';deadline=time.monotonic()+150
        while time.monotonic()<deadline:
            if phase=='await-cancel':
                if self.adb('exec-out','run-as',PACKAGE,'cat',REQUEST,check=False).strip()!=b'cancel-file':time.sleep(.2);continue
                phase='cancel'
            if phase=='await-save':
                if self.adb('exec-out','run-as',PACKAGE,'cat',REQUEST,check=False).strip()!=b'save-file':time.sleep(.2);continue
                phase='save'
            if phase=='written':
                if self.adb('exec-out','cat',DESTINATION,check=False)==bytes([0,255,1,13,10]):
                    print('Cancelled and saved exact incoming binary through DocumentsUI',flush=True);return
                time.sleep(.2);continue
            nodes=self.window()
            if self.wait_for_system_ui(nodes):continue
            if not any(n.attrib.get('package','').endswith('documentsui')for n in nodes):time.sleep(.2);continue
            if phase=='cancel':
                self.capture('incoming-save-cancel');self.adb('shell','input','keyevent','4');phase='await-save';continue
            text=lambda value:next((n for n in nodes if n.attrib.get('text')==value),None)
            field=next((n for n in nodes if n.attrib.get('class')=='android.widget.EditText'),None)
            if phase=='save' and field is not None:
                self.tap(field);self.adb('shell','input','keycombination','113','29');self.adb('shell','input','text','shep-e2e-saved-binary.bin');phase='confirm';continue
            if phase=='confirm':
                save=next((n for n in nodes if n.attrib.get('text','').upper()=='SAVE' and n.attrib.get('enabled')=='true'),None)
                if save is not None:
                    self.capture('incoming-save-destination');self.tap(save);phase='written';continue
            downloads=text('Downloads')
            if downloads is not None:self.tap(downloads);continue
            roots=next((n for n in nodes if n.attrib.get('content-desc')=='Show roots'),None)
            if roots is not None:self.tap(roots)
        self.capture('incoming-save-failure');raise TimeoutError(f'Incoming picker stopped at {phase}')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--device');parser.add_argument('--prepare');args=parser.parse_args()
    if args.prepare:prepare(args.prepare)
    elif args.device:IncomingPicker(args.device).run()
    else:parser.error('Choose --device or --prepare')
