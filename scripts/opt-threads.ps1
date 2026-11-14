# Lists the threads of a live komorebi.exe (offline, throwaway cache): the
# module of each thread's start address, its name (GetThreadDescription, set
# by Rust's thread::Builder and by Windows components), its state and its CPU
# time. Evidence for the thread-count candidate in the optimization list
# (docs/PERF.md section 8). Needs an exe with the KOMOREBI_BENCH hook
# (scripts/opt-ab.ps1); the app quits itself after -Seconds.
#
#   pwsh -File scripts/opt-threads.ps1 -Exe <exe> -Cache <seeded copy under -Scratch> [-Seconds 6] [-After first|open]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Cache,
    [string]$Scratch = 'C:\dev-overflow\claude-tmp\komorebi-opt',
    [int]$Seconds = 6,
    [ValidateSet('first', 'open')][string]$After = 'first'
)
$ErrorActionPreference = 'Stop'
$cache = [IO.Path]::GetFullPath($Cache)
$Scratch = [IO.Path]::GetFullPath($Scratch)
$real = Join-Path $env:LOCALAPPDATA 'komorebi'
if ($cache.StartsWith($real, [StringComparison]::OrdinalIgnoreCase)) { throw "refusing the real cache folder $real" }
if (-not $cache.StartsWith($Scratch, [StringComparison]::OrdinalIgnoreCase)) { throw "the cache must be inside -Scratch: $cache" }
if (-not (Test-Path $cache)) { throw "missing $cache" }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ThreadNative {
    [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(uint access, bool inherit, uint tid);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] public static extern int GetThreadDescription(IntPtr h, out IntPtr name);
    [DllImport("kernel32.dll")] public static extern IntPtr LocalFree(IntPtr p);
    [DllImport("ntdll.dll")] public static extern int NtQueryInformationThread(IntPtr h, int cls, out IntPtr info, int len, IntPtr ret);
}
'@ -ErrorAction SilentlyContinue

$psi = New-Object Diagnostics.ProcessStartInfo $Exe
$psi.UseShellExecute = $false
foreach ($k in @($psi.EnvironmentVariables.Keys)) { if ("$k" -like 'KOMOREBI_*' -or "$k" -like 'GPUI_*') { $psi.EnvironmentVariables.Remove("$k") } }
$psi.EnvironmentVariables['KOMOREBI_OFFLINE'] = '1'
$psi.EnvironmentVariables['KOMOREBI_DB'] = $cache
$wait = if ($After -eq 'open') { "wait:500,open:0,wait:$($Seconds * 1000),quit" } else { "wait:$($Seconds * 1000),quit" }
$psi.EnvironmentVariables['KOMOREBI_BENCH'] = $wait
if ($psi.EnvironmentVariables['KOMOREBI_OFFLINE'] -ne '1') { throw 'KOMOREBI_OFFLINE must be 1' }
$p = [Diagnostics.Process]::Start($psi)
try {
    Start-Sleep -Milliseconds ($(if ($After -eq 'open') { 2500 } else { 1500 }))
    $p.Refresh()
    $mods = @($p.Modules | ForEach-Object { [pscustomobject]@{ Name = $_.ModuleName; Base = [uint64]$_.BaseAddress.ToInt64(); End = [uint64]($_.BaseAddress.ToInt64() + $_.ModuleMemorySize) } })
    $rows = foreach ($t in $p.Threads) {
        $h = [ThreadNative]::OpenThread(0x0840, $false, [uint32]$t.Id)
        $start = [IntPtr]::Zero; $desc = ''
        if ($h -ne [IntPtr]::Zero) {
            [void][ThreadNative]::NtQueryInformationThread($h, 9, [ref]$start, 8, [IntPtr]::Zero)
            $np = [IntPtr]::Zero
            if ([ThreadNative]::GetThreadDescription($h, [ref]$np) -ge 0 -and $np -ne [IntPtr]::Zero) { $desc = [Runtime.InteropServices.Marshal]::PtrToStringUni($np); [void][ThreadNative]::LocalFree($np) }
            [void][ThreadNative]::CloseHandle($h)
        }
        $a = [uint64]$start.ToInt64()
        $m = $mods | Where-Object { $a -ge $_.Base -and $a -lt $_.End } | Select-Object -First 1
        [pscustomobject]@{ tid = $t.Id; module = if ($m) { $m.Name } else { '?' }; name = $desc; state = "$($t.ThreadState)/$($t.WaitReason)"; cpu_ms = [math]::Round($t.TotalProcessorTime.TotalMilliseconds, 1) }
    }
    "threads: $(@($rows).Count)"
    $rows | Group-Object module, name | Sort-Object Count -Descending | ForEach-Object { '{0,4}  {1}' -f $_.Count, $_.Name }
    ''
    $rows | Sort-Object module, name | Format-Table -AutoSize | Out-String -Width 200
} finally {
    if (-not $p.HasExited) { [void]$p.WaitForExit(($Seconds + 5) * 1000); if (-not $p.HasExited) { $p.Kill() } }
}
