# Fails if any crate pinned to an exact version in Cargo.toml (`"=x.y.z"`)
# is linked in two versions (`cargo tree -d`): the HTML renderer's crates
# are pinned to the copies Slint already links, so a Slint bump must move
# them together. Run from the repository root (CI runs it on every push).
#
#   pwsh scripts/check-pins.ps1
$ErrorActionPreference = 'Stop'

# Pinned for a reason other than sharing one copy, and linked in older
# versions by other crates as usual.
$exempt = @{
    'windows-sys' = 'pinned for its feature list; other crates link older versions'
}

$pinned = Select-String -Path Cargo.toml -Pattern '^([A-Za-z0-9_-]+)\s*=\s*(?:\{[^}]*version\s*=\s*)?"=([^"]+)"' |
    ForEach-Object { $_.Matches[0].Groups[1].Value }
if (-not $pinned) { throw 'no exact pins found in Cargo.toml' }

$tree = cargo tree --locked --duplicates --depth 0 --prefix none --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'cargo tree failed' }
$versions = @{}
foreach ($line in $tree) {
    if ($line -match '^(\S+) v(\S+)') {
        if (-not $versions.ContainsKey($Matches[1])) { $versions[$Matches[1]] = [System.Collections.Generic.HashSet[string]]::new() }
        [void]$versions[$Matches[1]].Add($Matches[2])
    }
}

$failed = $false
foreach ($name in $pinned) {
    $seen = $versions[$name]
    if ($seen -and $seen.Count -gt 1) {
        if ($exempt.ContainsKey($name)) {
            Write-Output "ok   $name ($($seen -join ', '); exempt: $($exempt[$name]))"
        } else {
            Write-Output "FAIL $name is linked in $($seen.Count) versions: $($seen -join ', ')"
            $failed = $true
        }
    } else {
        Write-Output "ok   $name"
    }
}

Write-Output ''
Write-Output 'Manual check on a Slint bump: ui/widgets.slint reads the TextInput properties'
Write-Output 'anchor-position-byte-offset and cursor-position-byte-offset, which Slint marks'
Write-Output 'internal and undocumented (the formatting toolbar needs the selection). Check'
Write-Output 'that the new version still has them, and test Ctrl+B / Ctrl+I / Ctrl+K.'

if ($failed) { exit 1 }
