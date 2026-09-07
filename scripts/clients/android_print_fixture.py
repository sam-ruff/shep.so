#!/usr/bin/env python3
"""Synthetic native printing via Android's actual printer and Save as PDF UI."""
import argparse
import re
from pathlib import Path
import subprocess
import time
from android_compose_fixture import AndroidPicker, PACKAGE, ROOT
from android_forward_fixture import prepare

REQUEST = 'files/shep-print-request'
FIXTURE = 'files/shep-print-fixture.sqlite'
OUTPUT = ROOT / 'artifacts/flutter/native/print'

class PrintDriver(AndroidPicker):
    def capture_print(self, name):
        png = OUTPUT / f'{name}.png'
        png.write_bytes(self.adb('exec-out', 'screencap', '-p'))
        subprocess.run(['convert', str(png), str(png.with_suffix('.webp'))], check=True)
        png.unlink()

    def acknowledge(self, action):
        self.adb('shell','run-as',PACKAGE,'sh','-c',f"'echo done:{action} > {REQUEST}'")

    def print_dialog(self, action):
        phase = 'printer'
        name = 'shep-e2e-print-' + action.removeprefix('save:') + '.pdf'
        destination = '/sdcard/Download/' + name
        if action != 'cancel': self.adb('shell','rm','-f',destination)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if phase == 'written':
                raw=self.adb('exec-out','cat',destination,check=False)
                if raw.startswith(b'%PDF-'):
                    pdf=OUTPUT/name;pdf.write_bytes(raw)
                    text=subprocess.check_output(['pdftotext',str(pdf),'-']).decode()
                    expected=['Café project','sender@example.test','duplicate.bin','Complete café.'] if action=='save:formatted' else ['END OF COMPLETE ORIGINAL']
                    for marker in expected:
                        if marker not in text: raise AssertionError(f'Missing PDF text: {marker}')
                    if action=='save:long':
                        info=subprocess.check_output(['pdfinfo',str(pdf)]).decode()
                        if int(re.search(r'Pages:\s+(\d+)',info)[1]) <= 1: raise AssertionError('Long print was clipped to one page')
                    if 'private@example.test' in text: raise AssertionError('Bcc leaked into PDF')
                    subprocess.run(['pdftoppm','-f','1','-singlefile','-scale-to','1200','-png',str(pdf),str(OUTPUT/action.replace(':','-'))],check=True)
                    self.acknowledge(action);return
                time.sleep(.2);continue
            nodes=self.window()
            if self.wait_for_system_ui(nodes):continue
            if phase=='printer' and any(n.attrib.get('package')=='com.android.printspooler' for n in nodes):
                if action=='cancel':
                    self.capture_print('native-print-dialog');self.adb('shell','input','keyevent','4');self.acknowledge(action);return
                pdf=next((n for n in nodes if n.attrib.get('text')=='Save as PDF'),None)
                select=next((n for n in nodes if n.attrib.get('text')=='Select a printer'),None)
                if pdf is not None and not any(n.attrib.get('resource-id')=='com.android.printspooler:id/print_button' for n in nodes): self.tap(pdf); continue
                if select is not None: self.tap(select); continue
                button=next((n for n in nodes if n.attrib.get('resource-id')=='com.android.printspooler:id/print_button' and n.attrib.get('enabled')=='true'),None)
                if button is not None:self.capture_print('native-print-ready');self.tap(button);phase='picker';continue
            if phase in ('picker','confirm') and any(n.attrib.get('package','').endswith('documentsui') for n in nodes):
                field=next((n for n in nodes if n.attrib.get('class')=='android.widget.EditText'),None)
                if phase=='picker' and field is not None:
                    self.tap(field);self.adb('shell','input','keycombination','113','29');self.adb('shell','input','text',name);phase='confirm';continue
                if phase=='confirm':
                    save=next((n for n in nodes if n.attrib.get('text','').upper()=='SAVE' and n.attrib.get('enabled')=='true'),None)
                    if save is not None:self.tap(save);phase='written';continue
                downloads=next((n for n in nodes if n.attrib.get('text')=='Downloads'),None)
                roots=next((n for n in nodes if n.attrib.get('content-desc')=='Show roots'),None)
                if downloads is not None:self.tap(downloads)
                elif roots is not None:self.tap(roots)
            time.sleep(.2)
        self.capture_print('native-print-failure');raise TimeoutError(f'Print stopped at {phase}: {action}')

    def run(self):
        OUTPUT.mkdir(parents=True,exist_ok=True)
        source=OUTPUT/'fixture.sqlite';source.unlink(missing_ok=True);prepare(source)
        self.adb('shell','run-as',PACKAGE,'rm','-f',REQUEST,FIXTURE,check=False)
        deadline=time.monotonic()+600
        while self.adb('exec-out','run-as',PACKAGE,'cat',REQUEST,check=False).strip()!=b'waiting-before-profile-open':
            if time.monotonic()>deadline:raise TimeoutError('Print fixture not requested')
            time.sleep(.2)
        self.adb('shell','run-as',PACKAGE,'sh','-c',f"'cat > {FIXTURE}.tmp'",data=source.read_bytes())
        self.adb('shell','run-as',PACKAGE,'mv',FIXTURE+'.tmp',FIXTURE)
        while time.monotonic()<deadline:
            action=self.adb('exec-out','run-as',PACKAGE,'cat',REQUEST,check=False).decode().strip()
            if action in ('cancel','save:formatted','save:long'):self.print_dialog(action)
            elif action=='complete':self.acknowledge(action);return
            time.sleep(.2)
        raise TimeoutError('Print test did not complete')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--device',required=True)
    PrintDriver(parser.parse_args().device).run()
