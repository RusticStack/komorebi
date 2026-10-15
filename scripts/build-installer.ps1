# Compiles the Windows installer around a built komorebi.exe and prints the
# installer's SHA-256.
#
#   pwsh scripts/build-installer.ps1 [-Exe target\release\komorebi.exe] [-Version 0.3.0]
#
# Needs Inno Setup 6.3 or later (ISCC.exe on PATH or in its usual folders;
# winget install JRSoftware.InnoSetup). The version defaults to the one
# inside the exe. Output: dist\komorebi-<version>-setup.exe
param(
    [string]$Exe = 'target\release\komorebi.exe',
    [string]$Version
)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$exePath = (Resolve-Path -LiteralPath (Join-Path $root $Exe) -ErrorAction SilentlyContinue)
if (-not $exePath) { $exePath = Resolve-Path -LiteralPath $Exe }
$exePath = $exePath.Path

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'Inno Setup 6 not found (winget install JRSoftware.InnoSetup)' }

# The script takes komorebi.exe from a folder, by that name.
$stage = Join-Path ([IO.Path]::GetTempPath()) "komorebi-installer-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force $stage | Out-Null
try {
    Copy-Item -LiteralPath $exePath (Join-Path $stage 'komorebi.exe')
    if (-not $Version) {
        $info = (Get-Item -LiteralPath (Join-Path $stage 'komorebi.exe')).VersionInfo
        $Version = '{0}.{1}.{2}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart
    }
    & $iscc "/DAppVersion=$Version" "/DBinDir=$stage" (Join-Path $root 'installer\komorebi.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)" }
} finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}

$setup = Join-Path $root "dist\komorebi-$Version-setup.exe"
if (-not (Test-Path $setup)) { throw "expected $setup was not written" }
$hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
"$setup"
"SHA-256  $hash"
