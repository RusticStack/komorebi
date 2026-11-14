# Runs Komorebi's CI on this machine, natively, with the pinned toolchain:
# the same steps, in the same order, as the `build` and `deny` jobs of
# .github/workflows/ci.yml. Run it before pushing; GitHub Actions only runs
# on demand and for pull requests from forks.
#
#   pwsh -NoProfile -File scripts/ci-local.ps1 [-Fast] [-From <step>] [-Scratch <dir>]
#
# Steps (the -From names): fxc, fmt, clippy, check, lint, test, test-app,
# pins, build, size, manifest, boot, boot-seeded, uia, deny.
#
# -Fast      skips the steps that need the release build: build, size,
#            manifest, boot, boot-seeded and uia.
# -From      starts at that step and skips the ones before it (they show as
#            "skipped" in the summary).
# -Scratch   where the throwaway caches and snapshots go (default
#            $env:TEMP\komorebi-ci). Never the app's own cache.
#
# It stops at the first failure (non-zero exit code); the UI Automation dump
# is informational, as in CI, and only warns. Honours CARGO_TARGET_DIR (the
# exe is then $env:CARGO_TARGET_DIR\release\komorebi.exe). komorebi.exe is only
# ever started through scripts/gpui-boot-check.ps1 and
# scripts/gpui-uia-check.ps1, which set KOMOREBI_OFFLINE, a throwaway
# KOMOREBI_DB and KOMOREBI_SNAPSHOT themselves and refuse the app's own cache.
# The boot checks open a window on the desktop for a few seconds.
param(
    [switch]$Fast,
    [ValidateSet('fxc', 'fmt', 'clippy', 'check', 'lint', 'test', 'test-app', 'pins',
        'build', 'size', 'manifest', 'boot', 'boot-seeded', 'uia', 'deny')]
    [string]$From = 'fxc',
    [string]$Scratch = (Join-Path $env:TEMP 'komorebi-ci')
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$scratchFull = [IO.Path]::GetFullPath($Scratch)
$real = Join-Path $env:LOCALAPPDATA 'komorebi'
if ($scratchFull.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Error "refusing: -Scratch $scratchFull is in the app's own cache folder"
    exit 2
}
New-Item -ItemType Directory -Force -Path $scratchFull | Out-Null

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$exe = Join-Path $targetDir 'release\komorebi.exe'
$ps = (Get-Process -Id $PID).Path   # the pwsh running this script

# Runs a native command, echoing it, and throws when its exit code is not 0.
function Invoke-Native([string]$File, [string[]]$Arguments) {
    "> $File $($Arguments -join ' ')"
    & $File @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed (exit $LASTEXITCODE)" }
}
function Invoke-Cargo([string[]]$Arguments) { Invoke-Native 'cargo' $Arguments }
function Invoke-Script([string]$Script, [string[]]$Arguments) {
    Invoke-Native $ps (@('-NoProfile', '-File', (Join-Path $root "scripts\$Script")) + $Arguments)
}
function Reset-Files([string[]]$Paths) {
    foreach ($p in $Paths) {
        foreach ($f in @($p, "$p-wal", "$p-shm", "$p.log")) { Remove-Item $f -Force -ErrorAction SilentlyContinue }
    }
}

$bootDb = Join-Path $scratchFull 'gpui-boot.sqlite3'
$bootPng = Join-Path $scratchFull 'gpui-boot.png'
$seedDb = Join-Path $scratchFull 'gpui-seed.sqlite3'
$seedPng = Join-Path $scratchFull 'gpui-boot-mail.png'
$uiaDb = Join-Path $scratchFull 'gpui-uia.sqlite3'
$uiaTxt = Join-Path $scratchFull 'gpui-uia.txt'

# Release = needs the release build (-Fast skips it); Info = informational.
$steps = @(
    @{ Id = 'fxc'; Title = 'find-fxc'; Run = {
            $out = @(& $ps -NoProfile -File (Join-Path $root 'scripts\find-fxc.ps1') 2>&1)
            $code = $LASTEXITCODE
            $out | Out-Host
            if ($code -ne 0) { throw 'find-fxc failed' }
            # CI sets GPUI_FXC_PATH through GITHUB_ENV; do the same here.
            $m = ($out | Out-String) | Select-String -Pattern '(?m)^fxc: (.+?) \(version'
            if ($m) { $env:GPUI_FXC_PATH = $m.Matches[0].Groups[1].Value }
        } },
    @{ Id = 'fmt'; Title = 'cargo fmt --check'; Run = { Invoke-Cargo @('fmt', '--all', '--check') } },
    @{ Id = 'clippy'; Title = 'cargo clippy'; Run = {
            Invoke-Cargo @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings') } },
    @{ Id = 'check'; Title = 'cargo check (app)'; Run = {
            Invoke-Cargo @('check', '-p', 'komorebi-gpui', '--all-targets', '--locked') } },
    @{ Id = 'lint'; Title = 'xtask lint'; Run = { Invoke-Cargo @('xtask', 'lint') } },
    @{ Id = 'test'; Title = 'cargo test'; Run = { Invoke-Cargo @('test', '--locked') } },
    @{ Id = 'test-app'; Title = 'cargo test (app, i18n, xtask)'; Run = {
            Invoke-Cargo @('test', '-p', 'komorebi-gpui', '-p', 'komorebi-i18n', '--locked')
            Invoke-Cargo @('test', '-p', 'xtask', '--locked') } },
    @{ Id = 'pins'; Title = 'check-pins'; Run = { Invoke-Script 'check-pins.ps1' @() } },
    @{ Id = 'build'; Release = $true; Title = 'release build'; Run = {
            Invoke-Cargo @('build', '--release', '--locked', '-p', 'komorebi-gpui') } },
    @{ Id = 'size'; Release = $true; Title = 'xtask size'; Run = { Invoke-Cargo @('xtask', 'size') } },
    @{ Id = 'manifest'; Release = $true; Title = 'check-manifest'; Run = {
            Invoke-Script 'check-manifest.ps1' @($exe) } },
    @{ Id = 'boot'; Release = $true; Title = 'boot check (WARP)'; Run = {
            Reset-Files @($bootDb)
            Invoke-Script 'gpui-boot-check.ps1' @('-Exe', $exe, '-Cache', $bootDb, '-Out', $bootPng, '-Warp', '-Fallback') } },
    @{ Id = 'boot-seeded'; Release = $true; Title = 'boot check, seeded (WARP)'; Run = {
            Reset-Files @($seedDb)
            Invoke-Cargo @('run', '--release', '--locked', '--example', 'seed', '--', $seedDb, '2000')
            Invoke-Script 'gpui-boot-check.ps1' @('-Exe', $exe, '-Cache', $seedDb, '-Out', $seedPng, '-Warp', '-Fallback', '-Select', '0') } },
    @{ Id = 'uia'; Release = $true; Info = $true; Title = 'UI Automation dump (informational)'; Run = {
            Reset-Files @($uiaDb)
            Invoke-Script 'gpui-uia-check.ps1' @('-Exe', $exe, '-Cache', $uiaDb, '-Warp', '-DumpPath', $uiaTxt) } },
    @{ Id = 'deny'; Title = 'cargo deny'; Run = {
            Invoke-Cargo @('deny', '--workspace', '--locked', 'check', 'licenses', 'advisories') } }
)

$ids = @($steps | ForEach-Object { $_.Id })
$fromIndex = [array]::IndexOf($ids, $From)
$results = [System.Collections.Generic.List[object]]::new()
$failed = $null
$total = [Diagnostics.Stopwatch]::StartNew()
$flags = ''
if ($Fast) { $flags += ', -Fast' }
if ($From -ne 'fxc') { $flags += ", from $From" }
"ci-local: repo $root, target dir $targetDir, scratch $scratchFull$flags"

for ($i = 0; $i -lt $steps.Count; $i++) {
    $s = $steps[$i]
    if ($i -lt $fromIndex -or ($Fast -and $s.Release)) {
        $results.Add([pscustomobject]@{ Step = $s.Title; Status = 'skipped'; Time = '' })
        continue
    }
    if ($failed) {
        $results.Add([pscustomobject]@{ Step = $s.Title; Status = 'not run'; Time = '' })
        continue
    }
    "`n=== [$($i + 1)/$($steps.Count)] $($s.Title) ==="
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $status = 'ok'
    try { & $s.Run }
    catch {
        $status = if ($s.Info) { 'warn' } else { 'FAILED' }
        Write-Host "ci-local: $($s.Title): $_" -ForegroundColor Red
        if (-not $s.Info) { $failed = $s.Title }
    }
    $sw.Stop()
    $results.Add([pscustomobject]@{ Step = $s.Title; Status = $status; Time = '{0:N1} s' -f $sw.Elapsed.TotalSeconds })
}
$total.Stop()

"`n=== ci-local summary ==="
$results | Format-Table -AutoSize | Out-String -Width 200 | Out-Host
'total: {0:N1} s ({1:N1} min)' -f $total.Elapsed.TotalSeconds, $total.Elapsed.TotalMinutes
if ($failed) {
    Write-Host "ci-local: FAILED at '$failed'" -ForegroundColor Red
    exit 1
}
'ci-local: all steps passed'
exit 0
