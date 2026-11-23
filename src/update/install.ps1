$ErrorActionPreference = 'Stop'
$stage = $env:VIME_UPDATE_STAGING
$exe = $env:VIME_UPDATE_EXE
$install = [IO.Path]::GetDirectoryName($exe)
$package = Join-Path $stage 'package'
$backup = Join-Path $stage 'backup'
$changes = [Collections.Generic.List[object]]::new()
$lock = $null
$cleanup = $false
$failure = $null

try {
    if ([IO.Path]::GetDirectoryName($stage) -ne $install -or
        -not [IO.Path]::GetFileName($stage).StartsWith('.vime-update-')) {
        throw 'Invalid update staging directory.'
    }
    $cleanup = $true
    $parent = Get-Process -Id ([int]$env:VIME_UPDATE_PID) -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(30000)) {
        throw 'ViME did not exit.'
    }

    $lock = [IO.FileStream]::new(
        (Join-Path $install '.vime-update.lock'), [IO.FileMode]::OpenOrCreate,
        [IO.FileAccess]::Write, [IO.FileShare]::None, 1, [IO.FileOptions]::DeleteOnClose)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $stream = [IO.File]::OpenRead($exe)
        try { $hash = [BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '') }
        finally { $stream.Dispose() }
    } finally {
        $sha256.Dispose()
    }
    if ($hash -eq $env:VIME_UPDATE_ORIGINAL_HASH) {
        $files = @(Get-ChildItem -LiteralPath $package -Recurse -File -Force |
            Where-Object { $_.FullName -ne (Join-Path $package 'vime.exe') })
        # Update the executable last so it marks a completed installation.
        $files += Get-Item -LiteralPath (Join-Path $package 'vime.exe')
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($package.Length + 1)
            if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Unexpected link in the update: $relative"
            }
            $target = if ($relative -eq 'vime.exe') { $exe } else { Join-Path $install $relative }
            if ([IO.File]::Exists($target) -and
                ([IO.File]::GetAttributes($target) -band [IO.FileAttributes]::ReparsePoint)) {
                throw "Unexpected link in the installation: $relative"
            }
            $directory = [IO.Path]::GetDirectoryName($target)
            while ($directory -ne $install) {
                if ([IO.Directory]::Exists($directory) -and
                    ([IO.File]::GetAttributes($directory) -band [IO.FileAttributes]::ReparsePoint)) {
                    throw "Unexpected linked directory in the installation: $relative"
                }
                $directory = [IO.Path]::GetDirectoryName($directory)
            }
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
            $saved = Join-Path $backup $relative
            $change = @{ Target = $target; Backup = $saved; Added = $false }
            $changes.Add($change)
            if ([IO.File]::Exists($target)) {
                [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($saved)) | Out-Null
                [IO.File]::Replace($file.FullName, $target, $saved)
            } else {
                [IO.File]::Move($file.FullName, $target)
                $change.Added = $true
            }
        }
    }
} catch {
    $failure = $_.Exception.Message
    for ($index = $changes.Count - 1; $index -ge 0; $index--) {
        $change = $changes[$index]
        try {
            if ([IO.File]::Exists($change.Backup)) {
                if ([IO.File]::Exists($change.Target)) {
                    [IO.File]::Replace($change.Backup, $change.Target, [NullString]::Value)
                } else {
                    [IO.File]::Move($change.Backup, $change.Target)
                }
            } elseif ($change.Added) {
                [IO.File]::Delete($change.Target)
            }
        } catch {
            $cleanup = $false
            $failure += "`n$($_.Exception.Message)`n$backup"
        }
    }
} finally {
    if ($lock) { $lock.Dispose() }
    if ($cleanup) {
        try { [IO.Directory]::Delete($stage, $true) } catch {}
    }
}

if ($env:VIME_UPDATE_RESTART -eq '1' -and $cleanup) {
    try {
        $start = [Diagnostics.ProcessStartInfo]::new($exe)
        $start.WorkingDirectory = $install
        $start.UseShellExecute = $false
        [Diagnostics.Process]::Start($start).Dispose()
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
