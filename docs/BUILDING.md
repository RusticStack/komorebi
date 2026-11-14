# Building Komorebi

## Install from source

To build Komorebi yourself on Windows 10/11 (x64):

1. Install [Rust](https://rustup.rs) (rustup picks the toolchain pinned in
   `rust-toolchain.toml`) and the Visual Studio Build Tools with the
   "Desktop development with C++" workload (the MSVC linker, the Windows
   SDK, its `rc.exe`, which embeds the icon, version information and
   manifest, and its `fxc.exe`, which compiles GPUI's shaders in release
   builds; [GPUI-BUILD.md](GPUI-BUILD.md)).
2. Build and run:

   ```powershell
   git clone https://github.com/RusticStack/komorebi
   cd komorebi
   cargo build --release --locked -p komorebi-gpui   # target\release\komorebi.exe
   .\target\release\komorebi.exe
   ```

`komorebi.exe` is a single file (the C runtime is linked in); copy it
anywhere. Its data lives in `%LOCALAPPDATA%\komorebi` (see PRIVACY.md).

Stack: Rust + [GPUI](https://gpui.rs) with GPUI Kit (vendored, `third_party/`),
`rusqlite`, `imap`, `lettre`, `mail-parser`, `keyring-core` (with the Windows Credential Manager store).
The package `komorebi` (the repository root) is the library; the package
`komorebi-gpui` (`crates/komorebi-gpui`) is the app and builds `komorebi.exe`.


## Checks

```powershell
cargo xtask check                  # fmt, clippy, tests, lints, size of an existing release exe
cargo test --locked --workspace    # every test, the library's integration tests included
pwsh scripts/check-pins.ps1        # exact-pinned crates linked once
cargo deny --workspace --locked check licenses advisories   # cargo install cargo-deny
```

CI (`.github/workflows/ci.yml`) runs these, then the release build, its
size check and the boot checks below.

## Verifying a build

After `cargo build --release --locked`:

- **Size:** `cargo xtask size` checks `komorebi.exe` (Komorebi 1.0, the
  release exe, docs/GPUI-BUILD.md) against 21,500,000 bytes.
- **Manifest:** `pwsh scripts/check-manifest.ps1 target\release\komorebi.exe`
  (one RT_MANIFEST with PerMonitorV2, Common Controls 6 and the rest).
- **Version:** `(Get-Item target\release\komorebi.exe).VersionInfo`
  shows the Cargo version, and Explorer shows the icon.
- **Boot:** run it offline against a new seeded cache on WARP; it draws the
  first frame, captures the window to a file, checks that the folder pane,
  the list and the reader are not blank and exits non-zero if any is (this
  touches no mail and no network):

  ```powershell
  cargo run --release --locked --example seed -- $env:TEMP\seed.sqlite3 2000
  pwsh scripts/gpui-boot-check.ps1 -Exe target\release\komorebi.exe `
    -Cache $env:TEMP\seed.sqlite3 -Out $env:TEMP\boot.png -Warp -Select 0   # exit 0
  ```

  `KOMOREBI_TIMING=1` prints the launch timings (the budget is 200 ms to the
  first frame).
