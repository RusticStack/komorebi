# Interleaved A/B harness for the optimization pass (docs/PERF.md section 8).
# Takes two komorebi.exe paths and measures both the same way, round by round,
# alternating which goes first (A B, B A, A B ...) so drift hits both.
#
#   pwsh -File scripts/opt-ab.ps1 -A <exe> -B <exe> [-Rounds 9] [-IdleRounds 3]
#        [-Out <dir>] [-Scratch <dir>] [-Noise <results.json of an A=B run>]
#        [-Metrics launch,mem,open,read,scroll,type,idle] [-Warp] [-Prepare]
#        [-Cache <seeded cache for open and read>] [-OpenRow 0]
#
# Per round, per exe (median, min, max and the run count go to results.json
# and report.md):
#   launch  first frame drawn, in-app (KOMOREBI_TIMING) and wall (Process.Start to
#           the line on stderr): an empty cache, the seeded 2,000-message
#           cache and the 55k one (the cached list is drawn in that first
#           frame; the [timing] marks between main and the frame are kept too)
#   mem     private bytes, private working set, working set, peak working
#           set, threads and handles, 2 s after the first frame (live window,
#           no snapshot) and, with the in-app script, 2 s after the heavy
#           message (row 0, tests/fixtures/reader/04-heavy-statement.eml) is
#           open and its tiles in view are drawn
#   open    that message: open to layout, to first tile, to all visible tiles,
#           to complete_view (all of them in a rendered frame, images decoded,
#           the whole page laid out) and complete_ahead (also the tiles of the
#           read-ahead window) (in-app clock from ReaderView::open,
#           crates/komorebi-gpui/src/bench.rs)
#   read    the same message scrolled -ReadPx per frame for -ReadFrames frames,
#           to its end and back: frames with a visible tile not painted yet,
#           those tiles over all frames, and the longest such stretch
#   scroll  300 wheel steps over the 2,000 list, one per frame: wheel to frame
#           rendered, and process cycles per frame
#   type    200 keys into a new message's body, one per frame: key to frame
#           rendered (includes the wait for the next display frame)
#   idle    QueryProcessCycleTime and context switches over 30 s, visible and
#           minimized (IdleRounds rounds: they are slow)
#
# The machine load is read (Win32_Processor.LoadPercentage, mean of 5 reads)
# at the start and the end of the batch and at the start of every round; the
# batch is marked INVALID when the start or end load is above -MaxLoad (25).
#
# Safety: every launch sets KOMOREBI_OFFLINE=1 and KOMOREBI_DB (a copy of a
# seeded cache under -Scratch; anything else is refused, and the app's own
# cache folder is refused) on the child's environment, asserted before it
# starts; all other KOMOREBI_* variables of this process are removed from the
# child's. Runs that are not a first-frame exit use KOMOREBI_BENCH, whose
# script ends in `quit` (an exe without the hook is closed by this script and
# killed after a timeout). Never a real account, never .env*.
#
# An exe without the in-app hook (a build of main before the harness
# commit) is detected by its missing KOMOREBI_BENCH string; open, scroll and
# type are then skipped for it (null in the JSON).
#
# -Prepare builds nothing: it seeds the two caches (needs the seed example,
# -SeedExe, built with `cargo build --release --example seed`).
param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [string]$LabelA = 'A',
    [string]$LabelB = 'B',
    [int]$Rounds = 9,
    [int]$IdleRounds = 3,
    [int]$IdleSeconds = 30,
    [string[]]$Metrics = @('launch', 'mem', 'open', 'read', 'scroll', 'type', 'idle'),
    [string]$Scratch = 'C:\dev-overflow\claude-tmp\komorebi-opt',
    [string]$Out,
    [string]$Noise,
    [double]$MaxLoad = 25,
    # Wait (in 15 s steps, at most this many minutes in all) for the load to be
    # at or below -MaxLoad before the batch and before each round; then go on
    # anyway (the batch is marked INVALID if the start or end load is above).
    [double]$WaitQuiet = 0,
    [switch]$Warp,
    [switch]$Prepare,
    [string]$SeedExe,
    [int]$LaunchTimeoutMs = 30000,
    [int]$ScrollFrames = 300,
    # Which cache and row the open and read runs use (another message than the
    # heavy statement: a cache seeded with -SeedExe from a folder of .eml).
    [string]$Cache,
    [int]$OpenRow = 0,
    [int]$ReadFrames = 240,
    [int]$ReadPx = 120,
    [int]$TypeKeys = 200
)
$ErrorActionPreference = 'Stop'
$Metrics = @($Metrics | ForEach-Object { $_ -split ',' })
$repo = Split-Path -Parent $PSScriptRoot
$Scratch = [IO.Path]::GetFullPath($Scratch)
if (-not $Out) { $Out = Join-Path $Scratch ("runs\" + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$Out = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Force $Out, (Join-Path $Scratch 'work') | Out-Null
$work = Join-Path $Scratch 'work'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class OptNative {
    [DllImport("kernel32.dll")] public static extern bool QueryProcessCycleTime(IntPtr h, out ulong cycles);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
// A child whose stderr lines are stamped (ms since Process.Start) on .NET
// threads, not PowerShell ones.
public class OptChild {
    public Process P = new Process();
    public Stopwatch Sw;
    readonly List<string> lines = new List<string>();
    readonly List<double> times = new List<double>();
    readonly object gate = new object();
    public OptChild(ProcessStartInfo psi) {
        P.StartInfo = psi;
        P.ErrorDataReceived += (s, e) => {
            if (e.Data == null) return;
            lock (gate) { times.Add(Sw.Elapsed.TotalMilliseconds); lines.Add(e.Data); }
        };
        P.OutputDataReceived += (s, e) => { };
        Sw = Stopwatch.StartNew();
        P.Start();
        P.BeginErrorReadLine();
        P.BeginOutputReadLine();
    }
    public int Count { get { lock (gate) { return lines.Count; } } }
    public string Line(int i) { lock (gate) { return lines[i]; } }
    public double Time(int i) { lock (gate) { return times[i]; } }
    public int Find(string needle, int from) {
        lock (gate) {
            for (int i = from; i < lines.Count; i++) if (lines[i].Contains(needle)) return i;
        }
        return -1;
    }
    public string[] All() { lock (gate) { return lines.ToArray(); } }
}
'@ -ErrorAction SilentlyContinue

# ---------------------------------------------------------------- helpers

function Median([double[]]$v) {
    $s = @($v | Sort-Object)
    if ($s.Count -eq 0) { return $null }
    $m = [int][math]::Floor($s.Count / 2)
    if ($s.Count % 2 -eq 1) { $s[$m] } else { ($s[$m - 1] + $s[$m]) / 2 }
}

function Machine-Load([int]$reads = 5) {
    $v = @()
    for ($i = 0; $i -lt $reads; $i++) {
        $v += (Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
        if ($i -lt $reads - 1) { Start-Sleep -Seconds 1 }
    }
    [math]::Round(($v | Measure-Object -Average).Average, 1)
}

function Wait-Quiet([string]$why) {
    if ($WaitQuiet -le 0) { return [double](Machine-Load 3) }
    $load = [double](Machine-Load 3)
    while ($load -gt $MaxLoad -and $script:quietSpent -lt $WaitQuiet * 60) {
        Write-Host "waiting for a quiet machine ($why): load $load%, waited $([int]$script:quietSpent) s of $($WaitQuiet * 60) s"
        Start-Sleep -Seconds 12
        $script:quietSpent += 15
        $load = [double](Machine-Load 3)
    }
    [double]$load
}
$script:quietSpent = 0

function Assert-Throwaway([string]$path) {
    if (-not $path) { throw 'KOMOREBI_DB is required' }
    $full = [IO.Path]::GetFullPath($path)
    $real = Join-Path $env:LOCALAPPDATA 'komorebi'
    if ($full.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) { throw "refusing the real cache folder $real" }
    if (-not $full.StartsWith($Scratch, [StringComparison]::OrdinalIgnoreCase)) { throw "the cache must be inside -Scratch ($Scratch): $full" }
    $full
}

function Has-Hook([string]$exe) {
    [Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($exe)).Contains('KOMOREBI_BENCH')
}
# The child's environment: a clean slate of KOMOREBI_* and GPUI_* switches,
# then the safety pair and the run's own. Asserted before the start.
function Start-App([string]$exe, [string]$db, [hashtable]$envs) {
    $psi = New-Object Diagnostics.ProcessStartInfo $exe
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    foreach ($k in @($psi.EnvironmentVariables.Keys)) {
        if ("$k" -like 'KOMOREBI_*' -or "$k" -like 'GPUI_*') { $psi.EnvironmentVariables.Remove("$k") }
    }
    $psi.EnvironmentVariables['KOMOREBI_OFFLINE'] = '1'
    $psi.EnvironmentVariables['KOMOREBI_DB'] = (Assert-Throwaway $db)
    foreach ($k in $envs.Keys) { $psi.EnvironmentVariables[$k] = [string]$envs[$k] }
    if ($Warp) { $psi.EnvironmentVariables['KOMOREBI_GPUI_FORCE_WARP'] = '1' }
    if ($psi.EnvironmentVariables['KOMOREBI_OFFLINE'] -ne '1') { throw 'KOMOREBI_OFFLINE must be 1' }
    [void](Assert-Throwaway $psi.EnvironmentVariables['KOMOREBI_DB'])
    [OptChild]::new($psi)
}

function Stop-App([OptChild]$ch, [int]$graceMs = 6000) {
    $p = $ch.P
    try {
        if (-not $p.HasExited) {
            [void]$p.CloseMainWindow()
            if (-not $p.WaitForExit($graceMs)) { $p.Kill(); [void]$p.WaitForExit(5000) }
        }
    } catch { }
}

# Waits for a line containing $needle after index $from; returns its index, or
# -1 on timeout or when the process ends first.
function Wait-Line([OptChild]$ch, [string]$needle, [int]$ms, [int]$from = 0) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $ms) {
        $i = $ch.Find($needle, $from)
        if ($i -ge 0) { return $i }
        if ($ch.P.HasExited) { return $ch.Find($needle, $from) }
        [Threading.Thread]::Sleep(2)
    }
    -1
}

function Sleep-Until([OptChild]$ch, [double]$stampMs) {
    $left = $stampMs - $ch.Sw.Elapsed.TotalMilliseconds
    if ($left -gt 0) { [Threading.Thread]::Sleep([int][math]::Ceiling($left)) }
}

function Read-Mem([Diagnostics.Process]$p, [string]$prefix) {
    $p.Refresh()
    $ws = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter "IDProcess = $($p.Id)"
    @{
        "${prefix}private_mb"    = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
        "${prefix}private_ws_mb" = [math]::Round($ws.WorkingSetPrivate / 1MB, 2)
        "${prefix}ws_mb"         = [math]::Round($p.WorkingSet64 / 1MB, 2)
        "${prefix}peak_ws_mb"    = [math]::Round($p.PeakWorkingSet64 / 1MB, 2)
        "${prefix}threads"       = $p.Threads.Count
        "${prefix}handles"       = $p.HandleCount
    }
}

function Cycles([Diagnostics.Process]$p) {
    $c = [uint64]0
    [void][OptNative]::QueryProcessCycleTime($p.Handle, [ref]$c)
    [double]$c
}

function Wait-Window([Diagnostics.Process]$p) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 15000) {
        $p.Refresh()
        if ($p.MainWindowHandle -ne 0) { return $p.MainWindowHandle }
        [Threading.Thread]::Sleep(50)
    }
    [IntPtr]::Zero
}

