# TODO

Scope and budgets live in `Project Komorebi - Email Client Plan.md`. This file
tracks what is done and what is left.

## Status (2026-10-07)

The rewrite replaced the GPUI/AI prototype (~17.8k lines) with a small
Slint app (about 10k lines of Rust, about half of it tests, plus 1.2k
lines of Slint and 1k lines of dev tools in `examples/`).

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
Release binary: 12.95 MB (size-optimized, fat LTO, static CRT; was 16.0 MB).

The software renderer exposes no first-frame hook, so first paint was
not timed in-process; an external check (process start → visible
window) measured ~50-135 ms. (Since the product round's Stage 22,
`KOMOREBI_TIMING` prints "first frame drawn" from the winit event hook:
~42-46 ms on the seeded 2000-message cache.) The OpenGL renderer measured ~275 ms to first
paint, so it is no longer built.

Measured live on a real Postale.io account (1,257 messages in 5 folders,
~103 ms round trip), release build, read-only probe (`examples/probe`):

| | Before the sync work | After |
|---|---|---|
| Steady-state pass (nothing changed) | 1,268 ms, 46 KB, 11 commands | 113 ms, 0.7 KB, 1 command |
| Changed inbox (5 new, 1 flag) | 43 KB | 2.9 KB |
| First inbox rows visible | ~900 ms | 468 ms |
| Inbox body prefetch | +300 bodies (~23 MB) every pass, unbounded | the 300-message window once (~22 MB in 8 MB passes), then nothing |
| Connect + TLS + login | ~530-590 ms (5 round trips; unchanged, run-to-run noise) | |
| New-mail latency | up to 5 min | up to 60 s (Postale doesn't push new mail over IDLE) |

A cold server (first ever access to a folder) took ~100 s for the inbox's
headers on the first run: that is Postale's Dovecot building its cache, not
the client, and it happens once. Sending to yourself takes ~1.8 s and the
provider delivers in 1-2 s. Every real body in the cache parses without
panics (`examples/parse_cache`).

Learned on Postale's Dovecot, and handled:
- A session that keeps a folder selected never sees mail delivered later,
  not even after NOOP or IDLE; a fresh SELECT or STATUS does. So new mail
  comes from the 1-minute one-command pass, and any action on a UID at or
  above the folder's UIDNEXT at SELECT time re-SELECTs first (a STORE or
  MOVE on a UID the session doesn't know succeeds without doing anything).
- The imap crate's NOOP, SEARCH and LIST parsers reject legal unsolicited
  responses (`* n FETCH (FLAGS ...)`, `* OK [...]`) that arrive while a
  folder is selected, so LIST-STATUS, STATUS and the stale-connection NOOP
  run with nothing selected (UNSELECT, which never expunges), and the sync
  worker leaves its folder whenever it goes idle.
- The crate's IDLE timeout restarts on every `* OK Still here` keepalive
  (every 2 minutes), so a wait longer than that never ended; IDLE waits
  now have a real deadline enforced in Komorebi's stream layer.

## Done

- [x] Plan reset: scope, non-goals, budgets.
- [x] SQLite cache (WAL), with one connection per thread.
- [x] Purelymail and Postale.io presets, plus manual IMAP/SMTP settings.
- [x] Passwords in Windows Credential Manager only.
- [x] Per-account sync and interactive workers.
- [x] Header-only first sync, newest first: a first batch of 50, then
      batches sized for about 1.5 s round trips (at most double the last,
      50 to 500); a dropped connection halves the size for the retry.
- [x] One `UID FETCH 1:* (FLAGS)` per folder per sync gives deletions and
      read/flag/answered state in one round trip; UIDVALIDITY reset.
      Messages flagged \Deleted are hidden. Actions on a folder whose
      UIDVALIDITY changed are refused and the folder resynced. A header
      Date that is missing or more than a day ahead falls back to
      INTERNALDATE (new rows only). A response the parser can't read drops
      the connection without a retry.
- [x] Steady-state pass in one round trip: with LIST-STATUS and CONDSTORE,
      one `LIST ... RETURN (STATUS (MESSAGES UIDNEXT UIDVALIDITY
      HIGHESTMODSEQ))` (else LIST plus a STATUS per folder with CONDSTORE),
      and a folder whose STATUS equals the fingerprint of its last complete
      sync, with as many cached messages, is skipped. The fingerprint is
      recorded only after an error-free sync with no action in flight; a
      failed action clears it. A pass runs every minute (one command when
      nothing changed). Start, Refresh and every 60th pass (about hourly)
      sync every folder in full. Folders holding \Deleted messages
      never skip (those aren't cached).
- [x] Changed-folder delta (CONDSTORE): a folder whose STATUS moved on from
      its fingerprint fetches only the new-UID tail (`UID FETCH n:*`, at most
      one header batch), the flags changed since its HIGHESTMODSEQ
      (`CHANGEDSINCE`), and, only if the cached count differs from EXISTS,
      a `UID SEARCH ALL` whose length must equal EXISTS before any cache
      row is removed. Anything else (UIDNEXT or modseq going back, a
      message to re-add) takes the full path in the same pass. CHANGEDSINCE
      never reports expunges, so the count check and the hourly full pass
      are the backstops. Folder syncs the UI asks for run in full; IDLE wakes
      are STATUS-gated (skip or delta).
- [x] On-open body fetch, plus inbox body prefetch (bodies ≤ 512 KB among
      the newest 300, at most 8 MB per sync pass, in batches of 25 or
      2 MB). Prefetch runs after the pass, a batch at a time, and yields
      to queued syncs; the inbox syncs first.
- [x] Reader: text bodies, HTML→text, Reply-To, saving attachments
      (sanitized file names).
- [x] Composer: new, reply, reply all, forward (with original
      attachments), Cc/Bcc, attachments; SMTP send; copy appended to Sent;
      \Answered set. Each send runs on its own thread and reports one of:
      sent; sent but the Sent copy failed (composer cleared either way);
      not sent (refused or unreachable; 552/452 says the server found it
      too large); or unconfirmed (the connection broke mid-session: "check
      Sent before resending"). Not sent and unconfirmed keep the draft,
      whose Message-ID stays the same across resends. Nothing is resent
      automatically. (Since the product round's Stage 5, sends go through
      the Outbox, with retries only for mail that surely did not go out.)
- [x] Actions: read/unread, flag, archive, delete (→ Trash, expunge in
      Trash), move. Without UIDPLUS, a delete or COPY-move leaves the
      message flagged \Deleted on the server (never a bare EXPUNGE). A
      sync running at the same time skips messages with a pending action,
      so moved mail doesn't reappear and flag changes stick; a failed action
      resyncs the source folder.
- [x] Connection handling: IMAP connects try each resolved address
      (IPv6 and IPv4 alternating, 4 s each); a connection unused for over
      a minute is checked with a NOOP before use and replaced if dead; a
      connection that fails and can't be replaced ends the sync pass (the
      1-minute timer retries). A rejected sign-in (or no saved password)
      stops all LOGIN attempts for the account until Refresh, and is
      reported once. The interactive connection signs in after the first
      successful sync. STARTTLS refuses data sent ahead of the handshake.
- [x] IMAP IDLE on the inbox (a third, read-only connection per account,
      started after the inbox first syncs): a change only asks the sync
      worker for a STATUS-gated sync of the inbox (skip or delta). On a real
      Postale account it woke 0.8 s after another session changed a flag,
      but never for newly delivered mail, so there the 1-minute pass is
      what finds new mail; servers that push new mail show it in about a
      second. Each wait has a hard deadline (see above). A dropped
      connection reconnects with backoff (5 s doubling to 10 min, jittered)
      and catches up. No IDLE capability, a NO/BAD to IDLE or three sign-in
      refusals in a row (connection limits) stop it quietly; the 1-minute
      pass remains. A rejected sign-in pauses it until Refresh retries.
      Residual risk: a server answering IDLE with a tagged OK and no `+`
      still hits an `unreachable!()` in the imap crate (abort in release).
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
- [x] Tests: cache (incl. v1→v2→v3→v4 migration, worker refusal of an
      unmigrated cache, write-lock waits, query plan), MIME, HTML→text
      (incl. a seeded fuzz), IMAP against a scripted local fake server
      (sync, skip gate, delta, header ramp, UIDVALIDITY, sign-in latch,
      IDLE), the worker scheduler, SMTP send outcomes (fake server), roles,
      formatting.

- [x] Product round, Stage 1 (2026-10-08): reader correctness and a crash
      note. The attachment list (one filter, `mime::listed`, so
      save-by-index stays consistent) hides a Content-ID part the HTML
      body references unless it is explicitly an attachment (unnamed
      inline parts stay hidden as before), a `text/calendar` alternative
      when an `.ics` file carries the same event (by UID; with no `.ics`,
      the alternative is listed as `invite.ics`), and the signature part
      of a top-level `multipart/signed` (`application/pkcs7-signature`,
      `x-pkcs7-signature`, `pgp-signature`; not verified, no marker); the
      signed part is the body. Corpus: 01, 04 and 11 list nothing, 13
      lists only `invite.ics`, 14 only `lies.png`; two synthetic signed
      fixtures in `tests/fixtures/mime/`. The renderer keeps no-break
      spaces at span edges (parley trimmed them), so Word pseudo-bullets
      keep their gap. To/Cc in the reader header show display names. A
      panic appends one line (UTC time, version, thread, file:line; never
      the panic message) to `crash.log` beside the cache, capped at 64 KB;
      the next launch shows a note until dismissed, once per new entry
      (`crash.seen`). `panic = "abort"` stays.
- [x] Product round, Stage 2 (2026-10-08): fixed-width mail in narrow
      panes is scaled to fit. The layout reports `Doc::natural_width`, the
      right edge of what it paints (boxes, clipped images, visible glyph
      runs without trailing spaces). When that is more than 4 px past the
      column, the worker lays the message out once more at that width
      (at most 1000 px; the dark-rules pass shares that pass) and draws it
      at column / width: tiles are rasterized at device scale times that,
      so they blit 1:1 and stay sharp. Below 0.6 the scale stays at 0.6 and
      the paper card scrolls sideways (Shift+wheel or its bar; drags still
      select). One `PageTransform` on the worker maps every coordinate
      (tiles, pointer, selection, links, anchors), so the UI thread only
      sees view px. Content that overflows further on a wider page (sized
      relative to it, e.g. 14) is not fixed-width and stays at the column,
      cut as before. Mail that reflows is unchanged. At 900×700 (464 px
      column) 02, 03, 05 and 09 now scale (0.82, 0.78, 0.72, 0.77), at
      1280×800 (576 px) 01, 03 and 05; 05's amounts are on screen at both.
      Open to first tile on a scaled message costs the extra layout pass
      (900×700, medians of 5: 05 18→34 ms, 02 24→35 ms, 09 6→9 ms; 03 and
      04 unchanged); cheaper tiles do not offset it. Release binary +26.5
      KiB (14,411,776 bytes, 13.74 MiB); launch on the seeded 2000-message
      cache unchanged (window 3.7 ms, list ready 5.7 ms, medians of 7).
- [x] Product round, Stage 3 (2026-10-08): composer state pivot,
      keyboard and accessibility. Drafts live in Rust
      (`src/app/composer.rs`: `Drafts` keyed by `DraftId`, each `Draft`
      with its account, kind, fields, reply metadata, attachments,
      Message-ID and send token); the composer's fields are a view that
      reports edits, leaving a field and the From choice back. Starting a
      reply, forward, `mailto:` or New mail (while the composer shows)
      keeps a draft the user changed, in memory; one the app only filled in
      is replaced. With the composer hidden, New mail brings back the draft
      in progress or the newest kept one. A send's outcome finds its draft
      by token, so it clears only that draft and only once delivered, even
      if another draft is showing; a draft that is sending can't be sent
      again. Keys in the composer: Ctrl+Enter / Alt+S send, Ctrl+Shift+C /
      Ctrl+Shift+B show Cc and Bcc and focus one, Ctrl+Shift+A attaches, Esc
      closes and keeps the draft ("Draft kept"). New message focuses To,
      reply the top of the body. Fields carry accessible labels; the
      Cc/Bcc control takes Tab focus (Space/Enter). Leaving To/Cc/Bcc checks
      the addresses (`smtp::check_addresses`) and shows the error under the
      field, rechecked as you type until right; a line under the toolbar
      says why Send is off (no recipient, a wrong address). The message
      actions (reply, forward, read, flag, archive, delete, move,
      next/previous) are also refused in Rust while the composer or Settings
      shows. Headless window tests drive the real app (keys, focus, guard,
      switching). Release binary +65 KiB (14,478,336 bytes, 13.81 MiB);
      launch on the seeded 2000-message cache unchanged within noise
      (window 3.8-4.1 ms, list ready 5.7-6.2 ms, medians of 7 over two
      runs; before 3.8 / 5.9 ms).
- [x] Product round, Stage 4 (2026-10-08): persisted drafts and the
      Drafts view. Cache schema v5 (migrated in place) adds `drafts` and
      `draft_attachments`. A draft worth keeping (the user wrote something,
      or it was sent) is saved two seconds after the last edit (one UPDATE
      on the UI thread: about 0.02-0.06 ms for a 0.2-20 KB body, 0.3 ms
      median for 200 KB) and at once when the composer closes, another
      draft takes its place, the From account changes, a message is opened
      over it or the window closes; an emptied or untouched one is deleted
      instead. Its Message-ID is minted on the first save and kept across
      restarts. Attachments are copied once, when added, to
      `cache.drafts\<draft id>\<attachment id>` beside the cache (never into
      SQLite, never by the autosave), and read back with the 100 MB cap; a
      copy that is missing is left out with a note. "Drafts (n)" in the nav
      appears once the count (one indexed query 250 ms after start) is
      above zero, lists the saved drafts (recipients, subject, start of
      the text, time saved; searchable) and opens one on click. Saved
      drafts are offered once at start ("Resume draft" on the status
      line, only if nothing else shows there); the composer never opens by
      itself. Reply on a message with a saved draft reopens it, and the
      message row shows a "Draft" tag (`reply_to_msg` is cleared when the
      message leaves the cache, so a reused message id is never tagged).
      Discard asks first (in-app prompt) when the draft has anything the
      user wrote, then deletes the row and the copies; a delivered send
      deletes them too; failed and unconfirmed sends keep the draft. Removing
      an account deletes its drafts and their copies. User data: the cache
      stays plaintext; this adds draft text and recipients to it, and
      plaintext attachment copies in `cache.drafts` (see PRIVACY.md).
      Release binary +89.5 KiB (14,569,984 bytes, 13.89 MiB; includes
      render changes made in the tree at the same time); launch on the
      seeded 2000-message cache unchanged within noise (window 3.8 ms,
      list ready 6.0 ms, medians of 9 interleaved runs; before 3.7 /
      5.7 ms).
- [x] Product round, Stage 5 (2026-10-08): outbox, undo send, send guards
      and single instance. Cache schema v6 (migrated in place) adds `raw`,
      `attempts`, `next_try` and `last_error` to `drafts`: the outbox is
      drafts with `state` > 0 (1 undo window, 2 queued, 3 sending, 4
      failed, 5 unconfirmed), and every step is one conditional UPDATE, so
      the UI, the 1-minute pass and a send thread can't move a row twice; a
      send takes a row with one `state 2 → 3` statement. Send asks first
      (one bar with "Send anyway") about an empty subject, or text that
      mentions an attachment (attach…, anex…; quoted and forwarded lines
      left out) with nothing attached. Then the draft is saved as it is,
      frozen, the composer closes and "Sending in 5 s" offers Undo (back to
      the composer unchanged, attachments included) while a worker builds
      the message once with the draft's Message-ID and stores it. The row
      is queued when the window and the build are both done; a build that
      ends after Undo stores nothing (the cancel flag is read under the
      write lock, and Undo also clears `raw`). The same bytes go over SMTP
      (`send_raw`) and to Sent (behind the Bcc header); the row is deleted
      right after the server accepts the message, before the Sent copy.
      Not reachable or 4xx: queued again after 30 s, doubling, at most 6
      tries (the 1-minute pass sends what is due), then failed; 5xx:
      failed; cut off mid-session: unconfirmed, never retried by itself; a
      send running when the app stopped becomes unconfirmed at the next
      start, and a Send whose window the exit cut short is sent then. The
      "Outbox (n)" nav entry (while non-empty) lists the rows with their
      state; Edit, Retry (asks first when unconfirmed) and Discard (asks).
      Removing an account with drafts or outbox rows asks and says how
      many will be lost. A named mutex `Local\Komorebi-<FNV-1a of the
      cache path>` keeps one app per cache; a second start brings the
      first window forward and exits. User data: the cache stays
      plaintext; this adds each outbox message as built (attachments
      encoded in it) until it is sent, discarded or its account removed.
      Release binary +131 KiB (14,704,128 bytes, 14.02 MiB); launch on the
      seeded 2000-message cache unchanged within noise (window 3.7 ms, list
      ready 5.7 ms, medians of 9 interleaved runs; before 3.5 / 5.8 ms).
- [x] Product round, Stage 6 (2026-10-08): account editing, signatures and
      the settings store. Cache schema v7 (migrated in place) adds
      `accounts.signature` and a `settings(key, value)` table, read once at
      start with the accounts (under 0.1 ms, below the timing marks'
      resolution); missing or unreadable keys take their defaults. First
      key: `undo_send_secs` (default 5, at most 60; no UI until Stage 19).
      Settings → Edit opens the account form filled in (address fixed,
      password empty and never shown: empty keeps the saved one; servers
      with an IMAP SSL/TLS or STARTTLS choice that switches 993/143). A new
      password or other servers are checked on a thread, IMAP then SMTP
      (`smtp::verify_smtp`: connect, EHLO, AUTH, NOOP, QUIT; no message),
      and the error names the server; adding an account now runs the same
      check, so a wrong SMTP password is caught at add time. On success the
      password is saved first, then the row (`Db::update_account`; folders,
      messages and drafts stay); new servers or a changed Sent choice
      replace the account's workers (they hold the account as spawned),
      and a new password alone lifts the sign-in latch and syncs. Name and
      signature save at once, offline too. Each account in Settings shows
      its last sync from this session (`Event::Syncing` now says whether
      the error was unreachability); a rejected sign-in shows a red dot on
      the account in the navigation, "Sign-in rejected" with "Update
      password" in Settings, and "Update password" on the status line.
      `SIGN_IN_HINT` now points to Settings. Remove always asks ("Removes
      this account and its downloaded mail from this computer. Mail on the
      server is not affected.", plus the unsent count) and deletes the
      credential before the row; a Credential Manager failure stops it and
      says "Windows Credential Manager is not available" (also for reads
      and saves). Signatures: new mail, replies and forwards start with
      `\n\n-- \n<signature>` once, at compose start (after a `mailto:`
      body, above the quote or forwarded message otherwise); another From
      account swaps the block while it is unchanged (a saved draft only
      while it holds its account's current block); a signature alone is
      not worth saving; quoting leaves out the original's trailing `-- `
      block (at most 10 lines after it). User data: the cache stays
      plaintext; this adds each account's signature and the settings
      values. Release binary +75 KiB (14,781,952 bytes, 14.10 MiB); launch
      on the seeded 2000-message cache unchanged within noise (window 3.8
      ms, list ready 6.1 ms, medians of 9 interleaved runs; before 3.6 /
      5.7 ms).
- [x] Product round, Stage 7 (2026-10-08): HTML alternative and HTML
      quoted replies (`src/mail/compose_html.rs`, panic-free lints like
      the renderer). The composer stays plain text; Markdown-lite markers
      (`**bold**`, `*italic*`/`_italic_` at word boundaries, `- ` and `1. `
      lists with one nesting level (numbered ones start at 1), `> ` quotes,
      `[label](url)` and bare http(s) links; only http, https and mailto
      become links) are converted to conservative email HTML (Segoe
      UI/Arial 14 px wrapper,
      `<p>` with kept line breaks, blockquotes, the `-- ` signature in
      `<div class="sig">`). HTML goes out only when bold, italic, a list
      or a `[label](url)` link is typed (bare URLs and `> ` quotes alone
      don't count, so replies to plain mail stay plain) or a quote is
      attached; then the message is multipart/alternative (text/plain with
      links as `label (url)`, then text/html), inside multipart/mixed with
      attachments. Otherwise it is built byte for byte as before. A reply
      or forward of an HTML message (the HTML the reader renders) stores
      the sanitized original as `drafts.quoted_html` (the column reserved
      in v5; no migration): a tag and attribute allowlist over the
      renderer's tokenizer, no images (cid ones too), no `<style>`,
      `<link>`, scripts, forms, frames, classes or ids, no CSS `url()`,
      image functions, expressions, escapes or positioning, links only
      when followable. The sent HTML is the typed text, then
      `<div class="komorebi_quote">` with the "On ... wrote:" line and the
      original in a blockquote (a forward: the heading and a
      From/Sent/To/Subject table). The stored quote records the line count
      and an FNV-1a hash of the text quote from that line on; if the user
      edits the text quote (answers between quoted lines, trims it,
      deletes the line), the whole text is converted instead. Tests: MIME
      structure per case with distinct boundaries and utf-8 charsets,
      plain output unchanged, João/Reunião round trip, a reply through the
      fake SMTP server, converter and sanitizer cases plus a mutation loop
      and linear-time hostile lines, every corpus HTML message replied to
      and rendered back with no remote content (`tests/compose_html.rs`),
      and the app flow (reply, send, built outbox message). `seed
      --html-reply` adds such a reply for snapshot QA. User data: the
      cache stays plaintext; this adds, per reply or forward draft of an
      HTML message, the sanitized HTML it quotes (removed with the draft
      once sent or discarded). Release binary +24.5 KiB (14,807,040 bytes,
      14.12 MiB); launch on the seeded 2000-message cache unchanged within
      noise (window 4.3 ms, list ready 6.2 ms, medians of 9 interleaved
      runs; before 4.1 / 6.1 ms).
- [x] Product round, Stage 8 (2026-10-08): formatting toolbar on a
      TextInput-based body. Spike passed on the pinned Slint 1.18.1, so
      there is no caret-only fallback: the composer body is `ComposeBody`
      (`ui/widgets.slint`), a raw TextInput in a ScrollView styled like the
      Fluent TextEdit (padding, selection colours, focus underline,
      scroll-to-caret, Cut/Copy/Paste/Select all menu, one accessible text
      field "Message body"), which reports the TextInput's internal
      `anchor-position-byte-offset` / `cursor-position-byte-offset`. The
      headless window test `formatting_keys_act_on_the_body_selection`
      checks they are UTF-8 byte offsets after typing "ação 😀" one
      character at a time (how a dead key or IME commit arrives), Shift+
      arrow/Home selection and undo, and that `set-selection-offsets`
      restores the computed selection after the text is replaced (typing
      then replaces exactly it). Checked once with a temporary test (not
      kept: it hard-codes pixel positions): a mouse drag selection gives
      the same offsets, and clicking a toolbar button keeps the body's
      focus and selection.
      A real Portuguese dead-key keyboard and a Windows IME window could not
      be driven headless; they go through the same TextInput insert path.
      Toolbar above the body: Bold, Italic, Link, Bulleted list, Numbered
      list, Quote (icon-only with tooltips, about 210 px, so it fits the
      narrowest composer at the 720 px minimum window). Keys in the body:
      Ctrl+B, Ctrl+I, Ctrl+K, Ctrl+Shift+8 and Ctrl+Shift+7; Slint reports
      the shifted character, which depends on the layout, so `8 * (` and
      `7 & /` are accepted (US, UK, Portuguese, Spanish, German, French;
      so on a US layout Ctrl+Shift+9 also makes bullets); Ctrl+Shift+B
      stays Bcc. The edits are pure functions over byte offsets
      (`src/app/composer/markup.rs`, panic-free lints; offsets off a char
      boundary are no edit): `wrap_selection` (`**` bold, `_` italic so the
      two nest; whitespace at the ends stays outside the markers and part
      of a word grows to the whole word, since the converter leaves
      markers inside a word as text; several
      lines wrap per line; markers inside or just outside the selection
      come off; an empty selection gets an empty pair around the caret),
      `toggle_line_prefix` (`- `, `1. ` counting from 1 after each blank
      line, `> `; the caret keeps its column, a selection grows to whole
      lines; switching list kinds swaps markers) and `insert_link`
      (`[label](url)`, a bare web address with no selection, `https://` or
      `mailto:` added, other schemes refused with "Not a web or e-mail
      address", brackets percent-encoded). Ctrl+K opens a Link field under
      the toolbar (Enter inserts, Esc closes only it; the selection comes
      back). Replacing the text clears the TextInput's own undo history, so
      the draft keeps the text before the last toolbar edit and Ctrl+Z puts
      it back while the body is unchanged since (typing after it is undone
      by the field first). Converter fix: an emphasis marker may now be
      followed by the other one (`**_x_**`, `_**x**_` were left as text).
      Typing latency unchanged: a 20 KB body, key plus three full
      1280×800 frames in a release headless test, median 10.4 ms with
      `ComposeBody` vs 10.3 ms with the old TextEdit (noise); a toolbar
      edit on that body 10.5 ms. `KOMOREBI_TIMING` covers startup only.
      New snapshot action `compose-format`. Release binary +105.5 KiB
      (14,915,072 bytes, 14.22 MiB, 14.92 MB: 85 KB under the 15.0 MB
      ceiling); launch on the seeded 2000-message cache unchanged within
      noise (window 3.9-4.4 ms, list ready 5.9-6.6 ms, medians of 9 over
      three runs; Stage 7: 4.3 / 6.2 ms).
- [x] Product round, Stage 9 (2026-10-08): recipient autocomplete from a
      derived address index (not a contacts database; the plan's non-goal
      now says so). Cache schema v8: table `addresses` (account, lower-cased
      address, newest display name, lower-cased name for matching, last
      seen, weight; deleted with the account) and `accounts.addr_backfill`.
      User data: the cache stays plaintext; this adds the addresses and
      display names of senders and of the recipients of sent mail, already
      in the cached headers, as a second copy (PRIVACY.md). Storing headers
      updates it in the same transaction, only for rows actually added:
      the sender +1, and in the Sent folder each To/Cc address +3; Junk,
      Drafts, the account's own address and anything that is not a single
      `local@domain` are left out; the newest non-empty name wins. Mail
      cached before v8 is indexed by the sync worker after its first pass,
      newest first, 5,000 messages per transaction with the mark moved in
      the same transaction (idempotent, never on the launch path; about
      11-19 ms per chunk on a 20,000-message file cache), one chunk per
      idle step before the inbox prefetch. `Db::rebuild_addresses` drops an
      account's index and marks its mail for the backfill again. After two
      typed characters, `suggest_addresses` (two index range searches on
      the address and the lower-cased name, ordered by weight then last
      seen, 6 shown) runs on the UI thread for the draft's From account:
      0.05-0.33 ms on 10,000 addresses (0.33 ms when 1,000 of them match;
      the first version, a LIKE scan that also matched later words of the
      name, took 2.4 ms, so only prefixes of the address or the whole name
      match). Addresses already in To/Cc/Bcc are not suggested. The token
      at the caret is completed (the caret is taken from the edit, since a
      line field reports only its text: just past the changed span), and
      accepting replaces it with `Name <addr>, ` (names with specials
      quoted, so `"Doe, Jane"` stays one address) and puts the caret after
      it. The list sits in the layout under the address rows (one list
      for To/Cc/Bcc; three lists, one under each field, cost 18 KB more),
      Up/Down/Enter/Tab/Esc work it from the composer's key capture, Esc
      with no list still closes the composer, moving the caret or leaving
      the field closes it, a click takes a row. `mime::addresses` and
      `format::recipients` now share `smtp::mailbox` /
      `smtp::mailbox_parts`. New snapshot action `compose-suggest`.
      Release binary +57.5 KiB (14,973,952 bytes, 14.28 MiB, 14.97 MB: 26
      KB under the 15.0 MB ceiling); launch on the seeded 2000-message
      cache unchanged (window 3.7-3.8 ms, list ready 5.7-6.0 ms, medians of
      9-12 per run over three runs; 3 of 33 runs had a ~125-140 ms stall
      between "database open" and "model ready", a step this stage does
      not touch, still under 200 ms).
- [x] Product round, Stage 10 (2026-10-08): attachments by drop and
      paste, a size counter and safe Open. No schema change. Drops use
      winit's own OLE drop target through Slint's `unstable-winit-030`
      window-event hook (`WindowEvent::HoveredFile` / `DroppedFile` /
      `HoveredFileCancelled`, registered in `main.rs`; winit 0.30 enables
      drag and drop on Windows by default, Slint does not turn it off, and
      the hook runs before Slint handles any event, with any renderer), so
      the HWND/`WM_DROPFILES` fallback was not needed. While a file hovers,
      the composer card gets a 2 px accent border (Slint has no dashed
      border). A drop of several files arrives one file at a time and is
      attached as one batch; with no composer showing it is ignored. Ctrl+V
      in the composer first checks the clipboard for CF_HDROP (files copied
      in Explorer): those are attached and the key is swallowed, otherwise
      the field pastes text (`files::hdrop_paths` parses the block as a pure
      function, bounds-checked). Dialog, drop and paste share one path:
      files are read on a short-lived thread with the 100 MB limit
      (folders refused), the limit is checked again when they arrive, and
      the copies into `cache.drafts` are made once, as before (on the UI
      thread, by the draft save). A line above the attachments shows
      "Attachments 18.4 MB of 25 MB" (base64 size), amber from 80%, red
      over 25 MB, when Send is off ("Attachments are over 25 MB; remove
      some to send."; it used to be only a warning). The composer's
      attachment chips wrap, as many 200 px chips to a row as fit. A click
      on an attachment in the reader opens it (the button saves, as
      before): the part is written to `cache.open\<random>\<name>` beside
      the cache (not `%LOCALAPPDATA%\komorebi\open`, so a throwaway
      `KOMOREBI_DB` never writes into the real app folder; names fitted to
      MAX_PATH), marked with a `Zone.Identifier` stream (ZoneId=3) and
      opened with ShellExecuteW on a short-lived thread ("No app on this
      computer opens ..." if none does). Programs and scripts are refused
      with a note to save instead (the plan's list plus `.pif .vbe .wsh
      .msp .reg .jar`, by the last extension, so `invoice.pdf.exe` too).
      Saved attachments get the same stream. Folders in `cache.open` older
      than this start are removed one second after start, on a thread. The
      sanitizer now also catches `NUL .txt`, `COM¹`-`COM³`/`LPT¹`-`LPT³` and
      `CONIN$`/`CONOUT$`. To fit the size ceiling, Open is the chip click
      rather than a second button per chip (9 KB of generated UI), and the
      app's short-lived threads and UI-thread callbacks now share one boxed
      `background` / `on_ui` helper. New snapshot action `compose-attach`.
      Not checked: a real drag from Explorer and a real Explorer copy
      could not be driven from the headless session (see Next). User
      data: the cache stays plaintext; opened attachments are plaintext
      copies in `cache.open` until the next start (PRIVACY.md). Release
      binary +25 KiB (14,999,552 bytes, 14.30 MiB, 14.9996 MB: 448 bytes
      under the 15.0 MB ceiling); launch on the seeded 2000-message cache
      unchanged within noise (window 4.0 ms, list ready 6.0 ms, medians of
      11 interleaved runs; before 4.4 / 6.5 ms in the same runs).
- [x] Product round, Stage 11 (2026-10-08): server Drafts folder sync.
      Cache schema v9 (migrated in place) adds to `drafts` a change count
      (`rev`, bumped by every field save and attachment change) and the
      server copy: `server_rev`, `server_folder` (folder id, cleared when
      the folder goes), `server_uidvalidity` and `server_uid`. Drafts saved
      before v9 count as changed, so the first sync after the upgrade
      copies them. The sync worker does all draft copies, one at a time
      (`Job::SaveDraft { row }`, `Job::DeleteDraft(copy)`): it builds the
      draft from the cache with the send builder (`smtp::build_draft`: Bcc
      kept, an `X-Komorebi-Draft-Id: <row>` header, no recipient needed,
      addresses that aren't valid yet left out), appends it to the folder
      with role Drafts flagged `\Draft` only, finds the new copy's UID by a
      fresh SELECT and `UID SEARCH UNDELETED HEADER Message-ID` (newest
      match; imap 2.4.1 drops the tagged APPEND reply, so APPENDUID can't
      be read), records it (only while the row is still a draft, `state`
      0), and only then removes the previous copy with the delete path:
      `\Deleted`, then `UID EXPUNGE <uid>` only under UIDPLUS, never a
      bare EXPUNGE, and nothing if the folder's UIDVALIDITY changed. A copy
      made while the draft was sent or discarded is removed at once; one
      made from another From account is handed to that account's worker
      (`Event::DraftCopyLeft`). The app asks for a copy after an autosave
      at most every 30 s per draft (`server_save_due`), and always when
      the composer closes or another draft takes its place; the worker
      skips a draft unchanged since its last copy, and after each
      successful sync pass copies every changed draft (so drafts written
      offline go up on the next connection). A delivered send removes the
      copy from its send thread; Discard, an emptied draft and Discard in
      the Outbox ask the account's worker to remove it. No folder is
      created: with no Drafts folder (once the folder list is known), a
      refused APPEND or a copy that can't be found, copies stop for the
      session after one status note; connection errors stay quiet (the
      draft is safe locally). Drafts with over 10 MiB of attachments stay
      local (each copy uploads them again). Loading server drafts into the
      composer is deferred (C8). Tests (fake IMAP): replace with UIDPLUS
      expunges only the old UID; without UIDPLUS no EXPUNGE is sent; Send
      pressed during the upload removes the new copy; a copy the search
      can't find deletes nothing and stops copies; a missing Drafts
      folder keeps the draft local and says so once; an offline save goes
      up after the next sync pass; a delivered send removes the copy; a
      copy of another account goes to that account; the 30 s throttle; v9
      migration and change counting. Snapshot `compose` at 1280×800 and
      900×700, light and dark: no layout change. User data: the cache
      stays plaintext; this adds each draft's change count and the
      location (folder, UIDVALIDITY, UID) of its server copy, and drafts
      now also exist as messages in the user's own Drafts folder on the
      server (PRIVACY.md). Release binary 15,017,984 bytes (+18,432 against Stage 10's)
      was 17,984 bytes over the 15.0 MB ceiling, so the bundled SQLite is now
      built without its unused modules (FTS3/5, R-tree, STAT4, dbstat,
      soundex; `LIBSQLITE3_FLAGS` in `.cargo/config.toml`): 14,715,904 bytes,
      14.03 MiB, 14.72 MB (285 KB under the ceiling). Launch on the seeded
      2000-message cache unchanged within noise (window 4.1-4.4 ms, list
      ready 6.2-6.5 ms, medians of 11 interleaved runs over two runs; Stage
      10's binary 4.0-4.1 / 6.0-6.2 ms in the same runs).
- [x] Product round, Stage 12 (2026-10-08): multi-select and batched
      actions. No schema change. Ctrl+click toggles a row, Shift+click and
      Shift+Up/Down select from the anchor (the last row clicked or
      Ctrl+clicked, as in Explorer) to the cursor, Ctrl+A in the list
      selects the view and Esc clears. The selection is a set of ids in
      `rows::Selection` beside the lazy list (`MailRow.selected`), so
      selecting 50k rows reads nothing; a reload keeps only listed ids.
      Ctrl/Shift selection never opens or marks a row read: the open
      message stays only while it is all that is selected, otherwise the
      reader pane shows "N messages selected" with Archive, Delete, Move
      (the view's account only; off in All inboxes with several accounts),
      Mark read / unread and Flag / Unflag (shared `MoveList` popup).
      Delete and Ctrl+Q act on the selection (Ctrl+Q: all read if any is
      unread). Actions apply to the cache at once (`Db::briefs`,
      `set_seen_many`, `set_flagged_many`, `delete_messages`, one
      transaction each) and queue one job per (account, folder):
      `Job::SetFlags`, `Job::Move` and `Job::Delete` now carry `uids:
      Vec<u32>`, and every UID of a batch is in `Pending` until its job
      ends. The IMAP side sends one STORE / MOVE / COPY / UID EXPUNGE per
      compressed UID set (`1:5,7,9:12`), split at 4,000 characters for
      huge sparse selections; with COPY each set is copied, flagged and
      (UIDPLUS only) expunged before the next, so a failure leaves every
      set moved or untouched. After a bulk delete or move the row that took
      the first removed row's place opens. Tests: UID set compression and
      splitting; a 200-message delete through the fake IMAP server is one
      `UID STORE 1:200` plus one `UID EXPUNGE 1:200`, a two-folder move one
      `UID MOVE` per folder; selection logic (toggle, range, Shift+arrows,
      select all, clear, reload); neighbour after removal; batch cache
      updates; one job per folder; a headless window walk (Ctrl+click,
      Shift+click, Shift+Down, Ctrl+A, the buttons, Ctrl+Q, Esc, Delete,
      Down). Snapshot `KOMOREBI_SNAPSHOT_SELECT=3-7` (new range form) at
      1280×800 and 900×700, light and dark: the panel; `=3` unchanged.
      Release binary 14,808,576 bytes (+92,672 against Stage 11's), 14.12
      MiB, 14.81 MB (191 KB under the ceiling). Launch on the seeded
      2000-message cache unchanged within noise (window 3.9 ms, cache
      loaded 6.3 ms, medians of 15 interleaved runs; Stage 11's binary 4.2
      / 6.4 ms); on a 50k-message cache 3.7 / 13.8 ms (Stage 11's 4.2 /
      11.7 ms, within run-to-run noise), and 200 Shift+Down steps over the
      50k list add no measurable time.
- [x] Product round, Stage 13 (2026-10-08): row context menu, Junk and
      undo for destructive actions. No schema change. Right-clicking a row
      shows one menu (Reply, Reply all, Forward, Mark as read / unread,
      Flag / Remove flag, Archive, Delete, Move to ›, Report junk or Not
      junk) on the selection; a row outside the selection becomes the
      selection first, without opening it or marking it read
      (`Selection::right_click`), and Drafts, Outbox and the composer have
      no row menu. The `ContextMenuArea` is shared and never opens by
      itself (`enabled: false`): the row's right-press updates the
      selection and the labels, then calls `show()`. A More (…) button in
      the reader header opens the same menu; neither has a keyboard
      opener yet (Menu key / Shift+F10, Stage 21). Report junk moves to the
      account's Junk-role folder; with none, "This account has no Junk
      folder, so nothing was moved." and nothing moves. Not junk (on
      messages in Junk) marks them read at once and moves them to the Inbox.
      `$Junk`/`$NotJunk` stay deferred. Archive, Delete, Move and Junk are
      held for 5 s ("Moved to Archive", "3 messages deleted", with Undo
      and Ctrl+Z): the rows leave the list and the unread counts at once,
      but stay in the cache, and their jobs wait in `App::held` (one
      action at a time; the next one, the window closing, or removing an
      account sends the previous one at once). Sending queues the jobs,
      which marks the UIDs pending, then deletes the rows from the cache,
      as before; a reload or sync meanwhile keeps them out of the list.
      Undo just lists them again (reopening the message that was open, or
      selecting them), so nothing ever reaches the server. At exit the app
      waits up to 3 s while any worker still has a message action queued or
      running (`Workers::busy`), so a move sent at close still reaches the
      server; before, an action just before closing could be lost to the
      process exit (then the next sync re-listed the message). Tests: the
      right-click rule; the Undo labels; a headless walk (archive, the
      held job and unread count, a reload, Undo, a three-message delete,
      Ctrl+Z, a second action sending the first, the window closing);
      Report junk with and without a Junk folder and Not junk from the
      Junk view; right-click on the open row, another row and a row of a
      selection, Reply from the menu; a real right-press on a row of the
      headless window (the pointer path to the menu); the Report junk and Not junk jobs
      through the fake IMAP server (`UID MOVE 5 "Junk"`; `UID STORE 7
      +FLAGS.SILENT (\Seen)` then `UID MOVE 7 "INBOX"`). Snapshot
      `KOMOREBI_SNAPSHOT_ACTION=archive` (with `SELECT=3`, and `3-7`: "5
      messages moved to Archive") and `junk` (the no-Junk line on the
      seeded cache) at 1280×800 and 900×700, light and dark. Release binary
      14,846,976 bytes (+38,400 against Stage 12's), 14.16 MiB, 14.85 MB
      (153 KB under the ceiling). Launch on the seeded 2000-message cache
      unchanged (window 4.1 ms, cache loaded 6.1 ms, medians of 15 runs;
      a few runs had 25-150 ms "database open" outliers from machine
      load while other builds ran, still under 200 ms).

- [x] Product round, Stage 14 (2026-10-08): filters, mark all read,
      list keys, delayed mark-read. No schema change. Unread and Flagged
      toggles under the search box (`Db::message_ids_filtered` with
      `Filter { All, Unread, Flagged }`; the toggles hide in Drafts and the
      Outbox and switch off with the view, like the search). Unread names
      `INDEXED BY messages_folder_seen`: left to itself SQLite walked the
      folder's date index and read every row's flag (11 ms against
      0.01 ms for 26 unread of 48,700 in one folder); Flagged has no index
      and filters the date order (12 ms there). A row that stops matching
      stays until the next reload. More (…) beside Refresh: "Mark all as
      read" for the view (`Db::mark_folder_read`, one transaction, then
      `Job::MarkAllRead` per folder: one `UID STORE 1:<highest cached UID>
      +FLAGS.SILENT (\Seen)`, re-SELECTing a stale selection first; the
      UIDs it marked locally stay pending, so a sync meanwhile can't show
      them unread). Empty lists say "No matches" (with Clear search), "No
      unread messages", "No flagged messages", "Loading…", that the last
      check failed or the server can't be reached, or "Nothing here".
      Keys: the window forwards focus to the list, so the arrows work
      before any click; Home/End; PageUp/PageDown by the rows in view;
      Enter opens the selected (or first) message and focuses the page
      (`focus-page`); Esc clears the search, then the selection and the
      open message (the page's own Esc only while it has a text selection
      or a focused link); Ctrl+Shift+U toggles read, Insert flags, Ctrl+E
      archives (Ctrl+E rather than Backspace, which would archive from an
      emptied search box; Insert and Ctrl+E are off while the search box
      has the focus). The list takes the focus back whenever the reader
      closes, so keys never land on a page that went away. Delayed
      mark-read: `mark_read_delay_ms` (default 1000; 0 at once, -1 never,
      at most 60 s; Settings screen in Stage 19) through `ReadWait`, one
      message at a time: moving on, closing the reader or marking it by
      hand cancels the wait. Tests: the filter queries and the EXPLAIN
      QUERY PLAN assertion (Unread on one and on two folders); the
      setting's values; `mark_folder_read`; mark-all-read through the fake
      IMAP server is one SELECT and one STORE; the read wait (pure state);
      the empty-state texts; headless walks: the list keys from start-up,
      Esc in its three steps (also typed into the search box, where Insert
      and Ctrl+E do nothing), Enter and the page focus, the action keys,
      every list key inert while composing; the delayed mark (fast
      navigation, a hand mark meanwhile, closing, Never and Now); filters,
      empty states and Mark all as read. Snapshots
      `KOMOREBI_SNAPSHOT_ACTION=unread` and `read-all` (on a copy) at
      1280×800 and 900×700, light and dark. Release binary 14,909,440
      bytes (+62,464 against Stage 13's), 14.22 MiB, 14.91 MB (91 KB under
      the ceiling). Launch on the seeded 2000-message cache: window
      4.7 ms, cache loaded 6.8 ms (medians of 15; Stage 13 recorded 4.1 /
      6.1 ms, this machine's run-to-run spread); on a 50k-message cache
      cache loaded 11.5-12.7 ms, also with the Unread filter on (a few
      runs 48-97 ms from machine load, still under 200 ms).

- [x] Product round, Stage 15 (2026-10-08): accent-folded search and the
      all-folders scope. No schema change. `src/fold.rs` lower-cases and
      strips Latin accents from a table (á à â ã ä å → a, ç → c, ß → ss,
      æ → ae, œ → oe, ø → o, ł → l, combining marks dropped; other
      scripts are only lower-cased; already-folded text is borrowed, not
      copied). Every connection registers it as a deterministic SQL
      function `fold()` (rusqlite's `functions` feature, no new crate, no
      `Cargo.lock` change); the list search matches the folded query
      (LIKE characters escaped) against `fold()` of subject, sender name
      and address, preview and, new, To and Cc, so Sent is searchable by
      recipient. Drafts and the Outbox search the same way. "All folders"
      (shown while text is typed) searches every cached folder of every
      account, titled "All folders", each row tagged with its folder and
      Sent rows shown by recipient (`rows::Kind::Mixed`); Move offers the
      folders of the selection's account; Mark all as read and the other
      folder actions keep to the view; the scope resets with the view.
      Decision on a stored `search_text` column: not needed. Measured
      with `KOMOREBI_TIMING=1` (which now prints each search) on a 50,003
      message cache, release build: 35-75 ms over every folder for
      "joao", and 76-88 ms in a worst case where every row has an
      accented subject, sender, To, Cc and a 200-character accented
      preview (the plan's threshold was about 100 ms; large scopes are
      still debounced by 100 ms). No FTS5. `PRAGMA optimize` when the app
      closes (planner statistics, `sqlite_stat1`: counts, no mail text;
      the list keeps its covering date index with them) and
      `wal_checkpoint(PASSIVE)` after each sync pass. Tests: the fold
      cases ("JOSÉ" → "jose", "Ação" → "acao", "Straße" → "strasse",
      decomposed accents, other scripts and stray marks never panic);
      "joao" finds "João" only where searched and in Archive with all
      folders; To/Cc matches; the SQL function on NULL, numbers and
      invalid UTF-8; LIKE characters (`%`, `_`, `\`) match literally;
      folded draft search; a headless walk (no match in the inbox, both
      rows with their folder tags in All folders, typing on keeps the
      scope, Mark all as read leaves Archive alone, Clear search untags,
      another view and Drafts reset the scope). `examples/seed` now gives
      the 12th newest inbox message the sender "João Gonçalves" and adds
      two Archive messages from him and one Sent to him. Snapshot
      `KOMOREBI_SNAPSHOT_SEARCH=joao` (one inbox hit) and with
      `KOMOREBI_SNAPSHOT_ACTION=search-all` (four hits tagged Inbox, Sent,
      Archive, Archive) at 1280×800 and 900×700, light and dark. Release
      binary 14,952,960 bytes (+43,520 against Stage 14's), 14.26 MiB,
      14.95 MB (47 KB under the ceiling). Launch on the seeded
      2000-message cache: window 4.9 ms, cache loaded 7.0 ms (medians of
      15, the first run without planner statistics; Stage 14 recorded
      4.7 / 6.8 ms); on the 50k cache 5.1 / 11.8 ms.

- [x] Product round, Stage 16 (2026-10-08): sender trust notice,
      encrypted mail and unsubscribe. No schema change: everything is
      worked out on open from the cached raw message. `src/trust.rs`
      (pure functions, panic-free lints): the topmost
      `Authentication-Results` header whose authserv-id has the IMAP
      host's registrable domain is read (comments stripped, quoted
      strings kept, at most 10 headers of 8 KB); `dmarc=fail`, or spf
      fail/softfail and dkim fail with no pass (a `dmarc=pass` also
      counts as a pass), warns; headers further down (forgeable by the
      sender) and other servers' are ignored, and none matching shows
      nothing. No ARC. A display name with an address or a bare domain
      (`www.`, a scheme, or a common TLD, so `John.Doe` and `Dr. Smith`
      don't count) whose registrable domain differs from the sender's,
      an `xn--` sender domain (decoded with `idna::punycode::decode`,
      `idna` now a direct dependency pinned to the locked 1.1.0, so the
      only `Cargo.lock` change is that edge), and a domain label mixing
      Latin with Cyrillic or Greek are flagged. The registrable domain is
      `links::registrable` (now shared): the last two labels, or three
      under a generic second level of a country code (`co.uk`, `com.br`,
      `com.pt`); a heuristic, not the public suffix list. At most two
      reasons, the server's verdict first, in one line ("Be careful with
      this sender: ...") in the reader's existing note slot above the
      body, in the warning colour. The real address already shows next to
      the display name in every header (elided in narrow panes; the
      notice names the real domain). `mime::Body` gains `auth`,
      `encrypted` and `unsubscribe`. Encrypted: a root
      `application/(x-)pkcs7-mime` with `smime-type` enveloped or
      authEnveloped (or none; `signed-data` is not encryption), a root
      `multipart/encrypted`, or `-----BEGIN PGP MESSAGE-----` in the first
      4 KiB of the text. The reader shows "This message is encrypted
      (S/MIME). Komorebi cannot decrypt it; save the encrypted file ..."
      in the same slot (a sender notice goes first), never HTML, and the
      MIME kinds show no text; the ciphertext is the one attachment
      (`smime.p7m`, named so when unnamed; `encrypted.asc`, with the
      PGP/MIME `Version: 1` part hidden; for inline PGP the armored block
      as `message.asc`, after the listed parts), so Save and Open are the
      attachment chip's. Because the ciphertext is an attachment,
      encrypted mail gets the paperclip in the list and Forward attaches
      it (for inline PGP, `message.asc` beside the quoted armor). The list
      preview is "Encrypted message"
      (`Body::preview`, used by `store_body` and `examples/seed`; rows
      cached before this keep their old preview until the body is stored
      again). List-Unsubscribe: URIs in angle brackets, whitespace inside
      ignored; the first `mailto:` wins, else the first `https:` (plain
      `http:`, other schemes and One-Click POST are not used). An accent
      "Unsubscribe" text button at the right of the To line: mailto opens
      the composer prefilled through the `mailto:` link path (address,
      subject, body), https goes through the link confirm prompt with its
      warnings (`Reader::follow`, shared with clicked links); nothing is
      fetched or sent on open. Tests: `src/trust.rs` (parsing, topmost
      and forged headers, the spf/dkim rule, name domains with dotted
      names that are not domains, punycode and mixed scripts, the
      two-reason line, hostile input), `src/mail/mime.rs` (unsubscribe
      parsing: angle brackets, several URIs, folding, mailto subject and
      body; header caps; the 4 KiB inline scan; opaque-signed S/MIME),
      `tests/notice_corpus.rs` over ten synthetic fixtures in
      `tests/fixtures/notice/` (each verdict, the forged lower-hop header
      ignored, no notice without headers, encrypted previews and files,
      unsubscribe choice), and a headless reader walk (notice,
      Unsubscribe into the composer and into the confirm prompt, the
      encrypted state, nothing left over from the previous message).
      Snapshot QA with `seed -- <new> 0 tests/fixtures/notice` and
      `KOMOREBI_SNAPSHOT_SELECT=0..9` at 1280×800 and 900×700, light and
      dark: the notice wraps to two lines at 900 px under the header
      without crowding it; Unsubscribe sits on the To line. A first
      version with its own notice row, a lock/warning icon, a "Save
      encrypted file" button and the Unsubscribe button in the command bar
      cost 62 KB (over the ceiling; the command bar also overflowed at
      1280 px), so the note slot, the attachment chip and a plain text
      button are reused. Release binary 14,988,800 bytes (+35,840 against
      Stage 15's), 14.29 MiB, 14.99 MB (11 KB under the ceiling). Launch
      on the seeded 2000-message cache unchanged (window 5.7 ms, cache
      loaded 8.7 ms, medians of 15 interleaved runs; Stage 15's binary
      6.3 / 10.5 ms in the same runs, a loaded machine). Not checked: the
      authserv-id convention of Postale and Purelymail (by hand, with the
      user's authorization, later).


- [x] Product round, Stage 17 (2026-10-09): meeting invites: card, RSVP
      and Save .ics. Schema v10 adds `rsvp` (account, UID, RECURRENCE-ID
      as written, the invite's SEQUENCE when answered, the answer, unix
      time; primary key account + UID + occurrence, deleted with the
      account). User data: the cache stays plaintext; this adds one row
      per answered invite with those fields only (no title, attendees or
      reply text). `src/mail/ical.rs` (panic-free lints as in render/;
      refuses over 256 KB, 20,000 lines or 5,000 properties before
      reading) unfolds lines, splits names, quoted parameters and values,
      unescapes TEXT, strips control and bidi characters and reads the
      first VEVENT: METHOD, UID, SEQUENCE, DTSTAMP, DTSTART/DTEND/DURATION,
      SUMMARY, LOCATION, DESCRIPTION (cut at 2,000 characters), URL,
      STATUS, ORGANIZER, ATTENDEE (CN, PARTSTAT, RSVP), RRULE,
      RECURRENCE-ID, VTIMEZONE STANDARD/DAYLIGHT offsets (yearly
      BYMONTH/BYDAY rules, UNTIL honoured) and the Google and Microsoft
      conference properties; anything unreadable is `None` and the message
      shows as before. Times: UTC converts through chrono `Local`; a TZID
      with its VTIMEZONE uses the observance in force at that wall time
      (the latest onset at or before it, this year or last); any other
      TZID shows the wall time with the zone's name, floating times say
      "floating time", dates are all-day (exclusive end). No zone table;
      wall times are never resolved through `Local`, so ambiguous or
      missing local hours can't panic. `mime::Body.invite` (boxed) comes
      from the first `text/calendar` part, else the first `.ics` file;
      `mime::calendar_file` gives its bytes for Save .ics. The reader card
      (above the body, outside the tile renderer): title (struck through,
      first line, when cancelled), a state line (Cancelled, "You
      accepted.", "This event is over.", Updated invitation for SEQUENCE >
      0), when/organizer/where/repeat lines ("Repeats weekly on Mon, Wed
      until ...", daily/monthly/yearly, "every 2 weeks"; anything else
      "Repeats"), and two rows of focusable, labelled buttons from one
      repeater: Accept, Tentative, Decline (only for a REQUEST with an
      organizer that is not cancelled, not over and not RSVP=FALSE for
      the user); then Change (once answered), Join (URL, the conference
      properties or a web address in LOCATION, through the link confirm
      prompt), Details (the description) and Save .ics (rfd, fixed name
      `invite.ics`, Mark of the Web). "Me" is the attendee with the
      account's address, else the one attendee among the message's To/Cc
      (only when exactly one is, so the user never answers as another
      recipient; the prompt names that address), else the prompt says the
      user is not listed and the reply answers as the account. The prompt
      names the exact recipient and the sending
      account, warns when the organizer's registrable domain differs from
      the sender's, and has a third button, "Don't send a reply" (answer
      kept only here). The REPLY (`ical::reply`, `ical::reply_message`)
      goes only to the ORGANIZER (no Cc, no other attendee): same UID,
      SEQUENCE and RECURRENCE-ID, the organizer, one ATTENDEE with
      PARTSTAT, the start, the title, a fresh DTSTAMP, CRLF and folding at
      75 octets on character boundaries; subject "Accepted: <title>"; a
      line of text plus `text/calendar; method=REPLY; charset=utf-8` as
      multipart/alternative (`smtp::Outgoing.calendar`). It is built at
      once and queued already built (`Db::queue_reply`, `drafts.kind` 4,
      straight to Queued), so it never passes through Drafts, the undo
      window or a rebuild that would lose the calendar part; the Outbox
      offers Retry and Discard for it but not Edit (`outbox_unsend`
      refuses kind 4), and it is not linked to the invite (no \Answered).
      The ledger row is written after queuing; an answer to an older
      SEQUENCE no longer counts. Tests: `src/mail/ical.rs` (unfolding,
      quoted parameters, escapes, garbage, caps, durations, offsets, nth
      weekdays, repeat lines, REPLY echo/fold/round trip, when lines),
      `tests/invite_corpus.rs` over eight synthetic fixtures in
      `tests/fixtures/invite/` (Google REQUEST, Outlook REQUEST with a
      Windows TZID and VTIMEZONE, CANCEL, update SEQUENCE 2, PUBLISH
      `.ics` file, all-day, weekly recurring with an unknown TZID,
      malformed; oversized built in the test), VTIMEZONE across both DST
      changes and the missing/doubled hours, Lisbon Z time, the unknown
      TZID label, all-day, no RSVP on CANCEL, and a byte-mutation loop
      (3,000 mutants by default, 200,000 run once; each under 500 ms);
      `src/mail/mod.rs` sends a REPLY through the outbox to the fake SMTP
      server (exactly one RCPT TO, the organizer; PARTSTAT, UID and
      SEQUENCE echoed; lines at most 75 octets; parsed back through
      `ical.rs`; the fake server now records RCPT lines); `src/db.rs`
      `migrates_v9_to_v10` and the ledger and queued reply; a headless
      card walk in `src/app/composer.rs` (answers offered, the prompt's
      recipient, queued reply and ledger, Change, Decline without a reply,
      the not-listed and other-domain warnings, CANCEL, no card after).
      Snapshot QA with `seed -- <new> 0 tests/fixtures/invite` and
      `KOMOREBI_SNAPSHOT_SELECT=0..7`, plus `KOMOREBI_SNAPSHOT_ACTION=rsvp`
      (new) on 0, at 1280×800 and 900×700, light and dark: both button
      rows fit at 900 px, the prompt shows the three buttons. Size: the
      stage itself adds 127,488 bytes (15,121,408 against a 14,993,920
      build of the same tree with the card, the reply and the parser
      stubbed out; a first version with eight separate buttons cost
      another 26 KB), over the 15.0 MB ceiling, so the credential store
      is now set up through `keyring-core` and `windows-native-keyring-store`
      (=1.1.0, `default-features = false`) instead of the `keyring`
      wrapper, whose `v1` feature enabled the store's `search` (which
      links `regex`; imap's own regex is only reached from SASL
      `authenticate`, never called). The store, its version and its
      default configuration are the ones `keyring` v4 used, so saved
      passwords keep their credential names. `Cargo.lock` only loses
      packages (keyring, its macOS/Linux stores, secret-service and their
      crypto and `num` dependencies; windows-native-keyring-store no longer
      depends on regex). Release binary 14,149,120 bytes (-839,680
      against Stage 16's), 13.49 MiB, 14.15 MB. Launch on the seeded
      2000-message cache: window 4.2 ms, cache loaded 6.6 ms, against 4.3
      / 7.0 ms for the stubbed build (medians of 15 interleaved runs, a
      few cache loads 17-140 ms from machine load in both; Stage 16
      recorded 5.7 / 8.7 ms on a loaded machine). Not
      checked: real Google/Outlook/Postale invites and how their
      calendars take the REPLY (by hand, with the user's authorization).
- [x] Product round, Stage 18 (2026-10-09): Save as .eml, Show original,
      Copy Message-ID, Forward as attachment. No schema change. The row
      menu and the reader's More menu gain Forward as attachment, Save as
      .eml, Show original and Copy Message-ID (one message; a selected row
      opens first, as Reply does); Ctrl+S and Ctrl+U while the reader
      shows. A message whose body isn't cached is fetched on the
      interactive worker first (`Job::FetchBody` again, so a failed first
      download is retried; "Fetching message…"), and the action runs on
      its `BodyReady` if the message is still open (`App.raw_wait`); a
      failure says "Couldn't download this message: …"; with no worker
      (offline) "This message isn't downloaded yet.". Save as .eml writes
      the cached bytes verbatim, named `<subject> (<YYYY-MM-DD>).eml`
      (`mail::source::eml_name`: `mime::sanitize_filename`, at most 120
      characters, "message" for an empty subject); the dialog opens from a
      zero timer, never inside `with_app`, and `save_file` now writes on a
      worker thread (attachments and .ics too), with Mark of the Web.
      Show original (`mail::source::original`, panic-free lints): the
      header block up to the first empty line, one line per field,
      unfolded and RFC 2047 decoded (mail-parser's unstructured parser,
      runs of white space made one), then the body source (lossy UTF-8),
      each cut at 256 KB with "[N more bytes]"; C0/C1 controls, DEL and
      bidi/zero-width characters show as `<U+XXXX>`, tabs as spaces. A
      read-only overlay in the Settings style (selectable Consolas text,
      Message-ID line, Copy Message-ID, Copy headers, Copy all, Esc
      closes; the list keys are off while it shows). Copy Message-ID puts
      `<id@host>` on the clipboard. Forward as attachment starts a Forward
      draft "Fwd: <subject>" with nothing quoted and one attachment
      `<subject>.eml` holding the raw bytes; `smtp::content_type` maps
      `.eml` to message/rfc822 (so the type survives draft save and
      reload, and an attached `.eml` file goes out the same way), sent as
      it is with 7bit (ASCII) or 8bit when it has no NUL, only CRLF line
      ends and lines of at most 998 octets, otherwise base64 (RFC 2046
      5.2.1 allows only 7bit/8bit/binary, but base64 is the only way to
      send such bytes intact; lettre's own chooser uses a 76-octet line
      limit, so the check is Komorebi's). The attachment counter and send
      size limit apply as to any attachment. Tests:
      `src/mail/source.rs` (unfold/decode/escape, odd and noisy input,
      the 256 KB cut, names), `src/mail/smtp.rs` (the attachment parses
      back with mail-parser as message/rfc822 with its name, the expected
      transfer encoding for ASCII, 8-bit, a 998-octet line, a 999-octet
      line, a bare LF and a NUL, and the nested message byte for byte),
      `src/mail/mod.rs` (a body fetched on demand from the fake IMAP
      server is stored byte for byte and its original view decodes),
      `tests/mime_corpus.rs` (source view and forward round trip over the
      14 corpus files), and two headless walks in `src/app/composer.rs`
      (exported bytes equal the cached raw, Ctrl+U, Esc, Copy Message-ID,
      Forward as attachment; and the wait for a missing body: offline
      notice, failure, another message's body ignored, the draft once the
      body arrives). Snapshot QA: `KOMOREBI_SNAPSHOT_ACTION=source` (new)
      with `KOMOREBI_SNAPSHOT_SELECT=0` at 1280×800 and 900×700, light and
      dark, plus a synthetic 2 MB message (folded and encoded headers, an
      escape and a bidi override in the subject, a base64 PDF; source cut
      at 256 KB). Release binary 14,254,592 bytes (+105,472 against Stage
      17's 14,149,120), 13.59 MiB, 14.25 MB. Launch on the seeded
      2000-message cache: window 4.3 ms, cache loaded 6.4 ms (medians of
      15; Stage 17 recorded 4.2 / 6.6 ms). Not checked: the save dialog
      and the menus by hand (see Next), and how real servers and clients
      take a forwarded message/rfc822 sent as 8bit or base64.
- [x] Product round, Stage 19 (2026-10-09): Settings structure, theme,
      title count and account housekeeping. Cache schema v11 (migrated in
      place) adds `accounts.enabled` (default on). Settings has four
      sections under tabs (Accounts, General, Privacy, About; one scroll
      view for all of them), opens with Ctrl+, and closes with Esc (from
      the account form, Esc goes back to the list first; the list keys
      work again after either). Accounts: each account shows an Enabled
      checkbox, Up/Down (`Db::move_account` numbers the accounts 1..n in
      their order, then swaps, so the equal positions of older caches still
      move), Edit and Remove; Add account and Edit open the form as a page
      of its own with Back. A switched-off account keeps its cache, shows
      greyed in the navigation and Settings ("Off: no connections; its
      downloaded mail stays"), and gets no workers (`App::start_workers`
      spawns only for enabled accounts, at start and when switched on, which
      also syncs it in full and drains the Outbox; switching off sends a
      held move first, then drops the workers); mail sent from it waits in
      the Outbox. With more than one account, each has a colour dot (six
      mid-tone colours by account order) on its rows in All inboxes, its
      navigation header and its Settings row. General (`settings` keys,
      read once at start): `theme` (`system`/`light`/`dark`; Light and Dark
      are set on Fluent's Palette before the window shows; System leaves
      the style tracking Windows at start, and once a choice was made it
      follows winit's `ThemeChanged`, since any set replaces the binding;
      an open HTML message is laid out again through the existing
      `changed dark`), `check_interval_secs` (60/120/300/900; the timer
      restarts on change, and the app asks for a full pass at least
      hourly, as the workers' every-60th-pass rule would stretch with the
      interval), `undo_send_secs` (0/5/10/20/30), `mark_read_delay_ms`
      (right away, 1 s, 3 s, never) and `flash_taskbar` (off by default:
      `FlashWindowEx` with `FLASHW_TRAY | FLASHW_TIMERNOFG` when the
      inboxes' unread total rises while the window doesn't have the
      focus). A value saved by hand between the offered ones shows as the
      nearest. Privacy: the remote-image senders (moved here) and Download
      again per account, after a prompt that says mail on the server is
      not affected: one transaction deletes the account's messages (their
      bodies go with them; a draft replying to one keeps its threading
      headers but loses the link) and address index entries and clears the
      folder fingerprints; the folders stay, so drafts' server copies stay
      tracked, and the account, its password, drafts, Outbox, invitation
      answers and the settings are kept. Its workers are replaced first
      (none while it is off or offline: it then downloads at its next
      sync). About: name and version (`CARGO_PKG_VERSION`), Apache-2.0,
      the repository link (`CARGO_PKG_REPOSITORY`, now set in Cargo.toml;
      opened like a confirmed web link), the data folder as selectable
      text, and the Slint badge. Title: "Inbox (7) - Komorebi" from the
      inboxes' unread total ("Komorebi" at zero). The single-instance
      check found the running window by its exact title "Komorebi", which
      the count breaks, so it now walks the top-level windows of winit's
      class for a title `is_app_title` accepts. Window place: winit's
      Moved/Resized events record the outer position and inner size while
      the window is neither maximized nor minimized; closing saves them
      with the maximized state (`window`: "x y w h max", physical pixels);
      the next start puts the window back before it shows, kept on the
      monitor holding enough of its title bar (shrunk to fit and pulled
      fully onto it) or centred on the primary monitor when that monitor
      is gone (`fit_to_monitors` over `EnumDisplayMonitors` work areas;
      windows-sys `Win32_Graphics_Gdi`). Snapshots ignore the saved
      place. Tests: `migrates_v10_to_v11`, account order and Enabled,
      Download again keeps the account, folders, drafts with their server
      copy, answers and settings while messages, bodies and addresses go
      (`src/db.rs`), the new settings keys and the geometry value,
      `fit_to_monitors` (kept, monitor removed, too large, partly off,
      title bar above the screen, a sliver, maximized kept, no monitor)
      and the title strings (`src/main.rs`), the nearest-choice mapping and
      dot colours (`src/app/settings.rs`), and headless walks in
      `src/app/composer.rs`: a switched-off account spawns no workers
      (switched on it does, switched off they go), Download again (prompt
      text, Cancel, then the list empties and the account, folders, draft
      and settings stay), the title count following read state, the
      General choices applied, saved and shown again, and Ctrl+, / Esc.
      Checked live with the release build, offline on a throwaway copy:
      the title, a second start exiting and leaving one window, the place
      saved on close and restored, a place on a missing monitor centred on
      the primary one, and maximized restored with its normal place kept.
      Snapshot QA: `KOMOREBI_SNAPSHOT_ACTION=settings`, `settings:general`,
      `settings:privacy`, `settings:about` (new) and `settings-edit` at
      1280x800 and 900x700, light and dark; the saved theme as System,
      Light and Dark without `KOMOREBI_THEME`; a two-account copy (dots in
      All inboxes, the navigation and Settings) and one with the second
      account switched off. Release binary 14,474,240 bytes (+219,648
      against Stage 18's 14,254,592), 13.80 MiB, 14.47 MB (526 KB under
      the ceiling; most of it is the generated Slint code of the larger
      dialog). Launch on the seeded 2000-message cache: window 4.2 ms,
      cache loaded 6.6 ms (medians of 16; a few runs had 15-135 ms cache
      loads from machine load; Stage 18 recorded 4.3 / 6.4 ms). User data:
      the cache stays plaintext; this adds each account's Enabled flag and
      the settings values above, including the window's position and size.
      Not checked: the taskbar flash and the System theme following a
      Windows theme switch by hand, and Ctrl+, on non-US keyboard layouts.
- [x] Product round, Stage 20 (2026-10-09): connection resilience, error
      messages and the IMAP fence. No schema change. Errors
      (`src/mail/errors.rs`, where the error predicates moved): a worker
      error becomes a `Problem` (class and full text) classed Offline,
      Auth, TlsCert, TooLarge, Quota, TooManyConnections or Other, each
      class one sentence with a next step (Other shows its own text);
      `Event::Syncing` and `ActionFailed` carry it, and the app shows a
      class once per account until a sync goes through or Refresh (Other
      by its text). Settings keeps "Can't reach <host>" for Offline and
      the sentence otherwise. Latch: a code-less NO to LOGIN whose text
      says too many connections, temporary or try again (Dovecot's
      "Maximum number of connections from user+IP exceeded") no longer
      latches the sign-in (fake-server test first showed it did);
      `[UNAVAILABLE]`, `[LIMIT]` and `[INUSE]` already didn't, and an SMTP
      454 on AUTH is a retry. Gone messages: a NO with `[EXPUNGEISSUED]`,
      "no longer exist", "no such message", `[NONEXISTENT]` or a folder no
      longer cached counts as done for flags, mark all read and delete
      (the folder resyncs quietly); a move says "Already gone from the
      server, so there was nothing to move." Waiting actions: a flag,
      mark-all-read, move or delete that failed only because the server
      couldn't be reached (Offline or TooManyConnections) is parked on the
      interactive worker with its messages kept pending, retried once
      after the next job that reached the server or a 30 s connect probe,
      and only then reported and undone (fingerprint cleared, folder
      resynced); after 5 minutes without a connection it is undone; in
      memory only. Resume: a 60 s timer compares wall-clock ticks; a gap
      over 2 minutes makes every worker drop its connection before its
      next command (no NOOP probe, no LOGOUT), cuts the IDLE socket and
      reconnects IDLE at once with its backoff reset (`IdleHandle::kick`),
      then syncs all accounts 3 s later. First sync: a folder sync
      storing at least 1,000 new headers sends `Event::Progress` per batch
      ("<account>: downloading 12,400 of 103,000 in Inbox" on the status
      line while it shows nothing else, and "Downloading … of …" in
      Settings). Folder gone: when the folder list drops the folder shown
      or the open message's folder, the reader closes and the view moves
      to that account's inbox. IMAP fence: only `src/mail/imap.rs` names
      the crate (a test walks src/, examples/ and tests/); every command
      goes through `Imap::run` (counted, timed, logged); callers see
      `imap::Failure`/`failure()`, `imap::WaitOutcome` (ours) and
      `imap::detail()` (probe). The stream follows the lines both ways,
      passing over literals (a client literal only after the server's
      `+`), and refuses a tagged reply whose tag is not the last
      command's (the crate's `assert_eq!` aborts on it, e.g. Dovecot's
      `DONE BAD`) and anything but `+` or a tagged NO/BAD as the first
      reply to IDLE (a tagged OK reached `unreachable!()`): the read
      fails and the connection is dropped; the IDLE thread quits on it.
      `src/mail/idle.rs` holds the IDLE thread (moved, plus `kick`).
      Fake-server cases: EXPUNGE during FETCH (set aside), unsolicited
      `* OK [HIGHESTMODSEQ]`/`[ALERT]` during FETCH, a truncated literal
      (a lost connection), ESEARCH and VANISHED (no panic), message lines
      inside a literal that look like tagged replies, mistagged replies,
      the tagged OK to IDLE. Diagnostics log (`src/diag.rs`, Settings →
      Privacy, key `diagnostics_log`, off by default):
      `logs\komorebi.log` beside the cache, rotated at 1 MB into `.1` and
      `.2`, lines `<UTC> account <id> <VERB> <status or [CODE]> <n> ms` for
      every IMAP command, CONNECT (with its class) and SMTP SEND; a test
      runs a fake IMAP and SMTP session with a sentinel password, subject
      and server text and finds none of them in the log. Damaged cache:
      `Db::open_ui` reads the schema page; on SQLITE_CORRUPT or NOTADB it
      renames the file (with `-wal`, `-shm` and the drafts folder) to
      `cache.damaged-<YYYYMMDD-HHMMSS>.sqlite3` and creates a new cache;
      the status line says so, names the file and asks to add the accounts
      again. A cache with a newer schema is refused with "This cache was
      made by a newer Komorebi …" and left untouched. Tests: errors
      classes and sentences; the latch cases (four refusal texts); gone
      flags and move; a parked flag change retried on reconnect, a parked
      move whose retry fails (reported once, undone), a parked delete
      given up; a wake replacing the connection without a probe; the IDLE
      kick reconnecting inside the backoff; progress events; the damaged
      and newer caches; the log redaction and rotation; the fence cases
      above; `woke()`; `thousands()`; a headless walk
      (`src/app/composer.rs`) for the show-once rule and the gone-folder
      fallback. Snapshot QA:
      `KOMOREBI_SNAPSHOT_ACTION=offline` and `offline-settings` (new: a
      made-up Offline problem on the first account) and `settings:privacy`
      (the log switch) at 1280x800 and 900x700, light and dark; a garbage
      `KOMOREBI_DB` (renamed aside, the notice on the welcome screen).
      Release binary 14,521,856 bytes (+47,616 against Stage 19's
      14,474,240), 13.85 MiB, 14.52 MB (478 KB under the ceiling). Launch
      on the seeded 2000-message cache: window 4.0 ms, cache loaded 6.4 ms
      (medians of 11; a few runs had 22-90 ms cache loads from machine
      load; Stage 19 recorded 4.2 / 6.6 ms). User data: the cache stays
      plaintext; this adds the `diagnostics_log` setting, and the opt-in
      log beside the cache (metadata only). A damaged cache is kept beside
      the new one until deleted by hand; its accounts are not carried over
      (whether to keep a minimal account mirror is still a user decision,
      RL-R6 in the plan's deferred items). Not done: the optional
      `--record` transcript mode of `examples/probe.rs` (it can only be
      exercised against a real account). Not checked: a real sleep/wake,
      a real connection limit, and the newer-cache dialog on screen.
- [x] Product round, Stage 21 (2026-10-09): keyboard focus and
      accessibility pass. No schema change. Focus (`ui/widgets.slint`):
      `FocusRing` (2 px, text colour, drawn over the control), `PressKeys`
      (a Tab stop that Space or Enter presses, never taking the focus on
      a click) and `FocusTrap`; IconButton (now a Tab stop by default),
      AccentButton, Chip (opening an attachment), a new `TextButton`
      (Unsubscribe, Update password, the repository link, View as plain
      text), FilterToggle, SettingsTab and the Move list's rows are Tab
      stops with a ring. The root key scope is no longer a Tab stop: the
      list's stop is `list-keys` around the ListView (its keys are still
      the root bindings), the folders are one stop with Up/Down, Home/End,
      Enter/Space and Left/Right for account headers (the focus stays
      there after opening a folder; the narrow rail's account lines are
      passed over), and the reader toolbar is one stop with Left/Right
      (roving: Tab returns to the button last reached; Move and the
      plain-text toggle are skipped while off), as is the composer's
      formatting row (so Subject to body is still two Tabs, not seven). The list's and folders'
      current row shows a ring only after a key moved the focus (global
      `Focus.keyboard`, cleared by a click on a row or the page); with no
      current row the ring goes round the list. F6 / Shift+F6 cycle
      folders, list and reader (the toolbar, the composer's To, the
      selection's Archive or the Outbox's first button). Shift+F10 or the
      Menu key opens the row menu under the current row (left for this
      stage by Stage 13). Dialogs: Settings
      start on the selected tab (the form on Name, or Password when
      editing), the link prompt on Open (Cancel when it has warnings, so
      Enter keeps its old meaning), the yes/no prompt on Cancel (Enter no
      longer could confirm a resend or removal), Discard on Discard (as
      before), Show original on Close; zero-size traps at both ends of
      each card send Tab and Shift+Tab round inside it (Settings: to the
      section's first/last control). A closed yes/no or discard prompt
      gives the focus back (to Settings under it, the composer's body, or
      the list; before, the list keys were dead until a click). Toast: its text is a live region
      (assertive for errors, polite otherwise; Slint 1.18 has no alert
      role) labelled "Error: …" for errors; Esc on its buttons dismisses
      it, and Esc in the list does once there is no search, open message
      or selection, but never while it offers Undo or Resume draft; errors
      still stay 12 s. Roles and labels: the list ("Messages", with the
      current row as the delegated focus, approximated among the rows
      built from the scroll offset), the folders ("Folders"), the search
      box ("Search mail"); message rows add Flagged, Has attachment and
      Replied; Unread, Flag and View as plain text are checkable; account
      headers are expandable; the reader header labels Subject, From,
      Sender address and Date, and the avatar is hidden. Contrast
      (`ui/theme.slint`): `accent-text` is `readable(readable(accent,
      row-selected), nav-selected)`, a pure callback answered by
      `settings::readable` (mix towards black or white until 4.5:1);
      unread subjects use the text colour; selected rows get a 1 px
      `row-border`; a disabled accent button has a neutral fill and a
      legible label; `row-height` replaces the 76 px literals. Glyphs:
      every icon code point (41) exists in Segoe MDL2 Assets 1.86 and
      Segoe Fluent Icons 1.54 as installed here, so none was substituted;
      the table lives in a test. Tests: `accent_text_reads_on_every_surface`
      (15 accents incl. yellow, lime, cyan, pink, light grey, over the
      theme's own surface colours, read from `theme.slint`, light and
      dark), `icons_are_mdl2_glyphs` (every `\u{E…}` in the UI, role.rs and
      app.rs is in the table; checked against the installed fonts when
      present), and headless walks (`src/app/composer.rs`): Tab from the
      list reaches Reply, Right/Left move and skip Move, the toolbar is
      one stop and Shift+Tab returns to it; F6 into the toolbar and
      composer, the folders' arrows and Enter keeping the focus; the
      prompt's Enter cancelling and Tab staying in it; Settings starting
      on its tab, Shift+Tab wrapping to the section's last control and
      Esc still closing after 30 Tabs (checked to fail without the trap);
      the focus coming back after the prompts; the formatting row as one
      stop; Esc and the status line; Shift+F10 setting the menu labels. Snapshot QA: `KOMOREBI_SNAPSHOT_FOCUS`
      (new: `toolbar[:n]`, `nav[:n]`, `list`, `tab:n`), `toolbar:3` (the
      focused Archive button) at 1280x800 and 900x700, light and dark,
      plus `nav:2` (wide and the narrow rail), `list` (dark) and
      `settings:general` (the tab's ring), `compose-format` (the row
      unchanged, the disabled Send's new fill). Release binary 14,659,072
      bytes (+136,704 against the 14,522,368 built from the tree before
      this stage), 13.98 MiB, 14.66 MB (341 KB under the ceiling). Launch
      on the seeded 2000-message cache, the two builds interleaved in
      rounds of 11 (medians): before window 4.5-4.6 ms, cache loaded
      7.9-8.2 ms; after 4.1-4.7 ms and 7.9-8.3 ms (noise; rounds under
      machine load reached ~30 ms medians for both builds alike). User data: none (no
      setting, no schema change). Not checked: a screen reader (see the
      Narrator item under Next).
- [x] Product round, Stage 22 (2026-10-09): reader engine robustness and
      comfort. No schema change. Text cap (`src/mail/render/layout.rs`):
      an inline group (in effect a paragraph) lays out at most 64 KB
      (`MAX_GROUP_TEXT`), cut at its last white space (or inside a longer
      word), then "[Message too long: View as plain text shows the rest]";
      the page then counts as shortened, so the existing "[Message
      shortened for display] View as plain text" line shows under it.
      Grapheme clusters keep at most 32 characters (`tame_clusters`, with
      `unicode-segmentation`, already linked through Slint and now pinned
      as a direct dependency: Cargo.lock gains that one edge, no new
      crate): parley 0.11 counts a cluster's characters in a `u8`, which a
      stack of 100k combining marks overflowed (a panic in debug builds,
      silent wrap-around in release). Worker (`src/app/reader.rs`): the
      generation counter and its stale-drop test were already there
      (`worker_drops_stale_generations`); new: a layout for another message
      frees the previous one's pages, tiles and fetched images (which also
      stops its fetches; before, only a consented layout reset the image
      cache and up to three pages stayed), and closing the reader frees
      everything (`Job::Cancel` now reaches a running worker; it never
      starts one). Glyph cache (`src/mail/render/paint.rs`): least recently
      used within 4 MB (trimmed to 3 MB before a band; outlines are counted
      too), instead of being dropped whole at 16 MB. Emoji: parley's
      fallback already picked Segoe UI Emoji (checked for U+1F3E0 and
      U+1F4E6 from Segoe UI, Arial and Georgia text); they painted as the
      font's monochrome outlines. Colour glyphs are now painted from their
      COLRv0 layers in CPAL colours (Segoe UI Emoji has v0 records for its
      flat designs; a COLRv1-only glyph falls back to each gradient's first
      stop; palette entry 0xFFFF is the text colour), into small cached
      pixmaps under the same budget, only for runs whose font has a COLR
      table. Dark dimming (`src/mail/render/style.rs`): in the dark theme,
      mail without dark rules of its own has its resolved colours remapped
      in Oklab: backgrounds with lightness over 0.6 (`bgcolor` included,
      and the default white canvas) become `1.22 - L` (white: the theme's
      surface grey), text under 0.5 becomes `0.92 - L/2` (black: a soft
      white), mid tones (a brand button) stay; borders follow whichever
      side they are on (the pass checks the layout deadline; the mutation
      fuzz now dims half its mutants). Text is then moved in lightness until it reads over
      its dimmed background at least as well as over the original, up to
      4.5:1; text all but invisible on its background (contrast under 1.5,
      a hidden preheader) is dimmed like the background, and text over a
      background image keeps its colour. Images are untouched, so a
      transparent logo drawn in black can vanish on the dark page (01's
      "Lumen" wordmark): "Show original colours" (`b-colors` in the reader
      toolbar, after View as plain text, shown in the dark theme when the
      page is dimmed; per message, like plain text) lays it out again
      without dimming. Pages are keyed on the mode (`Key::dim`). The
      toolbar's labels now depend on its own width (626 px, 660 px with the
      colour toggle) rather than only on the window's compact width, so the
      extra button never squeezes them (at 1280 px the dark theme shows
      icons only on dimmed mail; a 1100 px window, whose labels used to
      overlap, now shows icons). Failed images: after Load, the images bar
      comes back as "N images couldn't be loaded" once no fetch is pending
      (failed, refused, over the size or count caps). Timing: the first
      frame is printed under `KOMOREBI_TIMING` ("first frame drawn": the
      first `RedrawRequested` through the winit event hook, plus a zero
      timer that runs after Slint has painted it; the software renderer
      has no rendering notifier). Fonts warm on the render worker 1 s after
      start as before, now below normal priority and with an emoji so the
      fallback font loads too. Tests: `pathological_text_is_capped_and_quick`
      (a 2 MB paragraph, a 500 KB word, 100k combining marks and a 2,000
      conjunct Devanagari chain: 14-23 ms each in release, under 300 ms;
      80 ms in debug, under 3 s), `dimming_keeps_body_text_legible` (8
      text colours over 7 light backgrounds: every pair at 4.5:1 or more
      stays there, none gets worse; buttons, hidden text and dividers),
      `oklab_round_trips`, `render_comfort.rs` (`emoji_fall_back_to_segoe_ui_emoji_and_paint_in_colour`:
      three hues or more in the 48 px emoji row, none for black text;
      `glyph_cache_stays_within_its_budget`;
      `dark_theme_dims_light_mail_but_not_mail_with_dark_rules`: every
      glyph run of the dimmed digest at 4.5:1 or more, the same height, 08
      left alone), and in `reader.rs` `worker_counts_images_that_could_not_be_loaded`
      (fake fetcher), `worker_frees_the_previous_message_and_everything_on_cancel`,
      `worker_forwards_cancel_only_to_a_running_worker` and
      `failed_images_and_original_colours_reach_the_reader`. Fixtures:
      `tests/fixtures/reader/` (an emoji message and three heavy ones; a
      sibling of `tests/fixtures/html/`, so the 14-file corpus and its
      recorded heights stay as they are). Snapshot QA (`seed -- <new> 0
      tests/fixtures/reader` and `tests/fixtures/html`, new
      `KOMOREBI_SNAPSHOT_ACTION=original`): the emoji message at 1280x800
      and 900x700, light and dark (colour emoji, dimmed page); 01, 04 in
      the dark theme with dimming on and off, 05, 06, 07, 11 dimmed, 08
      (dark rules: not dimmed, no toggle). Open time on the 18-message
      heavy corpus (the 14 corpus files plus the 4 above, `KOMOREBI_SNAPSHOT_WALK=17`,
      5 runs, 1280x800, open to first tile): light median 18 ms, p95 111
      ms, max 121 ms; dark (dimmed) median 18 ms, p95 126 ms, max 133 ms.
      Peak working set / private (`examples/peak_memory`, the heavy
      corpus, against a build of this tree without the freeing): no
      message 35.7 / 16.5 MiB both; one message 38.9 / 18.2 MiB both;
      walking all 18: before 108.7 / 92.4 MiB, after 83.9 / 67.7 MiB.
      Release binary 14,721,536 bytes (+62,464 against Stage 21's
      14,659,072), 14.04 MiB, 14.72 MB (278 KB under the ceiling). Launch
      on the seeded 2000-message cache, interleaved with that build in
      rounds of 9 (medians): window 3.9-4.7 ms, cache loaded 15.6-16.1 ms
      for both (15.7-15.8 ms before), first frame drawn 41.6-46.4 ms
      (unmeasured before). The cache-loaded figure is about twice Stage
      21's on this run's machine load for both builds alike. User data:
      none (no setting, no schema change; "Show original colours" is not
      remembered).
- [x] Product round, Stage 23 (2026-10-09): release foundations and CI
      gates. `LICENSE` (Apache-2.0, verbatim), `NOTICE` (copyright "The
      Komorebi Authors", Slint's royalty-free attribution, SQLite, how to
      list every crate's license), `CHANGELOG.md` (Keep a Changelog, 0.2.0
      with the cache's schema history as its local-data notes). `build.rs`
      embeds `assets/komorebi.rc` through `embed-resource` (build
      dependency only): the icon `assets/komorebi.ico` (a blue tile, a sun
      rising behind a white envelope; PNG entries 16-256 px, 11 KB), a
      VERSIONINFO from the Cargo version, and a manifest with the UTF-8
      code page, the Windows 10/11 supportedOS, asInvoker and Common
      Controls 6 (themed message boxes). No dpiAwareness entry: winit
      already sets PerMonitorV2 at run time before the window opens. The
      window gets the resource's icon (16 px title bar, 32 px taskbar, at
      its DPI) on its first event; Slint leaves it alone while
      `Window.icon` is unset. The title bar follows the theme shown
      (`DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)` in
      `apply_theme`, again on the first event, on a Windows theme change
      and on focus, because winit resets it to the Windows theme on every
      system setting change). CI: clippy `-D warnings` (the imap-proto
      notice is a cargo report, not a lint), `scripts/check-pins.ps1`
      (fails if an exact-pinned crate is linked in two versions; windows-sys
      is exempt, pinned for its feature list while other crates link 0.52
      and 0.59; prints the TextInput internals to check on a Slint bump),
      a release build with a size step (fails above 15,000,000 bytes), an
      offline boot snapshot on a freshly seeded cache
      (`KOMOREBI_SNAPSHOT_CHECK=1` makes the app exit 1 when the list or the
      reader is blank: under 1% of their pixels unlike the background) with
      the BMP uploaded, and a `deny` job (`cargo deny --locked check licenses
      advisories`, `deny.toml`: Windows graph only, permissive licenses,
      MPL-2.0 and Slint's royalty-free license). Locally: `cargo deny
      --offline` passes licenses, bans, sources and advisories against the
      advisory database cached on 2026-09-26. Tests: `ink_counts_pixels_unlike_the_background`
      and `blank_panes_names_the_empty_ones` in `main.rs`. `cargo bloat
      --release --crates` (unstripped): usvg 261 KiB, resvg 78 KiB,
      roxmltree 29 KiB and svgtypes 23 KiB of `.text` are linked, and
      `image` (its decoders feature); both come from
      `i-slint-backend-winit` 1.18.1, which depends on `i-slint-core` with
      `svg` and `image-decoders` unconditionally on non-wasm targets, so
      no Slint feature removes them (a follow-up would need a patched
      Slint). Snapshot QA: the boot check passes at 1280x800 and 900x700,
      light and dark, with a message open, and fails ("blank reader")
      with none open; a live offline run reports both window icons set
      and the title bar dark under the System theme (Windows dark here)
      and with Dark, light with Light; the .exe has the manifest, icon
      group and version resources (FileVersion 0.2.0). The title bar icon
      and colours were not looked at on screen. Release binary 14,740,992
      bytes (+19,456 against Stage 22's 14,721,536), 14.06 MiB, 14.74 MB
      (259 KB under the ceiling). Launch on the seeded 2000-message cache
      (10 runs, medians): window 4.1 ms, cache loaded 6.1 ms, first frame
      drawn 32.6 ms (one run at 151 ms from machine load). User data: none
      (no setting, no schema change).
- [x] Product round, Stage 24 (2026-10-09): installer, signing hook and
      data removal. `installer/komorebi.iss` (Inno Setup 6, per-user:
      `PrivilegesRequired=lowest`, `%LOCALAPPDATA%\Programs\Komorebi`, Start
      menu shortcut, uninstaller, `CloseApplications`, x64, Windows 10+,
      `/SILENT`; the uninstaller asks with No as the default whether to
      delete `%LOCALAPPDATA%\komorebi`, a silent uninstall keeps it, and it
      never touches Credential Manager). `.github/workflows/release.yml` on
      `v*.*.*` tags and workflow_dispatch: the tag must equal the Cargo
      version, `cargo test`, the release build and its size check, the
      offline boot snapshot, an optional signing step (`scripts/sign.ps1`,
      signtool with an RFC 3161 timestamp, only when the `SIGN_CERT_PFX`
      secret exists) for the exe and again for the setup, ISCC (installed
      with choco if the image lacks it), the portable zip (exe, LICENSE,
      NOTICE, PRIVACY, README, CHANGELOG), the `.pdb` as its own asset,
      `SHA256SUMS`, the packages kept as a workflow artifact, and on a tag
      a draft GitHub Release whose notes are the version's CHANGELOG
      section (plus a SmartScreen note while unsigned). Dispatch is the dry
      run: everything but the release. `dist/` is ignored. Settings >
      Privacy > Remove all data (`src/app/wipe.rs`): a prompt (Cancel has
      the focus; it counts unsent drafts and Outbox messages), then every
      account's password through `App::forget_password` (the credential
      store, a no-op under `KOMOREBI_OFFLINE`, a fake in tests; the first
      failure stops it, with no file removed; Remove account uses the
      same seam now), then undo-window sends are cancelled, a held move is
      dropped (never sent), the workers stop, the diagnostics log stops
      and the event loop quits. After `ui.run()`, main drops the app
      state and the window (the list's repeater keeps the old Rows model,
      and so the cache open, until it draws again) and removes the cache
      with `-wal`/`-shm`/`-journal`, `cache.drafts`, `cache.open`,
      `crash.log`, `crash.seen`, `logs`, every `cache.damaged-*`, then the
      folder if it is named `komorebi` and empty; files still in use are
      retried for 10 s, and what is left is named in a message box. The
      close-time actions (held moves, draft saves, PRAGMA optimize) are
      skipped. Snapshot action `remove-data`. Tests: `wipe::tests` (every
      data file and the emptied folder go; other files and a non-app
      folder stay; a file held open without FILE_SHARE_DELETE is retried
      then reported; passwords stop at the first failure) and
      `remove_all_data_forgets_every_password_then_closes_the_cache`
      (Cancel does nothing; an unavailable fake store removes nothing;
      then every account's password, and after `close_for_removal` the
      cache files delete at once). The test harness now uses a no-op
      credential store, so no test reaches Credential Manager. A live
      offline run with a temporary snapshot action that confirmed the
      prompt (removed again before the final build) emptied a scratch
      `komorebi` folder holding a seeded cache, drafts, opened files,
      crash notes, logs and a damaged cache in under a second, leaving a
      sibling file. The installer and the workflow were not run here
      (no Inno Setup on this machine, no push): the pwsh steps and
      `scripts/sign.ps1` parse, the notes extraction and the version
      lookup were run locally; the dry run waits for workflow_dispatch.
      Snapshot QA: the Privacy section with the prompt at 1280x800 and
      900x700, light and dark. Release binary 14,762,496 bytes (+21,504
      against Stage 23's 14,740,992), 14.08 MiB, 14.76 MB (237.5 KB under
      the ceiling); `komorebi.pdb` is still written next to it with
      `strip = true`. Launch on the seeded 2000-message cache (10 runs,
      medians): window 4.2 ms, cache loaded 6.2 ms, first frame drawn
      31.5 ms. User data: no schema change; Remove all data deletes
      everything in the data inventory, the uninstaller only on Yes.
## Baseline for the product round (2026-10-08)

Release builds before and after Stage 1, same machine, offline, against a
cache seeded with `seed -- <new> 2000` (the DB every later stage measures on):

| | Before Stage 1 | After Stage 1 |
|---|---|---|
| `target\release\komorebi.exe` | 14,368,256 bytes (13.70 MiB) | 14,384,640 bytes (13.72 MiB) |
| Window created (median of 7) | 3.5 ms | 3.5 ms |
| Cache loaded / list ready (median of 7) | 5.6 ms | 5.4 ms |

Stage 1 adds 16 KiB, Stage 2 26.5 KiB, Stage 3 65 KiB, Stage 4 89.5 KiB, Stage 5 131 KiB, Stage 6 75 KiB, Stage 7 24.5 KiB, Stage 8 105.5 KiB, Stage 9 57.5 KiB, Stage 10 25 KiB, Stage 11 18 KiB (see their entries; Stage 11 crossed the ceiling by 17,984 bytes and the bundled SQLite's unused modules were then dropped, leaving 14,715,904 bytes), Stage 12 90.5 KiB (14,808,576 bytes), Stage 13 37.5 KiB (14,846,976 bytes), Stage 14 61 KiB (14,909,440 bytes), Stage 15 42.5 KiB (14,952,960 bytes), Stage 16 35 KiB (14,988,800 bytes), Stage 17 124.5 KiB (then the `keyring` wrapper and `regex` were dropped, leaving 14,149,120 bytes), Stage 18 103 KiB (14,254,592 bytes), Stage 19 214.5 KiB (14,474,240 bytes), Stage 20 46.5 KiB (14,521,856 bytes), Stage 21 133.5 KiB (14,659,072 bytes), Stage 22 61 KiB (14,721,536 bytes), Stage 23 19 KiB (14,740,992 bytes), Stage 24 21 KiB (14,762,496 bytes). Ceiling for the round: 15.0 MB. Launch must stay
under 200 ms and the cached-list load under 600 ms.

## Verification after the product round (2026-10-09)

Independent re-run of the gates on the finished tree, same machine:

- `cargo fmt --check`, `cargo clippy --all-targets --locked` (only the
  imap-proto future-incompat notice) and `cargo test --locked` pass:
  465 tests (lib 267, bin 139, integration 59).
- Release exe 14,763,008 bytes (14.76 MB, ceiling 15.0 MB).
- Launch on a new 2000-message seed, 7 runs: window created 3.7-4.8 ms,
  cache loaded 5.6-6.9 ms (one run 64.7 ms), first frame drawn 30-34 ms
  (that run 90.5 ms).
- A copy of the authorized Postale cache (290 bodies) migrates cleanly;
  `render_cache`: 282 rendered, 8 text-only, 0 panics, release layout and
  paint p50 5.6 ms, p95 11.7 ms, max 32.6 ms; `parse_cache`: 0 panics,
  p50 0.38 ms.
- Live `KOMOREBI_FLOW` against the authorized Postale account, with
  `KOMOREBI_NO_SERVER_DRAFTS=1`: PASS in 37 s. Flag, unread, read and
  unflag on a `[komorebi-test]` message; a reply and a formatted message
  with an attachment sent to the account itself through the outbox and
  undo window; the draft was saved locally and gone once sent; the
  message moved to Junk and back; both arrived, and the formatted one's
  HTML part kept bold, italic and the list.
- From comparing real mail with eM Client (outside the stages): consented
  `http://` image URLs are requested over HTTPS, `text-transform:
  capitalize`/`lowercase` work, and inline margins and padding are kept.

## Next

- [ ] Validate against a real Purelymail account (Postale.io is measured
      above). `examples/probe -- <env> <throwaway.sqlite3> --runs 3`;
      `examples/idle_check` tests IDLE wakes (flag toggles on its own
      `[komorebi-test]` message; `--mail` sends one to yourself).
- [x] Run the GUI against the real account end to end (open, flag, move,
      reply with `[komorebi-test]` messages only): `KOMOREBI_FLOW` passed
      on Postale on 2026-10-09 (see above).
- [ ] Postale only reports new mail to fresh SELECT/STATUS: consider
      dropping the third (IDLE) connection on servers where it never wakes
      for new mail, to save a connection.
- [ ] Smoke-test the static-CRT release build (TLS sign-in and Credential
      Manager) on an authorized account before any release.
- [ ] A cold-start measurement on a clean machine (the first-frame line
      under `KOMOREBI_TIMING` exists since Stage 22).
- [ ] Dimmed mail in the dark theme leaves images alone, so a transparent
      dark logo can vanish ("Show original colours" is the remedy). If real
      mail shows this often, consider a light plate behind transparent
      images on dimmed pages.
- [ ] On every Slint upgrade, check the TextInput's internal
      `anchor-position-byte-offset` and `cursor-position-byte-offset` that
      `ComposeBody` reads (the formatting test must pass; add this to the
      Slint pin-consistency check when it exists, Stage 23).
- [ ] Live check of server draft copies (Stage 11), only with the user's
      authorization for that account, self-addressed drafts only: write a
      draft to yourself with a `[komorebi-test]` subject, wait 30 s, edit
      it, close the composer; the Drafts folder (in another client or the
      webmail) must hold exactly one copy, flagged as a draft, with the
      last text. Then send it and check the copy is gone and Sent has the
      message. Note whether the server has UIDPLUS (without it the old
      copies stay flagged as deleted until another client expunges). The
      only server mail this deletes is Komorebi's own draft copies (with
      `X-Komorebi-Draft-Id`); no real mail is deleted.
- [ ] Open a server draft (from another device) in the composer (C8,
      deferred until server saves are proven on real providers).
- [ ] Check by hand once (Stage 10): drag files from Explorer onto the
      composer (accent border while hovering, all files attached, ignored
      with the composer closed), copy files in Explorer and Ctrl+V in the
      composer, and Open on a PDF (opens in Protected View / with the
      downloaded-file warning) and on an `.exe` (refused).
- [ ] Check by hand once (Stage 13; the snapshot can't capture menus):
      the row menu opens at the pointer on a right-click (also on a row
      scrolled down the list, and at 900 px wide), the More menu under its
      button, Move to › lists the folders, and the labels follow the
      selection (Mark as read / unread, Not junk in Junk).
- [ ] Check by hand once (Stage 18): Save as .eml from the More menu and
      with Ctrl+S (the dialog offers `<subject> (<date>).eml` in Downloads;
      the saved file opens in another mail app), on a message not yet
      downloaded too ("Fetching message…", then the dialog), and Forward as
      attachment to yourself (the attachment opens as a message in the
      receiving client).
- [ ] Check by hand once (Stage 19): with "Flash the taskbar button" on
      and the window in the background, new mail flashes the taskbar
      button until the window comes to the front; with the theme on
      System, switching Windows between light and dark follows at once
      (also after choosing Light or Dark and then System again); Ctrl+,
      opens Settings on a non-US keyboard layout; the combo boxes' lists
      open inside the dialog at 900 px wide.
- [ ] Check by hand once (Stage 20): after a real sleep of a few
      minutes the next check finds new mail within seconds (connections
      replaced, IDLE back); unplugging the network shows "Can't reach the
      server" once, and a flag change made meanwhile reaches the server
      once it is back; the diagnostics log (Settings → Privacy) fills with
      metadata lines only.
- [ ] Check by hand once with Narrator (Stage 21; the snapshot can't
      hear): Tab and F6 through the window announce each control by name
      (the toolbar's toggles as checked or not, account headers as
      expanded or collapsed); arrowing the message list and the folders
      reads the current row (the list's delegated focus is computed from
      the scroll offset: check it after scrolling far down); a status
      line is read as it appears, an error at once; the Settings and
      prompt dialogs keep Tab inside them; the focus rings show at 125%
      and 150% scale; Shift+F10 and the Menu key open the row menu under
      the current row (also scrolled down the list) and arrows move in it.
- [ ] Check by hand once (Stage 14): the list's More (…) menu opens
      under its button at 1280 and 900 px wide; Enter, then PageDown or
      Space scrolls a long page rather than the list; Esc from the page
      closes the message and the arrows still move through the list.
- [x] Search uses SQLite `LIKE`, which ignores case only for ASCII ("josé"
      does not match "José"). Done in Stage 15 (`fold()` in Rust, no
      stored column). Recipient suggestions still match lower-cased only
      (the address index's `folded` column predates `src/fold.rs`;
      switching it needs a rebuild of the index).
- [ ] Native HTML rendering in the reader. Stage 1 is done: the engine
      (`src/mail/render/`, own layout on parley + tiny-skia, no new crates)
      lays out and rasterizes the synthetic corpus (`tests/render_corpus.rs`,
      mutation harness in `tests/render_fuzz.rs`, `examples/render_html.rs`
      for visual review), but nothing in the app calls it yet. Measured with
      a temporary call: +0.39 MiB release binary. Stage 2 is done:
      `mime::parse_render_input` hands it the first HTML alternative, every
      Content-ID part (inline images too) and the same text as `parse_body`
      (`tests/mime_corpus.rs`). Stage 3 is done: the reader renders HTML
      on a worker thread (`src/app/reader.rs`) into 256 px tiles at device
      pixels, re-rendering on resize (debounced) and scale changes, with
      the text view as fallback on errors, caps or a 1 s timeout. Measured
      in the release build: open to first tile 4-35 ms on the corpus;
      tiles blit 1:1 (pixel-identical to a direct render) at 100%, 125% and
      150%, also at fractional scroll offsets except exact half-pixel ones,
      where a tile edge can be off by one device row. Stage 4 is done: text
      selection on the page (drag, Shift+click, double-click word,
      triple-click paragraph, Ctrl+A) runs on the worker against the laid-out
      page (`src/mail/render/select.rs`) and comes back as highlight
      rectangles; Ctrl+C or the context menu copies the visible text
      (hidden preheaders and list bullets left out) through a hidden
      TextInput, checked on the real Windows clipboard; "View as plain
      text" in the toolbar (per message, not saved); the page exposes the
      message text to screen readers. Stage 5 is done: links are hit-tested
      on the UI thread from the page's link areas (hover cursor and target
      strip, Tab focus ring and Enter, `#anchor` scroll, PageUp/PageDown/
      Space/Home/End when the page has focus); `mailto:` prefills the
      composer (a draft in progress is kept); web links open through
      `ShellExecuteW` only after a confirm prompt with phishing warnings
      (`src/mail/render/links.rs`, URLs parsed by the `url` crate already in
      the tree; the registrable-domain check is a heuristic, not the public
      suffix list); screen readers get each link as a button. Release
      binary +0.10 MiB for the stage. Stage 6 is done: remote images are
      blocked behind a banner (count and IP warning) with Load once (this
      open, in memory) and Always load from the exact From address (schema
      v4 `remote_allow` table; Settings lists the senders with Remove and
      Forget all). The renderer reports exactly the images consent would
      load (`Doc::remote_images`: no tracking pixels, no remote CSS or
      fonts); the reader fetches them with `src/fetch.rs` (HTTPS GET over
      `std::net` + `native-tls`, no new crate: no cookies, Referer or
      credentials, public addresses only, 5 s connect / 15 s total, 3
      redirects, 10 MB per image) on a pool of 4 threads, keeps at most
      32 MB for one message in memory, and lays the page out once more
      when the batch is in (not per image). `KOMOREBI_OFFLINE` refuses
      every fetch. Tested against a fake HTTPS server on 127.0.0.1 with a
      throwaway self-signed certificate (`tests/fixtures/tls/`). Release
      binary +0.10 MiB for the stage (13.69 MiB). Fixed-width (600 px)
      mail was clipped in a narrower reader pane (scaled to fit since the
      product round's Stage 2). Stage 7 (QA in the release app, synthetic corpus only,
      `seed -- <new> 0 tests/fixtures/html`): all 14 messages render as
      pages (no text fallback) at 1280×860 and 760×860, light and dark,
      plus mid-page scrolls of 02/05 and 125%/150% scale
      (`SLINT_SCALE_FACTOR`, sharp). Open to first tile 3-28 ms (14:
      10-20 ms, no hang). Launch unchanged against the table above with
      6,220 cached messages: window 3.4-4.2 ms, cache loaded 13-15 ms.
      Peak working set / private (`examples/peak_memory`): 35.6 / 17.1 MiB
      with no message open, 44.3 / 24.4 MiB after opening 01, 72.9 / 52.5
      MiB after opening all 14 in turn (`KOMOREBI_SNAPSHOT_WALK=13`).
      Release binary 13.70 MiB. The gated prototype defects (04 address
      card and line breaks, 07 deep quotes, 11 columns, 14 long word) are
      fixed; 08 drops the paper card in the dark theme. (Fixed in the
      product round's Stage 2: in narrow panes fixed-width mail (02, 03,
      05, 09; 04 below 440 px) was cut at the right, hiding e.g. statement
      amounts. Fixed in its Stage 1: named inline images listed as attachments, 13's duplicate
      `text/calendar` "attachment-1", 06's bullets touching their text.)
      A leading ASCII space inside a span is still dropped after other
      text (13: "Rui Mendes- Organizer"): parley trims span edges.
- [ ] Check by hand once, with the user's authorization for that account
      (Stage 16): which authserv-id Postale and Purelymail write in
      `Authentication-Results` (the sender notice trusts the topmost header
      whose id has the IMAP host's registrable domain), and that the
      server strips such headers arriving from outside. Until then a
      provider whose id is in another domain shows no DMARC notice.
- [ ] Code signing: choose the provider and certificate (REL-04), then
      add the `SIGN_CERT_PFX` / `SIGN_CERT_PASSWORD` secrets (or replace
      `scripts/sign.ps1` with the provider's step). The installer and
      release workflow exist since the product round's Stage 24.
- [ ] Run the release workflow once by hand (workflow_dispatch, the dry
      run) and look at the setup, the zip and `SHA256SUMS` it keeps.
- [ ] Before the first public release, once on a clean machine: install,
      first run, an authorized test account (only with the user's
      authorization for it), Remove all data, uninstall (both answers to
      the cache prompt).
- [ ] Confirm whether Purelymail saves sent mail server-side; if so,
      default "save a copy to Sent" off for that preset. Postale.io does not
      (measured: 4 test sends and 1 APPEND left exactly 1 new Sent copy), so
      its preset keeps "save a copy" on.
