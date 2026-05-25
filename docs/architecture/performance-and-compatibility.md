# Performance And Compatibility

Last reviewed: 2026-05-25.

## Current Posture

Komorebi must not claim production memory, latency, or Windows compatibility numbers unless they come from the commands in this document or a later reviewed harness.

P7 release automation was intentionally deferred. This P10 posture therefore separates:

- Browser harness measurements for large-list frontend behavior.
- Backend SQLite/FTS/sync/attachment measurements against disposable local data.
- Release executable idle-memory sampling.
- Windows host compatibility reporting.

Full installer validation, Tauri-shell E2E fixture driving, code signing, and CI remain P7 deferred work.

## Commands

Build first when measuring bundle or release executable behavior:

```powershell
pnpm build
pnpm tauri build --no-bundle
```

Run all current P10 measurements:

```powershell
pnpm perf:all
pnpm perf:release-memory
```

Individual commands:

```powershell
pnpm perf:bundle
pnpm perf:backend
pnpm perf:mail-shell
pnpm perf:compat
pnpm perf:release-memory
```

Generated reports are disposable and ignored:

- `test-results/perf/bundle-size.json`
- `test-results/perf/sqlite-fts-benchmark.json`
- `test-results/perf/sync-attachment-benchmark.json`
- `test-results/perf/release-memory.json`
- `test-results/compat/windows-compatibility.json`
- Playwright performance attachments under `test-results/perf/`

## Measurement Coverage

`pnpm perf:mail-shell` covers the browser `/e2e/` performance fixture:

- 10,000 message rows.
- Virtualized list row count after scrolling.
- Initial load latency.
- Plain and advanced search latency.
- Last-row scroll and selection latency.
- Chromium heap samples after load, search, and list interaction.

`pnpm perf:backend` covers disposable SQLite/backend fixtures:

- 10,000 persisted messages.
- FTS integrity and query latency.
- Initial and incremental cache insert timing.
- Flag-update timing.
- Representative attachment file write timing.

`pnpm perf:bundle` covers Vite output:

- Asset byte sizes.
- Gzip byte sizes.
- Largest JavaScript and total asset summaries.

`pnpm perf:release-memory` covers:

- Release `komorebi.exe` accountless idle process working set.
- Release private memory sample.

It does not cover large-mailbox, sync, composer, or attachment-heavy Tauri-shell memory. Those require a future Tauri-shell fixture harness from P7.

`pnpm perf:compat` covers the local Windows host:

- OS platform/release/version.
- Node, pnpm, Rust, Cargo, and MSVC tool visibility.
- Tauri product name, identifier, version, and window config.
- WebView2 Evergreen Runtime registry detection.
- Current support posture.

## Current Claim Rules

Allowed:

- "Browser performance fixture renders 10,000 messages with bounded mounted rows when `pnpm perf:mail-shell` passes."
- "SQLite/FTS benchmark reports are available under `test-results/perf/` after `pnpm perf:backend`."
- "The current release executable idle-memory sample is available after `pnpm perf:release-memory`."
- "Windows compatibility has been checked on the local host after `pnpm perf:compat`."

Forbidden until measured in a release Tauri-shell harness:

- "Sub-50MB memory footprint" as a production claim.
- "Instantaneous search" without benchmark context and data size.
- "Windows 10/11 fully validated" without installer/runtime matrix evidence.
- "Installer-ready" or "release-ready" without P7 packaging work.

## Windows Compatibility Notes

Official platform references:

- Tauri Windows prerequisites: https://v2.tauri.app/start/prerequisites/
- Microsoft WebView2 overview: https://developer.microsoft.com/microsoft-edge/webview2/
- Microsoft WebView2 distribution: https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution

Current target:

- Windows 10 and Windows 11.
- MSVC Rust target `x86_64-pc-windows-msvc`.
- Microsoft C++ Build Tools for local development.
- Microsoft Edge WebView2 Evergreen Runtime for the desktop shell.
- Windows Credential Manager/DPAPI for secrets.

## International Compatibility Notes

Existing coverage:

- English and `pt-PT` UI resources.
- Locale-aware dates, counts, sizes, and status copy.
- Long English/`pt-PT` desktop/narrow screenshots.
- Mixed-script desktop/narrow screenshots through `e2e/international-content.spec.ts`.
- MIME fixtures for Portuguese, Japanese, encoded headers, non-ASCII names, malformed input, phishing, and XSS payloads.

P10 international fixture coverage includes non-ASCII filenames, mixed-script sender/subject/body text, and RTL-like stress strings in the browser screenshot harness. Broader font fallback and true RTL locale support remain future work because the implemented UI languages are English and `pt-PT`.

## Documentation Sync

Keep this document synchronized with:

- `README.md`
- `TODO.md`
- `AGENTS.md`
- `package.json`
- `perf/mail-shell.benchmark.spec.ts`
- `src-tauri/tests/performance_benchmarks.rs`
- `e2e/international-content.spec.ts`
