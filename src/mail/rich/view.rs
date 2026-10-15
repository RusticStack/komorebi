//! The editor's picture: every top-level block of an [`Editor`]'s document
//! laid out with the reader's text engine, kept until an edit replaces
//! that block ([`Editor::take_changes`]), and placed one under the other.
//! Each block is painted on pieces of its own ([`View::pieces`]): an edit
//! repaints only the block it changed, and the blocks below just move.
//! Hit testing, the caret, the selection and moving up and down a line
//! all come from the same layouts.
//!
//! A paragraph longer than [`CHUNK_MAX`] is laid out in chunks cut at
//! spaces chosen by their content, so typing in it shapes one chunk again.
//!
//! A stored quote is HTML: it is laid out by the full renderer off the UI
//! thread (its layout recurses); until it arrives, a placeholder of
//! [`QUOTE_WAIT_H`] holds its place ([`View::quote_jobs`], [`View::set_quote`]).

use super::edit::{Editor, Flow, Line, Pos};
use super::model::*;
use crate::mail::render::canvas::{self, Band, Span, TextAlign};
use crate::mail::render::{self, Brush, Engine, Rect};
use parley::{Affinity, Cursor, Layout, Selection};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use tiny_skia::Pixmap;

/// CSS px per point.
const PX_PER_PT: f32 = 96.0 / 72.0;
/// Indent of one list level, of one indent step and of one quote level.
const LIST_STEP: f32 = 28.0;
const INDENT_STEP: f32 = 40.0;
const QUOTE_STEP: f32 = 14.0;
/// A table cell's padding (as sent: 4 px 8 px).
const CELL_PAD_X: f32 = 8.0;
const CELL_PAD_Y: f32 = 4.0;
/// Space above the stored quote (as sent: 1em).
const QUOTE_GAP: f32 = 14.0;
/// The stored quote's height until its layout arrives.
pub const QUOTE_WAIT_H: f32 = 48.0;
/// Space above a signature's logo.
const LOGO_GAP: f32 = 6.0;
/// The caret's width.
pub const CARET_W: f32 = 1.5;
/// The width shown for a selected line break.
const BREAK_W: f32 = 6.0;
/// A paragraph is laid out in chunks of about this many bytes past it.
pub const CHUNK_MAX: usize = 16 * 1024;
const CHUNK_MIN: usize = 4 * 1024;
/// Device rows of one piece.
pub const PIECE_ROWS: u32 = 256;
/// Chunk layouts kept for reuse.
const CHUNK_CACHE: usize = 512;

/// The colours the editor draws with (the window's theme resources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Automatic text.
    pub text: [u8; 4],
    pub link: [u8; 4],
    /// List markers' and quote bars' colour.
    pub subtle: [u8; 4],
    /// Table borders.
    pub border: [u8; 4],
    /// The field's background, that author colours must read on.
    pub bg: [u8; 4],
    pub dark: bool,
    /// High contrast: every author colour is drawn as `text`.
    pub contrast: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            text: [0x1a, 0x1a, 0x1a, 255],
            link: [0x00, 0x5f, 0xb8, 255],
            subtle: [0x80, 0x80, 0x80, 255],
            border: [0xc8, 0xc8, 0xc8, 255],
            bg: [255, 255, 255, 255],
            dark: false,
            contrast: false,
        }
    }
}

struct Chunk {
    /// Byte offset in the paragraph.
    start: usize,
    layout: Arc<Layout<Brush>>,
    /// Top, from the paragraph's top.
    y: f32,
}

struct ParaBox {
    chunks: Vec<Chunk>,
    /// Left edge of the text, from the block's left.
    x: f32,
    h: f32,
    quote: u8,
}

impl ParaBox {
    /// The chunk holding byte `off`, and the offset in it.
    fn chunk_at(&self, off: usize) -> Option<(&Chunk, usize)> {
        let i = self
            .chunks
            .partition_point(|c| c.start <= off)
            .saturating_sub(1);
        let c = self.chunks.get(i)?;
        Some((c, off.saturating_sub(c.start)))
    }

    fn chunk_at_y(&self, y: f32) -> Option<&Chunk> {
        let i = self.chunks.partition_point(|c| c.y <= y).saturating_sub(1);
        self.chunks.get(i)
    }

    fn width(&self) -> f32 {
        self.chunks
            .iter()
            .map(|c| c.layout.width())
            .fold(1.0, f32::max)
    }
}

struct CellBox {
    paras: Vec<ParaBox>,
    /// Each paragraph's top, from the cell's content top.
    ys: Vec<f32>,
}

struct TableBox {
    col_w: f32,
    cols: usize,
    /// Each row's top and height.
    rows: Vec<(f32, f32)>,
    cells: Vec<Vec<CellBox>>,
    h: f32,
}

impl TableBox {
    fn col_x(&self, c: usize) -> f32 {
        c as f32 * self.col_w
    }
    fn w(&self) -> f32 {
        self.cols as f32 * self.col_w
    }
}

struct SigBox {
    slots: Vec<Slot>,
    ys: Vec<f32>,
    /// Logo: top, width, height, picture mark.
    logo: Option<(f32, f32, f32, u64)>,
    h: f32,
}

struct QuoteBox {
    key: u64,
    h: f32,
    folded: bool,
}

