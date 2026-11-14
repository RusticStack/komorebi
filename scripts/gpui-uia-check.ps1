# UI Automation check of the GPUI app (docs/GPUI-TESTING.md "UIA dump"):
# starts komorebi.exe offline on a throwaway cache, waits for its
# window, prints the UI Automation tree (what Narrator reads; adapted from
# the spike's tools/uia-dump.ps1), asserts the expected names, then closes
# the window as a user would and waits for the app to exit.
#
#   pwsh scripts/gpui-uia-check.ps1 -Exe target\release\komorebi.exe `
#        -Cache $env:RUNNER_TEMP\uia.sqlite3 [-Warp] [-Expect 'Window:Komorebi','Button:Close']
#
# Exit 0 when every expected element is there, 1 when one is missing or the
# app misbehaved, 2 when refused (the cache is not a throwaway one).
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    # A throwaway cache; never the app's own (%LOCALAPPDATA%\komorebi).
    [Parameter(Mandatory = $true)][string]$Cache,
    # ControlType:Name pairs that must be in the tree.
    [string[]]$Expect = @('Window:Komorebi', 'Button:Minimize', 'Button:Maximize', 'Button:Close'),
    # Draw with WARP (machines without a GPU, CI).
    [switch]$Warp,
    [int]$TimeoutSec = 30,
    [int]$MaxNodes = 400,
    # Also save the tree here.
    [string]$DumpPath
)
$ErrorActionPreference = 'Stop'

$dbFull = [IO.Path]::GetFullPath($Cache)
$real = Join-Path $env:LOCALAPPDATA 'komorebi'
if ($dbFull.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Error "refusing: $dbFull is in the app's own cache folder"
    exit 2
}
if (-not (Test-Path $Exe)) { Write-Error "no exe at $Exe"; exit 1 }

# The safety switches, set and checked before the app starts.
$env:KOMOREBI_OFFLINE = '1'
$env:KOMOREBI_DB = $dbFull
$env:KOMOREBI_LANG = 'en'
if ($Warp) { $env:KOMOREBI_GPUI_FORCE_WARP = '1' }
if ($env:KOMOREBI_OFFLINE -ne '1' -or $env:KOMOREBI_DB -ne $dbFull) {
    Write-Error 'the safety switches are not set'
    exit 2
}
foreach ($v in 'KOMOREBI_SNAPSHOT', 'KOMOREBI_EXIT_AFTER_FIRST_FRAME') {
    Remove-Item "env:$v" -ErrorAction SilentlyContinue
}
"uia-check: KOMOREBI_OFFLINE=1 KOMOREBI_DB=$dbFull KOMOREBI_LANG=en warp=$([bool]$Warp)"

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]

$p = Start-Process -FilePath $Exe -PassThru
# Keep the handle, or .NET forgets the exit code of an exited process.
$null = $p.Handle
$failed = $false
try {
    $cond = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $p.Id)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $win = $null
    while (-not $win -and (Get-Date) -lt $deadline -and -not $p.HasExited) {
        $win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
        if (-not $win) { Start-Sleep -Milliseconds 200 }
    }
    if ($p.HasExited) { throw "the app exited early (code $($p.ExitCode))" }
    if (-not $win) { throw "no window within $TimeoutSec s" }
    # Let the first frame and its accessibility tree arrive.
    while ((Get-Date) -lt $deadline) {
        $buttons = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
            (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::Button)))
        if ($buttons.Count -gt 0) { break }
        Start-Sleep -Milliseconds 200
    }

    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $script:count = 0
    $script:seen = New-Object System.Collections.Generic.List[string]
    function Show($el, $depth) {
        if ($script:count -ge $MaxNodes -or $depth -gt 30) { return }
        $script:count++
        $c = $el.Current
        $type = $c.ControlType.ProgrammaticName -replace '^ControlType\.', ''
        $script:seen.Add("${type}:$($c.Name)")
        $extra = @()
        $pat = $null
        if ($el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pat)) {
            $extra += "selected=$($pat.Current.IsSelected)"
        }
        $pat = $null
        if ($el.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pat)) { $extra += 'invoke' }
        if ($c.IsKeyboardFocusable) { $extra += 'focusable' }
        $r = $c.BoundingRectangle
        $rect = if ($r.IsEmpty) { 'no-rect' } else { '{0:0},{1:0} {2:0}x{3:0}' -f $r.X, $r.Y, $r.Width, $r.Height }
        "{0}{1} '{2}' id='{3}' {4} [{5}]" -f ('  ' * $depth), $type, $c.Name, $c.AutomationId, ($extra -join ' '), $rect
        $child = $walker.GetFirstChild($el)
        while ($null -ne $child) {
            Show $child ($depth + 1)
            $child = $walker.GetNextSibling($child)
        }
    }
    $dump = @(Show $win 0)
    $dump += "nodes: $script:count"
    $dump
    if ($DumpPath) { $dump | Set-Content -Encoding utf8 $DumpPath }

    foreach ($want in $Expect) {
        if ($script:seen -contains $want) { "uia-check: found $want" }
        else { Write-Warning "uia-check: MISSING $want"; $failed = $true }
    }

    # Close as a user would (the window's Close), then wait for the exit.
    $pat = $null
    if ($win.TryGetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern, [ref]$pat)) {
        $pat.Close()
    } else {
        [void]$p.CloseMainWindow()
    }
    if (-not $p.WaitForExit(15000)) { throw 'the app did not exit within 15 s of Close' }
    "uia-check: exit code $($p.ExitCode)"
    if ($p.ExitCode -ne 0) { $failed = $true }
} catch {
    Write-Warning "uia-check: $_"
    $failed = $true
} finally {
    if (-not $p.HasExited) {
        Stop-Process -Id $p.Id -Force
        Write-Warning 'uia-check: the app was stopped'
    }
}
if ($failed) { exit 1 }
'uia-check: passed'
exit 0
