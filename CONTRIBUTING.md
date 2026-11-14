# Contributing to Komorebi

Thanks for helping. Komorebi is licensed GPL-3.0-or-later; by contributing you agree your changes are released under the same licence.

## Scope

- Komorebi is a fast, free and open-source Windows 10/11 email client: Rust + GPUI with GPUI Kit (Komorebi 1.0), a local SQLite cache, and IMAP/SMTP for Purelymail, Postale.io and generic servers. No AI, no telemetry.
- Scope is deliberately minimal; read `docs/SCOPE.md` (goals and non-goals) before proposing a feature.
- Never commit real mailbox data, local app-data, credentials or screenshots of real mail. Do not test against someone else's mailbox; automated tests use the local fakes only.

## Project Rules

- Check `git status --short --branch` before you start.
- Use Cargo (`Cargo.lock` is the active lockfile) and the pinned Rust toolchain in `rust-toolchain.toml`.
- In a pull request, list the checks you ran, known failures, and any migration or user-data implications.
- Generated/bulky outputs are not source: do not hand-edit or commit `target/`, runtime SQLite databases, or provider/mailbox data.
- Any UI or UX work (`crates/komorebi-gpui`: layout, colours, icons, wording, states) must follow `docs/FLUENT2.md`, the Fluent 2 design reference distilled from https://fluent2.microsoft.design/ and the official Fluent token source. Read it before designing or reviewing UI; when habit disagrees with it, the reference wins (Windows-specific rules first).
- Komorebi ships in English and European Portuguese (pt-PT; never Brazilian). The language follows the Windows display language (`src/i18n.rs`; pt-BR falls back to English) with a Settings override that switches live. Every user-facing string is translatable: the app writes them with `tr!("ctx" => "...", args)` and `trn!("one {n}", "many {n}", n)` from `crates/komorebi-i18n`, over the gettext catalog (whole sentences with `{}` placeholders, a context for ambiguous words, never glued fragments; [docs/GPUI-I18N.md](docs/GPUI-I18N.md)); no English sentence is built in the library layer (errors are typed and worded in the app); dates, numbers and sizes take a `Lang` in `src/format.rs`; machine text (crash.log, diagnostics log, protocol strings) stays English. After changing a string, run `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`, then translate the new entries in `translations/pt-PT/LC_MESSAGES/komorebi.po` with the glossary (ficheiro, ecrã, definições, palavra-passe, utilizador; AO90 spelling). `tests/i18n_catalog.rs` fails on untranslated, fuzzy, stale or placeholder-mismatched entries, Brazilian terms, and visible literals outside `tr!` (`translations/untranslated-allowlist.txt`). Check UI changes in both languages at 1280x800 and 900x700 (`KOMOREBI_LANG=pt-PT`; Portuguese runs 15 to 30 percent longer, so wrap or flex rather than clip).
- UI work also follows `docs/WINUI3.md`, the WinUI 3 / Windows 11 reference (theme resources, control metrics, and following the user's accent, light/dark mode, high contrast, text size, animation effects and title bar); on Windows it wins over `docs/FLUENT2.md` where they disagree.
- Performance work (speed, memory, exe size, build profile) follows `docs/PERF.md`: cite the rule, measure before and after on a release build (median and min of at least 7 runs), and revert what does not beat the run-to-run spread.

## Checks

Run the relevant checks before sending a change:

- `cargo check --all-targets`
- `cargo test`
- `cargo build`
- `cargo clippy --all-targets` (when touching non-trivial Rust)
- `cargo fmt --check` (when touching formatting)
- Development always uses the unoptimized `dev` profile (`cargo build`, `cargo check`, `cargo test`, snapshots; output in `target/debug`): never `--release` while iterating. Dependencies are optimized once in `[profile.dev.package."*"]` (GPUI and the HTML renderer are slow otherwise) and then cached, so rebuilds only recompile Komorebi. The `release` profile (fat LTO, about ten minutes) is for the final size check, timing/memory numbers and CI only: build it once per task, at the end, never in a loop.

Before pushing, run `pwsh -File scripts/ci-local.ps1` (or `cargo xtask ci`; `-Fast` skips the release build and the steps after it that need the exe, `-From <step>` resumes): it runs the CI steps natively on this PC and stops at the first failure. GitHub Actions does not run on every push: CI (`.github/workflows/ci.yml`) runs on demand and for pull requests from forks, on GitHub's `windows-2025` runner, and runs `scripts/find-fxc.ps1`, `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo check -p komorebi-gpui --all-targets --locked`, `cargo xtask lint` (the app's token and no-timers lints), `cargo test --locked`, `cargo test -p komorebi-gpui -p komorebi-i18n --locked`, `cargo test -p xtask --locked`, `scripts/check-pins.ps1`, a release build with a size check (`cargo xtask size`: komorebi.exe, the Komorebi 1.0 release exe, at most 21,500,000 bytes) and a manifest check (`scripts/check-manifest.ps1`), the app's boot check on WARP, empty and seeded (`scripts/gpui-boot-check.ps1`) and its UI Automation dump (`scripts/gpui-uia-check.ps1`, informational for now), and `cargo deny --workspace --locked check licenses advisories`.