# A fresh copy of a master cache for one launch.
function New-Copy([string]$master, [string]$tag) {
    $dst = Join-Path $work ("{0}-{1}.sqlite3" -f $tag, [guid]::NewGuid().ToString('N').Substring(0, 8))
    Copy-Item $master $dst
    $dst
}
function Remove-Copy([string]$db) {
    foreach ($s in '', '-wal', '-shm', '.key', '.previous') { Remove-Item "$db$s" -Force -ErrorAction SilentlyContinue }
}

# ------------------------------------------------------------ the caches

$heavyDir = Join-Path $Scratch 'heavy-eml'
$master = @{
    empty = $null
    k2    = Join-Path $Scratch 'seed-2k.warm.sqlite3'
    k55   = Join-Path $Scratch 'seed-55k.warm.sqlite3'
}

function Invoke-Prepare {
    if (-not $SeedExe -or -not (Test-Path $SeedExe)) { throw '-Prepare needs -SeedExe (cargo build --release --example seed)' }
    New-Item -ItemType Directory -Force $heavyDir | Out-Null
    Copy-Item (Join-Path $repo 'tests\fixtures\reader\04-heavy-statement.eml') $heavyDir -Force
    foreach ($spec in @(@('seed-2k.sqlite3', 2000), @('seed-55k.sqlite3', 55000))) {
        $db = Join-Path $Scratch $spec[0]
        $warm = $db -replace '\.sqlite3$', '.warm.sqlite3'
        if (Test-Path $warm) { "keeping $warm"; continue }
        Remove-Copy $db
        & $SeedExe $db $spec[1] $heavyDir | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "seed failed for $db" }
        Rename-Item $db $warm
    }
}

