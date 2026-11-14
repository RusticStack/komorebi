# CPU profile and allocation counts of four paths of the app: startup (to the
# first frame), scrolling the 2,000-message list, opening the heavy message
# and typing in the composer (docs/PERF.md section 8). It uses the harness
# branch's own tools, because ETW's CPU sampling needs an administrator and
# the Windows Performance Toolkit, which are not installed:
#
#   -ProfExe   komorebi.exe built with `--features sampler` and
#              CARGO_PROFILE_RELEASE_DEBUG=line-tables-only (the in-process
#              sampler, crates/komorebi-gpui/src/sampler.rs). Run it from its
#              build folder: komorebi.pdb must be beside it.
#   -AllocExe  komorebi.exe built with `--features alloc-count` (counting
#              global allocator, alloc_count.rs).
#
#   pwsh -File scripts/opt-prof.ps1 -ProfExe <exe> [-AllocExe <exe>] [-Runs 15] [-Out <dir>]
#
# Every launch is offline on a copy of the seeded cache under -Scratch (made
# by scripts/opt-ab.ps1 -Prepare) with the throwaway-cache switches set on the
# child's environment, asserted before it starts, and ends itself (the
# script's last step is `quit`).
param(
    [string]$ProfExe,
    [string]$AllocExe,
    [int]$Runs = 15,
    [string[]]$Scenarios = @('startup', 'scroll', 'open', 'type'),
    [string]$Scratch = 'C:\dev-overflow\claude-tmp\komorebi-opt',
    [string]$Out,
    [int]$Top = 20
)
$ErrorActionPreference = 'Stop'
$Scenarios = @($Scenarios | ForEach-Object { $_ -split ',' })
$Scratch = [IO.Path]::GetFullPath($Scratch)
if (-not $Out) { $Out = Join-Path $Scratch 'prof' }
$Out = [IO.Path]::GetFullPath($Out)
$raw = Join-Path $Out 'raw'
New-Item -ItemType Directory -Force $raw, (Join-Path $Scratch 'work') | Out-Null
$master = Join-Path $Scratch 'seed-2k.warm.sqlite3'
if (-not (Test-Path $master)) { throw "missing $master (scripts/opt-ab.ps1 -Prepare)" }

