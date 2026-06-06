$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$generated = Join-Path $repo "bindings-generated"
$target = Join-Path $repo "frontend\bindings"

function Assert-UnderRepo {
    param([string]$Path)
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if (-not $fullPath.StartsWith($repo, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to operate outside repository: $fullPath"
    }
}

Assert-UnderRepo $generated
Assert-UnderRepo $target

if (Test-Path -LiteralPath $generated) {
    Remove-Item -LiteralPath $generated -Recurse -Force
}

wails3 generate bindings -ts -d $generated
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if (Test-Path -LiteralPath $target) {
    Remove-Item -LiteralPath $target -Recurse -Force
}

Move-Item -LiteralPath $generated -Destination $target