# One launch on each master, so that the default-fold writes of a first
# launch are in the file every run copies ("warm" caches, docs/PERF.md 8).
function Warm-Masters([string]$exe) {
    foreach ($k in 'k2', 'k55') {
        $db = New-Copy $master[$k] 'prime'
        $ch = Start-App $exe $db @{ KOMOREBI_TIMING = '1'; KOMOREBI_EXIT_AFTER_FIRST_FRAME = '1' }
        [void]$ch.P.WaitForExit($LaunchTimeoutMs)
        Stop-App $ch
        Remove-Copy $db
    }
}

# --------------------------------------------------------------- metrics

function Parse-Timing([OptChild]$ch, [string]$prefix, [hashtable]$r) {
    foreach ($l in $ch.All()) {
        if ($l -match '^\[timing\] (.+?): ([0-9.]+) ms') {
            $name = ($Matches[1] -replace '[^A-Za-z0-9]+', '_').Trim('_')
            $r["${prefix}mark_$name"] = [double]$Matches[2]
        }
    }
}

function Run-Launch([string]$exe, [string]$key) {
    $db = if ($key -eq 'empty') { Join-Path $work ("empty-{0}.sqlite3" -f [guid]::NewGuid().ToString('N').Substring(0, 8)) } else { New-Copy $master[$key] $key }
    $r = @{}
    try {
        $ch = Start-App $exe $db @{ KOMOREBI_TIMING = '1'; KOMOREBI_EXIT_AFTER_FIRST_FRAME = '1' }
        if (-not $ch.P.WaitForExit($LaunchTimeoutMs)) { $ch.P.Kill(); throw "$exe did not exit" }
        $ch.P.WaitForExit()
        $i = $ch.Find('first frame drawn', 0)
        if ($i -lt 0 -or $ch.P.ExitCode -ne 0) { throw "launch failed (exit $($ch.P.ExitCode)): $($ch.All() -join ' | ')" }
        if ($ch.Line($i) -match 'first frame drawn: ([0-9.]+) ms') { $r["launch_${key}_app_ms"] = [double]$Matches[1] }
        $r["launch_${key}_wall_ms"] = [math]::Round($ch.Time($i), 1)
        Parse-Timing $ch "launch_${key}_" $r
        $r.Remove("launch_${key}_mark_first_frame_drawn")
    } finally { Remove-Copy $db }
    $r
}

