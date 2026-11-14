# Boot check of the GPUI app (docs/GPUI-TESTING.md "Pixel snapshots"):
# start komorebi.exe offline on a throwaway cache, let it draw its
# first frame, capture it to -Out (KOMOREBI_SNAPSHOT, PNG or BMP by the
# extension), check that the shell drew (KOMOREBI_SNAPSHOT_CHECK), and exit
# with the app's code: 0 drawn and checked, 1 blank or not captured, 2
# refused, 3 no DirectX 11 device.
#
#   pwsh scripts/gpui-boot-check.ps1 -Exe target\release\komorebi.exe `
#        -Cache $env:RUNNER_TEMP\gpui.sqlite3 -Out $env:RUNNER_TEMP\gpui-boot.png [-Warp]
#
# -Select <row> opens that row of a SEEDED cache (cargo run --example seed
# -- <file> 2000) and also checks that the list and the reading pane show
# something. -Fallback runs the check again
# with GPUI_DISABLE_DIRECT_COMPOSITION=1 when a run came out with a blank shell
# (the app's log says "blank title bar" or "pane"; not a timeout, a missing
# file, or a blank list or reading pane, which are real failures),
# for a runner session whose DWM does not compose DirectComposition
# surfaces (docs/GPUI-TESTING.md "CI"); the output says which mode passed.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    # A throwaway cache (it may not exist yet); never the app's own.
    [Parameter(Mandatory = $true)][string]$Cache,
    [Parameter(Mandatory = $true)][string]$Out,
    # Draw with WARP (machines without a GPU, CI).
    [switch]$Warp,
    [ValidateSet('light', 'dark')][string]$Theme = 'light',
    [string]$Size = '1280x800',
    [ValidateSet('en', 'pt-PT')][string]$Lang = 'en',
    [int]$TimeoutSec = 60,
    # The row to open (KOMOREBI_SNAPSHOT_SELECT) in a seeded cache.
    [ValidatePattern('^[0-9]*$')][string]$Select = '',
    # Try again without DirectComposition after a blank run.
    [switch]$Fallback
)
$ErrorActionPreference = 'Stop'

$dbFull = [IO.Path]::GetFullPath($Cache)
$real = Join-Path $env:LOCALAPPDATA 'komorebi'
if ($dbFull.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Error "refusing: $dbFull is in the app's own cache folder"
    exit 2
}
if (-not (Test-Path $Exe)) { Write-Error "no exe at $Exe"; exit 1 }
$outFull = [IO.Path]::GetFullPath($Out)

# The safety switches first (the app refuses a snapshot without them too).
$env:KOMOREBI_OFFLINE = '1'
$env:KOMOREBI_DB = $dbFull
if ($env:KOMOREBI_OFFLINE -ne '1' -or $env:KOMOREBI_DB -ne $dbFull) {
    Write-Error 'the safety switches are not set'
    exit 2
}
$env:KOMOREBI_SNAPSHOT = $outFull
$env:KOMOREBI_SNAPSHOT_CHECK = '1'
$env:KOMOREBI_SIZE = $Size
$env:KOMOREBI_THEME = $Theme
$env:KOMOREBI_LANG = $Lang
$env:KOMOREBI_TIMING = '1'
if ($Select -ne '') { $env:KOMOREBI_SNAPSHOT_SELECT = $Select }
else { Remove-Item Env:KOMOREBI_SNAPSHOT_SELECT -ErrorAction SilentlyContinue }
if ($Warp) { $env:KOMOREBI_GPUI_FORCE_WARP = '1' }
"boot-check: KOMOREBI_OFFLINE=1 KOMOREBI_DB=$dbFull snapshot=$outFull warp=$([bool]$Warp) select=$Select"

# One run of the app. Returns @{ Code; Blank } : the app's exit code (1 also
# for a run that left no file, or did not exit in time) and whether the log
# says the shell itself was blank (the title bar or the pane, which is what a
# window DWM does not compose looks like), not a blank list or reading pane.
# The log goes to the host with Out-Host: whatever the function writes to the
# pipeline would become part of its return value.
function Invoke-Boot([bool]$NoDirectComposition) {
    Remove-Item $outFull -ErrorAction SilentlyContinue
    if ($NoDirectComposition) { $env:GPUI_DISABLE_DIRECT_COMPOSITION = '1' }
    else { Remove-Item Env:GPUI_DISABLE_DIRECT_COMPOSITION -ErrorAction SilentlyContinue }
    $log = "$outFull.log"
    $p = Start-Process -FilePath $Exe -PassThru -NoNewWindow -RedirectStandardError $log
    # Keep the handle, or .NET forgets the exit code of an exited process.
    $null = $p.Handle
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force
        Get-Content $log -ErrorAction SilentlyContinue | Out-Host
        Write-Warning "boot-check: no exit within $TimeoutSec s (stopped)"
        return @{ Code = 1; Blank = $false }
    }
    $p.WaitForExit()
    $lines = @(Get-Content $log -ErrorAction SilentlyContinue)
    $lines | Out-Host
    $code = [int]$p.ExitCode
    if ($code -eq 0 -and -not (Test-Path $outFull)) {
        Write-Warning 'boot-check: exit 0 but no snapshot file'
        return @{ Code = 1; Blank = $false }
    }
    $shellBlank = $false
    foreach ($line in $lines) {
        if ($line -match 'snapshot check failed: blank .*(title bar|pane)') { $shellBlank = $true }
    }
    return @{ Code = $code; Blank = ($code -eq 1) -and $shellBlank }
}

$run = Invoke-Boot $false
[int]$code = $run.Code
if ($code -eq 1 -and $Fallback) {
    if ($run.Blank) {
        Write-Warning 'boot-check: the shell was blank with DirectComposition; again without it (GPUI_DISABLE_DIRECT_COMPOSITION=1)'
        $run = Invoke-Boot $true
        [int]$code = $run.Code
        if ($code -eq 0) { Write-Warning 'boot-check: passed only without DirectComposition' }
    }
    else {
        Write-Warning 'boot-check: failed, but not as a blank shell; no retry (a timeout, a missing file or a blank list or reading pane is a real failure)'
    }
}
"boot-check: exit code $code"
exit $code
