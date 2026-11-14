# Launch timing, memory and idle-CPU harness for the app (komorebi.exe),
# from the GPUI feasibility spike's tools/bench.ps1 (docs/GPUI-MIGRATION.md
# "Measurements"). A -Config that is the path of another exe (an older
# build to compare against) is started the same way. Every launch is
# offline on a throwaway cache: KOMOREBI_OFFLINE=1 and KOMOREBI_DB are set on
# the child's environment and asserted before it starts; the real cache
# folder is refused, and the cache must be inside -Scratch.
#
#   pwsh -File scripts/gpui-bench.ps1 -Mode timing -Runs 9 -Config gpui -Cache <seed copy> -Scratch <dir> -GpuiExe <exe>
#   pwsh -File scripts/gpui-bench.ps1 -Mode memory -Config gpui -Cache <seed copy> -Scratch <dir> -GpuiExe <exe>
#   pwsh -File scripts/gpui-bench.ps1 -Mode idle -Config gpui -Cache <seed copy> -Scratch <dir> -GpuiExe <exe> -Seconds 60 [-Minimize] [-Extra K=V,...]
param(
    [ValidateSet('timing', 'memory', 'idle')][string]$Mode = 'timing',
    [string[]]$Config = @('gpui'),
    [int]$Runs = 9,
    [string]$Cache,
    [int]$Seconds = 60,
    [switch]$Minimize,
    [string[]]$Extra = @(),
    [string]$GpuiExe = 'target\release\komorebi.exe',
    [Parameter(Mandatory = $true)][string]$Scratch,
    [int]$TimeoutMs = 20000
)
$ErrorActionPreference = 'Stop'
$Config = @($Config | ForEach-Object { $_ -split ',' })
$Extra = @($Extra | ForEach-Object { $_ -split ';' } | Where-Object { $_ })

Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class BenchNative {
    [DllImport("kernel32.dll")] public static extern bool QueryProcessCycleTime(IntPtr h, out ulong cycles);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
// A child whose stderr lines are queued (and the first frame line
// timestamped) on .NET threads, not PowerShell ones.
public class BenchChild {
    public Process P = new Process();
    public ConcurrentQueue<string> Lines = new ConcurrentQueue<string>();
    public Stopwatch Sw;
    public double FirstFrameWallMs = -1;
    public BenchChild(ProcessStartInfo psi) {
        P.StartInfo = psi;
        P.ErrorDataReceived += (s, e) => {
            if (e.Data == null) return;
            if (FirstFrameWallMs < 0 && e.Data.Contains("first frame drawn")) FirstFrameWallMs = Sw.Elapsed.TotalMilliseconds;
            Lines.Enqueue(e.Data);
        };
        P.OutputDataReceived += (s, e) => { };
        Sw = Stopwatch.StartNew();
        P.Start();
        P.BeginErrorReadLine();
        P.BeginOutputReadLine();
    }
}
'@ -ErrorAction SilentlyContinue

function Assert-Throwaway([string]$path) {
    if (-not $path) { throw 'KOMOREBI_DB (-Cache) is required' }
    $full = [IO.Path]::GetFullPath($path)
    $real = Join-Path $env:LOCALAPPDATA 'komorebi'
    if ($full.ToLower().StartsWith($real.ToLower())) { throw "refusing the real cache folder $real" }
    if (-not $full.ToLower().StartsWith($Scratch.ToLower())) { throw "the cache must be inside -Scratch: $full" }
    if (-not (Test-Path $full)) { throw "missing $full" }
    $full
}

function New-Start([string]$cfg, [hashtable]$envs) {
    $exe = switch ($cfg) {
        'gpui' { $GpuiExe }
        default { if (Test-Path $cfg) { $cfg } else { throw "unknown config $cfg" } }
    }
    $psi = New-Object Diagnostics.ProcessStartInfo $exe
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    # Children start from a clean slate of these switches.
    foreach ($k in @('KOMOREBI_TIMING', 'KOMOREBI_SNAPSHOT', 'KOMOREBI_EXIT_AFTER_FIRST_FRAME', 'KOMOREBI_GPUI_FORCE_WARP', 'KOMOREBI_GPUI_PREFLIGHT_FAIL')) { [void]$psi.EnvironmentVariables.Remove($k) }
    foreach ($k in $envs.Keys) { $psi.EnvironmentVariables[$k] = [string]$envs[$k] }
    if ($psi.EnvironmentVariables['KOMOREBI_OFFLINE'] -ne '1') { throw 'KOMOREBI_OFFLINE must be 1' }
    $db = $psi.EnvironmentVariables['KOMOREBI_DB']
    [void](Assert-Throwaway $db)
    $psi
}

function Base-Env([string]$cfg, [string]$db) {
    $e = @{ KOMOREBI_OFFLINE = '1'; KOMOREBI_DB = $db }
    foreach ($kv in $Extra) { $p = $kv.Split('=', 2); $e[$p[0]] = $p[1] }
    $e
}

# Wall clock: from just before Process.Start to the arrival of the first
# frame line on the child's stderr. In-app: the ms the line itself reports.
function Run-Timing([string]$cfg, [string]$db, [int]$i) {
    $e = Base-Env $cfg $db
    $e['KOMOREBI_TIMING'] = '1'
    $e['KOMOREBI_EXIT_AFTER_FIRST_FRAME'] = '1'
    $psi = New-Start $cfg $e
    $ch = [BenchChild]::new($psi)
    $p = $ch.P
    if (-not $p.WaitForExit($TimeoutMs)) { $p.Kill(); throw "$cfg did not exit" }
    $p.WaitForExit()
    $lines = @($ch.Lines.ToArray())
    $app = $null
    foreach ($l in $lines) { if ($l -match '\[timing\] first frame drawn: ([0-9.]+) ms') { $app = [double]$Matches[1] } }
    [pscustomobject]@{ config = $cfg; run = $i; app_ms = $app; wall_ms = [math]::Round($ch.FirstFrameWallMs, 1); exit = $p.ExitCode; log = ($lines -join ' | ') }
}

function Wait-Window([Diagnostics.Process]$p) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt $TimeoutMs) { Start-Sleep -Milliseconds 50; $p.Refresh() }
    $p.MainWindowHandle
}

