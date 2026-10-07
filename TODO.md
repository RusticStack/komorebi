# TODO

Scope and budgets live in `Project Komorebi - Email Client Plan.md`. This file
tracks what is done and what is left.

## Status (2026-10-07)

The rewrite replaced the GPUI/AI prototype (~17.8k lines) with a small
Slint app (about 4.7k lines of Rust including tests, plus 1.2k lines of Slint).

Measured on the development machine, release build, software renderer,
offline against throwaway caches:

| | 6,220 cached messages | 55,720 cached (50k in one inbox) |
|---|---|---|
| Window created | ~6 ms | ~6 ms |
| All cached messages in the list | ~20 ms | ~30 ms (~77 ms first run) |
| Full-window frame, 1280×820 | ~12 ms | ~11 ms |
| Memory (private / working set) | 8.6 / 27 MB | 10.7 / 29 MB |

The list is a lazy model over message ids, so its cost barely grows with
mailbox size. Before the optimization pass, 55k messages took ~280 ms and
private memory was ~14 MB with 6k messages. A full frame at 2560×1600 was
~45 ms (one-off full repaints; scrolling repaints only the changed region).
Release binary: 16.0 MB.

The software renderer exposes no first-frame hook, so first paint itself
is not timed in-process; an external check (process start → visible
window) measured ~50-135 ms. The OpenGL renderer measured ~275 ms to first
paint, so it is no longer built.

## Done

- [x] Plan reset: scope, non-goals, budgets.
- [x] SQLite cache (WAL), with one connection per thread.
- [x] Purelymail and Postale.io presets, plus manual IMAP/SMTP settings.
- [x] Passwords in Windows Credential Manager only.
- [x] Per-account sync and interactive workers.
- [x] Header-only first sync (newest first, 500 per batch).
- [x] One `UID FETCH 1:* (FLAGS)` per folder per sync gives deletions and
      read/flag/answered state in one round trip; UIDVALIDITY reset.
- [x] On-open body fetch, plus inbox body prefetch (newest 300, ≤ 512 KB).
- [x] Reader: text bodies, HTML→text, Reply-To, saving attachments
      (sanitized file names).
- [x] Composer: new, reply, reply all, forward (with original
      attachments), Cc/Bcc, attachments; SMTP send; copy appended to Sent;
      \Answered set.
- [x] Actions: read/unread, flag, archive, delete (→ Trash, expunge in
      Trash), move. A sync running at the same time skips messages with a
      pending action, so moved mail doesn't reappear and flag changes stick;
      a failed action resyncs the source folder.
- [x] Fluent 2 UI, light/dark following Windows, compact rail below
      1040 px width, keyboard shortcuts.
- [x] Optimization pass (Sonnet research + review workflow, 2026-10-07):
      bodies in their own table (schema v2, with migration), list index
      matching the sort, lazy list model, folder-role table, full HTML5
      entities via mail-parser's table, linear-time HTML scanning, numbered
      and nested lists, Message-ID on sent mail, quoted display names
      (reply-all fix), In-Reply-To fallback for threading, inline images
      not listed as attachments, hardened attachment names, KeyBinding
      shortcuts, form state local to the settings dialog, no OpenGL
      renderer, faster dev builds and CI.
- [x] Tests: cache (incl. migration and query plan), MIME, HTML→text, IMAP
      helpers, SMTP (fake server), roles, formatting.

## Next

- [ ] Validate against real Purelymail and Postale.io test accounts (needs
      explicit authorization and test credentials).
- [ ] First-frame timing hook for the software renderer, and a cold-start
      measurement on a clean machine.
- [ ] IMAP IDLE on the inbox for instant new mail (polling is every 5 minutes
      today).
- [ ] CONDSTORE/QRESYNC for cheaper flag sync on very large folders (the
      imap crate has no support; the flag FETCH sends ~50 bytes per message).
- [ ] Save drafts to the server Drafts folder.
- [ ] Search uses SQLite `LIKE`, which ignores case only for ASCII ("josé"
      does not match "José"). Fold case in Rust or add an ICU-free
      normalized column.
- [ ] Drafts live only in memory: closing the app loses an unsent draft.
- [ ] Clickable links in the reader.
- [ ] App icon, installer, code signing.
- [ ] Confirm whether Purelymail/Postale.io save sent mail server-side; if
      so, default "save a copy to Sent" off for that preset.
