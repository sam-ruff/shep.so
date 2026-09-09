#Requires -Version 5.1
[CmdletBinding()]
param([switch]$User, [switch]$AllUsers, [switch]$Yes, [string]$Version = '', [string]$InstallDirectory = '')

function Receive-ShepDownload {
    param([string]$Url, [string]$Destination)
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    for ($redirect = 0; $redirect -lt 10; $redirect++) {
        $uri = [Uri]$Url
        if ($uri.Scheme -ne 'https') { throw 'Release downloads require HTTPS.' }
        $request = [Net.HttpWebRequest]::Create($uri)
        $request.AllowAutoRedirect = $false
        $request.Timeout = 60000
        $request.ReadWriteTimeout = 60000
        $request.UserAgent = 'Shep-Installer'
        if ($uri.Host -eq 'api.github.com') { $request.Headers['X-GitHub-Api-Version'] = '2026-03-10' }
        $response = $request.GetResponse()
        try {
            if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                $Url = ([Uri]::new($uri, $response.Headers['Location'])).AbsoluteUri
                continue
            }
            if ([int]$response.StatusCode -ne 200) { throw 'Release download failed.' }
            $output = [IO.File]::Create($Destination)
            try { $response.GetResponseStream().CopyTo($output) } finally { $output.Dispose() }
            return
        } finally { $response.Dispose() }
    }
    throw 'Release download redirected too many times.'
}

function Select-ShepRelease {
    param($Release, [string]$Architecture)
    if ($Release.tag_name -notmatch '^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$' -or $Release.draft) { throw 'Invalid published release.' }
    $version = $Matches[1]
    $architectures = switch ($Architecture.ToLowerInvariant()) {
        'amd64' { @('amd64', 'x86_64') }; 'x86_64' { @('amd64', 'x86_64') }
        'arm64' { @('arm64', 'aarch64') }; 'aarch64' { @('arm64', 'aarch64') }
        default { @() }
    }
    $names = @($architectures | ForEach-Object { "shep-$version-windows-$_.tar.gz" })
    $archives = @($Release.assets | Where-Object { $_.name -in $names })
    $checksums = @($Release.assets | Where-Object { $_.name -eq 'SHA256SUMS' })
    if ($archives.Count -ne 1 -or $checksums.Count -ne 1) { throw "No unique Windows/$Architecture archive and checksum are published; nothing was installed." }
    foreach ($asset in @($archives[0], $checksums[0])) {
        if ($asset.browser_download_url -notmatch '^https://github\.com/sam-ruff/shep\.so/releases/download/[^\s]+$') { throw 'Unexpected release URL.' }
    }
    [PSCustomObject]@{ Version = $version; Name = $archives[0].name; Archive = $archives[0].browser_download_url; Checksums = $checksums[0].browser_download_url }
}

function Expand-ShepInstallFiles {
    param([string]$Tar, [string]$Archive, [string]$Directory)
    $members = @(& $Tar -tzf $Archive)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read release archive.' }
    foreach ($member in @('shep.exe', 'assets/launcher.png')) {
        if (@($members | Where-Object { $_ -ceq $member }).Count -ne 1) { throw "Archive is missing a unique $member." }
        $listing = @(& $Tar -tvzf $Archive -- $member)
        if ($LASTEXITCODE -ne 0 -or $listing.Count -ne 1 -or !$listing[0].StartsWith('-')) { throw 'Archive contains a non-regular install file.' }
    }
    foreach ($member in @('shep.exe', 'assets/launcher.png')) {
        # Native stdout must remain binary (PowerShell 5.1 pipelines decode text).
        if ($Archive.Contains('"')) { throw 'Invalid archive path.' }
        $info = New-Object Diagnostics.ProcessStartInfo
        $info.FileName = $Tar
        $info.Arguments = '-xOzf "' + $Archive + '" -- "' + $member + '"'
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $process = New-Object Diagnostics.Process
        $process.StartInfo = $info
        $destination = Join-Path $Directory ([IO.Path]::GetFileName($member))
        $output = [IO.File]::Create($destination)
        try {
            [void]$process.Start()
            $errorRead = $process.StandardError.ReadToEndAsync()
            $process.StandardOutput.BaseStream.CopyTo($output)
            $process.WaitForExit()
            if ($process.ExitCode -ne 0) { throw ('Archive extraction failed: ' + $errorRead.GetAwaiter().GetResult()) }
        } finally { $output.Dispose(); $process.Dispose() }
        if ((Get-Item -LiteralPath $destination).Length -eq 0) { throw 'Archive install files are empty.' }
    }
}

