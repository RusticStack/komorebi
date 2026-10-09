//! Text selection over a laid-out [`Doc`]: hit testing, word, paragraph and
//! whole-document selection, highlight rectangles and the selected text.
//!
//! A selection runs between two [`TextPos`]itions in display-list order,
//! across as many text layouts as it spans. Hidden text (transparent or
//! near-zero size, such as preheaders) is never selectable and never copied.

use super::layout::{Brush, Doc, Item, Rect};
use parley::{Affinity, Cursor, Layout, PositionedLayoutItem, Selection};
use std::ops::Range;

/// A caret position: a text item of [`Doc::items`] and a byte offset into
/// its layout's text. Ordered as the document reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub item: usize,
    pub index: usize,
}

/// A selectable text layout placed in the page.
struct Block<'a> {
    item: usize,
    x: f32,
    y: f32,
    layout: &'a Layout<Brush>,
    text: &'a str,
}

/// Whether a glyph run is drawn invisibly (see [`Doc::text_lines`]).
fn hidden(brush: &Brush, font_size: f32) -> bool {
    brush.color[3] == 0 || font_size < 0.5
}

impl Block<'_> {
    /// Byte ranges of text that is not drawn visibly, in text order.
    fn hidden_ranges(&self) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for line in self.layout.lines() {
            for run in line.runs() {
                for cluster in run.clusters() {
                    if !hidden(&cluster.first_style().brush, run.font_size()) {
                        continue;
                    }
                    let r = cluster.text_range();
                    match out.last_mut() {
                        Some(last) if last.end == r.start => last.end = r.end,
                        _ => out.push(r),
                    }
                }
            }
        }
        out.sort_by_key(|r| r.start);
        out
    }

    /// Whether any glyph of the layout is drawn visibly.
    fn visible(&self) -> bool {
        self.layout.lines().any(|line| {
            line.items().any(|item| match item {
                PositionedLayoutItem::GlyphRun(gr) => {
                    !hidden(&gr.style().brush, gr.run().font_size())
                }
                PositionedLayoutItem::InlineBox(_) => false,
            })
        })
    }

    /// Left and right edges of the drawn lines, in page coordinates.
    fn x_span(&self) -> (f32, f32) {
        let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
        for line in self.layout.lines() {
            let m = line.metrics();
            x0 = x0.min(m.offset);
            x1 = x1.max(m.offset + m.advance);
        }
        if x0 > x1 {
            (self.x, self.x)
        } else {
            (self.x + x0, self.x + x1)
        }
    }

    fn y_span(&self) -> (f32, f32) {
        (self.y, self.y + self.layout.height())
    }

    fn end(&self) -> usize {
        self.text.len()
    }

    /// The visible part of `range` of the text.
    fn visible_text(&self, range: Range<usize>, out: &mut String) {
        let mut at = range.start;
        let hidden = self.hidden_ranges();
        let cuts = hidden
            .iter()
            .filter(|h| h.start < range.end && h.end > range.start);
        for h in cuts {
            if h.start > at {
                out.push_str(self.text.get(at..h.start).unwrap_or(""));
            }
            at = at.max(h.end);
        }
        if at < range.end {
            out.push_str(self.text.get(at..range.end).unwrap_or(""));
        }
    }
}

/// Distance from `v` to the interval `lo..hi` (0 inside).
fn gap(v: f32, (lo, hi): (f32, f32)) -> f32 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0.0
    }
}

