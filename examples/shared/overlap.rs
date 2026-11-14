//! Lines of text drawn over one another, as a reader would see them: only
//! glyphs that are painted count (not transparent, not too small to draw,
//! not cut off by a clipping box), and only where they actually are on the
//! line. Shared by the `render_scan` and `render_html` examples. Reports
//! geometry only, never text.

use komorebi::mail::render::{Doc, Item};
use parley::PositionedLayoutItem;

/// The painted part of one line of text.
#[derive(Clone, Copy, Debug)]
// `render_scan` counts pairs only; `render_html` prints where they are.
#[allow(dead_code)]
pub struct Ink {
    /// Index of the text item in [`Doc::items`].
    pub item: usize,
    /// Line number within that item.
    pub line: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Every line with visible text, with the horizontal extent of its painted
/// glyphs and its line box height.
pub fn ink_lines(doc: &Doc) -> Vec<Ink> {
    let mut out = Vec::new();
    for (index, it) in doc.items.iter().enumerate() {
        let Item::Text {
            layout,
            x,
            y,
            clip_h,
            ..
        } = it
        else {
            continue;
        };
        let (Some(l), Some(text)) = (doc.layouts.get(*layout), doc.texts.get(*layout)) else {
            continue;
        };
        for (n, line) in l.lines().enumerate() {
            let m = line.metrics();
            // The painter stops at the first line below the clip.
            if m.block_max_coord > clip_h + 1.0 {
                break;
            }
            let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                if gr.style().brush.color[3] == 0 || run.font_size() < 0.5 {
                    continue;
                }
                let visible = text
                    .get(run.text_range())
                    .is_some_and(|t| t.chars().any(|c| !c.is_whitespace()));
                if !visible {
                    continue;
                }
                x0 = x0.min(gr.offset());
                x1 = x1.max(gr.offset() + gr.advance());
            }
            if x1 > x0 {
                out.push(Ink {
                    item: index,
                    line: n,
                    x: x + x0,
                    y: y + m.block_min_coord,
                    w: x1 - x0,
                    h: m.block_max_coord - m.block_min_coord,
                });
            }
        }
    }
    out
}

/// Pairs of painted lines that cross by more than half a line each way and
/// overlap sideways: text drawn over text.
pub fn overlap_pairs(lines: &[Ink]) -> Vec<(Ink, Ink)> {
    let mut sorted = lines.to_vec();
    sorted.sort_by(|a, b| a.y.total_cmp(&b.y));
    let mut out = Vec::new();
    for (i, a) in sorted.iter().enumerate() {
        for b in sorted.iter().skip(i + 1) {
            if b.y > a.y + a.h {
                break;
            }
            let dy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            let dx = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            if dy > 0.5 * a.h.min(b.h) && dx > 1.0 {
                out.push((*a, *b));
            }
        }
    }
    out
}