function Write-ShepIcon {
    param([string]$Png, [string]$Destination)
    # Vista and later accept PNG image data in an ICO container. The release's
    # 128px launcher is reused without needing Python or System.Drawing.
    $bytes = [IO.File]::ReadAllBytes($Png)
    if ($bytes.Length -lt 24 -or [BitConverter]::ToString($bytes[0..7]) -ne '89-50-4E-47-0D-0A-1A-0A') { throw 'Invalid launcher PNG.' }
    $width = $bytes[16] * 16777216 + $bytes[17] * 65536 + $bytes[18] * 256 + $bytes[19]
    $height = $bytes[20] * 16777216 + $bytes[21] * 65536 + $bytes[22] * 256 + $bytes[23]
    if ($width -lt 1 -or $width -gt 256 -or $height -lt 1 -or $height -gt 256) { throw 'Launcher PNG must be at most 256 pixels.' }
    $writer = New-Object IO.BinaryWriter([IO.File]::Create($Destination))
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]1)
        $writer.Write([byte]($width % 256)); $writer.Write([byte]($height % 256))
        $writer.Write([byte]0); $writer.Write([byte]0); $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$bytes.Length); $writer.Write([uint32]22); $writer.Write([byte[]]$bytes)
    } finally { $writer.Dispose() }
}

function New-ShepShortcut {
    param([string]$Path, [string]$Application)
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($Path)
        $shortcut.TargetPath = Join-Path $Application 'shep.exe'
        $shortcut.WorkingDirectory = $Application
        $shortcut.IconLocation = (Join-Path $Application 'shep.ico') + ',0'
        $shortcut.Description = 'Shep email and calendar'
        $shortcut.Save()
    } finally { if ($shell) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) } }
}

function Install-ShepPrepared {
    param([string]$Source, [string]$Destination, [string]$Programs)
    $Destination = [IO.Path]::GetFullPath($Destination)
    $parent = [IO.Path]::GetDirectoryName($Destination)
    if (Test-Path -LiteralPath $Destination) {
        if ((Get-Item -LiteralPath $Destination).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing to replace a linked installation.' }
        $marker = Join-Path $Destination 'installation.json'
        if (!(Test-Path -LiteralPath $marker) -or (Get-Content -Raw -LiteralPath $marker | ConvertFrom-Json).application_id -ne 'so.shep.Shep') { throw 'The destination is not an installed Shep application.' }
    }
    [void][IO.Directory]::CreateDirectory($parent)
    [void][IO.Directory]::CreateDirectory($Programs)
    $slot = Join-Path $parent ('.shep-install-' + [Guid]::NewGuid().ToString('N'))
    $previous = Join-Path $slot 'Previous'
    $replacement = Join-Path $slot 'New'
    $shortcut = Join-Path $Programs 'Shep.lnk'
    $stagedShortcut = Join-Path $Programs ('.shep-' + [Guid]::NewGuid().ToString('N') + '.lnk')
    $moved = $false; $installed = $false; $success = $false; $preserve = $false
    [void][IO.Directory]::CreateDirectory($slot)
    try {
        Copy-Item -LiteralPath $Source -Destination $replacement -Recurse
        New-ShepShortcut $stagedShortcut $Destination
        if (Test-Path -LiteralPath $Destination) { [IO.Directory]::Move($Destination, $previous); $moved = $true }
        [IO.Directory]::Move($replacement, $Destination); $installed = $true
        if (Test-Path -LiteralPath $shortcut) { [IO.File]::Replace($stagedShortcut, $shortcut, [NullString]::Value) }
        else { [IO.File]::Move($stagedShortcut, $shortcut) }
        $success = $true
    } catch {
        try {
            if ($installed) { [IO.Directory]::Delete($Destination, $true) }
            if ($moved) { [IO.Directory]::Move($previous, $Destination) }
        } catch {
            $preserve = $true
            Write-Warning "The previous application is preserved at $previous. Restore it before retrying."
        }
        throw 'Could not replace Shep. Close any open Shep windows and retry; the previous application has been kept.'
    } finally {
        if (Test-Path -LiteralPath $stagedShortcut) { Remove-Item -LiteralPath $stagedShortcut -Force }
        if (!$preserve) {
            try { Remove-Item -LiteralPath $slot -Recurse -Force }
            catch { Write-Warning "Old installation files remain at $slot; close Shep before removing them." }
        }
    }
    if (!$success) { throw 'Shep installation did not complete.' }
}

function Test-ShepAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try { (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) }
    finally { $identity.Dispose() }
}

function Invoke-ShepElevation {
    param([string]$Stage, [string]$Source, [string]$Destination, [string]$Programs)
    $manifest = Join-Path $Stage 'apply.json'
    @{ Source = $Source; Destination = $Destination; Programs = $Programs } | ConvertTo-Json | Set-Content -LiteralPath $manifest -Encoding UTF8
    $apply = Join-Path $Stage 'apply.ps1'
    $code = 'param([string]$Manifest)' + "`n" + '$ErrorActionPreference = ''Stop''' + "`n"
    $code += 'function New-ShepShortcut {' + ${function:New-ShepShortcut}.ToString() + "}`n"
    $code += 'function Install-ShepPrepared {' + ${function:Install-ShepPrepared}.ToString() + "}`n"
    $code += '$data = Get-Content -Raw -LiteralPath $Manifest | ConvertFrom-Json; Install-ShepPrepared $data.Source $data.Destination $data.Programs'
    Set-Content -LiteralPath $apply -Value $code -Encoding UTF8
    # Encode only an invocation of the fixed prepared script. Quote path data as
    # literal PowerShell strings; never evaluate release metadata or user paths.
    $command = "& '" + $apply.Replace("'", "''") + "' '" + $manifest.Replace("'", "''") + "'"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    try { $process = Start-Process -FilePath $powershell -Verb RunAs -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) -Wait -PassThru }
    catch { throw 'Administrator approval was cancelled or unavailable. Retry with -User to install for your account.' }
    if ($process.ExitCode -ne 0) { throw 'Administrator installation failed; the previous application has been kept.' }
}