enum Slot {
    Para(ParaBox),
    Table(TableBox),
    Sig(SigBox),
    Quote(QuoteBox),
}

impl Slot {
    fn h(&self) -> f32 {
        match self {
            Slot::Para(p) => p.h,
            Slot::Table(t) => t.h,
            Slot::Sig(s) => s.h,
            Slot::Quote(q) => q.h,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Marker {
    /// The top-level block it is drawn with.
    block: usize,
    x: f32,
    y: f32,
    text: String,
}

/// Where in the view a paragraph line is laid out.
struct At<'a> {
    para: &'a ParaBox,
    x: f32,
    y: f32,
}

/// A band of one block's picture: device rows `y_px..y_px + h_px` of the
/// page. Its `key` changes whenever its pixels would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub key: u64,
    pub y_px: u32,
    pub h_px: u32,
    pub block: usize,
}

pub struct View {
    width: f32,
    scale: f32,
    zoom: f32,
    theme: Theme,
    slots: Vec<Option<Slot>>,
    /// A number per layout of each slot, for the pieces' keys.
    gens: Vec<u64>,
    next_gen: u64,
    ys: Vec<f32>,
    height: f32,
    markers: Vec<Marker>,
    marker_layouts: HashMap<String, Layout<Brush>>,
    chunk_cache: HashMap<u64, Arc<Layout<Brush>>>,
    quotes: HashMap<u64, Arc<render::Doc>>,
    logo_bytes: HashMap<u64, Arc<[u8]>>,
    logo_pix: HashMap<u64, Pixmap>,
}

/// The quote's layout key: its HTML, the width and the theme.
fn quote_key(q: &QuoteBlock, width: f32, dark: bool) -> u64 {
    let mut h = fnv(q.html().as_bytes());
    h ^= u64::from(width.to_bits()).rotate_left(17);
    h ^= u64::from(dark);
    h
}

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x0100_0000_01b3).rotate_left(5)
}

/// Where a long paragraph is cut into chunks: after spaces picked by the
/// word before them (so an edit moves only the cuts near it), at least
/// [`CHUNK_MIN`] apart, and at [`CHUNK_MAX`] at the latest.
fn cut_points(text: &str) -> Vec<usize> {
    let mut cuts = vec![0];
    if text.len() <= CHUNK_MAX {
        return cuts;
    }
    let mut last = 0usize;
    let mut h: u32 = 0;
    for (i, b) in text.bytes().enumerate() {
        let next = i.saturating_add(1);
        if b == b' ' {
            if next.saturating_sub(last) >= CHUNK_MIN && h.trailing_zeros() >= 6 {
                cuts.push(next);
                last = next;
            }
            h = 0;
        } else {
            h = h.wrapping_mul(31).wrapping_add(u32::from(b)) | 1 << 31;
            h = h.rotate_left(3);
        }
        if next.saturating_sub(last) >= CHUNK_MAX && text.is_char_boundary(next) {
            cuts.push(next);
            last = next;
            h = 0;
        }
    }
    if cuts.last() == Some(&text.len()) && cuts.len() > 1 {
        cuts.pop();
    }
    cuts
}

