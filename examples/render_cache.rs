//! Run every cached HTML message through the reader's renderer (layout with
//! the app's 250 ms deadline, then the first 1200 px painted) and report
//! outcomes and timings. No message content is printed.
//!
//! ```text
//! cargo run --example render_cache -- <cache.sqlite3>
//! ```
//!
//! Use a debug build so panics are caught and counted, and a copy of the
//! cache (opening it migrates it).

use anyhow::{Context, Result};
use komorebi::db::Db;
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Engine, Opts};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: render_cache <cache.sqlite3>")?;
    let db = Db::open(path.as_ref())?;
    let ids = db.message_ids(&db.folders()?.iter().map(|f| f.id).collect::<Vec<_>>(), "")?;
    std::panic::set_hook(Box::new(|_| {}));
    let mut engine = Engine::new();
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let (mut times, mut heights, mut blocked) = (Vec::new(), Vec::new(), 0usize);
    for id in ids {
        let Some(raw) = db.message(id)?.and_then(|m| m.raw) else {
            continue;
        };
        let t = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let (_, input) = mime::parse_reader(&raw);
            input.html.as_ref()?;
            let opts = Opts {
                width_px: 760.0,
                ..Opts::default()
            };
            let doc = render::layout(
                &input,
                &opts,
                &mut engine,
                &Deadline::after(Duration::from_millis(250)),
            );
            Some(doc.map(|doc| {
                let painted = render::rasterize(&doc, &mut engine, 0, 1200, 1.0).is_some();
                (doc.height, doc.blocked_remote, painted)
            }))
        }));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let key = match result {
            Err(_) => {
                eprintln!("PANIC on message id {id}");
                "panic".to_string()
            }
            Ok(None) => "no html (text view)".to_string(),
            Ok(Some(Err(e))) => format!("fallback: {e:?}"),
            Ok(Some(Ok((h, b, painted)))) => {
                times.push(ms);
                heights.push(h);
                blocked += b;
                if painted {
                    "rendered"
                } else {
                    "rendered, paint failed"
                }
                .to_string()
            }
        };
        *outcomes.entry(key).or_default() += 1;
    }
    times.sort_by(f64::total_cmp);
    heights.sort_by(f32::total_cmp);
    let pct = |v: &[f64], p: f64| {
        v.get(((v.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!("outcomes: {outcomes:?}");
    println!(
        "rendered layout+paint ms: p50 {:.1}, p95 {:.1}, max {:.1}; heights px p50 {:.0}, max {:.0}; remote images blocked: {blocked}",
        pct(&times, 0.5),
        pct(&times, 0.95),
        pct(&times, 1.0),
        heights.get(heights.len() / 2).copied().unwrap_or(0.0),
        heights.last().copied().unwrap_or(0.0),
    );
    Ok(())
}
