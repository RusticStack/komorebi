# Captures the client area of the GPUI app's window to a PNG (PrintWindow
# with PW_RENDERFULLCONTENT, as Skua's skua-shot: it works for DirectX
# windows), then closes the window and waits for the app to exit. Every
# launch is offline on a throwaway cache: KOMOREBI_OFFLINE=1 and KOMOREBI_DB
# are set on the child's environment and asserted before it starts; the
# real cache folder is refused, and the cache must be inside -Scratch.
#
#   pwsh -File scripts/gpui-shot.ps1 -Exe <komorebi.exe> -Scratch <dir> -Out <png> [-Extra K=V;K=V] [-Width 1280 -Height 800] [-SettleMs 1500]
#
# The component gallery: -Extra KOMOREBI_GALLERY=light (or dark) with an exe
# built with `--features gallery`.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Scratch,
    [Parameter(Mandatory = $true)][string]$Out,
    [string[]]$Extra = @(),
    [int]$Width = 1280,
    [int]$Height = 800,
    [int]$SettleMs = 1500,
    [int]$TimeoutMs = 20000
)
$ErrorActionPreference = 'Stop'
$Extra = @($Extra | ForEach-Object { $_ -split ';' } | Where-Object { $_ })
$Scratch = [IO.Path]::GetFullPath($Scratch)

Add-Type -AssemblyName System.Drawing
# GDI only on the C# side (PrintWindow into a DIB, BGRA rows top-down); the
# PNG is written with System.Drawing from PowerShell.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ShotNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct BITMAPINFOHEADER {
        public uint biSize; public int biWidth, biHeight; public ushort biPlanes, biBitCount;
        public uint biCompression, biSizeImage; public int biXPelsPerMeter, biYPelsPerMeter;
        public uint biClrUsed, biClrImportant;
    }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr o);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern int GetDIBits(IntPtr dc, IntPtr bmp, uint start, uint lines, byte[] bits, ref BITMAPINFOHEADER bi, uint usage);
    public static int W, H, Scale;
    public static byte[] Capture(IntPtr h) {
        RECT r; GetClientRect(h, out r);
        int w = r.R - r.L, ht = r.B - r.T;
        if (w <= 0 || ht <= 0) throw new Exception("empty client area");
        IntPtr screen = GetDC(h), mem = CreateCompatibleDC(screen), bmp = CreateCompatibleBitmap(screen, w, ht);
        IntPtr old = SelectObject(mem, bmp);
        bool ok = PrintWindow(h, mem, 3); // PW_CLIENTONLY | PW_RENDERFULLCONTENT
        SelectObject(mem, old);
        var bi = new BITMAPINFOHEADER { biSize = 40, biWidth = w, biHeight = -ht, biPlanes = 1, biBitCount = 32 };
        var px = new byte[w * ht * 4];
        int lines = GetDIBits(mem, bmp, 0, (uint)ht, px, ref bi, 0);
        DeleteObject(bmp); DeleteDC(mem); ReleaseDC(h, screen);
        if (!ok) throw new Exception("PrintWindow failed");
        if (lines != ht) throw new Exception("GetDIBits copied " + lines + " of " + ht + " rows");
        for (int i = 3; i < px.Length; i += 4) px[i] = 255; // GDI leaves alpha undefined
        W = w; H = ht; Scale = (int)(GetDpiForWindow(h) * 100 / 96);
        return px;
    }
}
'@

function Save-Png([byte[]]$px, [int]$w, [int]$h, [string]$path) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $bmp.PixelFormat)
    [Runtime.InteropServices.Marshal]::Copy($px, 0, $data.Scan0, $px.Length)
    $bmp.UnlockBits($data)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

$db = Join-Path $Scratch ("shot-" + [guid]::NewGuid().ToString('N') + '.sqlite3')
$full = [IO.Path]::GetFullPath($db)
$real = Join-Path $env:LOCALAPPDATA 'komorebi'
if ($full.ToLower().StartsWith($real.ToLower())) { throw "refusing the real cache folder $real" }
if (-not $full.ToLower().StartsWith($Scratch.ToLower())) { throw "the cache must be inside -Scratch: $full" }

$psi = New-Object Diagnostics.ProcessStartInfo ([IO.Path]::GetFullPath($Exe))
$psi.UseShellExecute = $false
foreach ($k in @('KOMOREBI_TIMING', 'KOMOREBI_SNAPSHOT', 'KOMOREBI_EXIT_AFTER_FIRST_FRAME', 'KOMOREBI_GPUI_PREFLIGHT_FAIL', 'KOMOREBI_GALLERY')) { [void]$psi.EnvironmentVariables.Remove($k) }
$psi.EnvironmentVariables['KOMOREBI_OFFLINE'] = '1'
$psi.EnvironmentVariables['KOMOREBI_DB'] = $full
foreach ($kv in $Extra) { $p = $kv.Split('=', 2); $psi.EnvironmentVariables[$p[0]] = $p[1] }
if ($psi.EnvironmentVariables['KOMOREBI_OFFLINE'] -ne '1') { throw 'KOMOREBI_OFFLINE must be 1' }
if ($psi.EnvironmentVariables['KOMOREBI_DB'] -ne $full) { throw 'KOMOREBI_DB must be the throwaway cache' }

$proc = [Diagnostics.Process]::Start($psi)
try {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($proc.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if ($proc.HasExited) { throw "the app exited with $($proc.ExitCode) before showing a window" }
        Start-Sleep -Milliseconds 50; $proc.Refresh()
    }
    $h = $proc.MainWindowHandle
    if ($h -eq 0) { throw 'no window' }
    # Client size in physical px: the window is sized so that the client
    # area is -Width x -Height logical px at the window's scale.
    if ($Width -gt 0) { [void][ShotNative]::SetWindowPos($h, [IntPtr]::Zero, 40, 40, $Width, $Height, 0x0014) } # SWP_NOZORDER | SWP_NOACTIVATE
    Start-Sleep -Milliseconds $SettleMs
    $px = [ShotNative]::Capture($h)
    Save-Png $px ([ShotNative]::W) ([ShotNative]::H) ([IO.Path]::GetFullPath($Out))
    "captured $Out ($([ShotNative]::W) x $([ShotNative]::H) px at $([ShotNative]::Scale) percent)"
    [void][ShotNative]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) # WM_CLOSE
    if (-not $proc.WaitForExit($TimeoutMs)) { throw 'the app did not exit after WM_CLOSE' }
    "exit $($proc.ExitCode)"
} finally {
    if (-not $proc.HasExited) { $proc.Kill(); 'killed' }
    Remove-Item -ErrorAction SilentlyContinue $full, "$full-wal", "$full-shm"
}
