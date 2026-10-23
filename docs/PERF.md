# Komorebi performance reference

Binding for any speed, memory or size work. Product promises: launch < 200 ms, cached list < 600 ms, exe <= 20.5 MB, light on memory. Safety and correct mail semantics outrank speed (CONTRIBUTING.md). Panic strategy, `unsafe` and the panic-free lints in `src/mail/render` and `src/mail/ical.rs` are not performance levers.

**The meta-rule: a change stays only if it beats the baseline by more than the run-to-run spread, or shrinks exe/memory measurably without slowing anything. Cite the rule below in the commit or changelog note, with numbers. Cleverness that does not measure is reverted.**

## 1. Where time goes (measured, release, 2000 messages, 2026-10)

- Launch to first frame is about 37 ms in 0.3.0 and about 50 ms in 0.4.0 on a quiet machine (the cache is ready at about 20 ms; the extra time sits between database open and the first frame, not yet profiled). Roughly 45 percent is Windows/winit (window creation about 7 ms, ShowWindow about 9 ms). `AppWindow::new` is about 2 ms (fonts). Cache load (DB open, folders, ids, model) is 6 to 10 ms. Do not touch `chrome.rs`, IME or font setup for speed.
- The list is not the cost: 9 visible rows cost about 0.2 ms, `header()` is about 7 us. 20000 messages add about 2 ms at launch (id Vec, unread count scan).
- Reader open-to-first-tile is 3 to 55 ms, dominated by layout, shaping (parley) and image resampling, all pixel/text loops.
- Search is a full scan: about 1.3 to 2.3 ms at 2000 messages, 10 to 15 ms at 20000 (was 2.4 and 20 before the one-pass `fold_has`).
- Sync CPU is background-thread work and below one frame on the UI thread.

Measure before believing intuition. The surprises in this repo were the build profile (tiny-skia at `s`) and the double allocation in search, not hashing.

## 2. Data structures

Source: Rust Performance Book "Hashing"; std `HashMap` docs; K. Lynagh, kevinlynagh.com/notes/match-vs-lookup.

1. Key set fixed at compile time and small: `match` or `if` chains. LLVM lowers dense matches to range checks or jump tables. Do not build a map for it.
2. Runtime set of up to about 16 to 32 entries: linear scan or sorted slice. Hash cost on short keys dominates. Crossover evidence is anecdotal (gists, hybridmap), so benchmark at the real size and hit/miss ratio.
3. Built once, read many, ordered or ranged: sorted `Vec` + `binary_search_by_key`. Inserts are O(n), so not for hot mutation.
4. Dynamic and large: `HashMap`. Pre-size with `with_capacity` when the count is known. Use the `entry` API instead of get+insert.
5. Hasher: keep std SipHash for anything keyed by mail-derived text (HashDoS). A cheaper hasher is justified only when a profile shows hashing above a few percent of the phase and the keys are internal and bounded (glyph ids). Measured here: SipHash vs Fx on 20000 ids is 0.29 vs 0.07 ms, a once-per-Ctrl+A cost, so `Selection` stays. Sorted merge vs HashSet for 20000 UIDs saves 0.3 ms per full sync pass: not worth the code.
6. Do not add a crate (rustc-hash, ahash, smallvec, phf, memchr) for a sub-millisecond win. A std-only 15-line hasher is the fallback if rule 5 is ever met.

## 3. Allocation and copying

Source: Rust Performance Book "Heap Allocations", "Iterators", "Bounds Checks"; std `Cow` and `fmt` docs.

1. Never allocate per row or per item in a loop that runs per message: reuse a buffer (`clear()` keeps capacity), return `Cow` that borrows when nothing changes, `write!` into a reused `String` instead of `format!`.
2. One call over N items beats N small calls (SQL function dispatch, statement prepare, transaction commit). Batch writes in one transaction; parse outside the transaction so the write lock is short.
3. `Cow::into_owned()` at the boundary of a callback that cannot borrow (rusqlite scalar results) defeats the Cow: restructure so the function returns a bool/integer, not text.
4. Iterate (`chunks_exact`, `zip`) instead of indexing in hot loops; bounds checks matter less than expected, and `get_unchecked` is forbidden here. Panic-free modules use `get`, `checked_*`, `wrapping_*`.
5. Pre-size `Vec` when the length is known; do not `shrink_to_fit` on hot paths.
6. `Rc`/`Arc` only for data actually shared; `Box<str>` for immutable long-lived text. No crossover numbers exist for `SmallVec`/`Box<str>`: profile (DHAT) first.
7. Case folding on hot paths: check `iter().any(u8::is_ascii_uppercase)` first and match borrowed bytes; fold only when needed, into a reused buffer. Folding must stay identical to `src/fold.rs`; never swap in `str::to_lowercase` (Unicode folding changes results).
8. Memory numbers come from a run without `KOMOREBI_SNAPSHOT` (the snapshot adds about 12 MB of transient buffers): a live instance is about 32.6 MB working set and 11.7 MB private.

