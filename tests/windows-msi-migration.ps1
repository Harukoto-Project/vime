param(
    [Parameter(Mandatory)][string]$Msi,
    [Parameter(Mandatory)][string]$Bridge
)

$ErrorActionPreference = 'Stop'
if (-not $env:GITHUB_ACTIONS -or -not $env:RUNNER_TEMP) {
    throw 'Run this integration test on a fresh GitHub Actions Windows runner.'
}

$msiPath = (Resolve-Path -LiteralPath $Msi).Path
$bridgePath = (Resolve-Path -LiteralPath $Bridge).Path
$msiHash = (Get-FileHash -LiteralPath $msiPath).Hash
$local = [Environment]::GetFolderPath('LocalApplicationData')
$installed = Join-Path $local 'Programs\ViME\vime.exe'
$data = Join-Path $local 'ViME'
if ((Test-Path -LiteralPath $data) -or (Test-Path -LiteralPath $installed)) {
    throw 'The runner already has ViME data or an installation.'
}
$steamvr = New-Item -ItemType Directory -Path (Join-Path $data 'steamvr')
$settings = Join-Path $data 'settings.json'
'{}' | Set-Content -LiteralPath $settings
$settingsHash = (Get-FileHash -LiteralPath $settings).Hash
$manifest = Join-Path $steamvr.FullName 'vime.vrmanifest'
$powerShell = Join-Path ([Environment]::SystemDirectory) 'WindowsPowerShell\v1.0\powershell.exe'

foreach ($version in '1.0.0', '1.1.0') {
    # The Windows inbox tar used by the legacy updater uses the runner's code page.
    $legacy = New-Item -ItemType Directory -Path (Join-Path $env:RUNNER_TEMP "ViME [$version]")
    $oldExe = Join-Path $legacy.FullName 'vime.exe'
    'old executable fixture' | Set-Content -LiteralPath $oldExe
    $personalFile = Join-Path $legacy.FullName 'personal.txt'
    'keep this file' | Set-Content -LiteralPath $personalFile
    @{ applications = @(@{ binary_path_windows = $oldExe }) } | ConvertTo-Json -Depth 4 |
        Set-Content -LiteralPath $manifest
    $stage = New-Item -ItemType Directory -Path (Join-Path $legacy.FullName '.vime-update-test')
    $package = New-Item -ItemType Directory -Path (Join-Path $stage.FullName 'package')
    & "$env:SystemRoot/System32/tar.exe" -xf $bridgePath -C $package.FullName
    if ($LASTEXITCODE -ne 0) { throw 'Could not extract the migration ZIP' }
    $bridgeMsi = Join-Path $package.FullName 'vime-migration.msi'
    if ((Get-FileHash -LiteralPath $bridgeMsi).Hash -ne $msiHash -or
        (Get-Content -LiteralPath "$bridgeMsi.sha256") -ne $msiHash) {
        throw 'The legacy update ZIP does not contain the MSI for this release.'
    }

    $oldScript = (git show "v${version}:src/update/install.ps1") -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "Could not read the v$version updater" }
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($oldScript))
    $start = [Diagnostics.ProcessStartInfo]::new($powerShell, "-NoProfile -NonInteractive -EncodedCommand $encoded")
    $start.UseShellExecute = $false
    $start.Environment['VIME_UPDATE_EXE'] = $oldExe
    $start.Environment['VIME_UPDATE_STAGING'] = $stage.FullName
    $start.Environment['VIME_UPDATE_ORIGINAL_HASH'] = (Get-FileHash -LiteralPath $oldExe).Hash
    $start.Environment['VIME_UPDATE_PID'] = [string][int]::MaxValue
    $start.Environment['VIME_UPDATE_RESTART'] = if ($version -eq '1.0.0') { '1' } else { '0' }
    $oldUpdater = [Diagnostics.Process]::Start($start)
    if (-not $oldUpdater.WaitForExit(30000) -or $oldUpdater.ExitCode -ne 0) {
        throw "The v$version updater did not apply the migration ZIP"
    }
    if ($version -eq '1.1.0') {
        # The old updater applies on exit and leaves the installer for the next launch.
        if (Test-Path -LiteralPath $installed) { throw 'The MSI ran before the next launch.' }
        $relaunch = [Diagnostics.ProcessStartInfo]::new($oldExe)
        $relaunch.WorkingDirectory = $legacy.FullName
        $relaunch.UseShellExecute = $false
        [Diagnostics.Process]::Start($relaunch).Dispose()
    }

    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $sidecar = Join-Path $legacy.FullName 'vime-migration.msi.sha256'
    $stagedMsi = Join-Path $legacy.FullName 'vime-migration.msi'
    do {
        Start-Sleep -Milliseconds 500
    } until ((Test-Path -LiteralPath $installed) -and -not (Test-Path -LiteralPath $stagedMsi) -or
        [DateTime]::UtcNow -gt $deadline)

    $shortcut = Join-Path $legacy.FullName 'ViME.lnk'
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    $manifestTarget = (Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json).applications[0].binary_path_windows
    if ($target -ne $installed -or $manifestTarget -ne $installed -or
        (Test-Path -LiteralPath $oldExe) -or (Test-Path -LiteralPath $sidecar) -or
        (Test-Path -LiteralPath $stagedMsi) -or (Test-Path -LiteralPath $stage.FullName) -or
        (Get-Content -LiteralPath $personalFile) -ne 'keep this file' -or
        (Get-FileHash -LiteralPath $settings).Hash -ne $settingsHash) {
        throw "The v$version ZIP-to-MSI migration did not complete."
    }
    Start-Sleep -Seconds 2
    $running = Get-Process -Name vime -ErrorAction SilentlyContinue | Where-Object Path -EQ $installed
    if ($running) {
        $running | Stop-Process -Force -ErrorAction SilentlyContinue
        $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    }
    $uninstall = Start-Process -FilePath msiexec.exe -ArgumentList "/x `"$msiPath`" /qn /norestart" -PassThru -Wait
    if ($uninstall.ExitCode -ne 0 -or (Test-Path -LiteralPath $installed)) {
        throw "Could not uninstall the MSI after testing v$version."
    }
    Write-Output "Verified migration from v$version."
}
