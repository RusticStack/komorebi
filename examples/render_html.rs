//! Render an `.eml` file's HTML body to a PNG with the native renderer, for
//! visual review (for example against reference renders of the synthetic
//! corpus in `tests/fixtures/html/`). Not a test; nothing is fetched.
//!
//! ```text
//! cargo run --release --example render_html -- <message.eml> <out.png> [width] [scale]
//! cargo run --release --example render_html -- --cache <cache.sqlite3> <id> <out.png> [width] [scale] [--html <body.html>]
//! ```
//!
//! With `--cache`, the body comes from a message in a cache file, by id;
//! `--html` also writes that body out, for rendering it in a browser. The
//! output names the lines drawn over one another by position only, never
//! their text.

#[path = "shared/overlap.rs"]
mod overlap;

use anyhow::{Context, Result, bail};
use komorebi::db::Db;
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Engine, Opts, RenderInput};
use std::io::Write;
use std::time::Instant;

fn from_cache(path: &str, id: i64) -> Result<RenderInput> {
    let db = Db::open(path.as_ref())?;
    let raw = db
        .message(id)?
        .and_then(|m| m.raw)
        .with_context(|| format!("no cached body for id {id}"))?;
    Ok(mime::parse_reader(&raw).1)
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let html_out = match args.iter().position(|a| a == "--html") {
        Some(i) if i + 1 < args.len() => {
            let p = args.remove(i + 1);
            args.remove(i);
            Some(p)
        }
        _ => None,
    };
    let (input, name, mut rest) = if args.first().is_some_and(|a| a == "--cache") {
        let (Some(db), Some(id)) = (args.get(1), args.get(2)) else {
            bail!("usage: render_html --cache <cache.sqlite3> <id> <out.png> [width] [scale]");
        };
        let id: i64 = id.parse()?;
        (
            from_cache(db, id)?,
            format!("id {id}"),
            args.into_iter().skip(3),
        )
    } else {
        let Some(eml) = args.first().cloned() else {
            bail!("usage: render_html <message.eml> <out.png> [width] [scale]");
        };
        let raw = std::fs::read(&eml).with_context(|| format!("reading {eml}"))?;
        // A bare `.html` file is the body as it stands, for comparing against a
        // browser's rendering of the same file.
        let input = if eml.ends_with(".html") || eml.ends_with(".htm") {
            RenderInput {
                html: Some(String::from_utf8_lossy(&raw).into_owned()),
                ..Default::default()
            }
        } else {
            mime::parse_render_input(&raw)
        };
        (input, eml, args.into_iter().skip(1))
    };
    let Some(out) = rest.next() else {
        bail!("missing <out.png>");
    };
    let width: f32 = rest.next().map_or(Ok(600.0), |w| w.parse())?;
    let scale: f32 = rest.next().map_or(Ok(1.0), |s| s.parse())?;
    if let Some(p) = &html_out {
        std::fs::write(p, input.html.as_deref().unwrap_or_default())?;
    }
    let mut engine = Engine::new();
    let opts = Opts {
        width_px: width,
        scale,
        ..Default::default()
    };
    let t = Instant::now();
    let doc = render::layout(&input, &opts, &mut engine, &Deadline::none())
        .map_err(|e| anyhow::anyhow!("render failed: {e:?}"))?;
    let layout_ms = t.elapsed().as_secs_f64() * 1000.0;
    let h = (doc.height * scale).ceil() as u32;
    let width_px = (doc.width * scale).round() as u32;
    let file = std::io::BufWriter::new(std::fs::File::create(&out)?);
    let mut enc = png::Encoder::new(file, width_px, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    let mut sw = w.stream_writer()?;
    // Rasterize in bands, as the reader does with tiles. The canvas is
    // opaque, so premultiplied equals straight alpha.
    let mut paint_ms = 0.0;
    for y in (0..h).step_by(2048) {
        let rows = (h - y).min(2048);
        let t = Instant::now();
        let pm = render::rasterize(&doc, &mut engine, y, rows, scale).context("rasterize")?;
        paint_ms += t.elapsed().as_secs_f64() * 1000.0;
        sw.write_all(pm.data())?;
    }
    sw.finish()?;
    let mut so = std::io::stdout().lock();
    writeln!(
        so,
        "{name}: {}x{} css px (full {:.0}{}), layout {layout_ms:.1} ms, paint {paint_ms:.1} ms, {} links, {} remote blocked, {} trackers, {} images",
        doc.width,
        doc.height,
        doc.full_height,
        if doc.truncated { ", truncated" } else { "" },
        doc.links.len().saturating_sub(1),
        doc.blocked_remote,
        doc.trackers,
        doc.loaded.len(),
    )?;
    let pairs = overlap::overlap_pairs(&overlap::ink_lines(&doc));
    writeln!(so, "overlapping line pairs: {}", pairs.len())?;
    for (a, b) in &pairs {
        writeln!(
            so,
            "  item {} line {} at ({:.1},{:.1}) {:.1}x{:.1}  over  item {} line {} at ({:.1},{:.1}) {:.1}x{:.1}",
            a.item, a.line, a.x, a.y, a.w, a.h, b.item, b.line, b.x, b.y, b.w, b.h
        )?;
    }
    Ok(())
}
