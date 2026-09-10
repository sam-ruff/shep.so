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
            with (LOGS/'android-compose-integration.log').open('w') as driver_log:
                driver=subprocess.Popen([flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/attachments_android_test.dart','-d',device,'--flavor','preview'],cwd=ROOT/'flutter',env=compose_env,stdout=driver_log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    deadline=time.monotonic()+600
                    while driver.poll() is None:
                        if picker.poll() not in (None,0):raise RuntimeError('Compose picker stopped before the UI test finished')
                        if time.monotonic()>deadline:raise TimeoutError('Compose UI test did not finish')
                        time.sleep(.2)
                    if driver.returncode!=0:raise RuntimeError('Compose UI test failed; see android-compose-integration.log')
                finally:
                    if driver.poll() is None:
                        os.killpg(driver.pid,signal.SIGTERM)
                        try:driver.wait(timeout=5)
                        except subprocess.TimeoutExpired:os.killpg(driver.pid,signal.SIGKILL);driver.wait()
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

def forward(device, flutter, env):
    with (LOGS/'android-forward-fixture.log').open('w') as log:
        fixture=subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_forward_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            test_env=env.copy();test_env['SHEP_NATIVE_REPORT']='integration-forward-result'
            with (LOGS/'android-forward-integration.log').open('w') as driver_log:
                driver=subprocess.Popen([flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/forward_android_test.dart','-d',device,'--flavor','preview'],cwd=ROOT/'flutter',env=test_env,stdout=driver_log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    deadline=time.monotonic()+600
                    while driver.poll() is None:
                        if fixture.poll() not in (None,0):raise RuntimeError('Forward fixture stopped before the UI test finished')
                        if time.monotonic()>deadline:raise TimeoutError('Forward UI test did not finish')
                        time.sleep(.2)
                    if driver.returncode!=0:raise RuntimeError('Forward UI test failed; see android-forward-integration.log')
                finally:
                    if driver.poll() is None:
                        os.killpg(driver.pid,signal.SIGTERM)
                        try:driver.wait(timeout=5)
                        except subprocess.TimeoutExpired:os.killpg(driver.pid,signal.SIGKILL);driver.wait()
            if fixture.wait(timeout=15)!=0:raise RuntimeError('Forward fixture failed; see its log')
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:fixture.kill();fixture.wait()

def printing(device, flutter, env):
    with (LOGS/'android-print-fixture.log').open('w') as log:
        fixture=subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_print_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            test_env=env.copy();test_env['SHEP_NATIVE_REPORT']='integration-print-result'
            with (LOGS/'android-print-integration.log').open('w') as driver_log:
                driver=subprocess.Popen([flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/printing_android_test.dart','-d',device,'--flavor','preview'],cwd=ROOT/'flutter',env=test_env,stdout=driver_log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    deadline=time.monotonic()+600
                    while driver.poll() is None:
                        if fixture.poll() not in (None,0):raise RuntimeError('Print fixture stopped before the UI test finished')
                        if time.monotonic()>deadline:raise TimeoutError('Print UI test did not finish')
                        time.sleep(.2)
                    if driver.returncode!=0:raise RuntimeError('Print UI test failed; see android-print-integration.log')
                finally:
                    if driver.poll() is None:
                        os.killpg(driver.pid,signal.SIGTERM)
                        try:driver.wait(timeout=5)
                        except subprocess.TimeoutExpired:os.killpg(driver.pid,signal.SIGKILL);driver.wait()
            if fixture.wait(timeout=15)!=0:raise RuntimeError('Print fixture failed; see its log')
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:fixture.kill();fixture.wait()

def formatted(device, flutter, env):
    with (LOGS/'android-formatted-fixture.log').open('w') as log:
        fixture=subprocess.Popen([sys.executable,str(ROOT/'scripts/clients/android_html_fixture.py'),'--device',device],stdout=log,stderr=subprocess.STDOUT)
        try:
            test_env=env.copy();test_env['SHEP_NATIVE_REPORT']='integration-formatted-result'
            with (LOGS/'android-formatted-integration.log').open('w') as driver_log:
                driver=subprocess.Popen([flutter,'drive','--driver','test_driver/native_driver.dart','--target','integration_test/formatted_android_test.dart','-d',device,'--flavor','preview'],cwd=ROOT/'flutter',env=test_env,stdout=driver_log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    deadline=time.monotonic()+600
                    while driver.poll() is None:
                        if fixture.poll() not in (None,0):raise RuntimeError('Formatted native helper stopped before the UI test finished')
                        if time.monotonic()>deadline:raise TimeoutError('Formatted UI test did not finish')
                        time.sleep(.2)
                    if driver.returncode!=0:raise RuntimeError('Formatted UI test failed; see android-formatted-integration.log')
                finally:
                    if driver.poll() is None:
                        os.killpg(driver.pid,signal.SIGTERM)
                        try:driver.wait(timeout=5)
                        except subprocess.TimeoutExpired:os.killpg(driver.pid,signal.SIGKILL);driver.wait()
            if fixture.wait(timeout=15)!=0:raise RuntimeError('Native selection helper failed; see its log')
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:fixture.kill();fixture.wait()

def appium(device,flutter,env,formatted_reader=False,discovery_reader=False,creation_reader=False,enrollment_reader=False,sync_reader=False):
    if formatted_reader:
        run('android-formatted-fixture-check',[sys.executable,str(ROOT/'scripts/clients/generate_html_fixture.py'),'--check'],env=env)
    # Integration tests replace the APK; rebuild the review entry before Appium.
    run('android-preview-build', [flutter,'build','apk','--debug','--flavor','preview','--target','test/profile_sync_main.dart' if sync_reader else 'test/profile_enrollment_main.dart' if enrollment_reader else 'test/profile_creation_main.dart' if creation_reader else 'test/profile_discovery_main.dart' if discovery_reader else 'test/formatted_main.dart' if formatted_reader else 'test/preview_main.dart'], ROOT/'flutter')
    run('android-install', ['adb','-s',device,'install','-r',str(ROOT/'flutter/build/app/outputs/flutter-apk/app-preview-debug.apk')])
    native_automation(device,env,formatted_reader,discovery_reader,creation_reader,enrollment_reader,sync_reader)

def native_automation(device,env,formatted_reader=False,discovery_reader=False,creation_reader=False,enrollment_reader=False,sync_reader=False):
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
            run('android-sync-appium' if sync_reader else 'android-enrollment-appium' if enrollment_reader else 'android-creation-appium' if creation_reader else 'android-discovery-appium' if discovery_reader else 'android-formatted-appium' if formatted_reader else 'android-appium-e2e',['node','flutter/e2e/profile_sync.mjs','native'] if sync_reader else ['node','flutter/e2e/profile_enrollment.mjs','native'] if enrollment_reader else ['node','flutter/e2e/profile_creation.mjs','native'] if creation_reader else ['node','flutter/e2e/profile_discovery.mjs','native'] if discovery_reader else ['node','flutter/e2e/formatted_native.mjs'] if formatted_reader else ['npm','--prefix','flutter/e2e','run','native'],env=env)
        finally:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill();process.wait()

def google(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-google-result'
    report = ROOT/'artifacts/flutter/native/integration-google-result.json'
    report.unlink(missing_ok=True)
    run('android-google-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/google_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('google_controls') != ['consent-cancel-retry-cleanup', 'pending-browse-changed-choices']:
        raise RuntimeError('Google native scenarios did not report completion; an interrupted driver is not a pass')

def profiles(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-profiles-result'
    report = ROOT/'artifacts/flutter/native/integration-profiles-result.json'
    report.unlink(missing_ok=True)
    run('android-profiles-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/profile_history_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('profile_history') != ['two-device-conflicts-restart']:
        raise RuntimeError('Profile history native scenario did not report completion; an interrupted driver is not a pass')

def discovery(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-discovery-result'
    report = ROOT/'artifacts/flutter/native/integration-discovery-result.json'
    report.unlink(missing_ok=True)
    run('android-discovery-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/profile_discovery_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('profile_discovery') != ['discovery-retry-pages-appearance', 'discovery-pause-browse-resume']:
        raise RuntimeError('Profile discovery controls did not report completion; an interrupted driver is not a pass')

def creation(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-creation-result'
    report = ROOT/'artifacts/flutter/native/integration-creation-result.json'
    report.unlink(missing_ok=True)
    run('android-creation-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/profile_creation_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('profile_creation') != ['creation-review-retry-pages-appearance', 'creation-pause-browse-resume']:
        raise RuntimeError('Profile publication controls did not report completion; an interrupted driver is not a pass')

def enrollment(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-enrollment-result'
    report = ROOT/'artifacts/flutter/native/integration-enrollment-result.json'
    report.unlink(missing_ok=True)
    run('android-enrollment-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/profile_enrollment_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('profile_enrollment') != ['enrollment-review-retry-reconnect', 'enrollment-pause-browse-resume']:
        raise RuntimeError('Profile enrollment controls did not report completion; an interrupted driver is not a pass')

def sync(device, flutter, env):
    test_env = env.copy(); test_env['SHEP_NATIVE_REPORT'] = 'integration-sync-result'
    report = ROOT/'artifacts/flutter/native/integration-sync-result.json'
    report.unlink(missing_ok=True)
    run('android-sync-integration', [flutter, 'drive', '--driver', 'test_driver/native_driver.dart', '--target', 'integration_test/profile_sync_android_test.dart', '-d', device, '--flavor', 'preview'], ROOT/'flutter', env=test_env)
    if not report.exists() or json.loads(report.read_text()).get('profile_sync') != ['sync-controls-conflicts-receipts', 'sync-disconnect-pause']:
        raise RuntimeError('Profile sync controls did not report completion; an interrupted driver is not a pass')

def main(device, flutter='flutter', compose_only=False, outbox_only=False, incoming_only=False, appium_only=False, formatted_only=False, forward_only=False, print_only=False, google_only=False, profiles_only=False, discovery_only=False, creation_only=False, enrollment_only=False, sync_only=False):
    if not device.startswith('emulator-') or not device.removeprefix('emulator-').isdigit():
        raise ValueError('Only an explicit Android emulator is allowed; personal devices are refused')
    avd = subprocess.check_output(['adb', '-s', device, 'emu', 'avd', 'name'], text=True).splitlines()[0]
    if not avd.startswith('shep-e2e'):
        raise ValueError('Use a dedicated AVD named shep-e2e (or shep-e2e-...)')
    env = os.environ.copy()
    env['ANDROID_SERIAL'] = device
    env['APPIUM_HOME'] = str(ROOT / 'artifacts/appium')
    if sync_only:
        sync(device, flutter, env)
        appium(device, flutter, env, sync_reader=True)
        print('Android preference sync controls passed with isolated provider fixtures; live Google remains separate.')
        return
    if enrollment_only:
        enrollment(device, flutter, env)
        appium(device, flutter, env, enrollment_reader=True)
        print('Android enrollment controls passed with isolated provider fixtures; live Google remains separate.')
        return
    if creation_only:
        creation(device, flutter, env)
        appium(device,flutter,env,creation_reader=True)
        print('Android profile publication controls passed with an isolated provider fixture; live Google remains separate.')
        return
    if discovery_only:
        discovery(device, flutter, env)
        appium(device,flutter,env,discovery_reader=True)
        print('Android profile discovery controls passed with an isolated provider fixture; live Google remains separate.')
        return
    if profiles_only:
        profiles(device, flutter, env)
        print('Android profile history integration passed; Settings enrollment and cloud access remain separate work.')
        return
    if google_only:
        google(device, flutter, env)
        print('Android Google consent controls passed; other scenarios were not rerun.')
        return
    if print_only:
        printing(device,flutter,env)
        print('Android native Print/PDF scenario passed; other scenarios were not rerun.')
        return
    if forward_only:
        forward(device,flutter,env)
        print('Android Forward scenario passed; other scenarios were not rerun.')
        return
    if formatted_only:
        formatted(device,flutter,env)
        appium(device,flutter,env,formatted_reader=True)
        print('Android formatted-reader scenario passed; other scenarios were not rerun.')
        return
    if appium_only:
        appium(device,flutter,env)
        print('Android Appium controls passed; integration scenarios were not rerun.')
        return
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
    google(device,flutter,env)
    profiles(device,flutter,env)
    discovery(device,flutter,env)
    appium(device,flutter,env,discovery_reader=True)
    creation(device,flutter,env)
    appium(device,flutter,env,creation_reader=True)
    enrollment(device,flutter,env)
    appium(device,flutter,env,enrollment_reader=True)
    sync(device,flutter,env)
    appium(device,flutter,env,sync_reader=True)
    compose(device,flutter,env)
    forward(device,flutter,env)
    printing(device,flutter,env)
    incoming(device,flutter,env)
    formatted(device,flutter,env)
    appium(device,flutter,env,formatted_reader=True)
    outbox(device,flutter,env)
    captures=ROOT/'artifacts/flutter/native';captures.mkdir(parents=True,exist_ok=True)
    for name in ['native-reader-footer-light','native-reader-footer-dark','native-find-tail','native-find-quoted','native-credential-activation-failure','native-credential-cleanup-retry','native-account-removal-light','native-account-removal-dark','native-account-removal-cleanup','native-incoming-saved','native-sent-handover','sent-handover-reader','sent-handover-undo','native-draft-reopened','native-account-retry','native-production-startup','paged-swipe-undo','native-reply-attachments','native-outbox-review-light','native-outbox-review-dark','native-outbox-recovered-draft','native-outbox-empty','native-outbox-local-sent', 'native-sent-copy-review', 'native-sent-preferences', 'native-imap-local-sent-offline', 'native-imap-local-sent-reopened', 'native-imap-credential-recovery']:
        png=captures/f'{name}.png'
        subprocess.run(['convert',str(png),str(png.with_suffix('.webp'))],check=True)
        png.unlink()
    appium(device,flutter,env)
    print('Android integration and Appium passed sequentially; emulator remains available for review.')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--device',required=True)
    parser.add_argument('--flutter',default='flutter',help='Flutter executable (use an unpacked SDK when Snap bundles incompatible build tools)')
    parser.add_argument('--compose-only',action='store_true',help='Run only the saved native reply/attachment scenario during development')
    parser.add_argument('--outbox-only',action='store_true',help='Run only the saved native Outbox recovery scenario')
    parser.add_argument('--incoming-only',action='store_true',help='Run only the real incoming attachment save/cancel scenario')
    parser.add_argument('--appium-only',action='store_true',help='Rebuild and run the saved Appium controls after targeted integration checks')
    parser.add_argument('--formatted-only',action='store_true',help='Run the actual native formatted-reader and selection scenario')
    parser.add_argument('--forward-only',action='store_true',help='Run complete-source Forward and independent-draft controls')
    parser.add_argument('--print-only',action='store_true',help='Run the actual native printer cancel/retry and PDF scenario')
    parser.add_argument('--google-only',action='store_true',help='Run saved native Google consent/cancellation/cleanup controls')
    parser.add_argument('--profiles-only',action='store_true',help='Run native two-device profile history, conflict and restart integration')
    parser.add_argument('--discovery-only',action='store_true',help='Run native profile discovery retry, paging and pause controls')
    parser.add_argument('--creation-only',action='store_true',help='Run native profile publication review, retry and pause controls')
    parser.add_argument('--enrollment-only',action='store_true',help='Run native profile review, application, retry and reconnect controls')
    parser.add_argument('--sync-only',action='store_true',help='Run native preference sync switches, conflict decisions, lost receipts and disconnect controls')
    args=parser.parse_args()
    if sum([args.compose_only,args.outbox_only,args.incoming_only,args.appium_only,args.formatted_only,args.forward_only,args.print_only,args.google_only,args.profiles_only,args.discovery_only,args.creation_only,args.enrollment_only,args.sync_only])>1: parser.error('Choose only one targeted scenario')
    main(args.device,args.flutter,args.compose_only,args.outbox_only,args.incoming_only,args.appium_only,args.formatted_only,args.forward_only,args.print_only,args.google_only,args.profiles_only,args.discovery_only,args.creation_only,args.enrollment_only,args.sync_only)
