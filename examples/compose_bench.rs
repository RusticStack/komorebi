//! Typing latency in the composer's editor, without a window: one
//! keystroke is the proxy's edit applied to the model, the changed block
//! laid out again, the page re-placed, the caret found and the visible
//! tiles painted (what the composer does before Slint draws the frame).
//!
//! ```text
//! cargo run --release --example compose_bench [-- runs]
//! ```
//!
//! Documents: 2,000 lines with 20 tables (typing in a paragraph and in a
//! cell), and one 500 KB paragraph. Prints the median and minimum over the
//! runs of each run's median and 95th percentile, in milliseconds.

use komorebi::mail::render::Engine;
use komorebi::mail::render::canvas;
use komorebi::mail::rich::edit::Editor;
use komorebi::mail::rich::view::{Theme, View};
use komorebi::mail::rich::*;
use std::time::Instant;

const WIDTH: f32 = 720.0;
const SCALE: f32 = 1.5;
const VIEW_H: f32 = 600.0;
const KEYS: usize = 200;

fn long_doc() -> Doc {
    let mut blocks = Vec::new();
    let words = "The quick brown fox jumps over the lazy dog and keeps running";
    for i in 0..2000 {
        let mut p = Para::text(&format!("{i}: {words}"));
        if i % 7 == 0 {
            p.runs = vec![
                Run::styled(
                    format!("{i}: "),
                    Style {
                        b: true,
                        ..Style::default()
                    },
                ),
                Run::new(words),
            ];
        }
        blocks.push(Block::Para(p));
        if i % 100 == 50 {
            let mut t = Table::new(4, 4).unwrap_or_default();
            for (r, row) in t.rows.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    *cell = Cell::text(&format!("r{r} c{c}"));
                }
            }
            blocks.push(Block::Table(t));
        }
    }
    Doc {
        blocks,
        ..Doc::default()
    }
}

fn big_para() -> Doc {
    let line = "lorem ipsum dolor sit amet consectetur adipiscing elit ";
    let text: String = line.repeat(500 * 1024 / line.len());
    Doc {
        blocks: vec![Block::Para(Para::text(&text))],
        ..Doc::default()
    }
}

struct Stats {
    median: f64,
    p95: f64,
}

/// Type `KEYS` characters at byte `at` of the projection; the times.
fn run(doc: &Doc, at_line: usize, engine: &mut Engine) -> Stats {
    let mut ed = Editor::new(doc.clone());
    let mut view = View::new(WIDTH, SCALE, 1.0, Theme::default());
    view.sync(&mut ed, engine);
    let line = ed.lines().get(at_line).copied().unwrap_or(ed.lines()[0]);
    let mut caret = line.start + line.len.min(4);
    // Scroll the caret into the middle of the viewport.
    let cy = view.caret(&ed, caret).y;
    let top = (cy - VIEW_H / 2.0).max(0.0);
    let mut times = Vec::with_capacity(KEYS);
    let mut painted = std::collections::HashSet::new();
    for p in view.pieces(top, top + VIEW_H) {
        painted.insert(p.key);
    }
    for k in 0..KEYS {
        let c = if k % 10 == 9 { ' ' } else { 'x' };
        let t0 = Instant::now();
        let mut text = ed.text().to_string();
        text.insert(caret, c);
        caret = ed.apply(&text, Some(caret + 1)).expect("accepted");
        view.sync(&mut ed, engine);
        let r = view.caret(&ed, caret);
        // Paint the visible pieces not painted yet.
        for p in view.pieces(top, top + VIEW_H) {
            if painted.insert(p.key) {
                std::hint::black_box(view.paint_piece(&ed, &p, engine));
            }
        }
        std::hint::black_box(r);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    Stats {
        median: times[times.len() / 2],
        p95: times[times.len() * 95 / 100],
    }
}

fn report(name: &str, runs: &[Stats]) {
    let mut med: Vec<f64> = runs.iter().map(|s| s.median).collect();
    let mut p95: Vec<f64> = runs.iter().map(|s| s.p95).collect();
    med.sort_by(f64::total_cmp);
    p95.sort_by(f64::total_cmp);
    println!(
        "{name}: per-key median {:.2} ms (min {:.2}), p95 {:.2} ms (min {:.2}) over {} runs",
        med[med.len() / 2],
        med[0],
        p95[p95.len() / 2],
        p95[0],
        runs.len()
    );
}

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(7);
    let mut engine = Engine::new();
    let long = long_doc();
    let big = big_para();
    let t0 = Instant::now();
    let mut ed = Editor::new(long.clone());
    let mut view = View::new(WIDTH, SCALE, 1.0, Theme::default());
    view.sync(&mut ed, &mut engine);
    println!(
        "open 2,000 lines + 20 tables: {:.1} ms ({} lines, {:.0} px tall)",
        t0.elapsed().as_secs_f64() * 1000.0,
        ed.lines().len(),
        view.height()
    );
    let _ = canvas::MAX_LAYOUT_TEXT;
    let para: Vec<Stats> = (0..runs).map(|_| run(&long, 1000, &mut engine)).collect();
    report("long message, paragraph", &para);
    let cell_line = Editor::new(long.clone())
        .lines()
        .iter()
        .position(|l| l.pos.cell.is_some() && l.pos.item > 700)
        .unwrap_or(0);
    let cell: Vec<Stats> = (0..runs)
        .map(|_| run(&long, cell_line, &mut engine))
        .collect();
    report("long message, table cell", &cell);
    let big_runs: Vec<Stats> = (0..runs).map(|_| run(&big, 0, &mut engine)).collect();
    report("one 500 KB paragraph", &big_runs);
}
