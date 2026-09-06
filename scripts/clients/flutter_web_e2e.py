#!/usr/bin/env python3
"""Build the isolated mobile browser preview, own its server, run Playwright."""
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.request
ROOT=Path(__file__).resolve().parents[2]
LOGS=ROOT/'artifacts/logs'

def main():
    LOGS.mkdir(parents=True,exist_ok=True)
    with (LOGS/'flutter-web-build.log').open('w') as log:
        subprocess.run(['flutter','build','web','--target','test/preview_main.dart','--no-web-resources-cdn'],cwd=ROOT/'flutter',stdout=log,stderr=subprocess.STDOUT,check=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    with (LOGS/'flutter-web-server.log').open('w') as log:
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
            with (LOGS/'flutter-web-e2e.log').open('w') as log:
                subprocess.run(['npm','--prefix','flutter/e2e','run','web'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        finally:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
    print('Flutter Playwright scenarios passed; preview server stopped.')
if __name__=='__main__':main()
