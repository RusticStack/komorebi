# Finds fxc.exe, the HLSL compiler of the Windows SDK, which the GPUI app's
# release build compiles its shaders with (docs/GPUI-BUILD.md), and prints
# its path and version. In GitHub Actions it also sets GPUI_FXC_PATH for
# the steps after it, so the build log says which fxc was used. Exit 1 when
# there is none.
#
# Order: GPUI_FXC_PATH when it names a file, then the newest
# "Windows Kits\10\bin\<version>\x64\fxc.exe" (the SDK's own layout; GitHub's
# windows-2025 images ship the Windows 11 SDK there), then
# fxc.exe on PATH.
$ErrorActionPreference = 'Stop'

function Report($path) {
    $v = (Get-Item $path).VersionInfo.FileVersion
    "fxc: $path (version $v)"
    if ($env:GITHUB_ENV) {
        "GPUI_FXC_PATH=$path" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
        'fxc: GPUI_FXC_PATH set for the next steps'
    }
    exit 0
}

if ($env:GPUI_FXC_PATH) {
    if (Test-Path -PathType Leaf $env:GPUI_FXC_PATH) { Report $env:GPUI_FXC_PATH }
    Write-Warning "GPUI_FXC_PATH=$env:GPUI_FXC_PATH names no file; looking further"
}
$roots = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'),
    (Join-Path $env:ProgramFiles 'Windows Kits\10\bin')
) | Where-Object { $_ -and (Test-Path $_) }
$found = foreach ($root in $roots) {
    Get-ChildItem $root -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        ForEach-Object { Join-Path $_.FullName 'x64\fxc.exe' } |
        Where-Object { Test-Path -PathType Leaf $_ }
}
$newest = $found | Sort-Object { [version](Split-Path (Split-Path (Split-Path $_ -Parent) -Parent) -Leaf) } |
    Select-Object -Last 1
if ($newest) { Report $newest }
$onPath = Get-Command fxc.exe -ErrorAction SilentlyContinue
if ($onPath) { Report $onPath.Source }
Write-Error 'fxc: no fxc.exe (install the Windows 10/11 SDK, or set GPUI_FXC_PATH)'
exit 1
