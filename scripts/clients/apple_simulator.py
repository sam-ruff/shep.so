#!/usr/bin/env python3
"""Create/own one iOS Simulator, run the shared Flutter integration scenarios."""
import json
from pathlib import Path
import platform
import subprocess
import uuid
ROOT=Path(__file__).resolve().parents[2]

def main():
    if platform.system()!='Darwin': raise SystemExit('iOS Simulator requires macOS with Xcode; no Apple test was run.')
    runtimes=json.loads(subprocess.check_output(['xcrun','simctl','list','runtimes','--json']))['runtimes']
    choices=[r for r in runtimes if r.get('isAvailable') and r['name'].startswith('iOS')]
    if not choices: raise SystemExit('Install an iOS Simulator runtime in Xcode first.')
    runtime=max(choices,key=lambda r:tuple(int(v) for v in r['version'].split('.')))
    types=json.loads(subprocess.check_output(['xcrun','simctl','list','devicetypes','--json']))['devicetypes']
    phones=[d for d in types if d['name'].startswith('iPhone')]
    if not phones: raise SystemExit('No iPhone simulator device type is installed.')
    device_type=next((d for d in phones if d['name']=='iPhone 16'),phones[-1])
    name=f'shep-e2e-{uuid.uuid4().hex[:10]}'
    device=subprocess.check_output(['xcrun','simctl','create',name,device_type['identifier'],runtime['identifier']],text=True).strip()
    try:
        subprocess.run(['xcrun','simctl','boot',device],check=True)
        subprocess.run(['xcrun','simctl','bootstatus',device,'-b'],check=True)
        log=ROOT/'artifacts/logs/apple-integration.log';log.parent.mkdir(parents=True,exist_ok=True)
        with log.open('w') as output:
            for scenario in ['mail_test.dart','native_mail_test.dart']:
                subprocess.run((['flutter','drive','--driver','test_driver/native_driver.dart','--target',f'integration_test/{scenario}','-d',device] if scenario=='native_mail_test.dart' else ['flutter','test',f'integration_test/{scenario}','-d',device]),cwd=ROOT/'flutter',stdout=output,stderr=subprocess.STDOUT,check=True)
    finally:
        # Only this invocation's newly created UUID is touched.
        subprocess.run(['xcrun','simctl','shutdown',device],check=False)
        subprocess.run(['xcrun','simctl','delete',device],check=False)
    print('Apple simulator integration passed.')
if __name__=='__main__':main()
