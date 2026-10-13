# Komorebi performance reference

Binding for any speed, memory or size work. Product promises: launch < 200 ms, cached list < 600 ms, exe <= 20.0 MB, light on memory. Safety and correct mail semantics outrank speed (AGENTS.md). Panic strategy, `unsafe` and the panic-free lints in `src/mail/render` and `src/mail/ical.rs` are not performance levers.

**The meta-rule: a change stays only if it beats the baseline by more than the run-to-run spread, or shrinks exe/memory measurably without slowing anything. Cite the rule below in the commit or changelog note, with numbers. Cleverness that does not measure is reverted.**

## 1. Where time goes (measured, release, 2000 messages, 2026-10)

- Launch to first frame is about 37 ms. Roughly 45 percent is Windows/winit (window creation about 7 ms, ShowWindow about 9 ms). `AppWindow::new` is about 2 ms (fonts). Cache load (DB open, folders, ids, model) is 6 to 10 ms. Do not touch `chrome.rs`, IME or font setup for speed.
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

1. Current profile: `opt-level = "s"`, fat LTO, `codegen-units = 1`, `strip`. Never `"z"` (disables loop vectorization). Per-package `opt-level = 3` goes only on crates whose loops are the product promise (Slint core and renderer, tiny-skia, SQLite C code); every other crate stays `s`.
2. A per-package override changes that crate's codegen before LTO; generic code is instantiated in the caller and follows the caller's level. Verify every override with a build and a timing, not by reasoning. `libsqlite3-sys` takes its C flags from the package's `OPT_LEVEL` through the `cc` crate.
3. Raising the whole `komorebi` package to 3 overshoots the budget (measured 20.4 MB). Hot render code at 3 needs a separate workspace crate; do that only if a 9-run app measurement beats the spread.
4. Do not add `#[inline(never)]`/`#[inline(always)]` speculatively; measure with cargo-bloat first. Forced Slint features (SVG, 3.8 MB of ICU dictionaries for complex scripts) cannot be dropped from our Cargo.toml; removing them changes behaviour or needs a vendored patch: out of scope.
5. No `target-cpu=native` (the installer ships to unknown CPUs). PGO/BOLT are not planned (no Windows evidence).
6. Size changes are checked with `cargo bloat --release --crates` on an unstripped build in a scratch target dir; the CI check is the stripped exe <= 20,000,000 bytes.
7. Not applied (no app-level measurement was made). Delay-loading (`/DELAYLOAD`, `delayimp.lib`) of DLLs needed only after the first frame (crypt32, ws2_32, secur32) saves about 1 ms of process start that `KOMOREBI_TIMING` cannot see (its clock starts at `main`). Failures move to the first call: the TLS tests and one fake-server sync must pass.

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

- Build: `cargo build --release --locked`. Exe size: `(Get-Item target\release\komorebi.exe).Length` (<= 20,000,000).
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