function Run-One([string]$exe, [string]$bench, [hashtable]$extra) {
    $real = Join-Path $env:LOCALAPPDATA 'komorebi'
    $db = Join-Path $Scratch ("work\prof-{0}.sqlite3" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
    if ($db.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) { throw 'refusing the real cache folder' }
    Copy-Item $master $db
    try {
        $psi = New-Object Diagnostics.ProcessStartInfo $exe
        $psi.UseShellExecute = $false
        $psi.RedirectStandardError = $true
        $psi.RedirectStandardOutput = $true
        $psi.WorkingDirectory = Split-Path -Parent $exe
        foreach ($k in @($psi.EnvironmentVariables.Keys)) { if ("$k" -like 'KOMOREBI_*' -or "$k" -like 'GPUI_*') { $psi.EnvironmentVariables.Remove("$k") } }
        $psi.EnvironmentVariables['KOMOREBI_OFFLINE'] = '1'
        $psi.EnvironmentVariables['KOMOREBI_DB'] = $db
        $psi.EnvironmentVariables['KOMOREBI_TIMING'] = '1'
        $psi.EnvironmentVariables['KOMOREBI_BENCH'] = $bench
        $psi.EnvironmentVariables['KOMOREBI_PROF_DIR'] = $raw
        foreach ($k in $extra.Keys) { $psi.EnvironmentVariables[$k] = [string]$extra[$k] }
        if ($psi.EnvironmentVariables['KOMOREBI_OFFLINE'] -ne '1' -or $psi.EnvironmentVariables['KOMOREBI_DB'] -ne $db) { throw 'the safety switches are not set' }
        $p = [Diagnostics.Process]::Start($psi)
        $errTask = $p.StandardError.ReadToEndAsync()
        $null = $p.StandardOutput.ReadToEndAsync()
        if (-not $p.WaitForExit(60000)) { $p.Kill(); Write-Warning "$exe did not finish in 60 s (killed, run skipped): $bench"; return @() }
        $p.WaitForExit()
        $errTask.Result -split "`r?`n"
    } finally {
        foreach ($s in '', '-wal', '-shm') { Remove-Item "$db$s" -Force -ErrorAction SilentlyContinue }
    }
}

# ---------------------------------------------------------- CPU profiles

function Merge-Profile([string]$scenario) {
    $rows = @{}
    $files = @(Get-ChildItem $raw -Filter "$scenario*.tsv")
    $busy = 0
    foreach ($f in $files) {
        foreach ($r in (Import-Csv $f.FullName -Delimiter "`t")) {
            $key = $r.function
            if (-not $rows.ContainsKey($key)) { $rows[$key] = [pscustomobject]@{ function = $key; self = 0; incl = 0; file = ''; line = ''; best = -1; module = $r.module } }
            $e = $rows[$key]
            $e.self += [int]$r.self
            $e.incl += [int]$r.incl
            $busy += [int]$r.self
            if ([int]$r.self -gt $e.best) { $e.best = [int]$r.self; $e.file = $r.file; $e.line = $r.line }
        }
    }
    $all = @($rows.Values)
    $total = ($all | Measure-Object self -Sum).Sum
    $sb = New-Object Text.StringBuilder
    [void]$sb.AppendLine("## $scenario")
    [void]$sb.AppendLine()
    [void]$sb.AppendLine("$($files.Count) run(s), $total samples of threads that used CPU and were not in a wait (not milliseconds: a pass over ~35 threads takes longer than the 1 ms period, so read the percentages).")
    [void]$sb.AppendLine()
    [void]$sb.AppendLine('Self time, top ' + $Top + ':')
    [void]$sb.AppendLine()
    [void]$sb.AppendLine('| # | Self samples | % | Function | file:line (hottest instruction) |')
    [void]$sb.AppendLine('|---|---|---|---|---|')
    $i = 0
    foreach ($e in ($all | Sort-Object self -Descending | Select-Object -First $Top)) {
        $i++
        $loc = if ($e.file) { "$($e.file):$($e.line)" -replace '^.*\\(library|src|crates|third_party)\\', '$1\' } else { $e.module }
        [void]$sb.AppendLine(('| {0} | {1} | {2:N1} | `{3}` | {4} |' -f $i, $e.self, (100.0 * $e.self / [math]::Max(1, $total)), ($e.function -replace '\|', '/'), $loc))
    }
    [void]$sb.AppendLine()
    [void]$sb.AppendLine('Inclusive time (callers included), top ' + $Top + ' without the thread start frames:')
    [void]$sb.AppendLine()
    [void]$sb.AppendLine('| # | Incl samples | % | Function |')
    [void]$sb.AppendLine('|---|---|---|---|')
    $i = 0
    foreach ($e in ($all | Where-Object { $_.function -notmatch 'RtlUserThreadStart|BaseThreadInitThunk|^main$|lang_start|std::rt::|std::sys::.*::thread|__scrt_|invoke_main|thread::.*::spawn|FnOnce::call_once' } | Sort-Object incl -Descending | Select-Object -First $Top)) {
        $i++
        [void]$sb.AppendLine(('| {0} | {1} | {2:N1} | `{3}` |' -f $i, $e.incl, (100.0 * $e.incl / [math]::Max(1, $total)), ($e.function -replace '\|', '/')))
    }
    $sb.ToString()
}

$report = New-Object Text.StringBuilder
if ($ProfExe) {
    $ProfExe = [IO.Path]::GetFullPath($ProfExe)
    if (-not (Test-Path ([IO.Path]::ChangeExtension($ProfExe, 'pdb')))) { Write-Warning 'no komorebi.pdb beside the exe: functions will have no file and line' }
    Get-ChildItem $raw -Filter '*.tsv' -ErrorAction SilentlyContinue | Remove-Item -Force
    Get-ChildItem $raw -Filter '*.txt' -ErrorAction SilentlyContinue | Remove-Item -Force
    foreach ($sc in $Scenarios) {
        switch ($sc) {
            'startup' { 1..$Runs | ForEach-Object { Run-One $ProfExe 'sample-end,quit' @{ KOMOREBI_PROF_START = "startup_$_" } | Where-Object { $_ -match '^\[prof\]' } } }
            'scroll'  { 1..2 | ForEach-Object { Run-One $ProfExe "wait:1500,sample:scroll_$_,scroll:1500:60,sample-end,quit" @{} | Where-Object { $_ -match '^\[prof\]' } } }
            'open'    { 1..$Runs | ForEach-Object { Run-One $ProfExe "wait:800,sample:open_$_,open:0,sample-end,quit" @{} | Where-Object { $_ -match '^\[prof\]' } } }
            'type'    { 1..2 | ForEach-Object { Run-One $ProfExe "wait:1500,compose,wait:300,sample:type_$_,type:800,sample-end,quit" @{} | Where-Object { $_ -match '^\[prof\]' } } }
        }
        [void]$report.AppendLine((Merge-Profile $sc))
    }
}

# ------------------------------------------------------ allocation counts

if ($AllocExe) {
    $AllocExe = [IO.Path]::GetFullPath($AllocExe)
    $n = [math]::Min($Runs, 5)
    [void]$report.AppendLine('## Allocations (counting global allocator, all threads, median of ' + $n + ' runs)')
    [void]$report.AppendLine()
    [void]$report.AppendLine('| Path | Allocations | Bytes asked for | Peak live bytes (process) | Per unit |')
    [void]$report.AppendLine('|---|---|---|---|---|')
    function Alloc-Delta([string[]]$lines, [string]$from, [string]$to) {
        $get = {
            param($name)
            foreach ($l in $lines) { if ($l -match "^\[alloc\] $name allocs=(\d+) bytes=(\d+) live=(\d+) peak=(\d+)") { return [pscustomobject]@{ allocs = [double]$Matches[1]; bytes = [double]$Matches[2]; live = [double]$Matches[3]; peak = [double]$Matches[4] } } }
        }
        $a = if ($from) { & $get $from } else { [pscustomobject]@{ allocs = 0; bytes = 0; live = 0; peak = 0 } }
        $b = & $get $to
        if ($b) { [pscustomobject]@{ allocs = $b.allocs - $a.allocs; bytes = $b.bytes - $a.bytes; peak = $b.peak } }
    }
    function Median([double[]]$v) { $s = @($v | Sort-Object); if ($s.Count) { $s[[int][math]::Floor($s.Count / 2)] } }
    $paths = @(
        @{ name = 'Startup, process start to first frame'; bench = 'quit'; from = ''; to = 'first_frame'; unit = 1; unitName = 'launch' },
        @{ name = 'Open the heavy message (row 0), wait for its tiles'; bench = 'wait:800,open:0,quit'; from = 'open0'; to = 'open1'; unit = 1; unitName = 'open' },
        @{ name = 'Scroll the list, 300 wheel steps'; bench = 'wait:1500,scroll:300:60,quit'; from = 'scroll0'; to = 'scroll1'; unit = 300; unitName = 'frame' },
        @{ name = 'Type 200 keys in the composer'; bench = 'wait:1500,compose,type:200,quit'; from = 'type0'; to = 'type1'; unit = 200; unitName = 'key' }
    )
    foreach ($pa in $paths) {
        $d = @(1..$n | ForEach-Object { Alloc-Delta (Run-One $AllocExe $pa.bench @{}) $pa.from $pa.to } | Where-Object { $_ })
        if ($d.Count) {
            $a = Median ($d.allocs); $b = Median ($d.bytes); $pk = Median ($d.peak)
            [void]$report.AppendLine(('| {0} | {1:N0} | {2:N0} | {3:N0} | {4:N1} allocations and {5:N0} bytes per {6} |' -f $pa.name, $a, $b, $pk, ($a / $pa.unit), ($b / $pa.unit), $pa.unitName))
        }
    }
}

$text = $report.ToString()
$text | Set-Content (Join-Path $Out 'profile.md') -Encoding utf8
$text
"written: $(Join-Path $Out 'profile.md')"