function Run-MemFirst([string]$exe) {
    $db = New-Copy $master.k2 'mem'
    $r = @{}
    try {
        $ch = Start-App $exe $db @{ KOMOREBI_TIMING = '1' }
        try {
            $i = Wait-Line $ch 'first frame drawn' $LaunchTimeoutMs
            if ($i -lt 0) { throw 'no first frame' }
            Sleep-Until $ch ($ch.Time($i) + 2000)
            $r += Read-Mem $ch.P 'mem_first_'
        } finally { Stop-App $ch }
    } finally { Remove-Copy $db }
    $r
}

# The in-app script runs: open the heavy message, scroll, type.
function Run-Script([string]$exe, [string]$script, [scriptblock]$during, [string]$from = $master.k2) {
    $db = New-Copy $from 'bench'
    $r = @{}
    try {
        $ch = Start-App $exe $db @{ KOMOREBI_TIMING = '1'; KOMOREBI_BENCH = $script }
        try {
            $i = Wait-Line $ch 'first frame drawn' $LaunchTimeoutMs
            if ($i -lt 0) { throw "no first frame: $($ch.All() -join ' | ')" }
            & $during $ch $r
            if (-not $ch.P.WaitForExit(20000)) { throw 'the script did not finish' }
            Parse-Bench $ch $r
        } finally { Stop-App $ch }
    } finally { Remove-Copy $db }
    $r
}