impl View {
    pub fn new(width: f32, scale: f32, zoom: f32, theme: Theme) -> View {
        View {
            width: width.max(40.0),
            scale: scale.clamp(0.5, 4.0),
            zoom: zoom.clamp(0.5, 4.0),
            theme,
            slots: Vec::new(),
            gens: Vec::new(),
            next_gen: 1,
            ys: Vec::new(),
            height: 0.0,
            markers: Vec::new(),
            marker_layouts: HashMap::new(),
            chunk_cache: HashMap::new(),
            quotes: HashMap::new(),
            logo_bytes: HashMap::new(),
            logo_pix: HashMap::new(),
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Device px across the page.
    pub fn width_px(&self) -> u32 {
        (self.width * self.scale).ceil().max(1.0) as u32
    }

    /// A new width, device scale, text size or theme: everything is laid
    /// out again. True if anything changed.
    pub fn set_geometry(&mut self, width: f32, scale: f32, zoom: f32, theme: Theme) -> bool {
        let width = width.max(40.0);
        let scale = scale.clamp(0.5, 4.0);
        let zoom = zoom.clamp(0.5, 4.0);
        if (width - self.width).abs() < 0.25
            && (scale - self.scale).abs() < 0.001
            && (zoom - self.zoom).abs() < 0.001
            && theme == self.theme
        {
            return false;
        }
        if (scale - self.scale).abs() >= 0.001 {
            self.logo_pix.clear();
        }
        self.width = width;
        self.scale = scale;
        self.zoom = zoom;
        self.theme = theme;
        self.slots.clear();
        self.marker_layouts.clear();
        self.chunk_cache.clear();
        true
    }

    /// The bytes of a signature's logo, by its mark.
    pub fn set_logo(&mut self, hash: u64, bytes: Arc<[u8]>) {
        self.logo_pix.remove(&hash);
        self.logo_bytes.insert(hash, bytes);
    }

    /// Bring the layout up to date with `ed`. True if anything moved or
    /// changed (the pieces need asking for again).
    pub fn sync(&mut self, ed: &mut Editor, engine: &mut Engine) -> bool {
        let changes = ed.take_changes();
        let n = ed.doc().blocks.len();
        let full = changes.iter().any(|c| c.all) || self.slots.len() != self.gens.len();
        let mut changed = !changes.is_empty();
        if full || self.slots.is_empty() && n > 0 {
            self.slots = (0..n).map(|_| None).collect();
            self.gens = vec![0; n];
            changed = true;
        } else {
            for ch in &changes {
                let end = ch.at.saturating_add(ch.old_len);
                if end > self.slots.len() {
                    self.slots = (0..n).map(|_| None).collect();
                    self.gens = vec![0; n];
                    break;
                }
                self.slots.splice(ch.at..end, (0..ch.new_len).map(|_| None));
                self.gens.splice(ch.at..end, (0..ch.new_len).map(|_| 0));
            }
            if self.slots.len() != n {
                self.slots = (0..n).map(|_| None).collect();
                self.gens = vec![0; n];
            }
        }
        let doc = ed.doc();
        let mut laid = false;
        for (i, b) in doc.blocks.iter().enumerate() {
            if matches!(self.slots.get(i), Some(Some(_))) {
                continue;
            }
            let slot = self.layout_block(doc, b, self.width, engine);
            if let Some(s) = self.slots.get_mut(i) {
                *s = Some(slot);
            }
            if let Some(g) = self.gens.get_mut(i) {
                *g = self.next_gen;
                self.next_gen = self.next_gen.wrapping_add(1);
            }
            laid = true;
        }
        if laid || changed {
            self.place(doc, engine);
            if self.chunk_cache.len() > CHUNK_CACHE {
                self.chunk_cache.clear();
            }
        }
        laid || changed
    }

    /// Px of a size in points at the text size.
    fn px(&self, pt: SizePt) -> f32 {
        f32::from(pt.get()) * PX_PER_PT * self.zoom
    }

    fn author_color(&self, c: Option<ColorId>) -> [u8; 4] {
        match c {
            Some(c) if !self.theme.contrast => {
                let (r, g, b) = c.rgb();
                canvas::readable([r, g, b, 255], self.theme.bg)
            }
            _ => self.theme.text,
        }
    }

    fn base_span<'d>(&self, doc: &'d Doc) -> Span<'d> {
        Span {
            start: 0,
            end: 0,
            family: doc.font.stack(),
            size: self.px(doc.size),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: self.author_color(doc.color),
        }
    }

    /// The spans of `p`'s runs within bytes `r`, from `r.start`.
    fn spans<'d>(&self, doc: &'d Doc, p: &'d Para, r: &Range<usize>) -> Vec<Span<'d>> {
        let base = self.base_span(doc);
        let mut out = Vec::new();
        let mut at = 0usize;
        for run in &p.runs {
            let end = at.saturating_add(run.text.len());
            let (s, e) = (at.max(r.start), end.min(r.end));
            at = end;
            if s >= e {
                continue;
            }
            let st = &run.style;
            let color = if st.link.is_some() && (st.color.is_none() || self.theme.contrast) {
                self.theme.link
            } else if st.color.is_some() {
                self.author_color(st.color)
            } else {
                base.color
            };
            out.push(Span {
                start: s.saturating_sub(r.start),
                end: e.saturating_sub(r.start),
                family: st.font.map_or(base.family, |f| f.stack()),
                size: st.size.map_or(base.size, |z| self.px(z)),
                bold: st.b,
                italic: st.i,
                underline: st.u || st.link.is_some(),
                strike: st.s,
                color,
            });
        }
        out
    }

