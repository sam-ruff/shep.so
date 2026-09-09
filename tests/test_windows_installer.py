"""Execute the native PowerShell installer with owned transport/OS boundaries.

Linux PowerShell runs are filesystem/contract evidence, not Windows COM/UAC proof.
The default Windows runner uses its built-in PowerShell and tar.exe. No fixture
installs into an actual user/system application directory or invokes real UAC.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

from test_release_installer import ReleaseFixture, ROOT

PWSH = os.environ.get('SHEP_POWERSHELL') or shutil.which('pwsh') or shutil.which('powershell.exe')
if not PWSH and (ROOT / 'artifacts/tooling/powershell/runtime/pwsh').is_file():
    PWSH = str(ROOT / 'artifacts/tooling/powershell/runtime/pwsh')
TAR = shutil.which('tar.exe') or shutil.which('tar')


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


@unittest.skipUnless(PWSH and TAR, 'Windows installer contracts require PowerShell and tar')
class WindowsInstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="shep windows user's install ")
        self.root = Path(self.directory.name)
        self.fixture = ReleaseFixture()
        self.temporary = self.root / 'temporary'
        self.temporary.mkdir()
        self.app = self.root / 'User apps/Shep'
        self.programs = self.root / 'User Start Menu'
        self.env = dict(os.environ, TMPDIR=str(self.temporary), TEMP=str(self.temporary), TMP=str(self.temporary))
        self.seed()

    def seed(self, binary=b'fictional windows binary\x00\xff\xfe\x01', entries=None, checksum=None):
        if entries is None:
            entries = [('shep.exe', binary), ('assets/launcher.png', (ROOT / 'assets/launcher.png').read_bytes())]
        self.fixture.seed(entries=entries)
        name = 'shep-1.2.3-windows-amd64.tar.gz'
        root = '/sam-ruff/shep.so/releases/download/v1.2.3/'
        payload = self.fixture.files[root + self.fixture.name]
        self.fixture.files[root + name] = payload
        self.fixture.files[root + 'SHA256SUMS'] = f'{checksum or hashlib.sha256(payload).hexdigest()}  {name}\n'.encode()
        metadata = {'tag_name':'v1.2.3', 'draft':False, 'assets':[{'name':asset, 'browser_download_url':'https://github.com'+root+asset} for asset in (name,'SHA256SUMS')]}
        self.fixture.files['/repos/sam-ruff/shep.so/releases/latest'] = json.dumps(metadata).encode()
        self.fixture.files['/repos/sam-ruff/shep.so/releases/tags/v1.2.3'] = json.dumps(metadata).encode()

    def execute(self, command='Invoke-ShepInstall -User', setup='', success=True):
        script = self.root / 'fixture.ps1'
        script.write_text(f'''$ErrorActionPreference = 'Stop'
. {literal(ROOT / 'scripts/install-release-windows.ps1')}
function Get-ShepEnvironment {{
 [PSCustomObject]@{{ Tar={literal(TAR)}; Architecture='amd64'; Interactive=$false;
 UserApplication={literal(self.app)}; UserPrograms={literal(self.programs)};
 SystemApplication={literal(self.root / 'System apps/Shep')}; SystemPrograms={literal(self.root / 'System Start Menu')} }}
}}
$nativeDownload=${{function:Receive-ShepDownload}}
function Receive-ShepDownload {{
 param([string]$Url,[string]$Destination)
 if (!$Url.StartsWith('https://api.github.com/repos/sam-ruff/shep.so/') -and !$Url.StartsWith('https://github.com/sam-ruff/shep.so/releases/download/')) {{ throw 'Unexpected production URL' }}
 $path=([Uri]$Url).AbsolutePath
 Invoke-WebRequest -UseBasicParsing -Uri ('http://127.0.0.1:{self.fixture.server.server_port}'+$path) -OutFile $Destination
}}
function New-ShepShortcut {{
 param([string]$Path,[string]$Application)
 @{{ TargetPath=(Join-Path $Application 'shep.exe'); WorkingDirectory=$Application; IconLocation=(Join-Path $Application 'shep.ico')+',0' }} | ConvertTo-Json | Set-Content -LiteralPath $Path -Encoding UTF8
}}
function Test-ShepAdministrator {{ $false }}
$env:SystemRoot = {literal(self.root / 'Fictional Windows')}
{setup}
{command}
''')
        result = subprocess.run([PWSH, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', str(script)], env=self.env,
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertEqual(list(self.temporary.iterdir()), [], 'release staging must be cleaned')
        return result

    def tearDown(self):
        self.fixture.close()
        self.directory.cleanup()

    def test_binary_safe_install_native_identity_icon_and_atomic_update(self):
        self.execute()
        self.assertEqual((self.app / 'shep.exe').read_bytes(), b'fictional windows binary\x00\xff\xfe\x01')
        marker = json.loads((self.app / 'installation.json').read_text(encoding='utf-8-sig'))
        self.assertEqual(marker, {'application_id':'so.shep.Shep', 'version':'1.2.3'})
        shortcut = json.loads((self.programs / 'Shep.lnk').read_text(encoding='utf-8-sig'))
        self.assertEqual(shortcut['TargetPath'], str(self.app / 'shep.exe'))
        self.assertEqual(shortcut['IconLocation'], str(self.app / 'shep.ico')+',0')
        ico = (self.app / 'shep.ico').read_bytes()
        self.assertEqual(ico[:6], b'\x00\x00\x01\x00\x01\x00')
        self.assertEqual(ico[6:8], b'\x80\x80')
        self.assertEqual(ico[22:], (ROOT / 'assets/launcher.png').read_bytes())
        self.seed(binary=b'updated release\x00\x81')
        self.execute('Invoke-ShepInstall -User -Version 1.2.3')
        self.assertEqual((self.app / 'shep.exe').read_bytes(), b'updated release\x00\x81')
        self.assertEqual(list(self.app.parent.glob('.shep-install-*')), [])

    def test_corrupt_download_and_archive_links_preserve_installation(self):
        self.execute()
        original = (self.app / 'shep.exe').read_bytes()
        self.seed(checksum='0'*64)
        self.assertIn('checksum mismatch', self.execute(success=False).stderr)
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
            self.seed(entries=[('shep.exe',(kind,'unrelated')),('assets/launcher.png',b'fixture')])
            self.assertIn('non-regular', self.execute(success=False).stderr)
        self.assertEqual((self.app / 'shep.exe').read_bytes(), original)

    def test_shortcut_failure_after_bundle_switch_rolls_back_old_application(self):
        self.execute()
        original = (self.app / 'shep.exe').read_bytes()
        (self.programs / 'Shep.lnk').unlink()
        (self.programs / 'Shep.lnk').mkdir()  # A directory cannot atomically replace a .lnk file.
        self.seed(binary=b'new release that cannot finish its launcher')
        self.assertIn('previous application has been kept', self.execute(success=False).stderr)
        self.assertEqual((self.app / 'shep.exe').read_bytes(), original)
        self.assertTrue((self.programs / 'Shep.lnk').is_dir())
        self.assertEqual(list(self.app.parent.glob('.shep-install-*')), [])

    def test_system_elevation_cancel_and_success_use_only_staged_literal_paths(self):
        setup = '''function Start-Process {
 param($FilePath,$Verb,$ArgumentList,[switch]$Wait,[switch]$PassThru)
 if ($Verb -ne 'RunAs' -or !$Wait -or !$PassThru) { throw 'Wrong elevation contract' }
 throw 'fixture UAC cancellation'
}'''
        self.assertIn('approval was cancelled', self.execute('Invoke-ShepInstall -AllUsers -Yes', setup, False).stderr)
        self.assertFalse((self.root / 'System apps').exists())
        setup = '''function Start-Process {
 param($FilePath,$Verb,$ArgumentList,[switch]$Wait,[switch]$PassThru)
 if ($Verb -ne 'RunAs' -or $ArgumentList[-2] -ne '-EncodedCommand') { throw 'Wrong elevation contract' }
 $code=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($ArgumentList[-1]))
 & ([scriptblock]::Create($code))
 [PSCustomObject]@{ ExitCode=0 }
}'''
        self.execute('Invoke-ShepInstall -AllUsers -Yes', setup)
        self.assertTrue((self.root / 'System apps/Shep/shep.exe').is_file())
        self.assertTrue((self.root / 'System Start Menu/Shep.lnk').is_file())
        self.assertFalse(self.app.exists())

    def test_cancel_scope_invalid_version_and_missing_platform_do_not_install(self):
        self.assertIn('either -User or -AllUsers', self.execute('Invoke-ShepInstall -User -AllUsers', success=False).stderr)
        self.assertIn('Use a version', self.execute("Invoke-ShepInstall -User -Version '../other'", success=False).stderr)
        setup = '''$baseEnvironment=${function:Get-ShepEnvironment}
function Get-ShepEnvironment { $value=& $baseEnvironment; $value.Interactive=$true; $value }
function Read-Host { 'c' }
'''
        self.assertIn('cancelled', self.execute('Invoke-ShepInstall', setup, False).stderr)
        self.assertEqual(self.fixture.requests, [])
        setup = '''$baseEnvironment=${function:Get-ShepEnvironment}
function Get-ShepEnvironment { $value=& $baseEnvironment; $value.Architecture='unsupported'; $value }
'''
        self.assertIn('No unique Windows', self.execute(setup=setup, success=False).stderr)
        self.assertFalse(self.app.exists())

    def test_missing_release_duplicate_files_and_unrelated_application_are_rejected(self):
        self.fixture.files.pop('/repos/sam-ruff/shep.so/releases/latest')
        self.assertIn('No release could be downloaded', self.execute(success=False).stderr)
        self.seed(entries=[('shep.exe',b'one'),('shep.exe',b'two'),('assets/launcher.png',b'fixture')])
        self.assertIn('unique shep.exe', self.execute(success=False).stderr)
        self.seed()
        self.app.mkdir(parents=True)
        other = self.app / 'unrelated'
        other.write_text('keep this')
        self.assertIn('not an installed Shep', self.execute(success=False).stderr)
        self.assertEqual(other.read_text(), 'keep this')


    def test_default_is_user_custom_path_is_literal_and_insecure_transport_is_rejected(self):
        self.execute('Invoke-ShepInstall -Yes')
        self.assertTrue((self.app / 'shep.exe').is_file())
        self.assertFalse((self.root / 'System apps').exists())
        custom = self.root / "Custom user's apps/Shep"
        self.execute('Invoke-ShepInstall -User -InstallDirectory '+literal(custom))
        self.assertTrue((custom / 'shep.exe').is_file())
        previous_requests = list(self.fixture.requests)
        error = self.execute('& $nativeDownload '+literal('http://127.0.0.1:'+str(self.fixture.server.server_port)+'/unsafe')+' '+literal(self.root/'unexpected'), success=False)
        self.assertIn('require HTTPS', error.stderr)
        self.assertEqual(self.fixture.requests, previous_requests)
        self.assertFalse((self.root / 'unexpected').exists())

if __name__ == '__main__':
    unittest.main()