function Parse-Bench([OptChild]$ch, [hashtable]$r) {
    foreach ($l in $ch.All()) {
        if ($l -match '^\[bench\] open (parsed|page|first_tile|settled|complete_view|complete_ahead) ([0-9.]+)') { $r["open_$($Matches[1])_ms"] = [double]$Matches[2] }
        elseif ($l -match '^\[bench\] open view (page|text)') { $r['open_text_view'] = [double]($Matches[1] -eq 'text') }
        elseif ($l -match '^\[bench\] read frames=(\d+) blank_frames=(\d+) blank_tiles=(\d+) worst_blank_ms=([0-9.]+) moved=([0-9.]+)') {
            $r['read_blank_frames'] = [double]$Matches[2]
            $r['read_blank_tiles'] = [double]$Matches[3]
            $r['read_worst_blank_ms'] = [double]$Matches[4]
            $r['read_moved_px'] = [double]$Matches[5]
        }
        elseif ($l -match '^\[bench\] (scroll|type) (?:frames|keys)=(\d+) median=([0-9.]+) p95=([0-9.]+) max=([0-9.]+) min=([0-9.]+)(?: moved=([0-9.]+))?(?: typed=(\d+))?') {
            $n = $Matches[1]
            $r["${n}_count"] = [double]$Matches[2]
            $r["${n}_median_ms"] = [double]$Matches[3]
            $r["${n}_p95_ms"] = [double]$Matches[4]
            $r["${n}_max_ms"] = [double]$Matches[5]
            if ($Matches[7]) { $r["${n}_moved_px"] = [double]$Matches[7] }
            if ($Matches[8]) { $r["${n}_typed_bytes"] = [double]$Matches[8] }
        }
    }
}

function Run-Open([string]$exe) {
    Run-Script $exe "wait:2600,mark:first,open:$OpenRow,mark:opened,wait:6000,mark:final,quit" {
        param($ch, $r)
        $j = Wait-Line $ch 'mark opened' 30000
        if ($j -lt 0) { throw 'the message did not open' }
        Sleep-Until $ch ($ch.Time($j) + 2000)
        $mem = Read-Mem $ch.P 'mem_open_'
        foreach ($k in $mem.Keys) { $r[$k] = $mem[$k] }
    } $script:openCache
}

function Run-Read([string]$exe) {
    Run-Script $exe "wait:2600,open:$OpenRow,wait:300,read:${ReadFrames}:${ReadPx},quit" {
        param($ch, $r)
        $j = Wait-Line $ch 'read frames' 60000
        if ($j -lt 0) { throw 'the read did not finish' }
    } $script:openCache
}

function Run-Scroll([string]$exe) {
    Run-Script $exe "wait:1500,mark:s0,scroll:${ScrollFrames}:60,mark:s1,wait:300,quit" {
        param($ch, $r)
        $i0 = Wait-Line $ch 'mark s0' 20000
        if ($i0 -lt 0) { throw 'no scroll start' }
        $c0 = Cycles $ch.P
        $i1 = Wait-Line $ch 'mark s1' 60000
        if ($i1 -lt 0) { throw 'no scroll end' }
        $c1 = Cycles $ch.P
        $r['scroll_mcycles_per_frame'] = [math]::Round(($c1 - $c0) / 1e6 / $ScrollFrames, 3)
        $r['scroll_wall_s'] = [math]::Round(($ch.Time($i1) - $ch.Time($i0)) / 1000, 3)
    }
}

function Run-Type([string]$exe) {
    Run-Script $exe "wait:1500,compose,type:${TypeKeys},quit" { param($ch, $r) }
}

