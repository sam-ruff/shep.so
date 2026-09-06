#!/usr/bin/env python3
"""Run integration then Appium on one explicit isolated Android emulator."""
import argparse
import json
import os
from pathlib import Path
import socket
import signal
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
LOGS = ROOT / 'artifacts/logs'

def run(name, args, cwd=ROOT, env=None):
    LOGS.mkdir(parents=True, exist_ok=True)
    with (LOGS / f'{name}.log').open('w') as log:
        subprocess.run(args, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)

def compose(device, flutter, env):
    with (LOGS/'android-compose-picker.log').open('w') as log:
        picker = subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_compose_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            compose_env=env.copy();compose_env['SHEP_NATIVE_REPORT']='integration-compose-result'
            run('android-compose-integration', [flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/attachments_android_test.dart','-d',device,'--flavor','preview'], ROOT/'flutter', env=compose_env)
            if picker.wait(timeout=15) != 0:
                raise RuntimeError('Android file picker failed; see android-compose-picker.log')
        finally:
            if picker.poll() is None:
                picker.terminate()
                try: picker.wait(timeout=5)
                except subprocess.TimeoutExpired: picker.kill();picker.wait()

def outbox(device, flutter, env):
    with (LOGS/'android-outbox-fixture.log').open('w') as log:
        fixture = subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_outbox_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            test_env=env.copy();test_env['SHEP_NATIVE_REPORT']='integration-outbox-result'
            run('android-outbox-integration', [flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/outbox_android_test.dart','-d',device,'--flavor','preview'], ROOT/'flutter', env=test_env)
            if fixture.wait(timeout=15) != 0:
                raise RuntimeError('Outbox fixture handover failed; see android-outbox-fixture.log')
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try: fixture.wait(timeout=5)
                except subprocess.TimeoutExpired: fixture.kill();fixture.wait()

def incoming(device, flutter, env):
    with (LOGS/'android-incoming-fixture.log').open('w') as log:
        fixture=subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_incoming_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            test_env=env.copy();test_env['SHEP_NATIVE_REPORT']='integration-incoming-result'
            with (LOGS/'android-incoming-integration.log').open('w') as driver_log:
                driver=subprocess.Popen([flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/incoming_android_test.dart','-d',device,'--flavor','preview'],cwd=ROOT/'flutter',env=test_env,stdout=driver_log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    deadline=time.monotonic()+600
                    while driver.poll() is None:
                        if fixture.poll() not in (None,0):raise RuntimeError('Incoming picker stopped before the UI test finished')
                        if time.monotonic()>deadline:raise TimeoutError('Incoming UI test did not finish')
                        time.sleep(.2)
                    if driver.returncode!=0:raise RuntimeError('Incoming UI test failed; see android-incoming-integration.log')
                finally:
                    if driver.poll() is None:
                        os.killpg(driver.pid,signal.SIGTERM)
                        try:driver.wait(timeout=5)
                        except subprocess.TimeoutExpired:os.killpg(driver.pid,signal.SIGKILL);driver.wait()
            if fixture.wait(timeout=15)!=0:raise RuntimeError('Incoming file picker failed; see its log')
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:fixture.kill();fixture.wait()

def main(device, flutter='flutter', compose_only=False, outbox_only=False, incoming_only=False):
    if not device.startswith('emulator-') or not device.removeprefix('emulator-').isdigit():
        raise ValueError('Only an explicit Android emulator is allowed; personal devices are refused')
    avd = subprocess.check_output(['adb', '-s', device, 'emu', 'avd', 'name'], text=True).splitlines()[0]
    if not avd.startswith('shep-e2e'):
        raise ValueError('Use a dedicated AVD named shep-e2e (or shep-e2e-...)')
    env = os.environ.copy()
    env['ANDROID_SERIAL'] = device
    env['APPIUM_HOME'] = str(ROOT / 'artifacts/appium')
    if incoming_only:
        incoming(device,flutter,env)
        print('Android incoming-attachment scenario passed; other scenarios were not rerun.')
        return
    if outbox_only:
        outbox(device,flutter,env)
        print('Android Outbox scenario passed; other scenarios were not rerun.')
        return
    if compose_only:
        compose(device,flutter,env)
        print('Android compose scenario passed; other scenarios were not rerun.')
        return
    run('android-integration', [flutter,'test','integration_test/mail_test.dart','-d',device,'--flavor','preview'], ROOT/'flutter')
    run('android-native-integration', [flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/native_mail_test.dart','-d',device,'--flavor','preview'], ROOT/'flutter')
    compose(device,flutter,env)
    incoming(device,flutter,env)
    outbox(device,flutter,env)
    captures=ROOT/'artifacts/flutter/native';captures.mkdir(parents=True,exist_ok=True)
    for name in ['native-account-removal-light','native-account-removal-dark','native-account-removal-cleanup','native-incoming-saved','native-sent-handover','sent-handover-reader','sent-handover-undo','native-draft-reopened','native-account-retry','native-production-startup','paged-swipe-undo','native-reply-attachments','native-outbox-review-light','native-outbox-review-dark','native-outbox-recovered-draft','native-outbox-empty','native-outbox-local-sent', 'native-sent-copy-review', 'native-sent-preferences', 'native-imap-local-sent-offline', 'native-imap-local-sent-reopened', 'native-imap-credential-recovery']:
        png=captures/f'{name}.png'
        subprocess.run(['convert',str(png),str(png.with_suffix('.webp'))],check=True)
        png.unlink()
    # Integration tests replace the APK; rebuild the review entry before Appium.
    run('android-preview-build', [flutter,'build','apk','--debug','--flavor','preview','--target','test/preview_main.dart'], ROOT/'flutter')
    run('android-install', ['adb','-s',device,'install','-r',str(ROOT/'flutter/build/app/outputs/flutter-apk/app-preview-debug.apk')])
    manifest=ROOT/'artifacts/appium/node_modules/.cache/appium/extensions.yaml'
    if not manifest.exists():
        run('appium-driver-install',['appium','driver','install','uiautomator2@4.2.9'],env=env)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    env['APPIUM_PORT']=str(port)
    with (LOGS/'android-appium.log').open('w') as log:
        process=subprocess.Popen(['appium','--address','127.0.0.1','--port',str(port)],env=env,stdout=log,stderr=subprocess.STDOUT)
        try:
            deadline=time.monotonic()+30
            while True:
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/status',timeout=1) as response:
                        if json.load(response)['value']['ready']: break
                except (OSError,ValueError,KeyError): pass
                if process.poll() is not None or time.monotonic()>deadline: raise RuntimeError('Appium did not start; see its log')
                time.sleep(.2)
            run('android-appium-e2e',['npm','--prefix','flutter/e2e','run','native'],env=env)
        finally:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill();process.wait()
    print('Android integration and Appium passed sequentially; emulator remains available for review.')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--device',required=True)
    parser.add_argument('--flutter',default='flutter',help='Flutter executable (use an unpacked SDK when Snap bundles incompatible build tools)')
    parser.add_argument('--compose-only',action='store_true',help='Run only the saved native reply/attachment scenario during development')
    parser.add_argument('--outbox-only',action='store_true',help='Run only the saved native Outbox recovery scenario')
    parser.add_argument('--incoming-only',action='store_true',help='Run only the real incoming attachment save/cancel scenario')
    args=parser.parse_args()
    if sum([args.compose_only,args.outbox_only,args.incoming_only])>1: parser.error('Choose only one targeted scenario')
    main(args.device,args.flutter,args.compose_only,args.outbox_only,args.incoming_only)
