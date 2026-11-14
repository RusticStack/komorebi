//! Reader layout timing and a pixel-hash guard on the open-time corpus
//! (`tests/fixtures/reader` and `tests/fixtures/html`).
//!
//! Every test here is `#[ignore]`d: they print timings and hashes for
//! `cargo test --release --test render_bench -- --ignored --nocapture`.
//!
//! * `layout_timing` lays the heavy fixtures out the way the reader worker
//!   does (one warm [`Engine`], no time budget so slow builds still finish)
//!   and prints the cold and the median-of-N time, for one layout and for
//!   the two-pass open the reader does when a table is wider than the
//!   column (`KOMOREBI_BENCH_RUNS` sets N).
//! * `pixel_hashes` rasterizes every fixture at two widths and prints a hash
//!   of the pixels, of the line text and of the geometry. An optimization
//!   must keep these identical. They depend on the installed fonts, so they
//!   are compared between two builds on one machine:
//!   `KOMOREBI_RENDER_HASHES=<file>` writes them on the first run and asserts
//!   equality on the next.

mod common;

use common::{big_stack, doc_hashes, open_corpus};
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Doc, Engine, NoRemote, Opts, RenderInput, Reuse};
use std::path::PathBuf;
use std::time::Instant;

const HEAVY: [&str; 3] = [
    "04-heavy-statement",
    "02-heavy-newsletter",
    "03-heavy-digest",
];

/// The reader column of the 1280 px window, and the width the 700 px
/// statement table needs.
const COLUMN: f32 = 600.0;
const NATURAL: f32 = 708.0;

fn reader_input(name: &str) -> RenderInput {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reader")
        .join(format!("{name}.eml"));
    mime::parse_reader(&std::fs::read(path).unwrap()).1
}

fn opts(width_px: f32) -> Opts {
    Opts {
        width_px,
        ..Default::default()
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The reader worker's open: the column, then (a wide table) the natural
/// width, the second pass reusing the first.
fn open(input: &RenderInput, engine: &mut Engine) -> Doc {
    let none = Deadline::none();
    let mut reuse = Reuse::new(input);
    let first = render::layout_reusing(
        &mut reuse,
        &opts(COLUMN),
        engine,
        &none,
        &mut NoRemote,
        None,
    )
    .unwrap();
    if first.natural_width <= COLUMN + 4.0 {
        return first;
    }
    render::layout_reusing(
        &mut reuse,
        &opts(NATURAL),
        engine,
        &none,
        &mut NoRemote,
        std::env::var_os("KOMOREBI_BENCH_NOREUSE")
            .is_none()
            .then_some(&first),
    )
    .unwrap()
}

fn median_ms(runs: usize, mut f: impl FnMut()) -> (f64, f64) {
    let mut v: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            ms(t)
        })
        .collect();
    v.sort_by(f64::total_cmp);
    (v[v.len() / 2], v[0])
}

#[test]
#[ignore = "timing; run with --release --ignored --nocapture"]
fn layout_timing() {
    big_stack(|| {
        let runs: usize = std::env::var("KOMOREBI_BENCH_RUNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);
        for name in HEAVY {
            let input = reader_input(name);
            let mut engine = Engine::new();
            let t = Instant::now();
            drop(open(&input, &mut engine));
            let cold = ms(t);
            let (one, one_min) = median_ms(runs, || {
                drop(render::layout(
                    &input,
                    &opts(COLUMN),
                    &mut engine,
                    &Deadline::none(),
                ))
            });
            let (two, two_min) = median_ms(runs, || drop(open(&input, &mut engine)));
            // A resize: the column changes, the message does not.
            let first =
                render::layout(&input, &opts(COLUMN), &mut engine, &Deadline::none()).unwrap();
            let (resize, resize_min) = median_ms(runs, || {
                let mut reuse = Reuse::new(&input);
                let prev = std::env::var_os("KOMOREBI_BENCH_NOREUSE")
                    .is_none()
                    .then_some(&first);
                drop(render::layout_reusing(
                    &mut reuse,
                    &opts(560.0),
                    &mut engine,
                    &Deadline::none(),
                    &mut NoRemote,
                    prev,
                ))
            });
            println!(
                "{name}: cold open {cold:.1} ms; one layout median {one:.1} (min {one_min:.1}); \
                 open median {two:.1} (min {two_min:.1}); \n                 resize relayout median {resize:.1} (min {resize_min:.1}) ms, n={runs}"
            );
        }
    });
}

#[test]
#[ignore = "prints hashes; run with --release --ignored --nocapture"]
fn pixel_hashes() {
    let out = big_stack(|| {
        let mut engine = Engine::new();
        let mut out = String::new();
        for (name, input) in &open_corpus() {
            for w in [COLUMN, NATURAL] {
                let Ok(doc) = render::layout(input, &opts(w), &mut engine, &Deadline::none())
                else {
                    out.push_str(&format!("{name} {w}: error\n"));
                    continue;
                };
                let (px, text, geo) = doc_hashes(&doc, &mut engine, true);
                out.push_str(&format!(
                    "{name} {w}: px {px:016x} text {text:016x} geo {geo:016x}\n"
                ));
            }
        }
        out
    });
    print!("{out}");
    if let Ok(path) = std::env::var("KOMOREBI_RENDER_HASHES") {
        match std::fs::read_to_string(&path) {
            Ok(old) => assert_eq!(old, out, "render output changed against {path}"),
            Err(_) => std::fs::write(&path, &out).unwrap(),
        }
    }
}