function Run-Idle([string]$exe, [bool]$minimize, [bool]$hook) {
    $db = New-Copy $master.k2 'idle'
    $r = @{}
    $tag = if ($minimize) { 'idle_min_' } else { 'idle_vis_' }
    try {
        $envs = @{ KOMOREBI_TIMING = '1' }
        if ($hook) { $envs['KOMOREBI_BENCH'] = 'wait:60000,quit' }
        $ch = Start-App $exe $db $envs
        try {
            $i = Wait-Line $ch 'first frame drawn' $LaunchTimeoutMs
            if ($i -lt 0) { throw 'no first frame' }
            $p = $ch.P
            $h = Wait-Window $p
            if ($minimize) {
                [void][OptNative]::ShowWindow($h, 6)
                Start-Sleep -Milliseconds 500
                if (-not [OptNative]::IsIconic($h)) { throw 'the window did not minimize' }
            }
            Start-Sleep -Seconds 3
            $p.Refresh()
            $c0 = Cycles $p
            $th0 = @{}
            Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $($p.Id)" | ForEach-Object { $th0[$_.IDThread] = $_.ContextSwitchesPersec }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Start-Sleep -Seconds $IdleSeconds
            $c1 = Cycles $p
            $el = $sw.Elapsed.TotalSeconds
            $cs = 0
            Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter "IDProcess = $($p.Id)" | ForEach-Object { if ($th0.ContainsKey($_.IDThread)) { $cs += $_.ContextSwitchesPersec - $th0[$_.IDThread] } }
            $hz = [double]$script:cpuInfo.MaxClockSpeed * 1e6
            $r["${tag}pct_core"] = [math]::Round(100.0 * ($c1 - $c0) / ($el * $hz), 4)
            $r["${tag}mcycles_per_s"] = [math]::Round(($c1 - $c0) / 1e6 / $el, 3)
            $r["${tag}cs_per_s"] = [math]::Round($cs / $el, 2)
        } finally { Stop-App $ch }
    } finally { Remove-Copy $db }
    $r
}

# ------------------------------------------------------------- the batch

if ($Prepare) { Invoke-Prepare }
$script:openCache = if ($Cache) { [IO.Path]::GetFullPath($Cache) } else { $master.k2 }
if (-not (Test-Path $script:openCache)) { throw "no cache at $($script:openCache)" }
foreach ($m in $master.k2, $master.k55) { if (-not (Test-Path $m)) { throw "missing $m (run with -Prepare -SeedExe <seed.exe> first)" } }
foreach ($e in $A, $B) { if (-not (Test-Path $e)) { throw "no exe at $e" } }
$A = [IO.Path]::GetFullPath($A); $B = [IO.Path]::GetFullPath($B)
$exes = [ordered]@{ $LabelA = $A; $LabelB = $B }
if ($LabelA -eq $LabelB) { throw 'the labels must differ' }
$hook = @{}
foreach ($l in $exes.Keys) { $hook[$l] = Has-Hook $exes[$l] }

$script:cpuInfo = Get-CimInstance Win32_Processor | Select-Object -First 1
$loadStart = Wait-Quiet 'batch start'
$startedAt = Get-Date
"# opt-ab: $LabelA=$A hook=$($hook[$LabelA])  $LabelB=$B hook=$($hook[$LabelB])"
"# load at start $loadStart% (limit $MaxLoad%), cpu '$($script:cpuInfo.Name)', rounds $Rounds, idle rounds $IdleRounds, out $Out"

$samples = @{}       # metric -> label -> [double list]
$failures = @()
$roundLoads = @()
function Add-Sample([string]$label, $res) {
    $script:rawFile = Join-Path $Out 'raw.jsonl'
    $r = @($res) | Where-Object { $_ -is [hashtable] } | Select-Object -Last 1
    if (-not $r) { return }
    ([ordered]@{ label = $label; at = (Get-Date).ToString('s'); values = $r } | ConvertTo-Json -Depth 4 -Compress) | Add-Content $script:rawFile
    foreach ($k in $r.Keys) {
        if ($null -eq $r[$k]) { continue }
        if (-not $samples.ContainsKey($k)) { $samples[$k] = @{} }
        if (-not $samples[$k].ContainsKey($label)) { $samples[$k][$label] = New-Object Collections.Generic.List[double] }
        $samples[$k][$label].Add([double]$r[$k])
    }
}
function Try-Run([string]$label, [string]$what, [scriptblock]$f) {
    try { Add-Sample $label (& $f) }
    catch { $script:failures += "[$label] ${what}: $($_.Exception.Message)"; Write-Warning "[$label] ${what}: $($_.Exception.Message)" }
}

# Unmeasured: the first launch of a new exe pays Defender's scan and the page cache.
foreach ($l in $exes.Keys) { Warm-Masters $exes[$l] }