## 4. Codegen, profile and size

Source: Cargo book "Profiles"; rustc codegen options; Perf Book "Build Configuration", "Inlining"; cargo-bloat.

1. Current profile: `opt-level = "s"`, fat LTO, `codegen-units = 1`, `strip`. `"z"` (disables loop vectorization) only for code with no loop that matters: the generated UI crate `crates/komorebi-ui` and the cold crates listed in `Cargo.toml` (see 4.8); never for the `komorebi` package itself (see 4.8). Per-package `opt-level = 3` goes only on crates whose loops are the product promise (Slint core and renderer, tiny-skia, SQLite C code); every other crate stays `s`.
   Development uses the unoptimized `dev` profile only (dependencies optimized once and cached); `release` is built once at the end of a task for the size ceiling and for any timing or memory number, which `dev` cannot give.
2. A per-package override changes that crate's codegen before LTO; generic code is instantiated in the caller and follows the caller's level. Verify every override with a build and a timing, not by reasoning. `libsqlite3-sys` takes its C flags from the package's `OPT_LEVEL` through the `cc` crate.
3. Raising the whole `komorebi` package to 3 overshoots the budget (measured 20.4 MB). Hot render code at 3 needs a separate workspace crate; do that only if a 9-run app measurement beats the spread.
4. Do not add `#[inline(never)]`/`#[inline(always)]` speculatively; measure with cargo-bloat first. Forced Slint features (SVG, 3.8 MB of ICU dictionaries for complex scripts) cannot be dropped from our Cargo.toml; removing them changes behaviour or needs a vendored patch: out of scope.
5. No `target-cpu=native` (the installer ships to unknown CPUs). PGO/BOLT are not planned (no Windows evidence).
6. Size changes are checked with `cargo bloat --release --crates` on an unstripped build in a scratch target dir; the CI check is the stripped exe <= 20,500,000 bytes.
7. Not applied (no app-level measurement was made). Delay-loading (`/DELAYLOAD`, `delayimp.lib`) of DLLs needed only after the first frame (crypt32, ws2_32, secur32) saves about 1 ms of process start that `KOMOREBI_TIMING` cannot see (its clock starts at `main`). Failures move to the first call: the TLS tests and one fake-server sync must pass.

