param(
    [Parameter(Mandatory = $true)]
    [string]$CertificateThumbprint,

    [string]$TimestampUrl = "http://timestamp.digicert.com",

    [string]$Executable = "bin\Komorebi.exe"
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exePath = [System.IO.Path]::GetFullPath((Join-Path $repo $Executable))
if (-not $exePath.StartsWith($repo, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to sign outside repository: $exePath"
}
if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Executable not found: $exePath"
}

$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (-not $signtool) {
    throw "signtool.exe was not found. Install Windows SDK signing tools."
}

& $signtool.Source sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $exePath
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
