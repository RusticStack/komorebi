//! Drawing for a caller that lays out its own text (the composer's
//! editor): parley layouts built from styled spans with this engine's
//! fonts, and a painter for one band of device pixels with the reader's
//! glyph cache. Nothing here parses HTML.

use super::layout::{Brush, Doc, Rect};
use super::paint::{self, GlyphCache};
use super::{Engine, img, style};
use parley::{
    Alignment, AlignmentOptions, FontFamily, FontStyle, FontWeight, Layout, LineHeight,
    OverflowWrap, StyleProperty,
};
use std::borrow::Cow;
use tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// The look of bytes `start..end` of a text.
#[derive(Clone, Debug, PartialEq)]
pub struct Span<'a> {
    pub start: usize,
    pub end: usize,
    /// A CSS `font-family` list.
    pub family: &'a str,
    /// CSS px.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: [u8; 4],
}

/// The longest text one layout takes; the rest is not laid out.
pub const MAX_LAYOUT_TEXT: usize = super::MAX_GROUP_TEXT;

impl Engine {
    /// Lay out one paragraph of `text` (no line breaks) in `base` with
    /// `spans` over it, wrapped at `width` CSS px and aligned.
    pub fn text_layout(
        &mut self,
        text: &str,
        base: &Span<'_>,
        spans: &[Span<'_>],
        width: f32,
        align: TextAlign,
    ) -> Layout<Brush> {
        let mut end = text.len().min(MAX_LAYOUT_TEXT);
        while end > 0 && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let text = text.get(..end).unwrap_or("");
        let mut b = self.lcx.ranged_builder(&mut self.fcx, text, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(
            base.family.to_string(),
        ))));
        b.push_default(StyleProperty::FontSize(base.size.max(1.0)));
        b.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        b.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));
        b.push_default(StyleProperty::Brush(Brush {
            color: base.color,
            link: 0,
        }));
        for s in spans {
            let r = s.start.min(end)..s.end.min(end);
            if r.is_empty() {
                continue;
            }
            if s.family != base.family {
                b.push(
                    StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(s.family.to_string()))),
                    r.clone(),
                );
            }
            if (s.size - base.size).abs() > 0.01 {
                b.push(StyleProperty::FontSize(s.size.max(1.0)), r.clone());
            }
            if s.bold {
                b.push(StyleProperty::FontWeight(FontWeight::BOLD), r.clone());
            }
            if s.italic {
                b.push(StyleProperty::FontStyle(FontStyle::Italic), r.clone());
            }
            if s.underline {
                b.push(StyleProperty::Underline(true), r.clone());
            }
            if s.strike {
                b.push(StyleProperty::Strikethrough(true), r.clone());
            }
            if s.color != base.color {
                b.push(
                    StyleProperty::Brush(Brush {
                        color: s.color,
                        link: 0,
                    }),
                    r.clone(),
                );
            }
        }
        let mut layout = b.build(text);
        let width = if width.is_finite() {
            width.max(1.0)
        } else {
            1.0
        };
        layout.break_all_lines(Some(width));
        let al = match align {
            TextAlign::Left => Alignment::Start,
            TextAlign::Center => Alignment::Center,
            TextAlign::Right => Alignment::End,
            TextAlign::Justify => Alignment::Justify,
        };
        layout.align(al, AlignmentOptions::default());
        layout
    }

    /// A painter for the band of `pm`'s height starting at device row
    /// `y_px` of the page, at `scale` device px per CSS px. `pm` is not
    /// cleared.
    pub fn band<'a>(&'a mut self, pm: &'a mut Pixmap, y_px: f32, scale: f32) -> Band<'a> {
        paint::begin(&mut self.glyphs);
        Band {
            pm,
            y_px,
            scale: super::clamp_scale(scale),
            cache: &mut self.glyphs,
        }
    }
}

pub struct Band<'a> {
    pm: &'a mut Pixmap,
    y_px: f32,
    scale: f32,
    cache: &'a mut GlyphCache,
}

impl Band<'_> {
    fn dev(&self, r: Rect) -> Rect {
        Rect {
            x: r.x * self.scale,
            y: r.y * self.scale - self.y_px,
            w: r.w * self.scale,
            h: r.h * self.scale,
        }
    }

    /// The band's top and bottom in CSS px.
    pub fn span_css(&self) -> (f32, f32) {
        let top = self.y_px / self.scale;
        (top, top + self.pm.height() as f32 / self.scale)
    }

    pub fn fill(&mut self, r: Rect, color: [u8; 4], radius: f32) {
        let d = self.dev(r);
        paint::fill(self.pm, d, color, radius * self.scale);
    }

    /// A laid-out text with its top left at (`x`, `y`) CSS px.
    pub fn text(&mut self, l: &Layout<Brush>, x: f32, y: f32) {
        let (ox, oy) = (x * self.scale, y * self.scale - self.y_px);
        paint::text(l, self.pm, ox, oy, self.scale, self.cache);
    }

    /// A picture stretched to `r` (CSS px).
    pub fn image(&mut self, src: &Pixmap, r: Rect) {
        let d = self.dev(r);
        paint::image(self.pm, src, d);
    }

    /// A rendered document with its top left at (`x`, `y`), without its
    /// canvas colour.
    pub fn doc(&mut self, doc: &Doc, x: f32, y: f32) {
        paint::render_at(doc, self.pm, x, y, self.y_px, self.scale, self.cache);
    }
}

/// A PNG or JPEG decoded, scaled down towards `target_w` device px.
pub fn decode_image(bytes: &[u8], target_w: u32) -> Option<Pixmap> {
    let mut budget = img::MAX_PIXELS;
    img::decode(bytes, target_w, &mut budget).map(|d| d.pix)
}

/// Text colour `fg` (chosen for a white page) as it should be drawn on
/// `bg`: unchanged where it reads well enough, else moved in lightness as
/// the reader dims mail for the dark theme.
pub fn readable(fg: [u8; 4], bg: [u8; 4]) -> [u8; 4] {
    if style::contrast(fg, bg) >= 4.5 {
        return fg;
    }
    style::dim_text(fg, style::WHITE, bg)
}
