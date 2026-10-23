# Building Komorebi

## Install from source

To build Komorebi yourself on Windows 10/11 (x64):

1. Install [Rust](https://rustup.rs) (rustup picks the toolchain pinned in
   `rust-toolchain.toml`) and the Visual Studio Build Tools with the
   "Desktop development with C++" workload (the MSVC linker, the Windows
   SDK and its `rc.exe`, which embeds the icon, version information and
   manifest).
2. Build and run:

   ```powershell
   git clone https://github.com/RusticStack/komorebi
   cd komorebi
   cargo build --release --locked   # target\release\komorebi.exe
   .\target\release\komorebi.exe
   ```

`komorebi.exe` is a single file (the C runtime is linked in); copy it
anywhere. Its data lives in `%LOCALAPPDATA%\komorebi` (see PRIVACY.md).

Stack: Rust + [Slint](https://slint.dev) (Fluent style, software renderer),
`rusqlite`, `imap`, `lettre`, `mail-parser`, `keyring-core` (with the Windows Credential Manager store), `rfd`.


## Checks

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
pwsh scripts/check-pins.ps1        # exact-pinned crates linked once
cargo deny --locked check licenses advisories   # cargo install cargo-deny
```

CI (`.github/workflows/ci.yml`) runs these, then the release build, its
size check and the boot snapshot below.

## Verifying a build

After `cargo build --release --locked`:

- **Size:** `target\release\komorebi.exe` must be at most 20,500,000
  bytes (`(Get-Item target\release\komorebi.exe).Length`).
- **Version:** `(Get-Item target\release\komorebi.exe).VersionInfo`
  shows the Cargo version, and Explorer shows the icon.
- **Boot:** run it offline against a new seeded cache; it renders the
  window to a file, checks that the list and the reader are not blank and
  exits non-zero if either is (this touches no mail and no network):

  ```powershell
  cargo run --release --locked --example seed -- $env:TEMP\seed.sqlite3 2000
  $env:KOMOREBI_OFFLINE = 1; $env:KOMOREBI_DB = "$env:TEMP\seed.sqlite3"
  $env:KOMOREBI_SNAPSHOT = "$env:TEMP\boot.bmp"; $env:KOMOREBI_SNAPSHOT_SELECT = 0
  $env:KOMOREBI_SNAPSHOT_CHECK = 1; $env:KOMOREBI_TIMING = 1
  (Start-Process target\release\komorebi.exe -NoNewWindow -Wait -PassThru).ExitCode   # 0
  ```

  `KOMOREBI_TIMING` prints the launch timings (the budget is 200 ms to the
  first frame).

