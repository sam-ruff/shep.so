#!/usr/bin/env python3
"""Hand an isolated calendar profile to the saved Android controls."""
import argparse
import json
import sqlite3
import time

from android_compose_fixture import AndroidPicker, PACKAGE, ROOT

REQUEST = 'files/shep-calendar-request'
FIXTURE = 'files/shep-calendar-fixture.sqlite'


class CalendarFixture(AndroidPicker):
    def run(self):
        output = ROOT / 'artifacts/flutter/native/calendar'
        output.mkdir(parents=True, exist_ok=True)
        source = output / 'fixture.sqlite'
        source.unlink(missing_ok=True)
        with sqlite3.connect(source) as db:
            db.executescript((ROOT / 'flutter/rust/src/schema.sql').read_text())
            db.execute('INSERT INTO calendar_sources(id,source) VALUES(?,?)',
                       ('primary', json.dumps(dict(id='primary', name='Personal', read_only=False))))
            db.execute('INSERT INTO calendar_binding(id,subject) VALUES(1,?)', ('fixture-google-user',))
        self.adb('shell', 'run-as', PACKAGE, 'rm', '-f', REQUEST, FIXTURE, check=False)
        deadline = time.monotonic() + 900
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'waiting-before-profile-open':
            if time.monotonic() > deadline:
                raise TimeoutError('Calendar controls did not request their isolated profile')
            time.sleep(.3)
        self.adb('shell', 'run-as', PACKAGE, 'sh', '-c', f"'cat > {FIXTURE}.tmp'", data=source.read_bytes())
        self.adb('shell', 'run-as', PACKAGE, 'mv', f'{FIXTURE}.tmp', FIXTURE)
        deadline = time.monotonic() + 900
        while self.adb('exec-out', 'run-as', PACKAGE, 'cat', REQUEST, check=False).strip() != b'done':
            if time.monotonic() > deadline:
                raise TimeoutError('Calendar controls did not finish')
            time.sleep(.5)
        print('Calendar fixture controls finished', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--device', required=True)
    CalendarFixture(parser.parse_args().device).run()
