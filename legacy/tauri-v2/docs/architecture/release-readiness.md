# Release Readiness

Status: Windows release automation, unsigned bundle generation, and release policy are now defined, but public release remains gated on signing plus clean-machine installer/WebView2 verification.
Last reviewed: 2026-05-25.

## CI Gates

Windows CI is the release baseline. It must run:

- `pnpm install --frozen-lockfile`
- `pnpm check:artifacts`
- `pnpm security:frontend`
- `pnpm licenses:frontend`
- `cargo install cargo-audit --version 0.22.1 --locked`
- `pnpm security:rust`
- `pnpm check`
- `pnpm test:e2e`
- `pnpm tauri build --no-bundle`
- `pnpm test:tauri-webdriver`

CI does not publish binaries. Release artifacts require a separate, explicitly reviewed release workflow after signing is configured.

## Dependency And License Review

Production JavaScript advisories are checked with `pnpm audit --prod --audit-level moderate`, which uses the npm advisory database for production dependencies. Production JavaScript licenses are listed with `pnpm licenses list --prod`; current reviewed licenses are MIT, Apache-2.0, Apache-2.0 OR MIT, ISC, Unlicense, and DOMPurify's `(MPL-2.0 OR Apache-2.0)`.

Rust advisories are checked with RustSec `cargo audit` against `src-tauri/Cargo.lock` for the Windows x64 target. Current `cargo audit` output is clean for known vulnerabilities but reports 17 warning-class advisories in transitive Tauri lockfile dependencies: `RUSTSEC-2024-0411`, `RUSTSEC-2024-0412`, `RUSTSEC-2024-0413`, `RUSTSEC-2024-0414`, `RUSTSEC-2024-0415`, `RUSTSEC-2024-0416`, `RUSTSEC-2024-0417`, `RUSTSEC-2024-0418`, `RUSTSEC-2024-0419`, `RUSTSEC-2024-0420`, `RUSTSEC-2024-0429`, `RUSTSEC-2024-0370`, `RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`, `RUSTSEC-2025-0081`, `RUSTSEC-2025-0098`, and `RUSTSEC-2025-0100`. These are currently accepted as warning-class transitive Tauri/GTK/urlpattern advisories only; re-review them on every Tauri/runtime upgrade and before any non-Windows support claim. Any future vulnerability or direct dependency exception needs a documented advisory id, affected package, impact, mitigation, and removal date in this document before release.

## Generated Artifact Guard

`pnpm check:artifacts` fails if generated dependency/build/runtime output is tracked or staged. Blocked surfaces include `node_modules`, `dist`, Rust/Tauri target directories, generated Tauri permissions/schema output, test reports, diagnostics bundles, runtime SQLite cache files, and downloaded WebDriver binaries.

## Tauri WebDriver Smoke

`pnpm test:tauri-webdriver` launches the release executable through `tauri-driver` and Microsoft Edge WebDriver. It verifies the accountless desktop shell DOM, custom titlebar/sidebar brand, settings entry point, onboarding surface, and Tauri invoke bridge. Failure artifacts are written under ignored `test-results/tauri-webdriver/`.

Windows prerequisites:

- `cargo install tauri-driver --version 2.0.6 --locked`
- A matching `msedgedriver.exe` on `PATH`, or `KOMOREBI_NATIVE_WEBDRIVER` pointing to it.

CI installs pinned `tauri-driver`, downloads a matching Microsoft Edge Driver with `msedgedriver-tool` pinned to commit `8c4b34f5`, builds the release executable with `pnpm tauri build --no-bundle`, then runs the smoke.

## App Identity And Installer Metadata

Current app identity:

- Product name: `Komorebi`
- Version source: `package.json` and `src-tauri/tauri.conf.json`, currently `0.1.0`
- Bundle identifier: `com.komorebi.email`
- Publisher: `Komorebi`
- Icons: `src-tauri/icons/*`
- Windows installer targets: NSIS and MSI through Tauri's Windows bundler
- WebView2 runtime: configured as `downloadBootstrapper`

The unsigned release executable is verified by `pnpm tauri build --no-bundle`. Unsigned NSIS/MSI bundle generation is verified locally with:

```powershell
pnpm tauri build --bundles 'nsis,msi' --ci --no-sign
```

The 2026-05-25 run produced:

- `src-tauri/target/release/bundle/nsis/Komorebi_0.1.0_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/Komorebi_0.1.0_x64_en-US.msi`
- `src-tauri/target/release/bundle/msi/Komorebi_0.1.0_x64_pt-PT.msi`

`Get-AuthenticodeSignature` reports these artifacts as `NotSigned`; they are local validation artifacts only and must not be published as production-ready downloads.

`pnpm release:verify-artifacts` checks the expected release executable, NSIS installer, and MSI installers and fails unless all signatures are `Valid`. For local unsigned bundle validation only, `pnpm release:verify-artifacts:local` accepts unsigned artifacts while still proving the expected files exist and reporting their signature status.

## Code Signing Plan

Komorebi must not present unsigned installers as production-ready public downloads. Before public release:

- Acquire an organization or EV code-signing certificate for Windows.
- Store the certificate and password only in the release operator's secure store or CI secrets.
- Configure Tauri Windows signing environment variables or config for the selected signing backend.
- Timestamp signatures with a trusted timestamp authority.
- Verify signatures locally with Windows signature inspection before publishing.
- Run `pnpm release:verify-artifacts` after the signed build.
- Keep unsigned builds clearly labelled as local/test builds only.

## WebView2 And Installer Verification

Tauri uses WebView2 on Windows. Installer verification must cover:

- A clean supported Windows 10/11 machine with current WebView2.
- A machine or VM where WebView2 is missing or outdated enough to exercise bootstrap behavior.
- Install, launch, first-run accountless UI, close, reinstall/repair if supported, uninstall, and app-data retention/deletion expectations.
- Clear user-readable failure behavior if WebView2 installation fails.

## Updater Posture

The updater remains disabled. Do not add it until signing, release hosting, rollback, user consent, privacy behavior, and failure recovery have their own reviewed implementation and tests.
