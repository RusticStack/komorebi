//! Mutation harness for the HTML renderer, seeded from the adversarial
//! corpus message: truncation, deep nesting, huge attributes, spliced
//! segments and random bytes. Every mutant must lay out (or fail cleanly)
//! without panicking, within a bounded time and a capped peak heap, which a
//! counting allocator measures. A seeded xorshift keeps runs reproducible;
//! the seed is printed on failure. `KOMOREBI_FUZZ_ITERS` raises the count.

mod common;

use common::{input, raw};
use komorebi::mail::render::{self, Deadline, Engine, LAYOUT_BUDGET, Opts, RenderInput};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(n: usize) {
    let now = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grow(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if n > l.size() {
            grow(n - l.size());
        } else {
            LIVE.fetch_sub(l.size() - n, Relaxed);
        }
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Peak heap growth allowed per mutant.
const MAX_HEAP: usize = 512 << 20;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..1 + rng.below(3) {
        let at = rng.below(b.len());
        match rng.below(6) {
            0 => b.truncate(at),
            1 => {
                let tags: [&[u8]; 6] = [
                    b"<div>",
                    b"<table><tr><td>",
                    b"<b>",
                    b"<blockquote>",
                    b"<span style=\"display:inline-block\">",
                    b"<li>",
                ];
                let tag = tags[rng.below(tags.len())];
                let n = 1 + rng.below(2000);
                let ins: Vec<u8> = tag.iter().copied().cycle().take(tag.len() * n).collect();
                b.splice(at..at, ins);
            }
            2 => {
                let mut ins = b"<img width=\"99999\" height=\"2\" alt=\"".to_vec();
                ins.extend(std::iter::repeat_n(b'x', 1 + rng.below(200_000)));
                ins.extend_from_slice(
                    b"\" style=\"width:1e30px;font-size:-1e9px;line-height:NaN\">",
                );
                b.splice(at..at, ins);
            }
            3 => {
                let len = rng.below(4096).min(b.len() - at);
                let seg = b[at..at + len].to_vec();
                for _ in 0..rng.below(50) {
                    let to = rng.below(b.len());
                    b.splice(to..to, seg.iter().copied());
                }
            }
            4 => {
                const BYTES: &[u8] = b"<>/=\"' &;#{}:!-\0\xff";
                for _ in 0..1 + rng.below(500) {
                    let i = rng.below(b.len());
                    if let Some(x) = b.get_mut(i) {
                        *x = BYTES[rng.below(BYTES.len())];
                    }
                }
            }
            _ => {
                let css = format!(
                    "<style>{}</style>",
                    "* * * * * * td{padding:1px} @media (max-width:9999px){@media all{a{color:red}}} ".repeat(1 + rng.below(3000))
                );
                b.splice(at..at, css.into_bytes());
            }
        }
        b.truncate(render::MAX_HTML_BYTES + 1024);
        if b.is_empty() {
            b.push(b'<');
        }
    }
    b
}

#[test]
fn mutants_never_panic_hang_or_balloon() {
    let iters: u64 = std::env::var("KOMOREBI_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 24 } else { 200 });
    let base = input(&raw("14-adversarial-malformed"));
    let seed_html = base.html.clone().unwrap().into_bytes();
    common::big_stack(move || {
        let mut engine = Engine::new();
        for seed in 1..=iters {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let html = String::from_utf8_lossy(&mutate(&seed_html, &mut rng)).into_owned();
            let input = RenderInput {
                html: Some(html),
                inline: base.inline.clone(),
                ..Default::default()
            };
            let width = [320.0, 600.0, 900.0][rng.below(3)];
            let opts = Opts {
                width_px: width,
                scale: [1.0, 1.25, 2.0][rng.below(3)],
                dim: rng.below(2) == 1,
                ..Default::default()
            };
            let before = LIVE.load(Relaxed);
            PEAK.store(before, Relaxed);
            let t = Instant::now();
            if let Ok(doc) =
                render::layout(&input, &opts, &mut engine, &Deadline::after(LAYOUT_BUDGET))
            {
                let y = (doc.height * opts.scale / 2.0) as u32;
                let _ = render::rasterize(&doc, &mut engine, y, 256, opts.scale);
                // Selection reaches parley's cursor code on hostile layouts.
                if let Some((a, b)) = doc.text_bounds() {
                    let _ = (doc.selection_text(a, b), doc.selection_rects(a, b));
                }
                for _ in 0..4 {
                    let x = rng.below(width as usize + 40) as f32 - 20.0;
                    let y = rng.below(doc.height as usize + 40) as f32 - 20.0;
                    if let (Some(a), Some((w0, w1))) = (doc.text_at(x, y), doc.word_at(x, y)) {
                        let _ = (doc.selection_text(a, w1), doc.selection_rects(w0, a));
                    }
                    doc.block_at(x, y);
                }
            }
            let took = t.elapsed();
            let peak = PEAK.load(Relaxed).saturating_sub(before);
            assert!(took < Duration::from_secs(5), "seed {seed}: took {took:?}");
            assert!(peak < MAX_HEAP, "seed {seed}: peak heap {} MiB", peak >> 20);
        }
    });
}

/// Pathological text: a paragraph of nearly 2 MB and one 500 KB word are
/// cut at the per-group text cap (and the page says so), and a letter
/// with 100k combining marks keeps 32 of them; each lays out and paints a
/// band within 300 ms in a release build (debug builds get a relaxed
/// limit), and none trips parley's per-cluster counter.
#[test]
fn pathological_text_is_capped_and_quick() {
    let limit = Duration::from_millis(if cfg!(debug_assertions) { 3000 } else { 300 });
    let mut words = String::from("<p>");
    while words.len() < render::MAX_HTML_BYTES - 64 {
        words.push_str("lorem ipsum dolor sit amet ");
    }
    let cases = [
        ("2 MB paragraph", words),
        ("500 KB word", format!("<p>{}</p>", "x".repeat(500 * 1024))),
        (
            "100k combining marks",
            format!("<p>a{} b</p>", "\u{0301}".repeat(100_000)),
        ),
        // Alt text of a blocked image is laid out too.
        (
            "alt text of marks",
            format!(
                "<img width=200 height=60 alt=\"a{}\">",
                "\u{0301}".repeat(100_000)
            ),
        ),
        // Devanagari consonant + virama chains form one cluster too.
        (
            "conjunct chain of marks",
            format!("<p>{}\u{0915} b</p>", "\u{0915}\u{094D}".repeat(2_000)),
        ),
    ];
    common::big_stack(move || {
        let mut engine = Engine::new();
        // Fonts load once, outside the timings.
        let warm = RenderInput {
            html: Some("<p>warm</p>".into()),
            ..Default::default()
        };
        render::layout(&warm, &Opts::default(), &mut engine, &Deadline::none()).unwrap();
        for (name, html) in cases {
            let input = RenderInput {
                html: Some(html),
                ..Default::default()
            };
            let t = Instant::now();
            let doc = render::layout(
                &input,
                &Opts::default(),
                &mut engine,
                &Deadline::after(LAYOUT_BUDGET),
            )
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            render::rasterize(&doc, &mut engine, 0, 256, 1.0).unwrap();
            let took = t.elapsed();
            eprintln!("{name}: {took:?}");
            assert!(took < limit, "{name}: took {took:?}");
            let text: usize = doc.texts.iter().map(String::len).sum();
            if name.contains("marks") {
                // The cluster keeps its first 32 characters.
                assert!(text < 200, "{name}: {text} bytes laid out");
                continue;
            }
            assert!(doc.truncated, "{name}: the cut is reported");
            assert!(
                text <= 2 * render::MAX_GROUP_TEXT,
                "{name}: {text} bytes laid out"
            );
        }
    });
}