for ($round = 1; $round -le $Rounds; $round++) {
    $order = if ($round % 2 -eq 1) { @($LabelA, $LabelB) } else { @($LabelB, $LabelA) }
    $roundLoads += [pscustomobject]@{ round = $round; load = (Wait-Quiet "round $round") }
    Write-Host "$(Get-Date -Format T) round $round/$Rounds order $($order -join ' ') load $($roundLoads[-1].load)%"
    if ($Metrics -contains 'launch') {
        foreach ($key in 'empty', 'k2', 'k55') {
            foreach ($l in $order) { Try-Run $l "launch $key" { Run-Launch $exes[$l] $key } }
        }
    }
    if ($Metrics -contains 'mem') {
        foreach ($l in $order) { Try-Run $l 'mem first' { Run-MemFirst $exes[$l] } }
    }
    if ($Metrics -contains 'open') {
        foreach ($l in $order) { if ($hook[$l]) { Try-Run $l 'open' { Run-Open $exes[$l] } } }
    }
    if ($Metrics -contains 'read') {
        foreach ($l in $order) { if ($hook[$l]) { Try-Run $l 'read' { Run-Read $exes[$l] } } }
    }
    if ($Metrics -contains 'scroll') {
        foreach ($l in $order) { if ($hook[$l]) { Try-Run $l 'scroll' { Run-Scroll $exes[$l] } } }
    }
    if ($Metrics -contains 'type') {
        foreach ($l in $order) { if ($hook[$l]) { Try-Run $l 'type' { Run-Type $exes[$l] } } }
    }
}
if ($Metrics -contains 'idle') {
    for ($round = 1; $round -le $IdleRounds; $round++) {
        $order = if ($round % 2 -eq 1) { @($LabelA, $LabelB) } else { @($LabelB, $LabelA) }
        $roundLoads += [pscustomobject]@{ round = "idle$round"; load = (Wait-Quiet "idle round $round") }
        "idle round $round/$IdleRounds order $($order -join ' ') load $($roundLoads[-1].load)%"
        foreach ($min in $false, $true) {
            foreach ($l in $order) { Try-Run $l "idle min=$min" { Run-Idle $exes[$l] $min $hook[$l] } }
        }
    }
}

$loadEnd = Machine-Load 5
$valid = ($loadStart -le $MaxLoad) -and ($loadEnd -le $MaxLoad)
$maxRound = ($roundLoads | Measure-Object load -Maximum).Maximum

# ---------------------------------------------------------------- results

function Stats($list) {
    $v = @($list)
    if ($v.Count -eq 0) { return $null }
    $s = @($v | Sort-Object)
    [ordered]@{ n = $s.Count; median = [math]::Round((Median $s), 4); min = $s[0]; max = $s[-1]; mean = [math]::Round((($s | Measure-Object -Average).Average), 4) }
}

$noise = $null
if ($Noise -and (Test-Path $Noise)) { $noise = Get-Content $Noise -Raw | ConvertFrom-Json }

