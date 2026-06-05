param(
  [switch]$AllowUnsigned
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$expectedArtifacts = @(
  "src-tauri\target\release\komorebi.exe",
  "src-tauri\target\release\bundle\nsis\Komorebi_0.1.0_x64-setup.exe",
  "src-tauri\target\release\bundle\msi\Komorebi_0.1.0_x64_en-US.msi",
  "src-tauri\target\release\bundle\msi\Komorebi_0.1.0_x64_pt-PT.msi"
)

$allowUnsignedFromEnv = $env:KOMOREBI_ALLOW_UNSIGNED_RELEASE_ARTIFACTS -eq "1"
$allowUnsignedArtifacts = $AllowUnsigned -or $allowUnsignedFromEnv
$failed = $false

foreach ($relativePath in $expectedArtifacts) {
  $artifactPath = Join-Path $repoRoot $relativePath

  if (-not (Test-Path -LiteralPath $artifactPath)) {
    Write-Error "Missing release artifact: $relativePath"
    $failed = $true
    continue
  }

  $signature = Get-AuthenticodeSignature -LiteralPath $artifactPath
  $status = $signature.Status.ToString()

  if ($status -ne "Valid") {
    if ($allowUnsignedArtifacts) {
      Write-Warning "$relativePath is $status; accepted only because unsigned local artifacts were explicitly allowed."
      continue
    }

    Write-Error "$relativePath is $status. Public release artifacts must be signed and valid."
    $failed = $true
    continue
  }

  Write-Host "$relativePath signature is valid."
}

if ($failed) {
  exit 1
}

