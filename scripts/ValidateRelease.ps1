param(
    [string]$Executable = "bin\Komorebi.exe",
    [switch]$AllowUnsigned
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exePath = [System.IO.Path]::GetFullPath((Join-Path $repo $Executable))
if (-not $exePath.StartsWith($repo, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to validate outside repository: $exePath"
}
if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Executable not found: $exePath"
}

$file = Get-Item -LiteralPath $exePath
if ($file.Length -lt 1MB) {
    throw "Executable is unexpectedly small: $($file.Length) bytes"
}

$hashCommand = Get-Command Get-FileHash -ErrorAction SilentlyContinue
if ($hashCommand) {
    $sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $exePath).Hash
} else {
    $stream = [System.IO.File]::OpenRead($exePath)
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        $sha256 = -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString("x2") })
    } finally {
        $stream.Dispose()
    }
}
$signatureCommand = Get-Command Get-AuthenticodeSignature -ErrorAction SilentlyContinue
if ($signatureCommand) {
    try {
        $signature = Get-AuthenticodeSignature -LiteralPath $exePath -ErrorAction Stop
        $signatureStatus = $signature.Status.ToString()
        $signer = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { "" }
    } catch {
        $signatureStatus = "Unavailable"
        $signer = ""
    }
} else {
    $signatureStatus = "Unavailable"
    $signer = ""
}

$result = [ordered]@{
    executable = $exePath
    sizeBytes = $file.Length
    sha256 = $sha256
    signatureStatus = $signatureStatus
    signer = $signer
}

$result | ConvertTo-Json -Depth 4

if ($signatureStatus -ne "Valid" -and -not $AllowUnsigned) {
    throw "Executable signature is not valid. Re-run with -AllowUnsigned for local development validation only."
}
