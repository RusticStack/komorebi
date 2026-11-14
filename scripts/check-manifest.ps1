# Reads the RT_MANIFEST resources of a built exe (without running it) and
# fails unless there is exactly one, id 1, carrying everything Komorebi's
# manifest and GPUI's own manifest declared: asInvoker, Windows 10 and 11,
# Common Controls 6, the UTF-8 code page and per-monitor DPI awareness
# (PerMonitorV2, true/pm). See docs/GPUI-BUILD.md.
#
#   pwsh scripts/check-manifest.ps1 target/release/komorebi.exe [-Show]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [switch]$Show
)
$ErrorActionPreference = 'Stop'
$path = (Resolve-Path $Exe).Path

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class KomorebiManifest {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    delegate bool EnumNames(IntPtr module, IntPtr type, IntPtr name, IntPtr param);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern bool EnumResourceNamesW(IntPtr module, IntPtr type, EnumNames callback, IntPtr param);
    [DllImport("kernel32.dll")] static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll")] static extern IntPtr LoadResource(IntPtr module, IntPtr res);
    [DllImport("kernel32.dll")] static extern IntPtr LockResource(IntPtr data);
    [DllImport("kernel32.dll")] static extern uint SizeofResource(IntPtr module, IntPtr res);
    const uint LOAD_LIBRARY_AS_DATAFILE = 0x2, LOAD_LIBRARY_AS_IMAGE_RESOURCE = 0x20;
    static readonly IntPtr RT_MANIFEST = (IntPtr)24;

    // Each manifest as "id=text"; a name that is a string is reported as such.
    public static List<string> Read(string path) {
        var found = new List<string>();
        IntPtr module = LoadLibraryExW(path, IntPtr.Zero, LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE);
        if (module == IntPtr.Zero) throw new Exception("cannot load " + path + " as data: error " + Marshal.GetLastWin32Error());
        try {
            var names = new List<IntPtr>();
            EnumResourceNamesW(module, RT_MANIFEST, (m, t, n, p) => { names.Add(n); return true; }, IntPtr.Zero);
            foreach (var name in names) {
                long id = name.ToInt64();
                if (id >> 16 != 0) { found.Add("named=" + Marshal.PtrToStringUni(name)); continue; }
                IntPtr res = FindResourceW(module, name, RT_MANIFEST);
                uint size = SizeofResource(module, res);
                IntPtr ptr = LockResource(LoadResource(module, res));
                var bytes = new byte[size];
                Marshal.Copy(ptr, bytes, 0, (int)size);
                found.Add(id + "=" + System.Text.Encoding.UTF8.GetString(bytes));
            }
        } finally { FreeLibrary(module); }
        return found;
    }
}
'@

$manifests = [KomorebiManifest]::Read($path)
"$path`: $($manifests.Count) RT_MANIFEST resource(s)"
if ($manifests.Count -ne 1) { throw "expected exactly one RT_MANIFEST, found $($manifests.Count)" }
$id, $xml = $manifests[0].Split('=', 2)
if ($id -ne '1') { throw "the manifest is resource $id, not 1" }
if ($Show) { $xml }

$doc = [xml]$xml
$ns = New-Object Xml.XmlNamespaceManager $doc.NameTable
$ns.AddNamespace('v1', 'urn:schemas-microsoft-com:asm.v1')
$ns.AddNamespace('v3', 'urn:schemas-microsoft-com:asm.v3')
$ns.AddNamespace('c', 'urn:schemas-microsoft-com:compatibility.v1')
$ns.AddNamespace('ws2005', 'http://schemas.microsoft.com/SMI/2005/WindowsSettings')
$ns.AddNamespace('ws2016', 'http://schemas.microsoft.com/SMI/2016/WindowsSettings')
$ns.AddNamespace('ws2019', 'http://schemas.microsoft.com/SMI/2019/WindowsSettings')
$checks = [ordered]@{
    'asInvoker'                   = "//v3:requestedExecutionLevel[@level='asInvoker']"
    'Windows 10 and 11'           = "//c:supportedOS[@Id='{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}']"
    'Common Controls 6'           = "//v1:dependentAssembly/v1:assemblyIdentity[@name='Microsoft.Windows.Common-Controls' and @version='6.0.0.0']"
    'UTF-8 code page'             = "//ws2019:activeCodePage[text()='UTF-8']"
    'dpiAware true/pm'            = "//ws2005:dpiAware[text()='true/pm']"
    'dpiAwareness PerMonitorV2'   = "//ws2016:dpiAwareness[text()='PerMonitorV2']"
}
$failed = $false
foreach ($name in $checks.Keys) {
    if ($doc.SelectSingleNode($checks[$name], $ns)) { "ok   $name" } else { "FAIL $name"; $failed = $true }
}
if ($failed) { exit 1 }