impl Doc {
    /// The visible text layouts within the page, in display-list order.
    fn blocks(&self) -> impl Iterator<Item = Block<'_>> {
        self.items.iter().enumerate().filter_map(|(item, it)| {
            let Item::Text {
                layout,
                x,
                y,
                selectable: true,
                ..
            } = it
            else {
                return None;
            };
            if !y.is_finite() || *y >= self.height {
                return None;
            }
            let block = Block {
                item,
                x: *x,
                y: *y,
                layout: self.layouts.get(*layout)?,
                text: self.texts.get(*layout)?,
            };
            block.visible().then_some(block)
        })
    }

    /// The block nearest `(x, y)`: vertical distance first, then horizontal;
    /// the later block wins a tie, as it is drawn on top. Returns the
    /// vertical distance too.
    fn nearest(&self, x: f32, y: f32) -> Option<(Block<'_>, f32)> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let mut best: Option<(Block<'_>, f32, f32)> = None;
        for b in self.blocks() {
            let (dy, dx) = (gap(y, b.y_span()), gap(x, b.x_span()));
            if best
                .as_ref()
                .is_none_or(|(_, by, bx)| (dy, dx) <= (*by, *bx))
            {
                best = Some((b, dy, dx));
            }
        }
        best.map(|(b, dy, _)| (b, dy))
    }

    /// The caret position for a point in page coordinates. A point between
    /// layouts goes to the end of the one above or the start of the one
    /// below, whichever is nearer, so drags through margins behave.
    pub fn text_at(&self, x: f32, y: f32) -> Option<TextPos> {
        let (b, dy) = self.nearest(x, y)?;
        let index = if dy > 0.0 {
            if y < b.y { 0 } else { b.end() }
        } else {
            Cursor::from_point(b.layout, x - b.x, y - b.y).index()
        };
        Some(TextPos {
            item: b.item,
            index: index.min(b.end()),
        })
    }

    /// The word at a point (double-click).
    pub fn word_at(&self, x: f32, y: f32) -> Option<(TextPos, TextPos)> {
        let (b, _) = self.nearest(x, y)?;
        let range = Selection::word_from_point(b.layout, x - b.x, y - b.y).text_range();
        let pos = |index: usize| TextPos {
            item: b.item,
            index: index.min(b.end()),
        };
        Some((pos(range.start), pos(range.end)))
    }

    /// The paragraph (text layout) at a point (triple-click).
    pub fn block_at(&self, x: f32, y: f32) -> Option<(TextPos, TextPos)> {
        let (b, _) = self.nearest(x, y)?;
        Some((
            TextPos {
                item: b.item,
                index: 0,
            },
            TextPos {
                item: b.item,
                index: b.end(),
            },
        ))
    }

    /// The whole selectable text (Ctrl+A).
    pub fn text_bounds(&self) -> Option<(TextPos, TextPos)> {
        let mut blocks = self.blocks();
        let first = blocks.next()?;
        let start = TextPos {
            item: first.item,
            index: 0,
        };
        let last = blocks.last().unwrap_or(first);
        Some((
            start,
            TextPos {
                item: last.item,
                index: last.end(),
            },
        ))
    }

    /// The blocks from `a` to `b` (either order) with the byte range of
    /// each that is selected; empty ranges are left out.
    fn spans(&self, a: TextPos, b: TextPos) -> impl Iterator<Item = (Block<'_>, Range<usize>)> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        self.blocks()
            .skip_while(move |blk| blk.item < a.item)
            .take_while(move |blk| blk.item <= b.item)
            .filter_map(move |blk| {
                let start = if blk.item == a.item { a.index } else { 0 };
                let end = if blk.item == b.item {
                    b.index
                } else {
                    blk.end()
                };
                let end = end.min(blk.end());
                (start < end).then_some((blk, start..end))
            })
    }

    /// Highlight rectangles of the selection between `a` and `b`, in page
    /// coordinates, one per line of each layout.
    pub fn selection_rects(&self, a: TextPos, b: TextPos) -> Vec<Rect> {
        let mut out = Vec::new();
        for (blk, range) in self.spans(a, b) {
            let sel = Selection::new(
                Cursor::from_byte_index(blk.layout, range.start, Affinity::Downstream),
                Cursor::from_byte_index(blk.layout, range.end, Affinity::Upstream),
            );
            sel.geometry_with(blk.layout, |bb, _| {
                let r = Rect {
                    x: blk.x + bb.x0 as f32,
                    y: blk.y + bb.y0 as f32,
                    w: (bb.x1 - bb.x0) as f32,
                    h: (bb.y1 - bb.y0) as f32,
                };
                if r.w > 0.0 && r.h > 0.0 && r.y < self.height {
                    out.push(r);
                }
            });
        }
        out
    }

    /// The visible text between `a` and `b`; layouts are separated by a
    /// line break.
    pub fn selection_text(&self, a: TextPos, b: TextPos) -> String {
        let mut out = String::new();
        for (blk, range) in self.spans(a, b) {
            let len = out.len();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            let mark = out.len();
            blk.visible_text(range, &mut out);
            if out.len() == mark {
                out.truncate(len);
            }
        }
        out
    }
}