8. Size, 2026-10-15 (merged 0.7 tree: rules, contacts, calendar; unstripped scratch build with `cargo bloat`, stripped exe from the repo profile; sections of the exe sum to the file, so symbols add nothing). Merged tree at the old profile: 21,352,960 bytes (20.5 MB ceiling exceeded by 853 KB; 0.6.0 was 19,780,608). Slint's generated code was 3.6 MiB of the `komorebi` crate's 5.1 MiB `.text`, most of it `ensure_instantiated` and `new` of the window and its pages.
   - The generated UI is its own workspace crate (`crates/komorebi-ui`, built from `ui/*.slint` and `translations/`) at `opt-level = "z"`. This is why 4.1 can allow "z" there: it is instantiation code run once per window, while the pixel loops stay in `i-slint-core` and the software renderer at 3. The crate's build script names the catalog `komorebi` (Slint takes the crate name), so the `.po` files, the template and the catalog test are unchanged.
   - Cold dependencies at "z": ed25519-compact (187 KB to 52 KB), sha2, usvg, resvg, svgtypes, simplecss, roxmltree, winit, accesskit and its Windows and winit glue, lettre, muda. About 274 KB together. Pixel, shaping, font, decoding and mail-parsing crates stay as they were.
   - Result, release profile, stripped: 19,293,184 bytes, 2,059,776 smaller than the merged tree before and 487,424 smaller than 0.6.0. Snapshots (default, settings, shortcuts, compose-rich, calendar, pt-PT) are byte-identical to the build before. First frame, 9 interleaved runs on a seeded 2000-message copy: median 40.7 ms before, 39.1 ms after (min 36.4 / 36.9). Reader, open to first tile over 14 messages, sum of per-message medians: 194.1 ms before, 188.9 ms after (inside the spread).
   - Rejected: `opt-level = "z"` on the whole `komorebi` package (probe: 18,513,408 bytes, but the reader's open to first tile gets 14 percent slower, 168.9 to 192.8 ms over the same 14 messages, because `src/mail/render` shares the package). `SLINT_INLINING` is already off for Rust output. The SQLite trims of 5.7 (`DEFAULT_MEMSTATUS=0`, `OMIT_DEPRECATED`, `OMIT_SHARED_CACHE`, `OMIT_PROGRESS_CALLBACK`) saved 1 KB in `libsqlite3_sys` and made two mail tests fail (a foreign key error and a lost connection); not applied.

## 5. SQLite

Source: sqlite.org (wal, pragma, mmap, queryplanner, optoverview, lang_expr, c3ref/create_function); rusqlite docs.

1. Batch writes: one transaction per batch (`insert_headers` does; prefetch body storage should). WAL + `synchronous=NORMAL` is already set for the cache; drafts and outbox are the only copy, so think before loosening anything.
2. Check every hot query with `EXPLAIN QUERY PLAN`: want `SEARCH ... USING (COVERING) INDEX`, no `USE TEMP B-TREE` for the single-folder list. `LIKE '%x%'` is a scan by definition; only the per-row cost can be cut.
3. A scalar function called per row per column is expensive (dispatch, argument copy, result copy). Prefer one variadic deterministic function that reads columns with `get_raw(i).as_bytes()`, short-circuits, and returns an integer. Flags: `SQLITE_UTF8 | DETERMINISTIC | INNOCUOUS`. Replacing `fold(col) LIKE` with byte matching must keep: ASCII-only case folding, `%`, `_` and `\` literal, NULL in gives NULL out, non-text columns behave as before. Prove it with a differential test against the old query.
4. `mmap_size`: measured slower for scans here (31 to 43 ms) and risky on Windows (I/O errors become signals). Keep 0. A larger `cache_size` speeds scans but raises resident memory: keep the default unless large-cache search is reported slow.
5. Partial indexes (`WHERE seen = 0`, `WHERE flagged = 1`) are the right tool for six-figure caches but cost a schema migration on user data. Not before a measured need.
6. `prepare_cached` for statements run repeatedly; `PRAGMA optimize` on close (already done) feeds the planner.
7. SQLite C compile flags (not applied; `.cargo/config.toml` still only disables optional modules): only the safe trims (`DEFAULT_MEMSTATUS=0`, `OMIT_DEPRECATED`, `OMIT_SHARED_CACHE`, `LIKE_DOESNT_MATCH_BLOBS`, `OMIT_PROGRESS_CALLBACK`). Never `SQLITE_OMIT_AUTOINIT`: rusqlite does not call `sqlite3_initialize` and the exe segfaults at startup.

## 6. Startup and Slint

Source: critical-path / Amdahl reasoning; Slint docs (Model, property bindings); Microsoft docs (LoadImageW, CreateMutexW, ShowWindow).

1. Shorten the critical path by overlapping independent work (DB open, icon decode) with `AppWindow::new` on `std::thread`. Order is binding: the single-instance check first, then any thread that opens or migrates the cache. Propagate errors unchanged to the existing dialogs. `rusqlite::Connection` is `Send` but not `Sync`: move it, never share it.
2. Defer first-launch writes (`apply_default_folds`) until after the first frame; keep the in-memory state correct meanwhile.
3. Win32 first-call costs (the first `LoadImageW` on PNG ICO entries is about 1.8 ms) are one-time decoder init: pay them off the UI thread.
4. The software renderer redraws dirty regions; keep per-row updates to `row_changed` on that row, never a model swap. Row work is O(visible rows).
5. Do not patch winit/Slint to skip the Windows first-frame pre-render (about 2 ms net): vendoring strains pins and deny, and the harness cannot see a flash.
6. Window creation, ShowWindow and fonts are the OS/winit floor. Leave them.

## 7. Reader and render engine

Only rule 1 is applied (tiny-skia at 3). Rules 2 to 4 describe designs that were not implemented or measured in the 2026-10-13 pass; the code still has the plain `box_down`, `DOC_CACHE = 3` and eager prescale. Treat them as candidates that need an A/B first.

Source: Perf Book; parley and tiny-skia docs; Chromium image decode cache design.

1. Pixel loops (tiny-skia raster, resample) belong at `opt-level = 3` through a package override. Measured: tile raster 1.3 to 2.5x faster, exe 46 KB smaller.
2. Lazy image resampling is allowed only when output stays bit-identical: decide size and filter at layout time, charge the pixel budget at layout, key any cache on (image, w, h). Never resample per tile (tile edges change the sampling phase).
3. `box_down`: accumulate column sums into a reusable `u32` buffer, divide once per output. Sums cannot overflow (`MAX_PIXELS` 16 Mpx x 255 < 2^32). Use `wrapping_*`/`checked_*` for `arithmetic_side_effects`.
4. Caches are bounded by bytes, not counts. `DOC_CACHE = 3` documents can hold 54 MB for three 6000-cell statements: charge an estimated byte cost per document.
5. Shaping (`parley build()`, about 8 us per group) is third-party; our wrapper is about 1 us per group. Do not optimise it.
6. Tag interning, buffer reuse in `Sheet::matched`, Fx hashing of glyph keys, painting straight into the tile buffer: each is under 0.5 ms on typical mail. Incidental cleanups only, never a stage.

## 8. How to measure

All runs: release, `--locked`, nothing else compiling or rendering, throwaway `KOMOREBI_DB`, `KOMOREBI_OFFLINE=1`. Never the real cache, real mail or `.env*`.

- Build: `cargo build --release --locked`. Exe size: `(Get-Item target\release\komorebi.exe).Length` (<= 20,500,000).
- Launch: `KOMOREBI_TIMING=1` with `KOMOREBI_SNAPSHOT` on a seeded copy (`examples/seed.rs`, 2000 and 20000 messages). Report a fresh-copy (cold) number and a warm second launch: the first launch pays one-time default-fold writes.
- Reader: the HTML corpus walk prints the open-to-first-tile line. `examples/render_html.rs` renders a fixture to PNG: compare byte for byte before and after.
- Search/list/DB: an `Instant` example or `#[test] #[ignore]` harness, variants interleaved in one process, 15 runs, assert equal results. No criterion.
- Statistics: at least 7 runs (9 for app runs), report median and min and the spread. Interleave A and B (A B A B) so drift hits both. The delta must exceed the spread.
- Pixels: the offline snapshot matrix must show no differences except explained ones (relative dates).
- Memory: live instance without `KOMOREBI_SNAPSHOT`, read PeakWorkingSet64 and PrivateMemorySize64 after 2 s.
- Size attribution: unstripped scratch build, `cargo bloat --release --crates`.
- Gates: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `scripts/check-pins.ps1`, `cargo deny --locked check licenses advisories` (when Cargo.toml/lock change), `KOMOREBI_SNAPSHOT_CHECK`.
- Cold disk (first boot, Defender scan) can dwarf warm numbers; say which one a claim is about.

## 9. Rejected so far (do not retry without new evidence)

Vendored winit pre-render skip; dropping ICU dictionaries or SVG via patches (behaviour, maintenance); `mmap_size`; bigger `cache_size`; SQLite partial indexes now; faster hasher for `Selection` and UID sets; per-UID flag UPDATEs (2x slower than the scan); `SQLITE_OMIT_AUTOINIT`; touching IMAP parsing; tag enum refactor; whole-package opt-level 3.

Not done, not proven worse: `box_down` column sums, a byte-budget `DOC_CACHE`, lazy image prescale, `/DELAYLOAD`, the extra SQLite flags. No A/B was run for them; do not claim a gain without one.

## 10. Encrypted cache (`src/cachecrypt.rs`, 2026-10-15)

Designs weighed, against a 0.32 MB exe budget, the launch and list promises and WAL:

- SQLCipher or SQLite3MultipleCiphers: rejected. SQLCipher needs OpenSSL (or a vendored crypto library) and both replace the bundled SQLite build; well over the size room and a second crypto stack beside SChannel.
- (b) Column-level encryption (bodies, subjects, addresses) with a DPAPI key: rejected. Search (`fold_has` over subjects and addresses), the address index, sorting and every `LIKE` would need decrypting per row (the search scan is the hottest query), and metadata in other columns and the indexes would stay readable.
- (a) Chosen: a VFS shim over the default Windows VFS. AES-256-GCM through CNG (`bcrypt.dll`, no new crate; only the `Win32_Security_Cryptography` feature of the pinned `windows-sys`). SQLite keeps 28 reserve bytes per page (`SQLITE_FCNTL_RESERVE_BYTES`) for a random 96-bit nonce and the tag, so offsets and sizes stay SQLite's own. The additional data is the file kind and the page or frame index (swapped pages fail). Pages of the cache and of the `-wal` are sealed; the WAL header and frame headers, the `-shm` index and page 1's 16-byte header string (replaced on disk by `Komorebi AES256\0`, so an encrypted cache is told from a damaged one without its key) stay in the clear. A rollback journal is allowed only for the WAL switch of a new empty file and refuses any page-sized write; statement journals and temp files are refused (`temp_store = MEMORY`). `iVersion` 2 (no `xFetch`), so nothing memory-maps ciphertext. A tag failure is `SQLITE_IOERR_DATA`; in a whole-frame WAL recovery read it reads as zeros so the frame checksum drops it like a torn frame. SQLite leaves the reserve zeroed; a write with a non-zero reserve is refused, since WAL checksums cover it.
- Key: 32 random bytes per cache (`BCryptGenRandom`), stored as a DPAPI blob (current user, fixed entropy) in `<cache>.key`, written to a temp file, flushed and renamed. A missing or foreign key is a typed `KeyError` (or `IOERR_DATA`), never "damaged", so the cache is not set aside for it.
- Migration: copy into a new file through `ATTACH` (tables, then rows, then indexes and triggers, `user_version`, `sqlite_sequence`; `VACUUM INTO` was tried first and does not keep the reserve bytes the target needs), check with `quick_check` and every table's row count, rename the old file to `.previous`, rename the copy in, remove the old file, `-wal` and `-shm` (and the key after a decryption). `recover` at `open_ui` finishes or undoes a run cut at any step (tested at each).
- Default: opt-in, off. The cache is the only copy of accounts, drafts and the Outbox; a lost DPAPI key (password reset by an administrator, a profile moved to another computer) loses them for good, an automatic migration of every user's cache is the riskiest change possible, and BitLocker already covers a stolen disk. The setting itself is not built yet (TODO.md).
- 96-bit random nonces are safe to about 2^32 page writes per key (NIST SP 800-38D); `migrate` reuses a key file already beside the cache (one a crash left after the key step), else makes a new key; a decryption deletes the key. Known hole: a page that reads back as all zeros is taken as never written even inside the file, so zeroing a page is not caught by the tag (SQLite usually reports corruption, but a zeroed last overflow page can read as wrong data). Accepting zeros only at or past the end of file (`xFileSize`) closes it; check first that SQLite never leaves zero gaps inside a valid file.

Checked in the app: a dev build opened the encrypted 55,000-message cache (snapshot shows the full list), wrote through the VFS (WAL frames, checkpoint, `PRAGMA optimize`) and opened it again.

Measured (release, own target dir, 55,000-message seeded throwaway cache, 12 MB, `examples/db_bench.rs`, 15 runs per figure, two interleaved rounds, median / min in ms, plaintext then encrypted):

| Query | Plaintext | Encrypted |
|---|---|---|
| Open cache + all ids (cached list load) | 4.66 / 4.32, 4.47 / 4.34 | 3.91 / 3.73, 4.00 / 3.93 |
| All ids, warm | 1.99 / 1.96, 1.90 / 1.82 | 1.76 / 1.73, 1.76 / 1.74 |
| Flagged filter | 13.20 / 12.90, 13.10 / 12.74 | 14.28 / 13.98, 14.36 / 13.87 |
| Search (full scan, page cache smaller than the file) | 25.73 / 25.20, 25.32 / 24.84 | 26.81 / 26.12, 26.86 / 26.39 |
| Open message (header + message) x50 | 0.68 / 0.67, 0.66 / 0.65 | 0.65 / 0.65, 0.64 / 0.64 |

The list load and the open-message reads are within the spread (the encrypted file is about 9 percent smaller after its copy, which makes its open a little faster); search pays about 1 ms (4 percent) for decrypting every page it reads. Exe: 19,680,768 bytes before, 19,696,640 after (+15,872 bytes; 303,360 bytes of room left). First frame (9 interleaved app runs, plaintext 55,000 cache, `KOMOREBI_TIMING`): before median 44.7 / min 40.6 ms, after 46.9 / 40.1 ms; a second round of 9 pairs: before 45.3 / 40.8, after 46.3 / 40.2. Slow outliers came on both (to 64 and 73 ms), so the difference is inside the run-to-run spread. The release exe measured was built just before the page 1 marker (a 16-byte copy per page 1 read or write), so an app launch on an encrypted cache was not timed; the cache-load part of launch is the `db_bench` row above.