$result = [ordered]@{
    tool         = 'scripts/opt-ab.ps1'
    started      = $startedAt.ToString('s')
    finished     = (Get-Date).ToString('s')
    valid        = $valid
    load         = [ordered]@{ start = $loadStart; end = $loadEnd; max_round_start = $maxRound; limit = $MaxLoad; quiet_wait_s = $script:quietSpent; rounds = $roundLoads }
    machine      = [ordered]@{ cpu = $script:cpuInfo.Name; logical = $script:cpuInfo.NumberOfLogicalProcessors; max_mhz = $script:cpuInfo.MaxClockSpeed; os = (Get-CimInstance Win32_OperatingSystem).Caption; warp = [bool]$Warp }
    exes         = [ordered]@{}
    rounds       = $Rounds
    idle_rounds  = $IdleRounds
    failures     = $failures
    metrics      = [ordered]@{}
}
foreach ($l in $exes.Keys) {
    $f = Get-Item $exes[$l]
    $result.exes[$l] = [ordered]@{ path = $exes[$l]; bytes = $f.Length; sha256 = (Get-FileHash $exes[$l] -Algorithm SHA256).Hash; bench_hook = $hook[$l] }
}
foreach ($k in ($samples.Keys | Sort-Object)) {
    $entry = [ordered]@{}
    foreach ($l in $exes.Keys) {
        if ($samples[$k].ContainsKey($l)) {
            $st = Stats $samples[$k][$l]
            $st['samples'] = @($samples[$k][$l])
            $entry[$l] = $st
        }
    }
    if ($entry.Contains($LabelA) -and $entry.Contains($LabelB)) {
        $sa = $entry[$LabelA]; $sb = $entry[$LabelB]
        $d = $sb.median - $sa.median
        $spread = [math]::Max($sa.max - $sa.min, $sb.max - $sb.min)
        $entry['delta'] = [ordered]@{
            median_b_minus_a = [math]::Round($d, 4)
            pct              = if ($sa.median) { [math]::Round(100.0 * $d / $sa.median, 2) } else { $null }
            spread           = [math]::Round($spread, 4)
            beyond_spread    = ([math]::Abs($d) -gt $spread)
        }
        # Paired by round (the samples are in round order): |B - A| per round.
        $pa = @($sa.samples); $pb = @($sb.samples)
        $np = [math]::Min($pa.Count, $pb.Count)
        if ($np -gt 0) {
            $pairs = @(0..($np - 1) | ForEach-Object { [math]::Abs($pb[$_] - $pa[$_]) })
            $entry['delta']['paired_abs_median'] = [math]::Round((Median $pairs), 4)
            $entry['delta']['paired_abs_max'] = [math]::Round(($pairs | Measure-Object -Maximum).Maximum, 4)
        }
        if ($noise -and $noise.metrics.PSObject.Properties.Name -contains $k) {
            $nz = $noise.metrics.$k.delta
            if ($nz) {
                $floor = [double]$nz.paired_abs_max
                $entry['delta']['noise_floor'] = $floor
                $entry['delta']['verdict'] = if ([math]::Abs($d) -le $floor) { 'within noise' } elseif ($d -lt 0) { 'B lower' } else { 'B higher' }
            }
        }
    }
    $result.metrics[$k] = $entry
}
$result | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $Out 'results.json') -Encoding utf8

# Markdown
$md = New-Object Text.StringBuilder
[void]$md.AppendLine("# opt-ab: $LabelA vs $LabelB")
[void]$md.AppendLine()
[void]$md.AppendLine("- Started $($result.started), finished $($result.finished). Rounds $Rounds, idle rounds $IdleRounds ($IdleSeconds s each).")
[void]$md.AppendLine("- Batch: **$(if ($valid) { 'valid' } else { 'INVALID (machine load above ' + $MaxLoad + ' percent)' })**. Load at start $loadStart percent, at end $loadEnd percent, highest round start $maxRound percent.")
[void]$md.AppendLine("- Machine: $($result.machine.cpu), $($result.machine.logical) logical CPUs, $($result.machine.os)$(if ($Warp) { ', WARP' } else { ', GPU' }).")
foreach ($l in $exes.Keys) { [void]$md.AppendLine("- $l : ``$($exes[$l])`` $($result.exes[$l].bytes) bytes, sha256 $($result.exes[$l].sha256.Substring(0, 12)), in-app hook $($hook[$l])") }
if ($failures.Count) { [void]$md.AppendLine("- Failures: $($failures.Count) (see results.json)") }
[void]$md.AppendLine()
[void]$md.AppendLine("| Metric | $LabelA median (min to max) | n | $LabelB median (min to max) | n | B - A | % | spread | paired max | verdict |")
[void]$md.AppendLine('|---|---|---|---|---|---|---|---|---|---|')
foreach ($k in $result.metrics.Keys) {
    $e = $result.metrics[$k]
    $cell = { param($x) if ($x) { "$($x.median) ($($x.min) to $($x.max))", $x.n } else { '-', 0 } }
    $ca = & $cell $e[$LabelA]; $cb = & $cell $e[$LabelB]
    $d = $e['delta']
    $dd = if ($d) { "$($d.median_b_minus_a) | $($d.pct) | $($d.spread) | $($d.paired_abs_max) | $(if ($d.verdict) { $d.verdict } elseif ($d.beyond_spread) { 'beyond spread' } else { 'within spread' })" } else { '- | - | - | - | -' }
    [void]$md.AppendLine("| $k | $($ca[0]) | $($ca[1]) | $($cb[0]) | $($cb[1]) | $dd |")
}
$md.ToString() | Set-Content (Join-Path $Out 'report.md') -Encoding utf8
$md.ToString()
"results: $(Join-Path $Out 'results.json')"
if (-not $valid) { exit 3 }
