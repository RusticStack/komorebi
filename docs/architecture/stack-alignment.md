# Stack Alignment

Last reviewed: 2026-05-25.

This document records the implemented stack. It resolves drift from the original product plan without adding unused dependencies.

## Implemented Stack

- Desktop shell: Tauri v2 on Windows with Microsoft Edge WebView2, a decorationless main window, and React-rendered custom titlebar chrome.
- Backend: Rust 1.95.0, Tauri async runtime, blocking `native-tls` TCP adapters where live IMAP/SMTP protocol I/O is implemented, `rusqlite` bundled SQLite, `mail-parser`, `lettre`, `reqwest` blocking OAuth HTTP, Windows Credential Manager through `windows-sys`.
- Frontend: React 19, TypeScript, Vite, plain CSS, `@tanstack/react-virtual`, `react-i18next`, TipTap, DOMPurify.
- Tests: Vitest/React Testing Library, Rust unit and integration tests, Playwright browser E2E, Playwright browser performance harness, ignored opt-in live SMTP smoke.

## Plan Items Not Installed

The initial plan mentioned these as intended stack pieces, but they are not current dependencies:

- Tailwind CSS.
- `@tanstack/react-query`.
- Tokio as a direct dependency.
- `async-imap`.

Current decision:

- Keep plain CSS because the implemented shell already has a focused design system, screenshot coverage, and no Tailwind build pipeline.
- Keep local data-source helpers instead of React Query because current Tauri reads are command-backed, cancellable where needed, and covered by targeted tests.
- Keep the current blocking `native-tls` live IMAP adapter until a separate migration proves `async-imap` and any Tokio usage improve correctness or maintainability.
- Do not install unused dependencies just to match the older plan.

## Migration Rules

Any future stack migration must:

- Update `package.json`, `pnpm-lock.yaml`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, README, AGENTS, TODO, and relevant architecture docs.
- Include tests proving no privilege boundary, credential, sync, MIME, render, or i18n behavior regressed.
- Include bundle/performance measurements when frontend dependencies are added.
- Keep generated outputs out of source control.

## Documentation Sync

Keep this document synchronized with:

- `Project Komorebi - Email Client Plan.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
- `docs/architecture/frontend-mail-shell.md`
- `docs/architecture/windows-native-ui.md`
- `docs/architecture/sync-state-machine.md`
