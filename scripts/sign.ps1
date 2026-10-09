# Signs files with signtool (Windows SDK), for the optional signing steps
# in .github/workflows/release.yml. The certificate comes from the
# environment, never from the repository:
#
#   SIGN_CERT_PFX       the .pfx file, base64
#   SIGN_CERT_PASSWORD  its password
#   SIGN_TIMESTAMP_URL  an RFC 3161 timestamp server
#
#   pwsh scripts/sign.ps1 target/release/komorebi.exe [more files]
#
# The provider and certificate are the maintainer's choice (TODO.md); a
# cloud signing service would replace this script with its own step.
param([Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Files)
$ErrorActionPreference = 'Stop'

foreach ($name in 'SIGN_CERT_PFX', 'SIGN_CERT_PASSWORD', 'SIGN_TIMESTAMP_URL') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "$name is not set" }
}

# The newest x64 signtool of the installed Windows SDKs.
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object { [version]$_.Directory.Parent.Name } -ErrorAction SilentlyContinue |
    Select-Object -Last 1
if (-not $signtool) { throw 'signtool.exe (Windows SDK) not found' }

$pfx = Join-Path ([IO.Path]::GetTempPath()) "komorebi-sign-$([guid]::NewGuid()).pfx"
try {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:SIGN_CERT_PFX))
    foreach ($file in $Files) {
        & $signtool.FullName sign /fd SHA256 /td SHA256 /tr $env:SIGN_TIMESTAMP_URL `
            /f $pfx /p $env:SIGN_CERT_PASSWORD /d Komorebi $file
        if ($LASTEXITCODE -ne 0) { throw "signing $file failed" }
        & $signtool.FullName verify /pa $file
        if ($LASTEXITCODE -ne 0) { throw "$file does not verify" }
    }
} finally {
    Remove-Item -Force -ErrorAction SilentlyContinue $pfx
}
