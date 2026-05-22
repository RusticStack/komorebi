# Komorebi

Komorebi is a privacy-first Windows 10/11 desktop email client built with Tauri v2, React, TypeScript, Vite, and Rust.

The current implementation is an executable first-run shell with no seeded mailbox data. It does not connect to real mail accounts yet.

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

Provider authentication policy is documented in [docs/architecture/provider-auth.md](docs/architecture/provider-auth.md). Gmail and Microsoft backend auth uses OAuth2/XOAUTH2 only; generic password/app-password setup is limited to compatible generic IMAP/SMTP providers. The user-facing account setup UI remains future TODO work.

Token refresh, reconnect-needed state, provider revocation, and account-removal cleanup are documented in [docs/architecture/token-lifecycle.md](docs/architecture/token-lifecycle.md). React receives redacted auth status only; token refresh and revocation handling stay in Rust.

Tauri capability policy is documented in [docs/architecture/tauri-capabilities.md](docs/architecture/tauri-capabilities.md). The main webview currently has no frontend Tauri permissions; SQL, filesystem, opener, shell/process, HTTP, and custom-command access are denied unless a future feature adds a scoped capability and test coverage.

IPC and mutation boundaries are documented in [docs/architecture/ipc-and-mutation-boundaries.md](docs/architecture/ipc-and-mutation-boundaries.md). Server-affecting actions, credential access, attachment writes, destructive deletes, migrations, and sync cache mutation stay in typed Rust commands or internal Rust workers. Frontend SQL is disabled until a read-only policy is implemented and tested.

SQLite schema and migrations are documented in [docs/architecture/schema.md](docs/architecture/schema.md). The initial migration defines settings, accounts, folders, messages, attachments, templates, template versions, primary/foreign keys, uniqueness rules, indexes, timestamps, and schema-version tracking.

IPC input validation is documented in [docs/architecture/ipc-validation.md](docs/architecture/ipc-validation.md). Future commands must reject arbitrary paths, raw SQL, unchecked URLs, unsanitized HTML, unbounded payloads, and unknown or unowned account/folder/message IDs before side effects.

Safe email rendering is documented in [docs/architecture/message-rendering.md](docs/architecture/message-rendering.md). Untrusted email HTML is sanitized through a single React boundary, remote resources are blocked by default, links are rewritten for delegated opening, and plain-text fallback is preserved.

Attachment safety is documented in [docs/architecture/attachment-safety.md](docs/architecture/attachment-safety.md). Attachment storage paths are app-generated in Rust, sender filenames are display-only, path traversal is rejected, size/type limits are enforced, previews are opt-in and type-checked, and auto-open is forbidden.

Destructive mail action semantics are documented in [docs/architecture/destructive-mail-actions.md](docs/architecture/destructive-mail-actions.md). Move, archive, delete-to-trash, mark-read, and expunge behavior is typed in Rust, confirmation-aware, reversible where possible, and tested against local test mailbox state only.

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

Validate Tauri configuration without producing installers:

```powershell
pnpm tauri build --no-bundle
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
