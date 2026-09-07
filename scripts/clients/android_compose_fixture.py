#!/usr/bin/env python3
"""Seed an isolated profile before UI startup and drive Android's real picker.

Used alongside native_mail_test.dart by android_e2e.py. No running app state is
written: the fixture is handed over before NativeRepository opens its database.
All file selection/cancellation uses Android input against DocumentsUI.
"""
import argparse
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = 'so.shep.shep_mobile.preview'
OUTPUT = ROOT / 'artifacts/flutter/native/picker'
REQUEST = 'files/shep-compose-request'
FIXTURE = 'files/shep-compose-fixture.sqlite'


class AndroidPicker:
    def __init__(self, device):
        if not re.fullmatch(r'emulator-\d+', device):
            raise ValueError('A dedicated emulator is required')
        self.prefix = ['adb', '-s', device]
        avd = self.adb('emu', 'avd', 'name').decode().splitlines()[0]
        if not avd.startswith('shep-e2e'):
            raise ValueError('Personal devices and other AVDs are refused')
        OUTPUT.mkdir(parents=True, exist_ok=True)

    def adb(self, *args, data=None, check=True):
        result = subprocess.run(self.prefix + list(args), input=data, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, check=check, timeout=15)
        return result.stdout if result.returncode == 0 else b''

    def window(self):
        self.adb('shell', 'uiautomator', 'dump', '/data/local/tmp/shep-compose-window.xml')
        xml = self.adb('exec-out', 'cat', '/data/local/tmp/shep-compose-window.xml')
        (OUTPUT / 'last-window.xml').write_bytes(xml)
        return ET.fromstring(xml).findall('.//node')

    def wait_for_system_ui(self, nodes):
        if not any(n.attrib.get('text') == "System UI isn't responding" for n in nodes):
            return False
        wait = next((n for n in nodes if n.attrib.get('text') == 'Wait'), None)
        if wait is None:
            return False
        self.tap(wait)
        print('Waited for the dedicated emulator System UI to recover', flush=True)
        return True

    def tap(self, node, long=False):
        bounds = [int(v) for v in re.findall(r'\d+', node.attrib['bounds'])]
        x, y = (bounds[0] + bounds[2]) // 2, (bounds[1] + bounds[3]) // 2
        if long:
            self.adb('shell', 'input', 'swipe', str(x), str(y), str(x), str(y), '800')
        else:
            self.adb('shell', 'input', 'tap', str(x), str(y))

    def capture(self, name):
        png = OUTPUT / f'{name}.png'
        png.write_bytes(self.adb('exec-out', 'screencap', '-p'))
        subprocess.run(['convert', str(png), str(png.with_suffix('.webp'))], check=True)
        png.unlink()

    def prepare(self):
        # These exact files are owned by this saved fixture only.
        self.adb('shell', 'run-as', PACKAGE, 'rm', '-f', REQUEST, FIXTURE, check=False)
        source = OUTPUT / 'fixture.sqlite'
        source.unlink(missing_ok=True)
        case = json.loads((ROOT / 'shared/compose-fixtures.json').read_text())[0]
        with sqlite3.connect(source) as db:
            db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
            for index, email in enumerate(case['own']):
                account = dict(id='fixture' if index == 0 else 'second', name='Fixture account',
                               email=email, protocol='Pop3', host='mail.example.test', port=995,
                               username='fixture', smtp_host='smtp.example.test', smtp_port=465)
                db.execute('INSERT INTO accounts VALUES(?,?)', (account['id'], json.dumps(account)))
            db.execute('INSERT INTO folders VALUES(?,?)', ('fixture', '["INBOX","Archive"]'))
            db.execute('INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                       ('fixture:INBOX:42.7', 'fixture', '42.7', 'INBOX', case['sender'], case['own'][0],
                        case['subject'], 'First line', case['timestamp'], 0, 0, 0, case['text'], case['raw'].encode()))
        for name, data in [('shep-e2e-first.txt', b'Remove this attachment'),
                           ('shep-e2e-binary.bin', bytes([0, 255, 1, 13, 10]))]:
            local = OUTPUT / name
            local.write_bytes(data)
            self.adb('push', str(local), f'/sdcard/Download/{name}')
        return source.read_bytes()

    def run(self):
        fixture = self.prepare()
        deadline = time.monotonic() + 600
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'waiting-before-profile-open':
            if time.monotonic() > deadline:
                raise TimeoutError('Native test never requested its fixture')
            time.sleep(.5)
        # The test is waiting before opening its isolated cache or mounting UI.
        self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'cat > {FIXTURE}.tmp'", data=fixture)
        self.adb('shell', 'run-as', PACKAGE, 'mv', f'{FIXTURE}.tmp', FIXTURE)
        print('Fixture handed over before UI startup', flush=True)
        phase = 'cancel'
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            if phase == 'closed':
                if self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'picker-select':
                    time.sleep(.2)
                    continue
                phase = 'select'
            nodes = self.window()
            if self.wait_for_system_ui(nodes):
                continue
            if not any(n.attrib.get('package', '').endswith('documentsui') for n in nodes):
                time.sleep(.2)
                continue
            if phase == 'cancel':
                self.capture('picker-cancel')
                self.adb('shell', 'input', 'keyevent', '4')
                # The second open can follow the close faster than a UI dump.
                # Observe the test's next picker intent instead of missing that gap.
                phase = 'closed'
                continue
            by_text = lambda value: next((n for n in nodes if n.attrib.get('text') == value), None)
            first, second = by_text('shep-e2e-first.txt'), by_text('shep-e2e-binary.bin')
            if phase == 'select' and first is not None and second is not None:
                self.tap(first, long=True)
                phase = 'second'
                continue
            if phase == 'second' and second is not None:
                self.tap(second)
                phase = 'confirm'
                continue
            if phase == 'confirm':
                select = next((n for n in nodes if n.attrib.get('resource-id', '').endswith('action_menu_select') or n.attrib.get('text', '').upper() in ('SELECT', 'OPEN')), None)
                if select is not None:
                    self.capture('picker-multiple')
                    self.tap(select)
                    print('Cancelled one picker, selected two fixture files through DocumentsUI', flush=True)
                    return
            downloads = by_text('Downloads')
            if downloads is not None and phase == 'select':
                self.tap(downloads)
                continue
            roots = next((n for n in nodes if n.attrib.get('content-desc') == 'Show roots'), None)
            if roots is not None and phase == 'select':
                self.tap(roots)
        self.capture('picker-failure')
        raise TimeoutError(f'DocumentsUI selection stopped at {phase}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--device', required=True)
    picker = AndroidPicker(parser.parse_args().device)
    try:
        picker.run()
    except BaseException:
        # Return control to the isolated Flutter test after a native-picker
        # failure, so its deadline/assertions can report instead of hanging.
        nodes=picker.window()
        if picker.wait_for_system_ui(nodes):
            nodes=picker.window()
        if any(n.attrib.get('package', '').endswith('documentsui') for n in nodes):
            picker.adb('shell', 'input', 'keyevent', '4')
        raise
