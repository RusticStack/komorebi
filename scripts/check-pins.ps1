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
# Slint marks TextInput's anchor-position-byte-offset, cursor-position-byte-offset
# and preedit-text internal; ui/widgets.slint (ComposeBody) and ui/editor.slint
# read them. They must still be declared by the Slint compiler's TextInput
# (builtin_elements.rs), or the editor loses its selection and the IME's
# composition. A bump that renames them fails here with the names.
$meta = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$compiler = $meta.packages | Where-Object { $_.name -eq 'i-slint-compiler' } | Select-Object -First 1
if (-not $compiler) { throw 'i-slint-compiler is not in the dependency tree' }
$builtins = Get-Content -Raw (Join-Path (Split-Path $compiler.manifest_path) 'builtin_elements.rs')
foreach ($prop in 'anchor-position-byte-offset', 'cursor-position-byte-offset', 'preedit-text') {
    if ($builtins -match "property\s*<[^>]+>\s*$([regex]::Escape($prop))\s*;") {
        Write-Output "ok   TextInput $prop"
    } else {
        Write-Output "FAIL Slint $($compiler.version) no longer declares the TextInput property '$prop', which ComposeBody (ui/widgets.slint) and the editor (ui/editor.slint) read. Find its replacement, update both files, and test typing, Ctrl+B / Ctrl+K and an IME in a formatted message."
        $failed = $true
    }
}
if ($failed) { exit 1 }
