# Fails if any crate pinned to an exact version (`"=x.y.z"`) is linked in two
# versions (`cargo tree -d`), for each app:
#
# - the library (the root package): the pins in Cargo.toml. The HTML
#   renderer's crates are pinned to one copy each.
# - the app (crates/komorebi-gpui, komorebi.exe): its own pins, the GPUI
#   stack's in [workspace.dependencies], and the library's (it links the
#   library). The GPUI stack links some of the renderer's crates again in
#   older versions; those known pairs are listed below with the phase that
#   removes them, and any other second copy fails.
#
# Run from the repository root (CI runs it on every push).
#
#   pwsh scripts/check-pins.ps1
$ErrorActionPreference = 'Stop'

# Pinned for a reason other than sharing one copy, and linked in older
# versions by other crates as usual.
$exempt = @{
    'windows-sys' = 'pinned for its feature list; other crates link older versions'
}

# Second copies the GPUI stack brings into the app (not the library).
$gpuiKnown = @{
    'tiny-skia' = '0.11 through resvg 0.45 (gpui-component) and 0.46 (gpui-pre); size cut 2, phase 18 (docs/GPUI-MIGRATION.md)'
    'png'       = '0.17 through tiny-skia 0.11; size cut 2, phase 18'
    'gif'       = '0.13 through resvg 0.45 (gpui-component); size cut 2, phase 18'
    'roxmltree' = '0.20 through usvg 0.45 (gpui-component); size cut 2, phase 18'
    'zune-jpeg' = '0.4 through resvg 0.45 (gpui-component); size cut 2, phase 18'
    'zune-core' = '0.4 through zune-jpeg 0.4; size cut 2, phase 18'
    'windows'   = 'pinned to the copy gpui-pre-windows links; gpui-base links 0.58'
}

# The exact pins of a manifest: crate names (a `package = "..."` rename
# names the crate), from dependency lines and tables alike.
function Get-Pins([string]$manifest) {
    Select-String -Path $manifest -Pattern '^([A-Za-z0-9_-]+)\s*=\s*(\{[^}]*\}|"=[^"]+")' | ForEach-Object {
        $name = $_.Matches[0].Groups[1].Value
        $value = $_.Matches[0].Groups[2].Value
        if ($value -match '^"=' -or $value -match 'version\s*=\s*"=') {
            if ($value -match 'package\s*=\s*"([^"]+)"') { $Matches[1] } else { $name }
        }
    }
}

# Crate name to the set of versions linked, for one package's graph.
function Get-Versions([string]$package) {
    $tree = cargo tree --locked -p $package --duplicates --depth 0 --prefix none --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "cargo tree -p $package failed" }
    $versions = @{}
    foreach ($line in $tree) {
        if ($line -match '^(\S+) v(\S+)') {
            if (-not $versions.ContainsKey($Matches[1])) { $versions[$Matches[1]] = [System.Collections.Generic.HashSet[string]]::new() }
            [void]$versions[$Matches[1]].Add($Matches[2])
        }
    }
    $versions
}

$failed = $false
function Test-Pins([string]$label, [string]$package, [string[]]$pinned, [hashtable]$known) {
    Write-Output "== $label ($package)"
    $versions = Get-Versions $package
    foreach ($name in ($pinned | Sort-Object -Unique)) {
        $seen = $versions[$name]
        if ($seen -and $seen.Count -gt 1) {
            if ($exempt.ContainsKey($name)) {
                Write-Output "ok   $name ($($seen -join ', '); exempt: $($exempt[$name]))"
            } elseif ($known.ContainsKey($name)) {
                Write-Output "ok   $name ($($seen -join ', '); known: $($known[$name]))"
            } else {
                Write-Output "FAIL $name is linked in $($seen.Count) versions: $($seen -join ', ')"
                $script:failed = $true
            }
        } else {
            Write-Output "ok   $name"
        }
    }
    Write-Output ''
}

$rootPins = @(Get-Pins Cargo.toml)
if (-not $rootPins) { throw 'no exact pins found in Cargo.toml' }
$gpuiPins = @(Get-Pins crates/komorebi-gpui/Cargo.toml)
foreach ($required in 'gpui-pre', 'gpui-kit') {
    if ($rootPins -notcontains $required) { throw "Cargo.toml [workspace.dependencies] no longer pins $required exactly" }
}

# The root manifest's pins include the GPUI stack's workspace pins, which the
# library does not link (no versions seen: ok).
Test-Pins 'Library' 'komorebi' $rootPins @{}
Test-Pins 'App' 'komorebi-gpui' ($rootPins + $gpuiPins) $gpuiKnown

if ($failed) { exit 1 }