function Read-Mem([Diagnostics.Process]$p) {
    $p.Refresh()
    $ws = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter "IDProcess = $($p.Id)"
    [pscustomobject]@{
        private_mb    = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        private_ws_mb = [math]::Round($ws.WorkingSetPrivate / 1MB, 1)
        ws_mb         = [math]::Round($p.WorkingSet64 / 1MB, 1)
        peak_ws_mb    = [math]::Round($p.PeakWorkingSet64 / 1MB, 1)
        threads       = $p.Threads.Count
        handles       = $p.HandleCount
    }
}

function Start-Live([string]$cfg, [string]$db) {
    $e = Base-Env $cfg $db
    $e['KOMOREBI_TIMING'] = '1'
    $psi = New-Start $cfg $e
    $ch = [BenchChild]::new($psi)
    $script:errLines = $ch.Lines
    $ch.P
}

function Wait-Line([string]$pattern, [int]$ms) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $seen = @()
    while ($sw.ElapsedMilliseconds -lt $ms) {
        $l = $null
        while ($script:errLines.TryDequeue([ref]$l)) { $seen += $l; if ($l -match $pattern) { return $seen } }
        Start-Sleep -Milliseconds 25
    }
    $seen
}

function Stop-Live([Diagnostics.Process]$p) {
    if (-not $p.HasExited) { [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(5000)) { $p.Kill() } }
}

