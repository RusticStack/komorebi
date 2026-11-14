//! Lay out every cached HTML body with the native renderer and report, per
//! message id, how long the layout and the first screen took and whether any
//! lines of text were laid over one another. No message content is printed:
//! ids, sizes and numbers only. For checking long reply chains on a copy of a
//! real cache, offline (nothing is fetched; remote images stay blocked).
//!
//! ```text
//! cargo run --release --example render_scan -- <cache.sqlite3> [width ...]
//! ```
//!
//! Widths default to 600, 720 and 960 CSS px, at scale 1.

#[path = "shared/overlap.rs"]
mod overlap;

use anyhow::{Context, Result};
use komorebi::db::Db;
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Engine, Opts};
use std::time::Instant;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .context("usage: render_scan <cache.sqlite3> [width ...]")?;
    let mut widths: Vec<f32> = args.filter_map(|w| w.parse().ok()).collect();
    if widths.is_empty() {
        widths = vec![600.0, 720.0, 960.0];
    }
    let db = Db::open(path.as_ref())?;
    let folders = db.folders()?;
    let ids = db.message_ids(&folders.iter().map(|f| f.id).collect::<Vec<_>>(), "")?;
    let mut engine = Engine::new();
    let mut times: Vec<(f64, i64)> = Vec::new();
    let (mut html, mut with_overlap, mut failed, mut truncated) = (0, 0, 0, 0);
    println!(
        "id\thtml_bytes\twidth\tlayout_ms\tfirst_screen_ms\theight\ttruncated\tlines\toverlaps"
    );
    for id in ids {
        let Some(raw) = db.message(id)?.and_then(|m| m.raw) else {
            continue;
        };
        let (_, input) = mime::parse_reader(&raw);
        let Some(bytes) = input.html.as_ref().map(String::len) else {
            continue;
        };
        html += 1;
        let mut worst = 0.0f64;
        let mut any = false;
        for &width in &widths {
            let opts = Opts {
                width_px: width,
                ..Default::default()
            };
            let t = Instant::now();
            let doc = match render::layout(&input, &opts, &mut engine, &Deadline::none()) {
                Ok(d) => d,
                Err(e) => {
                    failed += 1;
                    println!("{id}\t{bytes}\t{width}\terror {e:?}");
                    continue;
                }
            };
            let layout_ms = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let _ = render::rasterize(
                &doc,
                &mut engine,
                0,
                (doc.height.min(800.0)).ceil() as u32,
                1.0,
            );
            let first_ms = t.elapsed().as_secs_f64() * 1000.0;
            let lines = overlap::ink_lines(&doc);
            let o = overlap::overlap_pairs(&lines).len();
            any |= o > 0;
            truncated += usize::from(doc.truncated);
            worst = worst.max(layout_ms + first_ms);
            println!(
                "{id}\t{bytes}\t{width}\t{layout_ms:.1}\t{first_ms:.1}\t{:.0}\t{}\t{}\t{o}",
                doc.height,
                doc.truncated,
                lines.len()
            );
        }
        with_overlap += usize::from(any);
        times.push((worst, id));
    }
    times.sort_by(|a, b| b.0.total_cmp(&a.0));
    let pct = |p: f64| {
        let mut v: Vec<f64> = times.iter().map(|t| t.0).collect();
        v.sort_by(f64::total_cmp);
        v.get(((v.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    eprintln!(
        "html bodies {html}; with overlapping lines {with_overlap}; layout errors {failed}; truncated layouts {truncated}; \
         layout + first screen ms (worst width) p50 {:.1} p95 {:.1} max {:.1}",
        pct(0.5),
        pct(0.95),
        pct(1.0)
    );
    let slowest: Vec<String> = times
        .iter()
        .take(10)
        .map(|(t, id)| format!("{id} ({t:.0} ms)"))
        .collect();
    eprintln!("slowest: {}", slowest.join(", "));
    Ok(())
}
