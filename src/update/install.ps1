$ErrorActionPreference = 'Stop'
$msi = $env:VIME_UPDATE_MSI
$legacy = $env:VIME_UPDATE_LEGACY_EXE
$sidecar = $env:VIME_UPDATE_SIDECAR
$installed = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\ViME\vime.exe'
$msiexec = Join-Path ([Environment]::SystemDirectory) 'msiexec.exe'
$mutex = $null
$locked = $false
$ready = $false
$failure = $null

function Start-Application([string]$path, [string[]]$arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new($path, ($arguments -join ' '))
    $start.WorkingDirectory = [IO.Path]::GetDirectoryName($path)
    $start.UseShellExecute = $false
    [Diagnostics.Process]::Start($start)
}

function Get-Sha256([IO.Stream]$stream) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '') }
    finally { $hash.Dispose() }
}

function Get-FileSha256([string]$path) {
    $file = [IO.File]::OpenRead($path)
    try { Get-Sha256 $file }
    finally { $file.Dispose() }
}

function Set-MigrationShortcut([string]$directory, [string]$target) {
    $shell = New-Object -ComObject WScript.Shell
    foreach ($name in 'ViME.lnk', 'ViME (MSI).lnk') {
        $path = Join-Path $directory $name
        if (Test-Path -LiteralPath $path) {
            $item = Get-Item -LiteralPath $path -Force
            if (-not $item.PSIsContainer -and
                -not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
                $shell.CreateShortcut($path).TargetPath -eq $target) {
                return
            }
            continue
        }
        $shortcut = $shell.CreateShortcut($path)
        $shortcut.TargetPath = $target
        $shortcut.WorkingDirectory = [IO.Path]::GetDirectoryName($target)
        $shortcut.Save()
        return
    }
    throw 'The ViME shortcut names are already in use.'
}

try {
    if ($legacy) {
        $mutex = [Threading.Mutex]::new($false, 'Local\ViME.MsiMigration')
        try { $locked = $mutex.WaitOne(0) }
        catch [Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { return }
    }
    $parent = Get-Process -Id ([int]$env:VIME_UPDATE_PID) -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(30000)) {
        throw 'ViME did not exit.'
    }

    $expected = $env:VIME_UPDATE_CHECKSUM
    if ($legacy) {
        $expected = ([IO.File]::ReadAllText($sidecar)).Trim()
    }
    $stream = [IO.File]::Open($msi, 'Open', 'Read', 'Read')
    try {
        if ($expected -notmatch '^[0-9a-f]{64}$' -or
            (Get-Sha256 $stream) -ne $expected) {
            throw 'The ViME installer checksum does not match.'
        }
        $setup = Start-Application $msiexec @('/i', "`"$msi`"", '/passive', '/norestart')
        try {
            $setup.WaitForExit()
            if ($setup.ExitCode -ne 0) {
                throw "Windows Installer exit code: $($setup.ExitCode)"
            }
        } finally {
            $setup.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
    if (-not [IO.File]::Exists($installed)) {
        throw 'The installed ViME executable was not found.'
    }

    if ($legacy) {
        if ((Get-FileSha256 $installed) -ne $env:VIME_UPDATE_LEGACY_HASH) {
            throw 'The installed ViME executable did not match the update.'
        }
        $refresh = Start-Application $installed @('--refresh-steamvr-manifest')
        try {
            $refresh.WaitForExit()
            if ($refresh.ExitCode -ne 0) {
                throw 'The SteamVR startup manifest could not be updated.'
            }
        } finally {
            $refresh.Dispose()
        }
        $ready = $true
        $directory = [IO.Path]::GetDirectoryName($legacy)
        Set-MigrationShortcut $directory $installed
        if ($legacy -ne $installed -and $env:VIME_UPDATE_IN_PLACE -ne '1') {
            if ((Get-FileSha256 $legacy) -ne $env:VIME_UPDATE_LEGACY_HASH) {
                throw 'The original ViME executable changed during installation.'
            }
            [IO.File]::Delete($legacy)
        }
        [IO.File]::Delete($sidecar)
        [IO.File]::Delete($msi)
    }
    $ready = $true
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    if ($mutex) { $mutex.Dispose() }
    if ($env:VIME_UPDATE_STAGING) {
        try {
            [IO.File]::Delete($msi)
            [IO.Directory]::Delete($env:VIME_UPDATE_STAGING)
        } catch {}
    }
}

if ($env:VIME_UPDATE_RESTART -eq '1') {
    try {
        $arguments = @()
        if ($env:VIME_UPDATE_STEAMVR -eq '1') { $arguments += '--steamvr-launch' }
        if ($ready) {
            (Start-Application $installed $arguments).Dispose()
        } elseif ($legacy -and [IO.File]::Exists($legacy)) {
            $arguments += '--skip-msi-migration'
            (Start-Application $legacy $arguments).Dispose()
        } elseif ($env:VIME_UPDATE_CURRENT_EXE -and [IO.File]::Exists($env:VIME_UPDATE_CURRENT_EXE)) {
            (Start-Application $env:VIME_UPDATE_CURRENT_EXE $arguments).Dispose()
        }
    } catch {
        $failure += "`n$($_.Exception.Message)"
    }
}
if ($failure) {
    if ($env:VIME_UPDATE_FAILED) {
        Add-Type -AssemblyName System.Windows.Forms
        [Windows.Forms.MessageBox]::Show(
            "$($env:VIME_UPDATE_FAILED)`n$failure", 'ViME', 'OK', 'Error') | Out-Null
    }
    [Console]::Error.WriteLine($failure)
    exit 1
}
