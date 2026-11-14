//! Shared helpers for the HTML renderer tests: load a synthetic corpus
//! message (`tests/fixtures/html/`) into a `RenderInput`.

#![allow(dead_code)]

use komorebi::mail::mime;
use komorebi::mail::render::RenderInput;
use std::path::PathBuf;

pub const FILES: [&str; 14] = [
    "01-mailchimp-campaign",
    "02-substack-post",
    "03-github-review",
    "04-order-receipt",
    "05-bank-statement",
    "06-outlook-word",
    "07-gmail-reply-chain",
    "08-dark-marketing",
    "09-image-heavy-promo",
    "10-transactional-buttons",
    "11-newsletter-2col",
    "12-personal-applemail",
    "13-calendar-invite",
    "14-adversarial-malformed",
];

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/html")
        .join(name)
}

pub fn raw(name: &str) -> Vec<u8> {
    std::fs::read(fixture(&format!("{name}.eml"))).unwrap()
}

/// What the reader would hand the renderer for this message.
pub fn input(raw: &[u8]) -> RenderInput {
    mime::parse_render_input(raw)
}

/// Run `f` on a thread with the render worker's 8 MiB stack.
pub fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Hashes of what a layout draws and says: the pixels (rasterized in
/// 256 px bands), the text lines and the geometry. Two layouts of the same
/// message that must be identical have equal hashes.
pub fn doc_hashes(
    doc: &komorebi::mail::render::Doc,
    engine: &mut komorebi::mail::render::Engine,
    all_bands: bool,
) -> (u64, u64, u64) {
    use komorebi::mail::render;
    use std::hash::{DefaultHasher, Hash, Hasher};
    const BAND: u32 = 256;
    let mut px = DefaultHasher::new();
    let h = doc.height.max(0.0) as u32;
    let mut y = 0;
    while y < h {
        let band = BAND.min(h - y);
        // Without `all_bands`: the first and last 8 bands.
        let edge = y < 8 * BAND || y + 8 * BAND >= h;
        if !all_bands && !edge {
            y += BAND;
            continue;
        }
        if let Some(pm) = render::rasterize(doc, engine, y, band, 1.0) {
            pm.data().hash(&mut px);
        }
        y += BAND;
    }
    let mut text = DefaultHasher::new();
    for l in doc.text_lines() {
        l.text.hash(&mut text);
    }
    let mut geo = DefaultHasher::new();
    doc.items.len().hash(&mut geo);
    doc.layouts.len().hash(&mut geo);
    doc.height.to_bits().hash(&mut geo);
    doc.width.to_bits().hash(&mut geo);
    doc.natural_width.to_bits().hash(&mut geo);
    doc.links.len().hash(&mut geo);
    doc.images.len().hash(&mut geo);
    for t in &doc.texts {
        t.hash(&mut geo);
    }
    (px.finish(), text.finish(), geo.finish())
}

/// The open-time corpus: the reader fixtures and the HTML corpus.
pub fn open_corpus() -> Vec<(String, RenderInput)> {
    let mut v = vec![];
    for name in [
        "01-emoji",
        "02-heavy-newsletter",
        "03-heavy-digest",
        "04-heavy-statement",
    ] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reader")
            .join(format!("{name}.eml"));
        let raw = std::fs::read(path).unwrap();
        v.push((format!("reader/{name}"), mime::parse_reader(&raw).1));
    }
    for name in FILES {
        v.push((format!("html/{name}"), input(&raw(name))));
    }
    v
}
