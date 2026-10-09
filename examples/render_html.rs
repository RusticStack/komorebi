//! Render an `.eml` file's HTML body to a PNG with the native renderer, for
//! visual review (for example against reference renders of the synthetic
//! corpus in `tests/fixtures/html/`). Not a test; nothing is fetched.
//!
//! ```text
//! cargo run --release --example render_html -- <message.eml> <out.png> [width] [scale]
//! ```

use anyhow::{Context, Result, bail};
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Engine, Opts};
use std::io::Write;
use std::time::Instant;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(eml), Some(out)) = (args.next(), args.next()) else {
        bail!("usage: render_html <message.eml> <out.png> [width] [scale]");
    };
    let width: f32 = args.next().map_or(Ok(600.0), |w| w.parse())?;
    let scale: f32 = args.next().map_or(Ok(1.0), |s| s.parse())?;
    let raw = std::fs::read(&eml).with_context(|| format!("reading {eml}"))?;
    let input = mime::parse_render_input(&raw);
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
        "{eml}: {}x{} css px (full {:.0}{}), layout {layout_ms:.1} ms, paint {paint_ms:.1} ms, {} links, {} remote blocked, {} trackers, {} images",
        doc.width,
        doc.height,
        doc.full_height,
        if doc.truncated { ", truncated" } else { "" },
        doc.links.len().saturating_sub(1),
        doc.blocked_remote,
        doc.trackers,
        doc.loaded.len(),
    )?;
    Ok(())
}
