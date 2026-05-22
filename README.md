# Komorebi

Komorebi is a privacy-first Windows 10/11 desktop email client scaffolded with Tauri v2, React, TypeScript, Vite, and Rust.

The current implementation is an executable placeholder shell: a static 3-pane mail layout that verifies the desktop scaffold. It does not connect to real mail accounts yet.

## Toolchain

Use the pinned versions below for local development:

- Node.js `24.16.0`, pinned in `.node-version` and `package.json`.
- pnpm `10.33.2`, pinned in `package.json` via `packageManager`.
- Rust `1.95.0` for `x86_64-pc-windows-msvc`, pinned in `rust-toolchain.toml`.
- Cargo lockfile: `src-tauri/Cargo.lock`.
- pnpm lockfile: `pnpm-lock.yaml`.

If `pnpm` is not available but Node is installed, install the pinned package manager:

```powershell
npm install -g pnpm@10.33.2
```

## Windows Prerequisites

Tauri development on Windows requires Microsoft C++ Build Tools and Microsoft Edge WebView2.

Install the Visual Studio Build Tools with the `Desktop development with C++` workload. Keep the Rust host toolchain on the MSVC target, `x86_64-pc-windows-msvc`, because this project pins and verifies that target.

WebView2 is the rendering engine used by Tauri on Windows. It is normally present on Windows 10 version 1803 and later, and on Windows 11. If a supported machine is missing WebView2, install the Microsoft Edge WebView2 Evergreen Runtime before running the app. Release packaging still needs a dedicated verification pass before public distribution.

Reference: https://v2.tauri.app/start/prerequisites/

## Security Model

The accepted credential architecture is documented in [docs/architecture/credential-model.md](docs/architecture/credential-model.md). Komorebi uses Windows Credential Manager/DPAPI for account secrets. Plaintext passwords and tokens are forbidden in SQLite, Tauri Store, browser storage, logs, diagnostics, and frontend state.

Provider authentication policy is documented in [docs/architecture/provider-auth.md](docs/architecture/provider-auth.md). Gmail and Microsoft account setup are disabled until OAuth2/XOAUTH2 is implemented; generic password/app-password setup is limited to compatible generic IMAP/SMTP providers.

## Setup

```powershell
pnpm install
```

## Development

Run the Vite frontend:

```powershell
pnpm dev
```

Run the Tauri desktop shell:

```powershell
pnpm tauri dev
```

Build the frontend:

```powershell
pnpm build
```

Run the full root check suite:

```powershell
pnpm check
```

Run individual root checks:

```powershell
pnpm fmt:check
pnpm lint
pnpm test
pnpm build
```

The root scripts fan out to frontend and Rust checks:

```powershell
pnpm lint:frontend
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
pnpm test:frontend
cargo test --manifest-path src-tauri/Cargo.toml --all-features
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
```

## Generated Outputs

Do not commit generated dependency, build, or runtime outputs such as `node_modules/`, `dist/`, `src-tauri/target/`, `src-tauri/gen/`, local app-data directories, disposable SQLite databases, or diagnostics logs.
