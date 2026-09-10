import hashlib
import json
import os
from pathlib import Path
import plistlib
import pty
import fcntl
import termios
import shutil
import subprocess
import sys
import tarfile
import unittest

from test_release_installer import ReleaseFixture, ROOT
import tempfile


@unittest.skipUnless(sys.platform == "linux" and shutil.which("node"), "macOS shell contract fixtures require Linux and Node")
class MacInstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="shep mac install ")
        self.root = Path(self.directory.name)
        self.fixture = ReleaseFixture()
        self.seed()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.temporary = self.root / "temporary"
        self.temporary.mkdir()
        self.env = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"], TMPDIR=str(self.temporary))
        self.tool("python3", "raise RuntimeError('Installer must not require Python')")
        self.tool("uname", "import sys; print('arm64' if '-m' in sys.argv else 'Darwin')")
        self.tool("curl", f'''import pathlib,sys,urllib.request,urllib.parse
assert '--proto' in sys.argv and '=https' in sys.argv
url=next(a for a in sys.argv if a.startswith('https://'))
path=urllib.parse.urlsplit(url).path
try:
 with urllib.request.urlopen('http://127.0.0.1:{self.fixture.server.server_port}'+path) as response:
  pathlib.Path(sys.argv[sys.argv.index('--output')+1]).write_bytes(response.read())
except Exception: sys.exit(22)
''')
        self.tool("osascript", '''import json,pathlib,subprocess,sys
source=sys.stdin.read()
bootstrap="const fs=require('fs'); global.ObjC={import:()=>{},unwrap:x=>x}; global.$={NSString:{alloc:{initWithContentsOfFileEncodingError:(p)=>fs.readFileSync(p,'utf8')}},NSUTF8StringEncoding:4};"
args=sys.argv[sys.argv.index('-')+1:]
code=bootstrap+source+";process.stdout.write(run("+json.dumps(args)+")+String.fromCharCode(10));"
subprocess.run(['node','-e',code],check=True)
''')
        self.tool("plutil", '''import pathlib,plistlib,sys
args=sys.argv[1:]; path=pathlib.Path(args[-1])
if args[0]=='-create': path.write_bytes(plistlib.dumps({}))
elif args[0]=='-insert':
 data=plistlib.loads(path.read_bytes());data[args[1]]=args[3]=='YES' if args[2]=='-bool' else args[3];path.write_bytes(plistlib.dumps(data))
elif args[0]=='-extract': print(plistlib.loads(path.read_bytes())[args[1]])
else: raise RuntimeError(args)
''')
        self.tool("sips", '''import shutil,sys
assert sys.argv[1]=='-z' and sys.argv[2]==sys.argv[3]
assert int(sys.argv[2]) in (16,32,64,128,256,512,1024)
shutil.copyfile(sys.argv[4],sys.argv[sys.argv.index('--out')+1])
''')
        self.tool("iconutil", '''import pathlib,sys
assert sys.argv[1:3]==['-c','icns']
assert len(list(pathlib.Path(sys.argv[3]).glob('*.png')))==10
pathlib.Path(sys.argv[sys.argv.index('-o')+1]).write_bytes(b'fictional icns output')
''')
        self.arguments = ["bash", str(ROOT / "scripts/install-release-macos.sh"), "--user", "--prefix", str(self.root / "Applications with spaces")]

    def seed(self, binary=b"fictional mac release", entries=None, checksum=None):
        self.fixture.seed(binary, entries, checksum)
        old = self.fixture.name
        name = 'shep-1.2.3-darwin-arm64.tar.gz'
        root = '/sam-ruff/shep.so/releases/download/v1.2.3/'
        payload = self.fixture.files[root + old]
        self.fixture.files[root + name] = payload
        self.fixture.files[root + 'SHA256SUMS'] = f'{checksum or hashlib.sha256(payload).hexdigest()}  {name}\n'.encode()
        metadata = {'tag_name':'v1.2.3', 'assets':[{'name':asset,'browser_download_url':'https://github.com'+root+asset} for asset in (name,'SHA256SUMS')]}
        self.fixture.files['/repos/sam-ruff/shep.so/releases/latest'] = json.dumps(metadata).encode()

    def tool(self, name, body):
        file = self.tools / name
        file.write_text(f'#!{sys.executable}\n'+body+'\n')
        file.chmod(0o755)

    def run_installer(self, success=True, arguments=None):
        result = subprocess.run(arguments or self.arguments, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertEqual(list(self.temporary.iterdir()), [], 'bootstrap must clean its staging')
        return result

    def tearDown(self):
        self.fixture.close()
        self.directory.cleanup()

    def test_native_bundle_metadata_icon_install_and_update_without_python_runtime(self):
        self.run_installer()
        app = self.root / 'Applications with spaces/Shep.app'
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleIdentifier'], 'so.shep.Shep')
        self.assertEqual(info['CFBundleExecutable'], 'shep')
        self.assertEqual(info['CFBundleShortVersionString'], '1.2.3')
        self.assertTrue((app / 'Contents/Resources/shep.icns').is_file())
        self.seed(binary=b'updated mac release')
        self.run_installer()
        self.assertEqual((app / 'Contents/MacOS/shep').read_bytes(), b'updated mac release')
        self.assertEqual(list(app.parent.glob('.shep-install.*')), [])

    def test_bad_checksum_or_symlink_binary_keeps_old_application(self):
        self.run_installer()
        app = self.root / 'Applications with spaces/Shep.app/Contents/MacOS/shep'
        self.seed(checksum='0'*64)
        self.assertIn('checksum mismatch', self.run_installer(False).stderr)
        self.seed(entries=[('shep',(tarfile.SYMTYPE,'/unrelated')),('assets/launcher.png',b'fixture')])
        self.assertIn('non-regular', self.run_installer(False).stderr)
        self.assertEqual(app.read_bytes(), b'fictional mac release')

    def test_failed_atomic_bundle_switch_restores_previous_application(self):
        self.run_installer()
        real_mv = shutil.which('mv')
        self.tool('mv', f'''import os,subprocess,sys
if sys.argv[1].endswith('/New.app'): sys.exit(23)
os.execv({real_mv!r},[{real_mv!r}]+sys.argv[1:])
''')
        self.seed(binary=b'failed new release')
        self.run_installer(False)
        app = self.root / 'Applications with spaces/Shep.app'
        self.assertEqual((app / 'Contents/MacOS/shep').read_bytes(), b'fictional mac release')
        self.assertEqual(list(app.parent.glob('.shep-install.*')), [])

    def test_missing_architecture_and_declined_system_elevation_do_not_install(self):
        self.tool('uname', "import sys; print('unsupported' if '-m' in sys.argv else 'Darwin')")
        self.assertIn('No unique macOS', self.run_installer(False).stderr)
        self.tool('uname', "import sys; print('arm64' if '-m' in sys.argv else 'Darwin')")
        self.tool('id', "print('1000')")
        marker = self.root / 'sudo-args.json'
        self.tool('sudo', f"import json,pathlib,sys; pathlib.Path({str(marker)!r}).write_text(json.dumps(sys.argv[1:])); sys.exit(1)")
        self.run_installer(False, self.arguments[:2]+['--system','--yes'])
        args=json.loads(marker.read_text())
        self.assertEqual(args[:2], ['--','bash'])
        self.assertEqual(args[-1], '/Applications/Shep.app')
        self.assertFalse((self.root / 'Applications with spaces').exists())


    def test_scope_conflict_and_terminal_cancellation_happen_before_download(self):
        self.assertIn('either --user or --system', self.run_installer(False, self.arguments[:2]+['--user','--system']).stderr)
        master, slave = pty.openpty()
        def attach_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        try:
            process = subprocess.Popen(self.arguments[:2], env=self.env, stdin=slave, stdout=slave,
                                       stderr=slave, preexec_fn=attach_terminal)
            os.write(master, b'c\n')
            self.assertEqual(process.wait(timeout=5), 1)
        finally:
            os.close(master)
            os.close(slave)
        self.assertEqual(self.fixture.requests, [])
        self.assertFalse((self.root / 'Applications with spaces').exists())
        self.assertEqual(list(self.temporary.iterdir()), [])

    def test_missing_release_and_unrelated_app_are_preserved(self):
        self.fixture.files.pop('/repos/sam-ruff/shep.so/releases/latest')
        self.assertIn('No release could be downloaded', self.run_installer(False).stderr)
        self.seed()
        app = self.root / 'Applications with spaces/Shep.app'
        (app / 'Contents').mkdir(parents=True)
        info = app / 'Contents/Info.plist'
        original = plistlib.dumps({'CFBundleIdentifier':'org.example.Unrelated'})
        info.write_bytes(original)
        self.assertIn('different application', self.run_installer(False).stderr)
        self.assertEqual(info.read_bytes(), original)

if __name__ == '__main__':
    unittest.main()