Work passes one gate before each commit: `cargo xtask check` (fmt, clippy, the app's, the library's and xtask's tests, the lints, the exe's size; `--release` builds the exe first, `--all` adds the library's integration tests). How its tests are written: `docs/GPUI-TESTING.md`.

For layout changes, verify the window at desktop and narrow widths (the compact rail starts below 1040 px). `KOMOREBI_SNAPSHOT` renders the window to a file against a throwaway `KOMOREBI_DB` with `KOMOREBI_OFFLINE=1`, without capturing the screen (see docs/ENVIRONMENT.md).

## Priorities

- Privacy and user data safety outrank feature speed.
- Correct mail semantics outrank UI polish.
- Launch < 200 ms and cached-list load < 600 ms are product promises; measure (release build, `KOMOREBI_TIMING=1`) before claiming numbers.
- Minimal, quiet, usable UI beats decorative surfaces.

## Stack

- UI of Komorebi 1.0: GPUI with GPUI Kit, in `crates/komorebi-gpui` (docs/GPUI-MIGRATION.md, docs/GPUI-BUILD.md, docs/GPUI-TESTING.md). It is the release exe (`komorebi.exe`, at most 21,500,000 bytes). There is no calendar UI in 1.0 (docs/SCOPE.md); the calendar library code (`src/cal`, `src/dav/caldav`, the calendar tables and their tests) stays, compiling and tested, for 1.1.0: never delete it.
- One library package (the root, `src/lib.rs`), the app `crates/komorebi-gpui` (the `komorebi` exe) and the catalog `crates/komorebi-i18n`: the library (`src/lib.rs`: `src/db.rs` SQLite cache, `src/mail/` IMAP sync, SMTP, MIME, HTML→text, the HTML layout engine in `src/mail/render/` (panic-free lints, caps and a layout deadline are mandatory there), `src/mail/rich/` the composer's own document model for formatted mail and signatures (panic-free lints and caps as in render; it generates the HTML and plain parts and reads back only its own tagged HTML strictly; no web engine, no remote content), `src/providers.rs` presets, `src/role.rs`, `src/rules.rs` local rules: the model, caps, evaluator and Sieve export (panic-free lints, as in render), with `src/db/rule_store.rs` their tables and `src/mail/rules.rs` the sync worker's pass on new Inbox mail (before the headers are stored; one STORE per flag and one MOVE per destination on the sync connection), `src/secrets.rs` Windows Credential Manager via `keyring-core` and its Windows store, `src/mail/ical.rs` meeting invitations: iCalendar reading (panic-free lints and caps, as in render) and the RSVP reply, `src/mail/imap.rs` the only file that names the `imap` crate (every command through `Imap::run`; its stream refuses replies the crate would abort on), `src/mail/errors.rs` error classes and the one-sentence messages, `src/mail/idle.rs` the IDLE thread, `src/diag.rs` the opt-in diagnostics log (metadata only), `src/theme.rs` the Windows appearance settings (accent ramp, light/dark, high contrast, text size, animation effects, accent title bars) read with plain Win32 calls, pure parsing and maths); the app is `crates/komorebi-gpui` (docs/GPUI-MIGRATION.md). The shared pure logic the views use lives in the library: `src/rows.rs`, `src/format.rs`, `src/tabs.rs`, `src/compose/`, `src/reader/`, `src/rules_page.rs`, `src/contacts_page.rs`, `src/invite.rs`, `src/onboard.rs`, `src/crash.rs` (panic hook: one metadata line to `crash.log` beside the cache, never the panic message). Dev tools: `examples/probe.rs` (read-only measurement against an account you own; `--send` sends one test message to the account itself), `examples/parse_cache.rs`, `examples/render_html.rs` (renders a corpus `.eml` to PNG), `examples/seed.rs` (optionally adds the synthetic `.eml` corpus), `examples/delivery_delay.rs` and `examples/modseq_check.rs` (read-only), `examples/idle_check.rs` (toggles `\Flagged` on its own `[komorebi-test]` message; `--mail` sends one to the account itself). Tools that send or write must only be pointed at an account you own, and are never run in CI.
- Do not store secrets in SQLite, logs, UI state, or test fixtures.

## Architecture Boundaries

- `crates/komorebi-gpui` owns presentation and UI state on the UI thread; the library has no UI toolkit in it.
- Network work runs only on the per-account worker threads in `src/mail/mod.rs` (sync, interactive, a short-lived thread per send, and a read-only IDLE thread that only notifies); the UI thread never blocks on IMAP/SMTP. Two user-initiated exceptions, each a short-lived thread started by one press and never in the background: "Review certificate" (`mail::pin`, reads a server's certificate, sends no credentials) and "Find settings", started by one press of Find settings or of the onboarding email step's Continue (never on typing or focus change; a preset domain skips it) (`mail::discover`: DNS SRV through the system resolver, then HTTPS GETs to `autoconfig.<domain>` and `<domain>` through `fetch::get_with`; it only prefills the form, never sends the password, and refuses under `KOMOREBI_OFFLINE`). The update check (`src/update.rs`) is the one other network use: a short-lived thread, started by Settings > About > Check for updates or at most once a day a few seconds after start when automatic checks are on, makes one HTTPS GET to `api.github.com` for the latest release (fixed `Komorebi/<version>` User-Agent, no identifiers, refused under `KOMOREBI_OFFLINE`); the installer, its `.sha256` and `.sig` are downloaded from `github.com` only after the user presses Download and install, and run only if the hash matches and the Ed25519 signature over the version and hash verifies against the compiled-in `PUBLIC_KEY`. The private key lives only at `%USERPROFILE%\.komorebi-release\update-signing.key`, never in the repository.
- Notifications are local: `src/notify.rs` decides (pure logic) and `src/win_shell.rs` shows them through the tray icon's native Windows notification; they come only from the existing sync/IDLE events, never a push service, never logged, no telemetry. Background mode (close to tray, start with Windows minimised through the HKCU Run key) is opt-in.
- Native file dialogs run a nested message loop: never hold app state across one.
- Tests use in-memory or temp-file databases and the local fake IMAP/SMTP servers in `src/mail/fake.rs`. Never use live mailboxes as automated tests.

## Known gaps

- Provider presets live in `src/providers.rs` as data: domains, MX host
  suffixes, servers, how each signs in, and the official page each fact
  comes from in a comment. A new preset needs that page, an icon in
  `assets/providers` (32 px and 64 px, see its README) and a test in the
  same file; MX suffixes only select a preset (`mail::discover`), so keep
  them to hosts the provider documents.
- Not yet validated against real Purelymail/Postale.io accounts; the onboarding flow (`crates/komorebi-gpui/src/views/onboarding`) is checked only offline, on the local fakes and in snapshots.
- Not yet validated against a real self-hosted server (Dovecot, Stalwart, Cyrus, Proton Bridge, ...): manual accounts, certificate pinning, "Find settings" and the capability fallbacks are tested only on the local fakes. A native-tls submission client is deferred (`TODO.md`).
- The local mail cache is plaintext unless encryption is turned on in Settings > Privacy; attachment copies stay plaintext.
- Release 0.4.0 is published. The installer is not Authenticode-signed (a certificate is pending, the signing step is optional); self-updates are verified with an Ed25519 signature.
- See `TODO.md`.

## Releases

- Changes reach `main` only through a pull request with passing checks.
- Releases are made by the maintainer by hand: release build, `scripts/build-installer.ps1`, `cargo run --example sign_update -- sign <installer> <version>`, then `gh release create` with the installer, its `.sha256` and its `.sig` (see docs/RELEASING.md). `.github/workflows/release.yml` builds the unsigned packages in CI on a version tag or by hand, for code signing and a draft release; the update signature stays a local step and the signing key lives outside the repository.
- If checks pass but product gaps remain, say that clearly. Buildable is not the same as production-complete.
