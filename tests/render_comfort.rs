//! Reader comfort on the renderer: colour emoji, dimmed colours in the
//! dark theme, and the per-group text cap, on the synthetic fixtures in
//! `tests/fixtures/reader` and inline HTML. Pixel checks are coarse (hues
//! and lightness), since system fonts change between Windows builds.

mod common;

use common::big_stack;
use komorebi::mail::mime;
use komorebi::mail::render::{self, Deadline, Doc, Engine, Opts, RenderInput};
use parley::PositionedLayoutItem;
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{MetadataProvider, string::StringId};
use std::path::PathBuf;

fn reader_fixture(name: &str) -> RenderInput {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reader")
        .join(format!("{name}.eml"));
    mime::parse_render_input(&std::fs::read(path).unwrap())
}

fn html(body: &str) -> RenderInput {
    RenderInput {
        html: Some(body.to_string()),
        ..Default::default()
    }
}

fn lay_out(input: &RenderInput, opts: &Opts) -> Doc {
    render::layout(input, opts, &mut Engine::new(), &Deadline::none()).unwrap()
}

fn emoji_font_installed() -> bool {
    let fonts = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts"));
    fonts.is_some_and(|f| f.join("seguiemj.ttf").exists())
}

/// Family names of the fonts that shape `needle` in `doc`.
fn fonts_for(doc: &Doc, needle: char) -> Vec<String> {
    let mut out = vec![];
    for (l, text) in doc.layouts.iter().zip(&doc.texts) {
        for line in l.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                if !text
                    .get(run.text_range())
                    .is_some_and(|t| t.contains(needle))
                {
                    continue;
                }
                let font = run.font();
                let f = FontRef::from_index(font.data.as_ref(), font.index).unwrap();
                let name = f
                    .localized_strings(StringId::FAMILY_NAME)
                    .english_or_first()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                assert!(f.colr().is_ok() || !name.contains("Emoji"));
                out.push(name);
            }
        }
    }
    out
}

/// Distinct hues (30-degree buckets) with at least 20 strongly coloured
/// pixels in an RGBA buffer.
fn hues(px: &[u8]) -> usize {
    let mut buckets = [0u32; 12];
    for p in px.chunks_exact(4) {
        let (r, g, b) = (f32::from(p[0]), f32::from(p[1]), f32::from(p[2]));
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        if max - min < 60.0 {
            continue;
        }
        let d = max - min;
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
        buckets[((h / 30.0) as usize).min(11)] += 1;
    }
    buckets.iter().filter(|&&n| n >= 20).count()
}

#[test]
fn emoji_fall_back_to_segoe_ui_emoji_and_paint_in_colour() {
    if !emoji_font_installed() {
        eprintln!("Segoe UI Emoji is not installed; skipped");
        return;
    }
    big_stack(|| {
        let mut engine = Engine::new();
        let input = reader_fixture("01-emoji");
        let doc = render::layout(&input, &Opts::default(), &mut engine, &Deadline::none()).unwrap();
        // Text in Segoe UI and Georgia alike falls back to the emoji font.
        for c in ['\u{1F3E0}', '\u{1F4E6}'] {
            let fonts = fonts_for(&doc, c);
            assert!(
                !fonts.is_empty() && fonts.iter().all(|f| f == "Segoe UI Emoji"),
                "{c}: {fonts:?}"
            );
        }
        // The 48 px row of three emoji: more than one hue.
        let pm = render::rasterize(&doc, &mut engine, 0, 90, 1.0).unwrap();
        assert!(hues(pm.data()) >= 3, "hues: {}", hues(pm.data()));
        // Plain black text paints no hue at all.
        let plain = html("<p style='font-size:48px'>Komorebi</p>");
        let doc = render::layout(&plain, &Opts::default(), &mut engine, &Deadline::none()).unwrap();
        let pm = render::rasterize(&doc, &mut engine, 0, 90, 1.0).unwrap();
        assert_eq!(hues(pm.data()), 0);
        // Colour glyphs are cached within the glyph cache's budget.
        assert!(engine_bytes(&mut engine) <= render::MAX_GLYPH_CACHE_BYTES * 2);
    });
}

fn engine_bytes(engine: &mut Engine) -> usize {
    engine.glyph_cache_bytes()
}

#[test]
fn glyph_cache_stays_within_its_budget() {
    big_stack(|| {
        let mut engine = Engine::new();
        // Many sizes of many glyphs: far more masks than the budget holds.
        let mut body = String::new();
        for size in 8..80 {
            body.push_str(&format!(
                "<p style='font-size:{size}px'>The quick brown fox jumps over the lazy dog 0123456789 ÀÉÎÕÜ</p>"
            ));
        }
        let doc = render::layout(
            &html(&body),
            &Opts::default(),
            &mut engine,
            &Deadline::none(),
        )
        .unwrap();
        let mut y = 0;
        while (y as f32) < doc.height * 2.0 {
            render::rasterize(&doc, &mut engine, y, 256, 2.0).unwrap();
            // Trimmed before each band; one band may add a little.
            assert!(
                engine_bytes(&mut engine) <= render::MAX_GLYPH_CACHE_BYTES * 2,
                "{} bytes",
                engine_bytes(&mut engine)
            );
            y += 256;
        }
    });
}

#[test]
fn dark_theme_dims_light_mail_but_not_mail_with_dark_rules() {
    big_stack(|| {
        let dim = Opts {
            dim: true,
            ..Default::default()
        };
        // A plain light message: dark paper, light text, same geometry.
        let light = reader_fixture("03-heavy-digest");
        let plain = lay_out(&light, &Opts::default());
        let dimmed = lay_out(&light, &dim);
        assert!(!plain.dimmed && dimmed.dimmed);
        assert_eq!(plain.bg, [255; 4]);
        assert!(dimmed.bg[..3].iter().all(|&c| c < 70), "{:?}", dimmed.bg);
        assert_eq!(plain.height, dimmed.height);
        let mut engine = Engine::new();
        let pm = render::rasterize(&dimmed, &mut engine, 0, 256, 1.0).unwrap();
        let mean = pm
            .data()
            .chunks_exact(4)
            .map(|p| u32::from(p[1]))
            .sum::<u32>()
            / (pm.width() * pm.height());
        assert!(mean < 90, "mean green {mean}");
        for it in &dimmed.items {
            if let render::Item::Text { layout, .. } = it {
                for line in dimmed.layouts[*layout].lines() {
                    for item in line.items() {
                        if let PositionedLayoutItem::GlyphRun(gr) = item {
                            let c = gr.style().brush.color;
                            let ratio = contrast(c, dimmed.bg);
                            assert!(ratio >= 4.5, "{c:?} on {:?}: {ratio:.2}", dimmed.bg);
                        }
                    }
                }
            }
        }
        // Mail with dark rules of its own keeps its colours.
        let input = mime::parse_render_input(&common::raw("08-dark-marketing"));
        let doc = lay_out(&input, &dim);
        assert!(doc.has_dark_rules && !doc.dimmed);
        assert_eq!(doc.bg, lay_out(&input, &Opts::default()).bg);
    });
}

/// WCAG contrast of two colours.
fn contrast(a: [u8; 4], b: [u8; 4]) -> f32 {
    let lum = |c: [u8; 4]| {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