function Get-ShepEnvironment {
    if ([Environment]::OSVersion.Platform -ne 'Win32NT') { throw 'This installer is for Windows.' }
    $tar = (Get-Command tar.exe -ErrorAction SilentlyContinue).Source
    if (!$tar) { throw 'Windows tar.exe is missing. Update Windows 10/11, then retry.' }
    $architecture = $env:PROCESSOR_ARCHITEW6432
    if (!$architecture) { $architecture = $env:PROCESSOR_ARCHITECTURE }
    [PSCustomObject]@{
        Tar = $tar; Architecture = $architecture
        Interactive = [Environment]::UserInteractive -and ![Console]::IsInputRedirected
        UserApplication = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs/Shep'
        UserPrograms = [Environment]::GetFolderPath('Programs')
        SystemApplication = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Shep'
        SystemPrograms = [Environment]::GetFolderPath('CommonPrograms')
    }
}

function Invoke-ShepInstall {
    [CmdletBinding()]
    param([switch]$User, [switch]$AllUsers, [switch]$Yes, [string]$Version = '', [string]$InstallDirectory = '')
    $ErrorActionPreference = 'Stop'
    $environment = Get-ShepEnvironment
    if ($User -and $AllUsers) { throw 'Choose either -User or -AllUsers.' }
    if (!$User -and !$AllUsers -and !$Yes -and $environment.Interactive) {
        $choice = Read-Host 'Install for [u]ser (default), [a]ll users, or [c]ancel? [u/a/c]'
        switch ($choice.ToLowerInvariant()) { '' {} 'u' {} 'user' {} 'a' { $AllUsers = $true } 'all' { $AllUsers = $true } default { throw 'Installation cancelled; nothing was changed.' } }
    }
    if ($AllUsers -and $InstallDirectory) { throw 'All-user installation cannot combine -InstallDirectory.' }
    $endpoint = 'https://api.github.com/repos/sam-ruff/shep.so/releases/latest'
    if ($Version) {
        $Version = $Version -replace '^v', ''
        if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw 'Use a version such as 1.2.3.' }
        $endpoint = 'https://api.github.com/repos/sam-ruff/shep.so/releases/tags/v' + $Version
    }
    $stage = Join-Path ([IO.Path]::GetTempPath()) ('shep-windows-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage)
    try {
        $metadata = Join-Path $stage 'release.json'
        try { Receive-ShepDownload $endpoint $metadata }
        catch { throw 'No release could be downloaded. Check https://github.com/sam-ruff/shep.so/releases and try again.' }
        $release = Select-ShepRelease (Get-Content -Raw -LiteralPath $metadata | ConvertFrom-Json) $environment.Architecture
        $archive = Join-Path $stage 'release.tar.gz'; $checksums = Join-Path $stage 'SHA256SUMS'
        Write-Host "Downloading Shep $($release.Version) for Windows..."
        Receive-ShepDownload $release.Archive $archive
        Receive-ShepDownload $release.Checksums $checksums
        $digests = @(Get-Content -LiteralPath $checksums | ForEach-Object {
            if ($_ -match '^([a-fA-F0-9]{64})\s+\*?(.+)$' -and $Matches[2] -ceq $release.Name) { $Matches[1] }
        })
        if ($digests.Count -ne 1) { throw 'Missing or duplicate archive checksum; nothing was installed.' }
        if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $digests[0]) { throw 'Release checksum mismatch; nothing was installed.' }
        $prepared = Join-Path $stage 'Shep'; [void][IO.Directory]::CreateDirectory($prepared)
        Expand-ShepInstallFiles $environment.Tar $archive $prepared
        Write-ShepIcon (Join-Path $prepared 'launcher.png') (Join-Path $prepared 'shep.ico')
        Remove-Item -LiteralPath (Join-Path $prepared 'launcher.png')
        @{ application_id = 'so.shep.Shep'; version = $release.Version } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $prepared 'installation.json') -Encoding UTF8
        if ($AllUsers) {
            $destination = $environment.SystemApplication
            $programs = $environment.SystemPrograms
        } else {
            $destination = $InstallDirectory
            if (!$destination) { $destination = $environment.UserApplication }
            $programs = $environment.UserPrograms
        }
        if ($AllUsers -and !(Test-ShepAdministrator)) { Invoke-ShepElevation $stage $prepared $destination $programs }
        else { Install-ShepPrepared $prepared $destination $programs }
        Write-Host "Installed $destination. Open Shep from Start; reopen an existing window after updating."
    } finally { Remove-Item -LiteralPath $stage -Recurse -Force }
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-ShepInstall @PSBoundParameters }
