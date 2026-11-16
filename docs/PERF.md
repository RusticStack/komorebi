# Komorebi performance reference

Binding for any speed, memory or size work. Product promises: launch < 200 ms, cached list < 600 ms, exe <= 21.5 MB (21,500,000 bytes; raised from 20.5 MB for Komorebi 1.0 on GPUI, 2026-10-25; the ceiling is komorebi.exe's, the Komorebi 1.0 release exe; the Slint app is deleted). Komorebi 1.0 is not light on memory: idle memory and CPU above the 0.7 Slint app's are accepted for 1.0 and optimizing them is 1.1 work (section 0). Safety and correct mail semantics outrank speed (CONTRIBUTING.md). Panic strategy, `unsafe` and the panic-free lints in `src/mail/render` and `src/mail/ical.rs` are not performance levers.

**The meta-rule: a change stays only if it beats the baseline by more than the run-to-run spread, or shrinks exe/memory measurably without slowing anything. Cite the rule below in the commit or changelog note, with numbers. Cleverness that does not measure is reverted.**

## 0. Komorebi 1.0 on GPUI: what is measured, what is not

Sections 1 to 7 below were measured on the 0.7 Slint app (deleted at the cut-over) and mostly do not apply to the GPUI app (`crates/komorebi-gpui`); they stay as the record and for the shared library. For 1.0 the owner accepted a higher idle memory and CPU than 0.7 had; the optimization pass (docs/GPUI-MIGRATION.md phase 18) is moved to 1.1. Nothing here claims a low memory figure for 1.0.

- Exe size. The last release build of the GPUI app (then still named `komorebi-gpui.exe`) measured it at 22,051,328 bytes (stage D, `main` alone 22,036,480), 551,328 bytes OVER the 21,500,000 ceiling, so the CI "Binary size" step fails until the exe is cut or the ceiling is moved (docs/GPUI-MIGRATION.md, stage D "other"). Earlier points: 11,074,048 bytes for the stage A skeleton. The tree was not rebuilt in release for the cut-over preparation, so the current size is not measured.
- Launch. Time to the first drawn frame, `KOMOREBI_TIMING=1`, seeded 2,000-message cache, offline, WARP, 1280x800, median of 9: 77.0 ms (range 75.0 to 80.7 ms; the cache loads in about 40 ms), against the 200 ms budget. The cached-list budget (600 ms) was not measured on its own: the list is drawn in that first frame. Cold first launches after a build were not measured for the full app.
- Memory and idle CPU. No figure for the full GPUI app exists. The feasibility spike (branch `spike/gpui`, `spike-gpui/REPORT.md` S2 and S3) measured a skeleton window with a list and no message open, against the 0.7 Slint app on the same cache (Get-Process and WorkingSetPrivate, 2 s after the first frame): private bytes 111.1 MB against 11.6 MB, private working set 22.1 MB against 10.7 MB, working set 55.2 MB against 34.1 MB, 35 threads against 11. Most of the private bytes were reserved and never touched: the DirectX runtime and driver were put at about 85 to 90 MB of commit, which the spike did not confirm with PIX or VMMap. Idle CPU over 60 s was 0.017 percent of one core visible and 0.006 percent minimized for the skeleton, against 0.012 and 0.136 percent for the Slint app. These are spike figures on a skeleton, not the shipped app; measure the release build before quoting any of them.
- 1.1 candidates (from the spike's optimization list, not done): lazy and smaller GPU resources, dropping a RenderImage's CPU bytes after upload, capping background executor threads, glyph atlas tuning, size trims, an explicit WARP fallback.

## 0.1 Appearance and background (1.1.2)

Measured 2026-11-16 on the release build (`komorebi.exe` 19,622,912 bytes, ceiling 21,500,000), Windows 11 build 26300, 1280x800 window, offline, a throwaway seeded cache, normal launches (not snapshots). Idle CPU is the process's CPU time over 10 s after 6 s of settling; memory is Get-Process private bytes and working set at that point. One run each, so differences under about 2 MB are noise.

| Setting | Idle CPU | Private | Working set |
| --- | --- | --- | --- |
| 1.1.1, defaults | 0.00% | 56.6 MB | 57.9 MB |
| Defaults | 0.00% | 56.8 MB | 59.9 MB |
| 4K picture (3840x2160, generated), Blur image 20 px, panes and sidebar 70%, Blur behind panes 20 px | 0.00% | 70.3 MB | 64.1 MB |
| 4K picture, Pixel art (cell 6, 16 colours, dithered), vignette 80% | 0.00% | 81.1 MB | 69.1 MB |
| 4K picture, ASCII (cell 7), vignette 80% | 0.00% | 76.9 MB | 68.6 MB |
| Mica, panes 80%, tint 50% | 0.00% | 52.5 MB | 59.5 MB |
| Acrylic, panes 80%, tint 50% | 0.00% | 52.6 MB | 59.6 MB |

- Defaults add no per-frame work: no picture is held, no element is added and no token changes (`the_defaults_add_nothing`, `a_background_picture_is_kept_drawn_and_let_go`).
- The picture is made once, off the UI thread, and drawn as one image (two with Blur behind panes); nothing is filtered per frame, so a picture costs no idle CPU. Moving a slider makes it again once the slider rests (120 ms); a picture made at the window's size (a style or a vignette) is made again once a resize or a theme change rests.
- At most two pictures are held: the stored picture and a quarter-size pane copy. The decoded original is dropped once they are made. Without a style, a picture blurred by sigma s is kept up to 4 times smaller a side (a pixel stays under a third of s): before that, the 4K picture at Blur 20 measured 130.4 MB private; after, 70.3 MB. A style is kept at the window's size so its cells sit on whole device pixels.
- Launch to the first frame with the defaults (`KOMOREBI_TIMING=1`, 2,000 messages, median of 7): 75.1 ms for 1.1.1, 75.3 ms for this build.

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
8. Memory numbers come from a run without `KOMOREBI_SNAPSHOT` (the snapshot adds about 12 MB of transient buffers): a live instance of the 0.7 Slint app is about 32.6 MB working set and 11.7 MB private (the GPUI app: section 0).

## 4. Codegen, profile and size

Source: Cargo book "Profiles"; rustc codegen options; Perf Book "Build Configuration", "Inlining"; cargo-bloat.

1. Current profile (speed first, owner's decision 2026-11-07; the exe's size matters least): `opt-level = 3`, fat LTO, `codegen-units = 1`, `strip`, `panic = "abort"`. `"z"` only for `ed25519-compact` and `sha2` (the update check's signature, once per update). Earlier profiles (`"s"` with per-package 3 for tiny-skia, SQLite and the text stack, and `"z"` for cold crates, the views and Kit) are in the git history (docs/PERF.md 4.8 is the record of what each cost in size).
   Development uses the unoptimized `dev` profile only (dependencies optimized once and cached); `release` is built once at the end of a task for the size ceiling and for any timing or memory number, which `dev` cannot give.
2. A per-package override changes that crate's codegen before LTO; generic code is instantiated in the caller and follows the caller's level. Verify every override with a build and a timing, not by reasoning. `libsqlite3-sys` takes its C flags from the package's `OPT_LEVEL` through the `cc` crate.
3. Raising the whole `komorebi` package to 3 overshoots the budget (measured 20.4 MB). Hot render code at 3 needs a separate workspace crate; do that only if a 9-run app measurement beats the spread.
4. Do not add `#[inline(never)]`/`#[inline(always)]` speculatively; measure with cargo-bloat first. Forced Slint features (SVG, 3.8 MB of ICU dictionaries for complex scripts) cannot be dropped from our Cargo.toml; removing them changes behaviour or needs a vendored patch: out of scope.
5. No `target-cpu=native` (the installer ships to unknown CPUs). PGO/BOLT are not planned (no Windows evidence).
6. Size changes are checked with `cargo bloat --release --crates` on an unstripped build in a scratch target dir; the CI check (`cargo xtask size`, `xtask/src/size.rs`) is the stripped exe against its ceiling: komorebi.exe <= 21,500,000 bytes.
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

- A/B harness: `pwsh -File scripts/opt-ab.ps1 -A <exe> -B <exe>` runs interleaved rounds (A B, B A, ...) of launch to first frame (empty, 2,000 and 55,000 caches), memory 2 s after the first frame and after opening the heavy message, open to tiles and to complete (every visible tile in a rendered frame, images decoded, the page laid out; then the read-ahead window too), blank tiles while reading it, list scrolling, composer typing and idle CPU (visible and minimized), and writes medians, min, max, the run count, the machine load and a verdict against a noise run (`-Noise`) to `results.json` and `report.md`. Run it with A and B the same exe first: that run is the noise floor, the smallest difference that counts. The in-app steps are `KOMOREBI_BENCH` (docs/ENVIRONMENT.md); `scripts/opt-prof.ps1` adds the in-process CPU sampler (`--features sampler`) and the counting allocator (`--features alloc-count`); `scripts/opt-threads.ps1` lists the threads. Seeding: `-Prepare -SeedExe <seed.exe>`.
- Build: `cargo build --release --locked -p komorebi-gpui`. Exe size: `cargo xtask size` (komorebi.exe <= 21,500,000, docs/GPUI-MIGRATION.md "Measurements").
- Launch: `KOMOREBI_TIMING=1` (`scripts/gpui-bench.ps1`) on a seeded copy (`examples/seed.rs`, 2000 and 20000 messages). Report a fresh-copy (cold) number and a warm second launch: the first launch pays one-time default-fold writes.
- Reader: the HTML corpus walk prints the open-to-first-tile line. `examples/render_html.rs` renders a fixture to PNG: compare byte for byte before and after.
- Search/list/DB: an `Instant` example or `#[test] #[ignore]` harness, variants interleaved in one process, 15 runs, assert equal results. No criterion.
- Statistics: at least 7 runs (9 for app runs), report median and min and the spread. Interleave A and B (A B A B) so drift hits both. The delta must exceed the spread.
- Pixels: the offline snapshot matrix must show no differences except explained ones (relative dates).
- Memory: live instance without `KOMOREBI_SNAPSHOT`, read PeakWorkingSet64 and PrivateMemorySize64 after 2 s.
- Size attribution: unstripped scratch build, `cargo bloat --release --crates`.
- Gates: `cargo xtask check`, `cargo test --locked --workspace`, `scripts/check-pins.ps1`, `cargo deny --workspace --locked check licenses advisories` (when Cargo.toml/lock change), `scripts/gpui-boot-check.ps1`.
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

## 11. Speed-first profile, allocator and PGO (2026-11-07)

Owner's decision: the exe's size matters least; responsiveness (scroll frame, typing, open to first tile, launch), then private and peak working set, then idle CPU. Every row is a 9-round `scripts/opt-ab.ps1` A/B against the row above it, judged against `noise-floor.json`; the full tables are in `C:\dev-overflow\claude-tmp\komorebi-opt\runs\speed-*`. The frame metrics (scroll, typing) include the wait for the display's next frame, so they move by a few tenths of a millisecond and sometimes against the process cycles per frame; read both.

| Step | Result (beyond the floor) | Exe bytes |
|---|---|---|
| start: views and Kit at "z" (904e0cd) | | 15,537,152 |
| revert both to "s" | open -6 ms; peak working set after open +0.5 MB; frames and launch equal | 16,774,656 |
| root opt-level 3, cold crates "z" | open -17 ms (-12 percent); scroll and typing frames +4 to +11 percent slower (two batches, load flagged) | 21,301,760 |
| mimalloc 2 + purge delay 0 | open -17 ms (-13 percent); working set +0.8 MB before a message is open, -1.5 MB (peak -1.9 MB) after; private bytes +6 and +12 MB | 21,188,096 |
| SegmentHeap on top | private bytes -1.2 and -1.7 MB, private working set -0.8 and -1.0 MB, peak unchanged, speed unchanged | 21,188,096 |
| UI stack (gpui-pre, taffy, Kit, the views) at "s", the rest 3 | scroll -7 percent, typing -10 percent (two batches, load flagged), open +5 to +7 ms, scroll Mcycles/frame +15 percent | 17,895,424 |
| whole lane vs the start (valid) | open -34.7 ms (-23 percent); working set after open -2.3 MB, peak -1.6 MB; private bytes +6.4 and +10.6 MB; frames, launch, idle equal | 17,895,424 |

Not kept:

- Root 3 with no "z" crate at all: 21,668,864 bytes, 168,864 over the 21,500,000 ceiling (not raised).
- mimalloc 3 (the crate's default): private bytes +27 to +32 MB, peak working set +25 MB after the heavy message (valid batch); the "v2" feature is used instead.
- PGO (below): not committed.

### PGO on Windows MSVC (measured, not in the release flow)

Works with the pinned toolchain 1.95.0: `RUSTFLAGS=-Cprofile-generate=<dir>` with `--target x86_64-pc-windows-msvc` (the explicit target keeps build scripts and proc macros uninstrumented), the harness workloads as training (`opt-ab.ps1 -A x -B x -Metrics launch,mem,open,scroll,type -Rounds 3`, with `LLVM_PROFILE_FILE=<dir>/komorebi-%p-%m.profraw`; 45 files of 20 MB), `llvm-profdata merge` from the rustup `llvm-tools` component (added to the 1.95.0 toolchain under `C:\dev-overflow\rustup`; the stable toolchain's tool is LLVM 23, the compiler's is 22), then `-Cprofile-use=<file>.profdata`. Instrumented build 14 minutes, instrumented exe 75 MB, training 3 minutes, final build 8 minutes.

A/B vs the same config without PGO (valid, load 5 percent): open to first tile -18.7 ms (-16 percent), scroll Mcycles/frame -10 percent, launch -4 percent (within floor), peak working set -1 MB; but scroll median +7.5 percent, p95 +4.8 percent, typing median +11.8 percent (beyond the floors), and the exe is 0.9 MB larger. Not committed: it loses on the frame metrics the owner ranks first, and the release needs a pipeline that CI cannot reasonably run on a Windows runner (a 26 MB profile that depends on the exact sources, a GUI session with a GPU or WARP for the training, 25 minutes of extra builds, `llvm-tools` in the toolchain).

## 12. Cached views in the mail window (`views/mail/panes.rs`, optimization pass)

GPUI rebuilds the element tree and lays out all of it every frame, so a frame costs by node count, not by what changed. The Skua fork's splice (`third_party/gpui-pre/src/window/splice.rs`) redraws only the top-level cached views that were notified and replays every other view's last prepaint and paint. The folder pane, the message list and the composer are cached views (`.cached(style)`), so a wheel step or a hover in the list, or a key in the composer, no longer lays out the rest of the window.

Rules for a cached view (a break shows as a pane that does not change, not as a failure):

1. A cached view replays its last frame until it, or a view inside it, is notified, or its bounds, content mask or text style change, or the window refreshes. The style given to `.cached(..)` has to be definite (a size or a flex), so the view is the box its parent laid out.
2. It has to `cx.observe` every entity its `render` reads (the mail view, the hub, the composer's inputs), and call `cx.notify()` from the observer. Globals (the language, the theme) are covered by `refresh_windows`, which the Settings choices call, and which bypasses every cache.
3. A handler that notifies "the whole window" from inside a pane (a row's hover, the wheel) notifies the pane (`MailView::notify_list`), or the cached folder pane and composer are redrawn for nothing and the splice falls back to a full draw.
4. A view that is not cached, or a model, notified inside a cached one makes the splice fall back to a full draw; so do a refresh, a focus or modality change, a resize, a drag, a prompt and the accessibility tree.
5. Tests do not use `render_frame` for what they assert: it refreshes and ignores every cache. They call `cx.run_until_parked()` and read the last drawn frame (`names`), or count draws (`panes::renders()`, `composer::renders()`, `cfg(test)`).

Measured (alloc-count fast build, 2,000 message cache, bench script of `scripts/opt-ab.ps1`): typing 6,779 allocations and 4.45 MB per key before, 1,801 and 1.10 MB with the composer cached; scrolling 6,715 allocations and 4.49 MB per frame before, 3,789 and 3.04 MB with the panes cached.

## 13. Optimization pass, integrated (opt/integ, 2026-11-14)

Lanes merged with `--no-ff` in this order: fork-win, fork-core, speed (contains fork-kit), app, render (contains lib), then overlap, plus the reader wiring of `RenderImage::from_shared_bgra` and a fix to the flaky `closed_port()` test helper. `opt/fast` is empty (reverted) and was skipped.

**Not done, deferred to a later update (owner's decision):** the combined re-measurement against the baseline, and the leave-one-out runs for mimalloc, the UI stack at "s" and the cached panes. Every change stays as each lane left it; nothing was reverted on timing. The numbers below are each lane's own, against its own parent; several batches were flagged INVALID by machine load (said per row). The combined batches run on the merged tree (runs/integ-01 to integ-06, integ-loo-a) were all INVALID for load (other builds and a game; 50 to 100 percent) and are not results. Two leave-one-out batches on the merged tree were valid: removing the UI-stack "s" setting (scroll median 3.79 -> 3.98 ms, typing 4.15 -> 4.14, Mcycles/frame 6.97 -> 6.17, memory equal, all within the floor) and removing the cached panes (scroll median 3.97 -> 3.54, Mcycles/frame 5.89 -> 7.53, typing equal, within the floor). They settle nothing; both configurations are kept.

| Lane | Change | Measured by the lane |
|---|---|---|
| fork-win | one D3D device, lazy path MSAA, shared-pixel images | valid: launch 137.0 -> 87.4 ms, private bytes after first frame 110.9 -> 49.9 MB, after open 124.9 -> 100.0 MB; `from_shared_bgra` -3.3 MB after open |
| fork-core | six per-frame patches | allocations per scroll frame -20 percent, bytes -52 percent, Mcycles/frame -10.8 percent; frame time +0.2 ms (scroll 3.44 -> 3.67, typing 3.02 -> 3.30), kept by the owner |
| speed/fork-kit | dependency trims, root opt-level 3, mimalloc 2 (purge delay 0), SegmentHeap, UI stack at "s" | valid whole-lane: open to first tile 153.5 -> 118.8 ms, working set after open -2.3 MB, peak -1.6 MB, private bytes +6 to +11 MB; the UI stack at "s": scroll -7 percent, typing -10 percent, Mcycles/frame +15 percent (batches INVALID for end load) |
| app | cached composer and panes | typing allocations per key 6,779 -> 1,801; scroll Mcycles/frame -19 to -22 percent; scroll median +0.5 ms once (batch INVALID), unconfirmed |
| render | layout reuse, fidelity F1 to F15, image pre-decode, white space, long threads laid out whole with a first look, size-based budget | photo newsletter complete view 297 -> 111 ms; t60-big first tile 47 -> 24 ms (page laid out whole instead of cut at 30,000 px) |
| overlap | one overlap check on painted glyphs, floats go below earlier floats, block-only line after a br keeps its height | correctness fixes (tests/render_overlap.rs) |

Merge-time fix: `tests/render_reuse.rs` failed on opt/render itself since 07ea921 (a picture decoded for a first pass at another width was reused). `wanted_images` now keeps a decoded picture only at the width a fresh decode would give; pixels equal a layout from scratch.

Exe: 18,014,208 bytes at the merged HEAD before this section (ceiling 21,500,000, not raised); recorded for information.
