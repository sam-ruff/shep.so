#!/usr/bin/env python3
"""Build the isolated mobile browser preview, own its server, run Playwright."""
import argparse
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.request
ROOT=Path(__file__).resolve().parents[2]
LOGS=ROOT/'artifacts/logs'

def main(formatted=False, discovery=False, creation=False):
    LOGS.mkdir(parents=True,exist_ok=True)
    prefix='flutter-creation-web' if creation else 'flutter-discovery-web' if discovery else 'flutter-formatted-web' if formatted else 'flutter-web'
    if formatted:
        subprocess.run([sys.executable,str(ROOT/'scripts/clients/generate_html_fixture.py'),'--check'],cwd=ROOT,check=True)
    with (LOGS/f'{prefix}-build.log').open('w') as log:
        subprocess.run(['flutter','build','web','--target','test/profile_creation_main.dart' if creation else 'test/profile_discovery_main.dart' if discovery else 'test/formatted_main.dart' if formatted else 'test/preview_main.dart','--no-web-resources-cdn'],cwd=ROOT/'flutter',stdout=log,stderr=subprocess.STDOUT,check=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    with (LOGS/f'{prefix}-server.log').open('w') as log:
        process=subprocess.Popen([sys.executable,'-m','http.server',str(port),'--bind','127.0.0.1','--directory',str(ROOT/'flutter/build/web')],stdout=log,stderr=subprocess.STDOUT)
        try:
            deadline=time.monotonic()+15
            while True:
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}',timeout=1):break
                except OSError:pass
                if process.poll() is not None or time.monotonic()>deadline:raise RuntimeError('Preview server did not start')
                time.sleep(.1)
            env=os.environ.copy();env['SHEP_FLUTTER_URL']=f'http://127.0.0.1:{port}'
            with (LOGS/f'{prefix}-e2e.log').open('w') as log:
                subprocess.run(['node','flutter/e2e/profile_creation.mjs','web'] if creation else ['node','flutter/e2e/profile_discovery.mjs','web'] if discovery else ['node','flutter/e2e/formatted.mjs'] if formatted else ['npm','--prefix','flutter/e2e','run','web'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        finally:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
    print('Flutter Playwright scenarios passed; preview server stopped.')
if __name__=='__main__':
    parser=argparse.ArgumentParser()
    group=parser.add_mutually_exclusive_group()
    group.add_argument('--formatted',action='store_true'); group.add_argument('--discovery',action='store_true'); group.add_argument('--creation',action='store_true')
    args=parser.parse_args(); main(args.formatted,args.discovery,args.creation)
