//! Run every cached message body through the reader's parsing code and
//! report failures and timings (no message content is printed).
//!
//! ```text
//! cargo run --example parse_cache -- <cache.sqlite3>
//! ```
//!
//! Run it in a debug build: panics are caught and counted (release builds
//! abort on panic).

use anyhow::{Context, Result};
use komorebi::db::Db;
use komorebi::mail::{html, mime};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: parse_cache <cache.sqlite3>")?;
    let db = Db::open(path.as_ref())?;
    let folders = db.folders()?;
    let ids = db.message_ids(&folders.iter().map(|f| f.id).collect::<Vec<_>>(), "")?;
    std::panic::set_hook(Box::new(|_| {}));
    let (mut bodies, mut panics, mut html_parts, mut empty) = (0, 0, 0, 0);
    let mut times = Vec::new();
    for id in ids {
        let Some(raw) = db.message(id)?.and_then(|m| m.raw) else {
            continue;
        };
        bodies += 1;
        let t = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let body = mime::parse_body(&raw);
            let _ = body.preview();
            let _ = komorebi::trust::notice(&body.auth, "example.com", "", "");
            for a in &body.attachments {
                let _ = mime::attachment(&raw, a.index);
            }
            // Also convert every HTML part directly, not just the first.
            let mut n = 0;
            if let Some(msg) = mail_parser::MessageParser::default().parse(&raw[..]) {
                for i in 0..msg.html_body.len() {
                    if let Some(h) = msg.body_html(i) {
                        let _ = html::html_to_text(&h);
                        n += 1;
                    }
                }
            }
            (body.text.is_empty(), n)
        }));
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        match result {
            Ok((is_empty, n)) => {
                html_parts += n;
                if is_empty {
                    empty += 1;
                }
            }
            Err(_) => {
                panics += 1;
                eprintln!("PANIC on message id {id}");
            }
        }
    }
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        times
            .get(((times.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "bodies {bodies}, html parts {html_parts}, empty text {empty}, panics {panics}; parse ms p50 {:.2} p95 {:.2} max {:.2}",
        pct(0.5),
        pct(0.95),
        pct(1.0)
    );
    Ok(())
}