$load = (Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
"# mode=$Mode load=$load% cpu='$($cpu.Name)' maxMHz=$($cpu.MaxClockSpeed) $(Get-Date -Format s)"

switch ($Mode) {
    'timing' {
        $dbFull = if ($Cache) { Assert-Throwaway $Cache } else { $null }
        $rows = @()
        # One warm-up launch per config (first launch writes default folds).
        foreach ($c in $Config) { $null = Run-Timing $c $dbFull 0 }
        for ($i = 1; $i -le $Runs; $i++) {
            foreach ($c in $Config) { $rows += Run-Timing $c $dbFull $i }
        }
        $rows | Format-Table config, run, app_ms, wall_ms, exit -AutoSize | Out-String -Width 200
        foreach ($c in $Config) {
            $r = $rows | Where-Object config -eq $c
            $a = @($r.app_ms | Sort-Object); $w = @($r.wall_ms | Sort-Object)
            "{0}: in-app median {1} min {2} max {3} | wall median {4} min {5} max {6} (n={7})" -f $c, $a[[int]($a.Count / 2)], $a[0], $a[-1], $w[[int]($w.Count / 2)], $w[0], $w[-1], $a.Count
        }
        $rows | Export-Csv -NoTypeInformation (Join-Path $Scratch "timing-$(Get-Date -Format HHmmss).csv")
    }
    'memory' {
        $dbFull = if ($Cache) { Assert-Throwaway $Cache } else { $null }
        foreach ($c in $Config) {
            $p = Start-Live $c $dbFull
            $seen = Wait-Line 'first frame drawn' 600000
            Start-Sleep -Seconds 2
            $m = Read-Mem $p
            $seen | Where-Object { $_ -match '\[(cycle|fill|idle2s|tests|timing)\]' } | ForEach-Object { "  $_" }
            "{0}: private {1} MB | private WS {2} MB | WS {3} MB | peak WS {4} MB | threads {5}" -f $c, $m.private_mb, $m.private_ws_mb, $m.ws_mb, $m.peak_ws_mb, $m.threads
            Stop-Live $p
        }
    }
    'idle' {
        $dbFull = if ($Cache) { Assert-Throwaway $Cache } else { $null }
        foreach ($c in $Config) {
            $p = Start-Live $c $dbFull
            $null = Wait-Line 'first frame drawn' $TimeoutMs
            $h = Wait-Window $p
            if ($Minimize) { [void][BenchNative]::ShowWindow($h, 6); Start-Sleep -Milliseconds 500; $state = "minimized=$([BenchNative]::IsIconic($h))" } else { $state = 'visible' }
            Start-Sleep -Seconds 3
            $p.Refresh()
            $c0 = [uint64]0; [void][BenchNative]::QueryProcessCycleTime($p.Handle, [ref]$c0)
            $t0 = $p.TotalProcessorTime
            $th0 = @{}; Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $($p.Id)" | ForEach-Object { $th0[$_.IDThread] = $_.ContextSwitchesPersec }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Start-Sleep -Seconds $Seconds
            $p.Refresh()
            $c1 = [uint64]0; [void][BenchNative]::QueryProcessCycleTime($p.Handle, [ref]$c1)
            $t1 = $p.TotalProcessorTime
            $el = $sw.Elapsed.TotalSeconds
            $cs = 0; Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $($p.Id)" | ForEach-Object { if ($th0.ContainsKey($_.IDThread)) { $cs += $_.ContextSwitchesPersec - $th0[$_.IDThread] } }
            $hz = [double]$cpu.MaxClockSpeed * 1e6
            $pctCycles = 100.0 * ($c1 - $c0) / ($el * $hz)
            $pctTime = 100.0 * ($t1 - $t0).TotalMilliseconds / ($el * 1000)
            "{0} ({1}, extra={2}): {3:N3} % of one core by cycles, {4:N3} % by CPU time, {5:N1} context switches/s over {6:N0} s" -f $c, $state, ($Extra -join ';'), $pctCycles, $pctTime, ($cs / $el), $el
            Stop-Live $p
        }
    }
}