    fn para_box(&mut self, doc: &Doc, p: &Para, width: f32, engine: &mut Engine) -> ParaBox {
        let x = f32::from(p.indent) * INDENT_STEP
            + f32::from(p.quote) * QUOTE_STEP
            + p.list
                .map_or(0.0, |(_, l)| (f32::from(l) + 1.0) * LIST_STEP);
        let w = (width - x).max(24.0);
        let align = match p.align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
            Align::Justify => TextAlign::Justify,
        };
        let base = self.base_span(doc);
        let mut chunks = Vec::new();
        if p.is_empty() {
            // One line's height, for the caret.
            let layout = engine.text_layout(" ", &base, &[], w, align);
            chunks.push(Chunk {
                start: 0,
                layout: Arc::new(layout),
                y: 0.0,
            });
        } else {
            let text = p.plain_text();
            let cuts = cut_points(&text);
            let many = cuts.len() > 1;
            let mut y = 0.0;
            for (k, &start) in cuts.iter().enumerate() {
                let end = cuts.get(k.saturating_add(1)).copied().unwrap_or(text.len());
                let r = start..end;
                let piece = text.get(r.clone()).unwrap_or("");
                let spans = self.spans(doc, p, &r);
                let layout = if many {
                    let mut key = fnv(piece.as_bytes());
                    for s in &spans {
                        key = mix(key, fnv(format!("{s:?}").as_bytes()));
                    }
                    key = mix(key, u64::from(w.to_bits()));
                    key = mix(key, align as u64);
                    match self.chunk_cache.get(&key) {
                        Some(l) => l.clone(),
                        None => {
                            let l = Arc::new(engine.text_layout(piece, &base, &spans, w, align));
                            self.chunk_cache.insert(key, l.clone());
                            l
                        }
                    }
                } else {
                    Arc::new(engine.text_layout(piece, &base, &spans, w, align))
                };
                let h = layout.height();
                chunks.push(Chunk { start, layout, y });
                y += h;
            }
        }
        let h = chunks
            .last()
            .map_or(1.0, |c| c.y + c.layout.height())
            .max(1.0);
        ParaBox {
            chunks,
            x,
            h,
            quote: p.quote,
        }
    }

    fn layout_block(&mut self, doc: &Doc, b: &Block, width: f32, engine: &mut Engine) -> Slot {
        match b {
            Block::Para(p) => Slot::Para(self.para_box(doc, p, width, engine)),
            Block::Table(t) => Slot::Table(self.table_box(doc, t, width, engine)),
            Block::Signature(s) => {
                let mut slots = Vec::with_capacity(s.blocks.len());
                for ib in s
                    .blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Para(_) | Block::Table(_)))
                {
                    slots.push(self.layout_block(doc, ib, width, engine));
                }
                let mut ys = Vec::with_capacity(slots.len());
                let mut y = 0.0;
                for sl in &slots {
                    ys.push(y);
                    y += sl.h();
                }
                let logo = s.logo.map(|l| {
                    let (w, h) = (f32::from(l.width).max(1.0), f32::from(l.height).max(1.0));
                    let k = (width / w).min(1.0);
                    let top = y + LOGO_GAP;
                    y = top + h * k;
                    (top, w * k, h * k, l.hash)
                });
                Slot::Sig(SigBox {
                    slots,
                    ys,
                    logo,
                    h: y,
                })
            }
            Block::Quote(q) => {
                let key = quote_key(q, width, self.theme.dark);
                let h = if q.folded {
                    0.0
                } else {
                    QUOTE_GAP + self.quotes.get(&key).map_or(QUOTE_WAIT_H, |d| d.height)
                };
                Slot::Quote(QuoteBox {
                    key,
                    h,
                    folded: q.folded,
                })
            }
        }
    }

    fn table_box(&mut self, doc: &Doc, t: &Table, width: f32, engine: &mut Engine) -> TableBox {
        let cols = t.col_count().max(1);
        let col_w = (width / cols as f32).max(2.0 * CELL_PAD_X + 16.0);
        let inner = col_w - 2.0 * CELL_PAD_X;
        let mut rows = Vec::with_capacity(t.rows.len());
        let mut cells = Vec::with_capacity(t.rows.len());
        let mut y = 0.0;
        for row in &t.rows {
            let mut boxes = Vec::with_capacity(row.len());
            let mut row_h: f32 = 0.0;
            for cell in row {
                let mut paras = Vec::with_capacity(cell.paras.len());
                let mut ys = Vec::with_capacity(cell.paras.len());
                let mut cy = 0.0;
                for p in &cell.paras {
                    let pb = self.para_box(doc, p, inner, engine);
                    ys.push(cy);
                    cy += pb.h;
                    paras.push(pb);
                }
                row_h = row_h.max(cy);
                boxes.push(CellBox { paras, ys });
            }
            let h = row_h + 2.0 * CELL_PAD_Y + 1.0;
            rows.push((y, h));
            y += h;
            cells.push(boxes);
        }
        TableBox {
            col_w,
            cols,
            rows,
            cells,
            h: y + 1.0,
        }
    }

    /// Place the blocks one under the other and number the lists.
    fn place(&mut self, doc: &Doc, engine: &mut Engine) {
        self.ys.clear();
        let mut y = 0.0f32;
        let mut counters: [Option<(ListKind, u32)>; 4] = [None; 4];
        let mut markers = Vec::new();
        for (i, b) in doc.blocks.iter().enumerate() {
            self.ys.push(y);
            let Some(Some(slot)) = self.slots.get(i) else {
                continue;
            };
            match (b, slot) {
                (Block::Para(p), Slot::Para(pb)) => {
                    list_marker(i, p, pb, y, &mut counters, &mut markers);
                }
                (Block::Signature(s), Slot::Sig(sb)) => {
                    let mut inner: [Option<(ListKind, u32)>; 4] = [None; 4];
                    for ((ib, sl), sy) in s
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, Block::Para(_) | Block::Table(_)))
                        .zip(&sb.slots)
                        .zip(&sb.ys)
                    {
                        if let (Block::Para(p), Slot::Para(pb)) = (ib, sl) {
                            list_marker(i, p, pb, y + sy, &mut inner, &mut markers);
                        } else {
                            inner = [None; 4];
                        }
                    }
                    counters = [None; 4];
                }
                _ => counters = [None; 4],
            }
            y += slot.h();
        }
        self.height = y.max(1.0);
        let size = self.px(doc.size);
        for m in &markers {
            if !self.marker_layouts.contains_key(&m.text) {
                let base = Span {
                    start: 0,
                    end: 0,
                    family: doc.font.stack(),
                    size,
                    bold: false,
                    italic: false,
                    underline: false,
                    strike: false,
                    color: self.theme.text,
                };
                let l = engine.text_layout(&m.text, &base, &[], 200.0, TextAlign::Left);
                self.marker_layouts.insert(m.text.clone(), l);
            }
        }
        // A marker ends 6 px before its item's text.
        self.markers = markers
            .into_iter()
            .map(|mut m| {
                let w = self.marker_layouts.get(&m.text).map_or(8.0, |l| l.width());
                m.x = (m.x - w - 6.0).max(0.0);
                m
            })
            .collect();
    }

    /// The stored quotes the view still waits for: (key, HTML, width,
    /// dark).
    pub fn quote_jobs(&self, ed: &Editor) -> Vec<(u64, String, f32, bool)> {
        ed.doc()
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Quote(q) if !q.folded => {
                    let key = quote_key(q, self.width, self.theme.dark);
                    (!self.quotes.contains_key(&key))
                        .then(|| (key, q.html().to_string(), self.width, self.theme.dark))
                }
                _ => None,
            })
            .collect()
    }

    /// A stored quote's layout arrived: its block is laid out again at the
    /// next [`View::sync`]. True if a block shows it.
    pub fn set_quote(&mut self, key: u64, doc: Arc<render::Doc>) -> bool {
        if self.quotes.len() > 8 {
            self.quotes.clear();
        }
        self.quotes.insert(key, doc);
        let mut changed = false;
        for s in &mut self.slots {
            if matches!(s, Some(Slot::Quote(q)) if q.key == key) {
                *s = None;
                changed = true;
            }
        }
        changed
    }

    // ---- geometry ------------------------------------------------------

    /// The paragraph box a line is laid out in, and its top left.
    fn at(&self, pos: Pos) -> Option<At<'_>> {
        let top = pos.top();
        let slot = self.slots.get(top)?.as_ref()?;
        let y0 = *self.ys.get(top)?;
        let (slot, y0) = match (pos.flow, slot) {
            (Flow::Top, s) => (s, y0),
            (Flow::Sig(_), Slot::Sig(sb)) => {
                // Signature items are its paragraphs and tables, in order.
                let s = sb.slots.get(pos.item)?;
                (s, y0 + sb.ys.get(pos.item).copied().unwrap_or(0.0))
            }
            _ => return None,
        };
        match (slot, pos.cell) {
            (Slot::Para(p), None) => Some(At {
                para: p,
                x: p.x,
                y: y0,
            }),
            (Slot::Table(t), Some((r, c))) => {
                let (ry, _) = *t.rows.get(r)?;
                let cell = t.cells.get(r)?.get(c)?;
                let para = cell.paras.get(pos.para)?;
                let py = *cell.ys.get(pos.para)?;
                Some(At {
                    para,
                    x: t.col_x(c) + CELL_PAD_X + para.x,
                    y: y0 + ry + CELL_PAD_Y + 1.0 + py,
                })
            }
            _ => None,
        }
    }

    /// An object line's box: the quote, or a logo.
    fn object_rect(&self, pos: Pos) -> Option<Rect> {
        let top = pos.top();
        let y0 = *self.ys.get(top)?;
        match self.slots.get(top)?.as_ref()? {
            Slot::Quote(q) => Some(Rect {
                x: 0.0,
                y: y0 + QUOTE_GAP,
                w: self.width,
                h: (q.h - QUOTE_GAP).max(16.0),
            }),
            Slot::Sig(sb) => {
                let (ly, w, h, _) = sb.logo?;
                Some(Rect {
                    x: 0.0,
                    y: y0 + ly,
                    w,
                    h,
                })
            }
            _ => None,
        }
    }

    fn line_for(ed: &Editor, offset: usize) -> Option<(&Line, usize)> {
        let (i, off) = ed.locate(offset)?;
        Some((ed.lines().get(i)?, off))
    }

    /// The caret at byte `offset` of the projection.
    pub fn caret(&self, ed: &Editor, offset: usize) -> Rect {
        let fallback = Rect {
            x: 0.0,
            y: 0.0,
            w: CARET_W,
            h: self.px(ed.doc().size) * 1.3,
        };
        let Some((line, off)) = Self::line_for(ed, offset) else {
            return fallback;
        };
        if line.pos.atom.is_some() {
            let Some(r) = self.object_rect(line.pos) else {
                return fallback;
            };
            let h = fallback.h.min(r.h);
            return if off == 0 {
                Rect {
                    x: r.x,
                    y: r.y,
                    w: CARET_W,
                    h,
                }
            } else {
                Rect {
                    x: (r.x + r.w).min(self.width - CARET_W),
                    y: r.y + r.h - h,
                    w: CARET_W,
                    h,
                }
            };
        }
        let Some(at) = self.at(line.pos) else {
            return fallback;
        };
        let Some((chunk, local)) = at.para.chunk_at(off) else {
            return fallback;
        };
        let aff = if off > 0 && off >= line.len {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        let bb =
            Cursor::from_byte_index(&chunk.layout, local, aff).geometry(&chunk.layout, CARET_W);
        Rect {
            x: at.x + bb.x0 as f32,
            y: at.y + chunk.y + bb.y0 as f32,
            w: CARET_W,
            h: ((bb.y1 - bb.y0) as f32).max(4.0),
        }
    }

    /// The box of a paragraph line, for hit testing.
    fn line_box(&self, l: &Line) -> Option<Rect> {
        match l.pos.atom {
            Some(_) => self.object_rect(l.pos),
            None => self.at(l.pos).map(|a| Rect {
                x: a.x,
                y: a.y,
                w: a.para.width(),
                h: a.para.h,
            }),
        }
    }

    /// The projection offset nearest a point (CSS px in the page).
    pub fn hit(&self, ed: &Editor, x: f32, y: f32) -> usize {
        let lines = ed.lines();
        let Some(last) = lines.last() else {
            return 0;
        };
        if !(x.is_finite() && y.is_finite()) || y < 0.0 {
            return 0;
        }
        if y >= self.height {
            return last.end();
        }
        // The blocks around the point, then the line nearest it,
        // vertically first.
        let i0 = self.ys.partition_point(|t| *t <= y).saturating_sub(1);
        let (lo, hi) = (i0.saturating_sub(1), i0.saturating_add(1));
        let first = lines.partition_point(|l| l.pos.top() < lo);
        let mut best: Option<(f32, f32, usize)> = None;
        for (li, l) in lines.iter().enumerate().skip(first) {
            if l.pos.top() > hi {
                break;
            }
            let Some(r) = self.line_box(l) else {
                continue;
            };
            let dy = gap(y, r.y, r.y + r.h);
            let dx = if l.pos.cell.is_some() {
                // A cell spans its column.
                let c0 = r.x - CELL_PAD_X - self.at(l.pos).map_or(0.0, |a| a.para.x);
                gap(x, c0, c0 + self.cell_w(l.pos))
            } else {
                0.0
            };
            if best.is_none_or(|(by, bx, _)| (dy, dx) < (by, bx)) {
                best = Some((dy, dx, li));
            }
        }
        let Some((dy, _, li)) = best else {
            return last.end();
        };
        let Some(l) = lines.get(li) else {
            return 0;
        };
        match l.pos.atom {
            Some(_) => {
                let r = self.object_rect(l.pos).unwrap_or_default();
                if y < r.y + r.h / 2.0 {
                    l.start
                } else {
                    l.end()
                }
            }
            None => {
                let Some(a) = self.at(l.pos) else {
                    return l.start;
                };
                let idx = if dy > 0.0 && y < a.y {
                    0
                } else if dy > 0.0 {
                    l.len
                } else {
                    match a.para.chunk_at_y(y - a.y) {
                        Some(c) => c.start.saturating_add(
                            Cursor::from_point(&c.layout, x - a.x, y - a.y - c.y).index(),
                        ),
                        None => 0,
                    }
                };
                l.start.saturating_add(idx.min(l.len))
            }
        }
    }

    fn cell_w(&self, pos: Pos) -> f32 {
        let Some(Some(slot)) = self.slots.get(pos.top()) else {
            return 0.0;
        };
        let slot = match (pos.flow, slot) {
            (Flow::Sig(_), Slot::Sig(sb)) => match sb.slots.get(pos.item) {
                Some(s) => s,
                None => return 0.0,
            },
            (_, s) => s,
        };
        match slot {
            Slot::Table(t) => t.col_w,
            _ => 0.0,
        }
    }

    /// The word around `offset` (double click).
    pub fn word(&self, ed: &Editor, offset: usize) -> (usize, usize) {
        let Some((line, off)) = Self::line_for(ed, offset) else {
            return (offset, offset);
        };
        if line.pos.atom.is_some() {
            return (line.start, line.end());
        }
        let Some(a) = self.at(line.pos) else {
            return (offset, offset);
        };
        let Some((c, local)) = a.para.chunk_at(off) else {
            return (offset, offset);
        };
        let g = Cursor::from_byte_index(&c.layout, local, Affinity::Downstream)
            .geometry(&c.layout, 1.0);
        let r =
            Selection::word_from_point(&c.layout, g.x0 as f32 + 0.5, ((g.y0 + g.y1) / 2.0) as f32)
                .text_range();
        let base = line.start.saturating_add(c.start);
        (
            base.saturating_add(r.start).min(line.end()),
            base.saturating_add(r.end).min(line.end()),
        )
    }

    /// One line (or `page` CSS px more) up or down from the caret, at
    /// `goal_x`.
    pub fn vertical(
        &self,
        ed: &Editor,
        offset: usize,
        goal_x: f32,
        down: bool,
        page: f32,
    ) -> usize {
        let c = self.caret(ed, offset);
        let step = page.max(0.0);
        let y = if down {
            c.y + c.h + 1.0 + step
        } else {
            c.y - 1.0 - step
        };
        if y < 0.0 {
            return 0;
        }
        if y >= self.height {
            return ed.text().len();
        }
        self.hit(ed, goal_x, y)
    }

    /// The start (or end) of the visual line the caret is on.
    pub fn line_edge(&self, ed: &Editor, offset: usize, end: bool) -> usize {
        let Some((line, off)) = Self::line_for(ed, offset) else {
            return offset;
        };
        if line.pos.atom.is_some() {
            return if end { line.end() } else { line.start };
        }
        let Some(a) = self.at(line.pos) else {
            return offset;
        };
        let Some((c, local)) = a.para.chunk_at(off) else {
            return offset;
        };
        let aff = if off > 0 && off >= line.len {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        let sel = Selection::from_byte_index(&c.layout, local, aff);
        let moved = if end {
            sel.line_end(&c.layout, false)
        } else {
            sel.line_start(&c.layout, false)
        };
        line.start
            .saturating_add(c.start)
            .saturating_add(moved.focus().index())
            .min(line.end())
    }

    /// Highlight rectangles of the selection `a..b`.
    pub fn selection(&self, ed: &Editor, a: usize, b: usize) -> Vec<Rect> {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = Vec::new();
        if a == b {
            return out;
        }
        let lines = ed.lines();
        let first = lines.partition_point(|l| l.end() < a);
        for l in lines.iter().skip(first) {
            if l.start > b {
                break;
            }
            let from = a.max(l.start).saturating_sub(l.start);
            let to = b.min(l.end()).saturating_sub(l.start);
            // The line's break is selected too.
            let with_break = b > l.end();
            if l.pos.atom.is_some() {
                if to > from
                    && let Some(r) = self.object_rect(l.pos)
                {
                    out.push(r);
                }
                continue;
            }
            let Some(at) = self.at(l.pos) else {
                continue;
            };
            let chunks = &at.para.chunks;
            for (k, c) in chunks.iter().enumerate() {
                let c_end = chunks.get(k.saturating_add(1)).map_or(l.len, |n| n.start);
                let (f, t) = (from.max(c.start), to.min(c_end));
                if t <= f {
                    continue;
                }
                let sel = Selection::new(
                    Cursor::from_byte_index(
                        &c.layout,
                        f.saturating_sub(c.start),
                        Affinity::Downstream,
                    ),
                    Cursor::from_byte_index(
                        &c.layout,
                        t.saturating_sub(c.start),
                        Affinity::Upstream,
                    ),
                );
                sel.geometry_with(&c.layout, |bb, _| {
                    out.push(Rect {
                        x: at.x + bb.x0 as f32,
                        y: at.y + c.y + bb.y0 as f32,
                        w: (bb.x1 - bb.x0) as f32,
                        h: (bb.y1 - bb.y0) as f32,
                    });
                });
            }
            if with_break && let Some((c, local)) = at.para.chunk_at(l.len) {
                let g = Cursor::from_byte_index(&c.layout, local, Affinity::Upstream)
                    .geometry(&c.layout, 1.0);
                let x = if l.len == 0 { at.x } else { at.x + g.x0 as f32 };
                out.push(Rect {
                    x,
                    y: at.y + c.y + g.y0 as f32,
                    w: BREAK_W,
                    h: (g.y1 - g.y0) as f32,
                });
            }
        }
        out
    }

    // ---- painting --------------------------------------------------------

    /// The pieces that cover the page from `top` to `bottom` (CSS px).
    pub fn pieces(&self, top: f32, bottom: f32) -> Vec<Piece> {
        let mut out = Vec::new();
        let (dtop, dbottom) = ((top * self.scale).max(0.0), bottom * self.scale);
        let first = self.ys.partition_point(|y| *y <= top).saturating_sub(1);
        for i in first..self.ys.len() {
            let Some(&y) = self.ys.get(i) else {
                break;
            };
            let next = self
                .ys
                .get(i.saturating_add(1))
                .copied()
                .unwrap_or(self.height);
            let (y0, y1) = ((y * self.scale).round(), (next * self.scale).round());
            if y0 > dbottom {
                break;
            }
            if y1 <= y0 || y1 < dtop {
                continue;
            }
            let (y0, y1) = (y0 as u32, y1 as u32);
            let generation = self.gens.get(i).copied().unwrap_or(0);
            let mut key = mix(generation, 0x9e37);
            for m in self.markers.iter().filter(|m| m.block == i) {
                key = mix(key, fnv(m.text.as_bytes()));
            }
            let mut k = 0u32;
            let mut at = y0;
            while at < y1 {
                let h = (y1 - at).min(PIECE_ROWS);
                if (at.saturating_add(h)) as f32 >= dtop && at as f32 <= dbottom {
                    out.push(Piece {
                        key: mix(mix(key, u64::from(k)), u64::from(h)),
                        y_px: at,
                        h_px: h,
                        block: i,
                    });
                }
                at = at.saturating_add(h);
                k = k.saturating_add(1);
            }
        }
        out
    }

    /// The pixels of a piece (premultiplied RGBA, transparent where
    /// nothing is drawn).
    pub fn paint_piece(&self, ed: &Editor, p: &Piece, engine: &mut Engine) -> Option<Pixmap> {
        let mut pm = Pixmap::new(self.width_px(), p.h_px.max(1))?;
        let y = *self.ys.get(p.block)?;
        let b = ed.doc().blocks.get(p.block)?;
        let slot = self.slots.get(p.block)?.as_ref()?;
        let mut band = engine.band(&mut pm, p.y_px as f32, self.scale);
        self.paint_slot(b, slot, 0.0, y, &mut band);
        for m in self.markers.iter().filter(|m| m.block == p.block) {
            if let Some(l) = self.marker_layouts.get(&m.text) {
                band.text(l, m.x, m.y);
            }
        }
        Some(pm)
    }

    fn paint_slot(&self, b: &Block, slot: &Slot, x: f32, y: f32, band: &mut Band<'_>) {
        match (b, slot) {
            (_, Slot::Para(p)) => self.paint_para(p, x, y, band),
            (_, Slot::Table(t)) => {
                let c = self.theme.border;
                let w = t.w();
                // Rules: the top and each row's bottom, then the columns.
                band.fill(Rect { x, y, w, h: 1.0 }, c, 0.0);
                for (ry, rh) in &t.rows {
                    band.fill(
                        Rect {
                            x,
                            y: y + ry + rh,
                            w,
                            h: 1.0,
                        },
                        c,
                        0.0,
                    );
                }
                for k in 0..=t.cols {
                    let cx = (x + t.col_x(k)).min(x + w - 1.0);
                    band.fill(
                        Rect {
                            x: cx,
                            y,
                            w: 1.0,
                            h: t.h,
                        },
                        c,
                        0.0,
                    );
                }
                for ((ry, _), row) in t.rows.iter().zip(&t.cells) {
                    for (ci, cell) in row.iter().enumerate() {
                        for (p, py) in cell.paras.iter().zip(&cell.ys) {
                            self.paint_para(
                                p,
                                x + t.col_x(ci) + CELL_PAD_X,
                                y + ry + CELL_PAD_Y + 1.0 + py,
                                band,
                            );
                        }
                    }
                }
            }
            (Block::Signature(s), Slot::Sig(sb)) => {
                let blocks = s
                    .blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Para(_) | Block::Table(_)));
                for ((ib, sl), sy) in blocks.zip(&sb.slots).zip(&sb.ys) {
                    self.paint_slot(ib, sl, x, y + sy, band);
                }
                if let Some((ly, w, h, hash)) = sb.logo {
                    let r = Rect { x, y: y + ly, w, h };
                    match self.logo_pix.get(&hash) {
                        Some(pix) => band.image(pix, r),
                        None => band.fill(r, self.theme.border, 2.0),
                    }
                }
            }
            (_, Slot::Quote(q)) => {
                if q.folded {
                    return;
                }
                let qy = y + QUOTE_GAP;
                match self.quotes.get(&q.key) {
                    Some(doc) => band.doc(doc, x, qy),
                    None => {
                        let bar = Rect {
                            x,
                            y: qy,
                            w: 2.0,
                            h: QUOTE_WAIT_H,
                        };
                        band.fill(bar, self.theme.subtle, 0.0);
                    }
                }
            }
            _ => {}
        }
    }

    fn paint_para(&self, p: &ParaBox, x: f32, y: f32, band: &mut Band<'_>) {
        for q in 0..p.quote {
            band.fill(
                Rect {
                    x: x + f32::from(q) * QUOTE_STEP,
                    y,
                    w: 2.0,
                    h: p.h,
                },
                self.theme.subtle,
                0.0,
            );
        }
        let (top, bottom) = band.span_css();
        for c in &p.chunks {
            let cy = y + c.y;
            if cy > bottom || cy + c.layout.height() < top {
                continue;
            }
            band.text(&c.layout, x + p.x, cy);
        }
    }

    /// Decode the logos the document shows at the device scale (once).
    /// True if one was decoded (its pieces change).
    pub fn prepare_logos(&mut self, ed: &Editor) -> bool {
        let mut any = false;
        for b in &ed.doc().blocks {
            let Block::Signature(s) = b else {
                continue;
            };
            let Some(l) = s.logo else {
                continue;
            };
            if self.logo_pix.contains_key(&l.hash) {
                continue;
            }
            let Some(bytes) = self.logo_bytes.get(&l.hash) else {
                continue;
            };
            let w = (f32::from(l.width) * self.scale).ceil() as u32;
            if let Some(pix) = canvas::decode_image(bytes, w) {
                self.logo_pix.insert(l.hash, pix);
                any = true;
            }
        }
        if any {
            // The signature's pieces are keyed by its layout: lay it out
            // again so they are painted with the picture.
            for (s, b) in self.slots.iter_mut().zip(&ed.doc().blocks) {
                if matches!(b, Block::Signature(_)) {
                    *s = None;
                }
            }
        }
        any
    }
}

fn gap(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0.0
    }
}

/// The list marker of paragraph `p` (laid out in `pb` at `y`, in top-level
/// block `block`), numbering with `counters` per level.
fn list_marker(
    block: usize,
    p: &Para,
    pb: &ParaBox,
    y: f32,
    counters: &mut [Option<(ListKind, u32)>; 4],
    out: &mut Vec<Marker>,
) {
    let Some((kind, level)) = p.list else {
        *counters = [None; 4];
        return;
    };
    let l = usize::from(level.min(MAX_LIST_LEVEL));
    for deeper in counters.iter_mut().skip(l.saturating_add(1)) {
        *deeper = None;
    }
    let n = match counters.get(l).copied().flatten() {
        Some((k, n)) if k == kind => n.saturating_add(1),
        _ => 1,
    };
    if let Some(slot) = counters.get_mut(l) {
        *slot = Some((kind, n));
    }
    let text = match kind {
        ListKind::Bullet => ["\u{2022}", "\u{25e6}", "\u{25aa}", "\u{2022}"]
            .get(l)
            .copied()
            .unwrap_or("\u{2022}")
            .to_string(),
        ListKind::Number => format!("{n}."),
    };
    out.push(Marker {
        block,
        x: pb.x,
        y,
        text,
    });
}
