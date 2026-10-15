//! Block and inline layout producing a display list; tables are in
//! table.rs and floats in float.rs. Text shaping and line breaking are
//! parley's.
//!
//! Layout recursion is capped at [`MAX_LAYOUT_DEPTH`]: deeper block content
//! is flattened into its container (no further indentation or box chrome)
//! with an explicit work list, so hostile nesting cannot overflow the stack
//! and deep quotes are not lost.

use super::Sources;
use super::dom::{Dom, Id, Kind, ROOT};
use super::img::{self, ImageData};
use super::style::{Align, Color, Display, Float, Len, LineH, Style, Styles, VAlign};
use super::{Deadline, links};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, InlineBox,
    InlineBoxKind, Layout, LayoutContext, LineHeight, OverflowWrap, PositionedLayoutItem,
    TextStyle, TextWrapMode, WhiteSpaceCollapse, WordBreak,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Brush {
    pub color: Color,
    /// Index into [`Doc::links`]; 0 for none.
    pub link: u32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush {
            color: [0, 0, 0, 255],
            link: 0,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub enum Item {
    Rect {
        r: Rect,
        color: Color,
        radius: f32,
    },
    Stroke {
        r: Rect,
        width: f32,
        color: Color,
        radius: f32,
    },
    /// `clip_h`: paint only lines whose bottom is within this height (alt
    /// text in placeholders). `selectable`: message text, as opposed to a
    /// list marker or alt text, which selection and copy skip.
    Text {
        layout: usize,
        x: f32,
        y: f32,
        clip_h: f32,
        selectable: bool,
    },
    Image {
        r: Rect,
        img: usize,
        clip: Option<Rect>,
        radius: f32,
    },
    Link {
        r: Rect,
        link: u32,
    },
    /// Where a `#fragment` link target starts: an index into
    /// [`Doc::anchors`].
    Anchor {
        y: f32,
        anchor: u32,
    },
}

/// A followable link: one per `<a href>` element.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Link {
    /// Sanitized (see `links::sanitize`).
    pub href: String,
    /// The visible text, whitespace collapsed and capped.
    pub text: String,
}

/// A laid-out message. Coordinates are CSS px from the top left.
pub struct Doc {
    pub items: Vec<Item>,
    pub layouts: Vec<Layout<Brush>>,
    /// The text of each layout (for hit testing, selection and copy).
    pub texts: Vec<String>,
    pub images: Vec<ImageData>,
    /// Followable links; index 0 is "no link".
    pub links: Vec<Link>,
    /// Names of the elements that `#fragment` links point at.
    pub anchors: Vec<String>,
    pub width: f32,
    /// The right edge of the visible content: past `width` when a box that
    /// cannot shrink (a fixed-width table or container, a wide image) did
    /// not fit, and so would be cut at the page edge.
    pub natural_width: f32,
    /// Height after the [`MAX_DOC_H`] cap.
    pub height: f32,
    /// Height before the cap.
    pub full_height: f32,
    /// The content went past [`MAX_DOC_H`] and was cut.
    pub truncated: bool,
    /// The canvas colour (body or html background, else white).
    pub bg: Color,
    /// Distinct remote URLs the message references; none were fetched.
    pub blocked_remote: usize,
    pub blocked_urls: Vec<String>,
    /// Remote images that are tracking pixels (never fetched, even with
    /// consent).
    pub trackers: usize,
    /// The message has `prefers-color-scheme: dark` rules.
    pub has_dark_rules: bool,
    /// The colours were dimmed for the dark theme.
    pub dimmed: bool,
    /// Image sources that loaded and decoded.
    pub loaded: Vec<String>,
    /// Distinct remote images the layout wanted and did not get: blocked,
    /// or not supplied by the resolver. Tracking pixels and remote CSS and
    /// fonts are never among them. Consent loads exactly these.
    pub remote_images: Vec<String>,
}

/// One visual line of text (see [`Doc::text_lines`]).
#[derive(Debug, Clone)]
pub struct TextLine {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
}

impl Doc {
    /// Visible text, one entry per laid-out line in display-list order.
    /// Transparent (hidden) and zero-size text is left out.
    pub fn text_lines(&self) -> Vec<TextLine> {
        let mut out = vec![];
        for it in &self.items {
            let Item::Text { layout, x, y, .. } = it else {
                continue;
            };
            let (Some(l), Some(text)) = (self.layouts.get(*layout), self.texts.get(*layout)) else {
                continue;
            };
            for line in l.lines() {
                let m = line.metrics();
                let mut s = String::new();
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(gr) = item else {
                        continue;
                    };
                    if gr.style().brush.color[3] == 0 || gr.run().font_size() < 0.5 {
                        continue;
                    }
                    // A run can span lines: keep this line's part.
                    let (run, line_r) = (gr.run().text_range(), line.text_range());
                    let part = run.start.max(line_r.start)..run.end.min(line_r.end);
                    s.push_str(text.get(part).unwrap_or(""));
                }
                if !s.trim().is_empty() {
                    out.push(TextLine {
                        x: *x + m.offset,
                        y: *y + m.block_min_coord,
                        w: m.advance,
                        h: m.block_max_coord - m.block_min_coord,
                        text: s.trim().to_string(),
                    });
                }
            }
        }
        out
    }

    /// Link hit areas with their index into [`Doc::links`], in display
    /// order.
    pub fn link_rects(&self) -> impl Iterator<Item = (Rect, u32)> {
        self.items.iter().filter_map(|it| match it {
            Item::Link { r, link } if *link != 0 => Some((*r, *link)),
            _ => None,
        })
    }

    /// The top of the element a `#fragment` link points at.
    pub fn anchor_y(&self, href: &str) -> Option<f32> {
        let name = links::fragment(href)?;
        let raw = href.strip_prefix('#')?;
        let index = self
            .anchors
            .iter()
            .position(|a| *a == name)
            .or_else(|| self.anchors.iter().position(|a| a == raw))?;
        self.items.iter().find_map(|it| match it {
            Item::Anchor { y, anchor } if *anchor as usize == index => Some(*y),
            _ => None,
        })
    }
}

/// Links per message; later ones stay inert.
const MAX_LINKS: usize = 10_000;
/// Link text kept per link, in characters.
const MAX_LINK_TEXT: usize = 256;

/// Document height cap (CSS px), enforced before any pixel allocation.
pub const MAX_DOC_H: f32 = 30_000.0;
/// Block nesting past this is flattened.
pub const MAX_LAYOUT_DEPTH: u32 = 48;
/// The widest inline margin or padding honoured.
const MAX_INLINE_SPACE: f32 = 200.0;
/// Text laid out per inline group (one paragraph, in effect): the rest is
/// cut at a white-space boundary and replaced by [`CAPPED_MARKER`], so one
/// pathological paragraph cannot stall shaping and line breaking.
pub const MAX_GROUP_TEXT: usize = 64 * 1024;
const CAPPED_MARKER: &str = "[Message too long: View as plain text shows the rest]";
/// Inline content wider than its line by less than this still fits
/// (side-by-side columns whose widths add up to the container's exactly).
const FIT_TOLERANCE: f32 = 2.0;

pub(super) enum Op {
    Open(Id, u32),
    Close,
    Text(String, bool),
    Br,
    Box(usize),
}

pub(super) struct BoxInfo {
    pub w: f32,
    pub h: f32,
    pub items: Vec<Item>,
    pub va: VAlign,
}

/// An empty inline box `w` wide (inline margins and padding), if visible.
fn spacer(w: f32, ops: &mut Vec<Op>, boxes: &mut Vec<BoxInfo>) {
    if w >= 0.5 && w.is_finite() {
        ops.push(Op::Box(boxes.len()));
        boxes.push(BoxInfo {
            w: w.min(MAX_INLINE_SPACE),
            h: 0.0,
            items: vec![],
            va: VAlign::Baseline,
        });
    }
}

struct Group {
    layout: Layout<Brush>,
    text: String,
    boxes: Vec<BoxInfo>,
    visible: bool,
    built_avail: f32,
    /// Contents depend on the available width (inline boxes, %-sized images).
    dep: bool,
}

pub(super) struct BoxOut {
    pub w: f32,
    pub h: f32,
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum Cls {
    Skip,
    Inline,
    Block,
    Float,
}

/// Work items for a block container's children.
enum Work {
    Node(Id),
    /// End of a flattened block: ends the current line.
    Break,
}

pub(super) struct Layouter<'a> {
    pub dom: &'a Dom,
    pub st: Vec<Style>,
    pub default: Style,
    pub fcx: &'a mut FontContext,
    pub lcx: &'a mut LayoutContext<Brush>,
    pub src: &'a mut Sources<'a>,
    pub out: Vec<Item>,
    pub doc: Doc,
    groups: HashMap<(Id, Id), Group>,
    group_min: HashMap<(Id, Id), f32>,
    pub widths: HashMap<Id, (f32, f32)>,
    pub tables: HashMap<Id, super::table::TableInfo>,
    img_cache: HashMap<String, Option<usize>>,
    /// Link ids by `<a>` element (0: inert).
    link_ids: HashMap<Id, u32>,
    /// Elements that `#fragment` links point at, with their anchor index.
    anchor_of: HashMap<Id, u32>,
    pub depth: u32,
    pub maxw: f32,
    /// Building inline groups for intrinsic widths (percentages are not
    /// resolvable): 0 = laying out, 1 = min-content, 2 = max-content.
    pub measuring: u8,
    /// Percentage-sized images met while building groups.
    pct_seen: u32,
    deadline: &'a Deadline,
    pub timed_out: bool,
    /// Pixels left for decoding and resampling images.
    pub budget: u64,
    /// Table cells seen so far (capped).
    pub cells: usize,
    /// Some inline group was cut at [`MAX_GROUP_TEXT`].
    text_capped: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout<'a>(
    dom: &'a Dom,
    styles: Styles,
    fcx: &'a mut FontContext,
    lcx: &'a mut LayoutContext<Brush>,
    src: &'a mut Sources<'a>,
    width: f32,
    scale: f32,
    deadline: &'a Deadline,
) -> Option<Doc> {
    let st = styles.v;
    let default = Style::root();
    // Every remote reference is refused; collect the distinct set.
    let mut blocked: Vec<String> = styles.remote_urls;
    let mut trackers = 0;
    for (i, n) in dom.nodes.iter().enumerate() {
        let Kind::Elem { tag, attrs } = &n.kind else {
            continue;
        };
        for (k, v) in attrs {
            let cand = match &**k {
                "src" | "background" | "poster" | "data" | "dynsrc" | "lowsrc" => true,
                "href" => matches!(&**tag, "link" | "base"),
                "srcset" => {
                    for part in v.split(',') {
                        let u = part.split_whitespace().next().unwrap_or("");
                        if super::css::is_remote(u) {
                            blocked.push(u.to_string());
                        }
                    }
                    false
                }
                _ => false,
            };
            if cand && super::css::is_remote(v) {
                blocked.push(v.trim().to_string());
                if &**tag == "img" && &**k == "src" && is_tracker(st.get(i).unwrap_or(&default)) {
                    trackers = usize::saturating_add(trackers, 1);
                }
            }
        }
    }
    blocked.sort();
    blocked.dedup();
    let (anchors, anchor_of) = anchors(dom);
    let bg = dom
        .nodes
        .iter()
        .enumerate()
        .filter(
            |(_, n)| matches!(&n.kind, Kind::Elem { tag, .. } if matches!(&**tag, "body" | "html")),
        )
        .filter_map(|(i, _)| st.get(i).map(|s| s.bg))
        .rfind(|bg| bg[3] > 0)
        .unwrap_or(if styles.dimmed {
            super::style::dim_bg(super::style::WHITE)
        } else {
            super::style::WHITE
        });
    let mut e = Layouter {
        dom,
        st,
        default,
        fcx,
        lcx,
        src,
        out: vec![],
        doc: Doc {
            items: vec![],
            layouts: vec![],
            texts: vec![],
            images: vec![],
            links: vec![Link::default()],
            anchors,
            width,
            natural_width: 0.0,
            height: 0.0,
            full_height: 0.0,
            truncated: false,
            bg,
            blocked_remote: blocked.len(),
            blocked_urls: blocked,
            trackers,
            has_dark_rules: styles.has_dark_rules,
            dimmed: styles.dimmed,
            loaded: vec![],
            remote_images: vec![],
        },
        groups: HashMap::new(),
        group_min: HashMap::new(),
        widths: HashMap::new(),
        tables: HashMap::new(),
        img_cache: HashMap::new(),
        link_ids: HashMap::new(),
        anchor_of,
        depth: 0,
        maxw: width * 4.0,
        measuring: 0,
        pct_seen: 0,
        deadline,
        timed_out: false,
        budget: img::MAX_TOTAL_PIXELS,
        cells: 0,
        text_capped: false,
    };
    let kids = dom.children(ROOT).to_vec();
    let h = e.block_list(ROOT, &kids, 0.0, 0.0, width);
    if e.timed_out {
        return None;
    }
    e.doc.full_height = h;
    e.doc.truncated = h > MAX_DOC_H || e.text_capped;
    e.doc.height = h.clamp(1.0, MAX_DOC_H);
    e.doc.items = std::mem::take(&mut e.out);
    e.doc.natural_width = content_right(&e.doc);
    e.doc.remote_images = std::mem::take(&mut e.src.missing);
    prescale_images(&mut e.doc, scale, &mut e.budget);
    Some(e.doc)
}

/// The right edge of what `doc` paints: boxes, images (as clipped) and
/// visible glyph runs (without a line's trailing space). Transparent and zero-size text is left out, so a
/// hidden preheader cannot make the page look wider than it is.
fn content_right(doc: &Doc) -> f32 {
    let mut right = 0.0f32;
    for it in &doc.items {
        let edge = match it {
            Item::Rect { r, .. } | Item::Stroke { r, .. } => r.x + r.w,
            Item::Image { r, clip, .. } => clip.map_or(r.x + r.w, |c| (c.x + c.w).min(r.x + r.w)),
            Item::Text { layout, x, .. } => {
                let Some(l) = doc.layouts.get(*layout) else {
                    continue;
                };
                let mut edge = 0.0f32;
                for line in l.lines() {
                    // A line's trailing space hangs past the line box.
                    let m = line.metrics();
                    let end = *x + m.offset + m.advance - m.trailing_whitespace;
                    for item in line.items() {
                        if let PositionedLayoutItem::GlyphRun(gr) = item
                            && gr.style().brush.color[3] != 0
                            && gr.run().font_size() >= 0.5
                        {
                            edge = edge.max((*x + gr.offset() + gr.advance()).min(end));
                        }
                    }
                }
                edge
            }
            Item::Link { .. } | Item::Anchor { .. } => continue,
        };
        right = right.max(fin(edge));
    }
    right
}

/// The names `#fragment` links in `dom` point at, and the elements that
/// carry them (by `id`, or `name` on `<a>`); the first element wins. Only
/// names some link uses are kept, so most messages pay nothing.
fn anchors(dom: &Dom) -> (Vec<String>, HashMap<Id, u32>) {
    type Attrs = [(Box<str>, Box<str>)];
    fn elem(n: &super::dom::Node) -> Option<(&str, &Attrs)> {
        match &n.kind {
            Kind::Elem { tag, attrs } => Some((tag, attrs)),
            Kind::Text(_) => None,
        }
    }
    let wanted: HashSet<String> = dom
        .nodes
        .iter()
        .filter_map(elem)
        .filter(|(tag, _)| *tag == "a")
        .filter_map(|(_, a)| a.iter().find(|(k, _)| &**k == "href"))
        .filter_map(|(_, v)| links::sanitize(v).as_deref().and_then(links::fragment))
        .collect();
    let mut names: Vec<String> = vec![];
    let mut of = HashMap::new();
    if wanted.is_empty() {
        return (names, of);
    }
    for (i, n) in dom.nodes.iter().enumerate() {
        let (Some((tag, a)), Ok(id)) = (elem(n), Id::try_from(i)) else {
            continue;
        };
        let name = a
            .iter()
            .find(|(k, v)| (&**k == "id" || &**k == "name" && tag == "a") && wanted.contains(&**v));
        if let Some((_, v)) = name
            && !names.iter().any(|x| **x == **v)
            && let Ok(index) = u32::try_from(names.len())
        {
            names.push(v.to_string());
            of.insert(id, index);
        }
    }
    (names, of)
}

/// A tracking pixel: tiny or hidden.
fn is_tracker(s: &Style) -> bool {
    let tiny = |l: Len| matches!(l, Len::Px(p) if p <= 2.0);
    s.display == Display::None || tiny(s.width) || tiny(s.height)
}

pub(super) fn shift(items: &mut [Item], dx: f32, dy: f32) {
    for it in items {
        match it {
            Item::Rect { r, .. } | Item::Stroke { r, .. } | Item::Link { r, .. } => {
                r.x += dx;
                r.y += dy;
            }
            Item::Image { r, clip, .. } => {
                r.x += dx;
                r.y += dy;
                if let Some(c) = clip {
                    c.x += dx;
                    c.y += dy;
                }
            }
            Item::Text { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Item::Anchor { y, .. } => *y += dy,
        }
    }
}

pub(super) fn px_of(l: Len) -> f32 {
    match l {
        Len::Px(p) => p,
        _ => 0.0,
    }
}

/// Finite and non-negative.
pub(super) fn fin(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

impl<'a> Layouter<'a> {
    pub fn style(&self, id: Id) -> &Style {
        self.st.get(id as usize).unwrap_or(&self.default)
    }

    /// Check the deadline; once it has passed every loop unwinds.
    pub fn expired(&mut self) -> bool {
        if !self.timed_out && self.deadline.expired() {
            self.timed_out = true;
        }
        self.timed_out
    }

    pub fn classify(&self, c: Id) -> Cls {
        match self.dom.node(c).map(|n| &n.kind) {
            None => Cls::Skip,
            Some(Kind::Text(_)) => Cls::Inline,
            Some(Kind::Elem { tag, .. }) => {
                let s = self.style(c);
                if s.display == Display::None {
                    return Cls::Skip;
                }
                if &**tag == "br" || &**tag == "img" && s.float == Float::None {
                    return Cls::Inline;
                }
                if s.float != Float::None
                    && !matches!(s.display, Display::Cell | Display::Row | Display::RowGroup)
                {
                    return Cls::Float;
                }
                match s.display {
                    Display::Inline | Display::InlineBlock => Cls::Inline,
                    _ => Cls::Block,
                }
            }
        }
    }

    // ------------------------------------------------------------------ blocks

    /// Lay out `kids` (children of `parent`) into the content box (x, y, w);
    /// returns the content height.
    pub fn block_list(&mut self, parent: Id, kids: &[Id], x: f32, y: f32, w: f32) -> f32 {
        let flat = self.depth >= MAX_LAYOUT_DEPTH;
        self.depth = self.depth.saturating_add(1);
        let h = self.block_list_inner(parent, kids, x, y, fin(w), flat);
        self.depth = self.depth.saturating_sub(1);
        h
    }

    fn block_list_inner(
        &mut self,
        parent: Id,
        kids: &[Id],
        x: f32,
        y: f32,
        w: f32,
        flat: bool,
    ) -> f32 {
        let mut cy = 0.0f32;
        let mut pend = 0.0f32;
        let mut run: Vec<Id> = vec![];
        let mut floats = super::float::Floats::default();
        let mut work: Vec<Work> = kids.iter().rev().map(|&k| Work::Node(k)).collect();
        // Lay out the pending inline run as one group.
        macro_rules! flush {
            () => {
                if !run.is_empty() {
                    let r = std::mem::take(&mut run);
                    let (l, rr) = floats.band(cy + pend, w);
                    let h = self.layout_group(parent, &r, x + l, y + cy + pend, w - l - rr);
                    if h > 0.0 {
                        cy += pend + h;
                        pend = 0.0;
                    }
                }
            };
        }
        while let Some(job) = work.pop() {
            if y + cy > MAX_DOC_H + 64.0 || self.expired() {
                break; // beyond the document cap: stop laying out
            }
            let c = match job {
                Work::Break => {
                    flush!();
                    continue;
                }
                Work::Node(c) => c,
            };
            let cls = self.classify(c);
            if flat && matches!(cls, Cls::Block | Cls::Float) {
                flush!();
                self.anchor(c, y + cy + pend);
                work.push(Work::Break);
                work.extend(self.dom.children(c).iter().rev().map(|&k| Work::Node(k)));
                continue;
            }
            match cls {
                Cls::Skip => {}
                Cls::Inline => run.push(c),
                Cls::Float => {
                    flush!();
                    self.place_float(c, &mut floats, x, y + cy, cy, w);
                }
                Cls::Block => {
                    flush!();
                    let s = self.style(c);
                    let (mt, mb) = (px_of(s.margin[0]), px_of(s.margin[2]));
                    let gap = pend.max(mt);
                    let (l, rr) = floats.band(cy + gap, w);
                    let out = self.box_at(c, x + l, y + cy + gap, w - l - rr, false);
                    cy += gap + out.h;
                    pend = mb;
                }
            }
        }
        flush!();
        (cy + pend).max(floats.bottom())
    }

    /// Lay out `id` with its margins zeroed (the caller applies them).
    pub fn box_at_nomargin(&mut self, id: Id, x: f32, y: f32, avail: f32) -> BoxOut {
        let Some(s) = self.st.get_mut(id as usize) else {
            return BoxOut { w: 0.0, h: 0.0 };
        };
        let saved = (s.margin, s.center_blocks);
        s.margin = [Len::Px(0.0); 4];
        s.center_blocks = false;
        let r = self.box_at(id, x, y, avail, true);
        if let Some(s) = self.st.get_mut(id as usize) {
            (s.margin, s.center_blocks) = saved;
        }
        r
    }

    pub fn box_at(&mut self, id: Id, x: f32, y: f32, cb_w: f32, shrink: bool) -> BoxOut {
        self.anchor(id, y);
        let mut s = self.style(id).clone();
        if let Len::Px(p) = s.width {
            s.width = Len::Px(p.min(self.maxw));
        }
        let (pad, bd) = (s.padding, s.border);
        let (ml_s, mr_s) = (s.margin[3], s.margin[1]);
        let (ml_px, mr_px) = (px_of(ml_s), px_of(mr_s));
        let auto_margin = |free: f32, own_width: bool| match (ml_s, mr_s) {
            (Len::Auto, Len::Auto) => free / 2.0,
            (Len::Auto, _) => free,
            _ if s.center_blocks && own_width && ml_px == 0.0 && mr_px == 0.0 => free / 2.0,
            _ => ml_px,
        };
        if s.display == Display::Table {
            let avail = fin(cb_w - ml_px - mr_px);
            let tw = self.table_width(id, avail);
            let ml = auto_margin(fin(cb_w - tw - ml_px - mr_px), true);
            let h = self.table_layout(id, x + ml, y, tw);
            return BoxOut { w: tw, h };
        }
        let hb = pad[1] + pad[3] + bd[1] + bd[3];
        let (vb_top, vb_bot) = (pad[0] + bd[0], pad[2] + bd[2]);
        let avail = fin(cb_w - ml_px - mr_px - hb);
        let mut cw = match s.width {
            Len::Px(p) => p,
            Len::Pct(p) => (cb_w - ml_px - mr_px) * p / 100.0,
            Len::Auto if shrink => {
                let (_, mx) = self.content_widths(id);
                mx.min(avail)
            }
            Len::Auto => avail,
        };
        match s.max_width {
            Len::Px(m) => cw = cw.min(m),
            Len::Pct(p) => cw = cw.min(cb_w * p / 100.0),
            Len::Auto => {}
        }
        if let Len::Px(m) = s.min_width {
            cw = cw.max(m);
        }
        let cw = fin(cw).min(self.maxw);
        let outer = cw + hb;
        let ml = auto_margin(fin(cb_w - outer - ml_px - mr_px), !shrink);
        let bx = x + ml;
        // Background placeholders, sized once the height is known.
        let bg_idx = self.out.len();
        if s.bg[3] > 0 {
            self.out.push(Item::Rect {
                r: Rect {
                    x: bx,
                    y,
                    w: outer,
                    h: 0.0,
                },
                color: s.bg,
                radius: s.radius,
            });
        }
        let img_idx = self.out.len();
        if let Some(ix) = self.load_bg(&s) {
            let r = Rect {
                x: bx,
                y,
                w: outer,
                h: 0.0,
            };
            self.out.push(Item::Image {
                r,
                img: ix,
                clip: Some(r),
                radius: s.radius,
            });
        }
        let kids = self.dom.children(id).to_vec();
        let mut ch = self.block_list(id, &kids, bx + bd[3] + pad[3], y + vb_top, cw);
        if let Len::Px(hh) = s.height {
            ch = ch.max(hh);
        }
        if s.display == Display::ListItem {
            self.marker(id, bx + bd[3] + pad[3], y + vb_top);
        }
        let h = vb_top + ch + vb_bot;
        if let Some(Item::Rect { r, .. }) = self.out.get_mut(bg_idx).filter(|_| s.bg[3] > 0) {
            r.h = h;
        }
        if let Some(Item::Image {
            r, clip: Some(c), ..
        }) = self.out.get_mut(img_idx)
        {
            r.h = h;
            c.h = h;
        }
        self.borders(bx, y, outer, h, &s);
        BoxOut { w: outer, h }
    }

    pub fn borders(&mut self, x: f32, y: f32, w: f32, h: f32, s: &Style) {
        let b = s.border;
        if b.iter().all(|&v| v <= 0.0) {
            return;
        }
        let c = s.border_color;
        if s.radius > 0.0 && b[0] > 0.0 && b.iter().all(|&v| v == b[0]) {
            self.out.push(Item::Stroke {
                r: Rect { x, y, w, h },
                width: b[0],
                color: c[0],
                radius: s.radius,
            });
            return;
        }
        let sides = [
            (Rect { x, y, w, h: b[0] }, c[0]),
            (
                Rect {
                    x,
                    y: y + h - b[2],
                    w,
                    h: b[2],
                },
                c[2],
            ),
            (Rect { x, y, w: b[3], h }, c[3]),
            (
                Rect {
                    x: x + w - b[1],
                    y,
                    w: b[1],
                    h,
                },
                c[1],
            ),
        ];
        for (r, color) in sides {
            if r.w > 0.0 && r.h > 0.0 && color[3] > 0 {
                self.out.push(Item::Rect {
                    r,
                    color,
                    radius: 0.0,
                });
            }
        }
    }

    fn marker(&mut self, li: Id, x: f32, y: f32) {
        let parent = self.dom.parent(li);
        let s = self.style(li).clone();
        if s.list_none || self.style(parent).list_none {
            return;
        }
        let text = if self.dom.tag(parent) == "ol" {
            let n = self
                .dom
                .children(parent)
                .iter()
                .take_while(|&&c| c != li)
                .filter(|&&c| self.dom.tag(c) == "li")
                .count();
            format!("{}.", n.saturating_add(1))
        } else {
            "\u{2022}".to_string()
        };
        let mut b =
            self.lcx
                .tree_builder(self.fcx, 1.0, true, &text_style(&s, brush(&s, 0), false));
        b.push_text(&text);
        let (mut l, text) = b.build();
        l.break_all_lines(None);
        let wmark = l.full_width();
        let layout = self.push_layout(l, text);
        self.out.push(Item::Text {
            layout,
            x: x - wmark - 6.0,
            y,
            clip_h: f32::INFINITY,
            selectable: false,
        });
    }

    fn push_layout(&mut self, l: Layout<Brush>, text: String) -> usize {
        self.doc.layouts.push(l);
        self.doc.texts.push(text);
        self.doc.layouts.len().saturating_sub(1)
    }

    // ------------------------------------------------------------------ inline groups

    /// Collect inline ops for `nodes`. Inline-block contents are laid out
    /// here, into boxes.
    fn collect(
        &mut self,
        nodes: &[Id],
        link: u32,
        avail: f32,
        ops: &mut Vec<Op>,
        boxes: &mut Vec<BoxInfo>,
        visible: &mut bool,
    ) {
        let dom = self.dom;
        for &n in nodes {
            if self.expired() {
                return;
            }
            let Some(node) = dom.node(n) else {
                continue;
            };
            let tag = match &node.kind {
                Kind::Text(t) => {
                    let s = self.style(n);
                    let txt = tame_clusters(s.transform.apply(t));
                    if s.pre || !txt.trim().is_empty() {
                        *visible = true;
                    }
                    ops.push(Op::Text(txt, s.pre));
                    continue;
                }
                Kind::Elem { tag, .. } => &**tag,
            };
            let s = self.style(n).clone();
            if s.display == Display::None {
                continue;
            }
            if tag == "br" {
                ops.push(Op::Br);
                *visible = true;
                continue;
            }
            if tag == "img" {
                self.image_box(n, link, avail, ops, boxes, visible);
                continue;
            }
            let mut l = link;
            if tag == "a"
                && let Some(h) = dom.attr(n, "href")
            {
                l = self.link_id(n, h);
            }
            let deep = self.depth >= MAX_LAYOUT_DEPTH;
            if s.display == Display::Inline || deep {
                // Horizontal margin, border and padding of an inline
                // element: empty boxes at its two ends.
                let side = |i: usize| {
                    let m = s.margin.get(i).map_or(0.0, |&m| px_of(m));
                    let b = s.border.get(i).copied().unwrap_or(0.0);
                    let p = s.padding.get(i).copied().unwrap_or(0.0);
                    (m + b + p).max(0.0)
                };
                ops.push(Op::Open(n, l));
                spacer(side(3), ops, boxes);
                self.collect(dom.children(n), l, avail, ops, boxes, visible);
                spacer(side(1), ops, boxes);
                ops.push(Op::Close);
                continue;
            }
            // Inline-block, or a block inside inline: a shrink-to-fit box.
            *visible = true;
            if self.measuring != 0 {
                let (mn, mx) = self.outer_widths(n);
                let w = if self.measuring == 1 { mn } else { mx };
                ops.push(Op::Box(boxes.len()));
                boxes.push(BoxInfo {
                    w,
                    h: 1.0,
                    items: vec![],
                    va: VAlign::Baseline,
                });
                continue;
            }
            let (ml, mr, mt, mb) = (
                px_of(s.margin[3]),
                px_of(s.margin[1]),
                px_of(s.margin[0]),
                px_of(s.margin[2]),
            );
            let saved = std::mem::take(&mut self.out);
            let r = self.box_at_nomargin(n, ml, mt, avail);
            let mut items = std::mem::replace(&mut self.out, saved);
            if l != 0 {
                items.push(Item::Link {
                    r: Rect {
                        x: ml,
                        y: mt,
                        w: r.w,
                        h: r.h,
                    },
                    link: l,
                });
            }
            ops.push(Op::Box(boxes.len()));
            boxes.push(BoxInfo {
                w: ml + r.w + mr,
                h: mt + r.h + mb,
                items,
                va: s.valign,
            });
        }
    }

    pub fn image_box(
        &mut self,
        n: Id,
        link: u32,
        avail: f32,
        ops: &mut Vec<Op>,
        boxes: &mut Vec<BoxInfo>,
        visible: &mut bool,
    ) {
        let s = self.style(n).clone();
        let src = self.dom.attr(n, "src").unwrap_or("").trim().to_string();
        let avail = avail.min(self.doc.width.max(1.0) * 4.0);
        let wa = match s.width {
            Len::Px(p) => Some(fin(p)),
            // Percentage-sized images add nothing to intrinsic widths.
            Len::Pct(_) if self.measuring != 0 => {
                self.pct_seen = self.pct_seen.saturating_add(1);
                Some(0.0)
            }
            Len::Pct(p) => Some(fin(avail * p / 100.0)),
            Len::Auto => None,
        };
        let ha = match s.height {
            Len::Px(p) => Some(fin(p)),
            _ => None,
        };
        let max_w = match s.max_width {
            Len::Px(m) => Some(m),
            Len::Pct(p) => Some(avail * p / 100.0),
            Len::Auto => None,
        };
        let skip = src.is_empty() || super::css::is_remote(&src) && is_tracker(&s);
        let target = wa.map_or(0.0, |w| w.min(avail)) as u32;
        let ix = if skip {
            None
        } else {
            self.load_image(&src, target)
        };
        let mut items = vec![];
        let (w, h) = match ix.and_then(|ix| {
            self.doc
                .images
                .get(ix)
                .map(|im| (ix, im.nat_w as f32, im.nat_h as f32))
        }) {
            Some((ix, nw, nh)) => {
                let mut w = wa.unwrap_or_else(|| ha.map_or(nw, |h| h * nw / nh));
                let mut h = ha.unwrap_or_else(|| wa.map_or(nh, |w| w * nh / nw));
                if let Some(m) = max_w.filter(|&m| w > m) {
                    if ha.is_none() {
                        h = h * m / w;
                    }
                    w = m;
                }
                // As in webmail, an image never exceeds its line.
                if w > avail && avail > 1.0 {
                    h = h * avail / w;
                    w = avail;
                }
                let (w, h) = (fin(w).min(self.maxw), fin(h).min(MAX_DOC_H));
                let r = Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                };
                if s.bg[3] > 0 {
                    items.push(Item::Rect {
                        r,
                        color: s.bg,
                        radius: s.radius,
                    });
                }
                items.push(Item::Image {
                    r,
                    img: ix,
                    clip: None,
                    radius: s.radius,
                });
                (w, h)
            }
            None => {
                // Blocked, missing or undecodable: reserve the declared box,
                // outline it and show the alt text in it.
                let alt = tame_clusters(
                    self.dom
                        .attr(n, "alt")
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(MAX_ALT_CHARS)
                        .collect(),
                );
                let (w, h) = match (wa, ha) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => (
                        w,
                        if alt.is_empty() {
                            0.0
                        } else {
                            (s.font_size * 1.6).max(20.0)
                        },
                    ),
                    (None, Some(h)) => (h, h),
                    (None, None) => (0.0, 0.0),
                };
                let w0 = max_w.map_or(w, |m| w.min(m));
                let w = w0.min(avail.max(1.0));
                let h = if w0 > 0.0 && w < w0 && wa.is_some() && ha.is_some() {
                    h * w / w0
                } else {
                    h
                };
                let (w, h) = (fin(w), fin(h).min(MAX_DOC_H));
                if w < 3.0 || h < 3.0 {
                    if !alt.is_empty() && wa.is_none() && ha.is_none() {
                        ops.push(Op::Open(n, link));
                        ops.push(Op::Text(alt, false));
                        ops.push(Op::Close);
                        *visible = true;
                    }
                    return;
                }
                let r = Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                };
                let fill = if s.bg[3] > 0 {
                    s.bg
                } else {
                    [128, 128, 128, 24]
                };
                let radius = s.radius.max(4.0).min(w.min(h) / 2.0);
                items.push(Item::Rect {
                    r,
                    color: fill,
                    radius,
                });
                items.push(Item::Stroke {
                    r,
                    width: 1.0,
                    color: [128, 128, 128, 80],
                    radius,
                });
                if !alt.is_empty() {
                    self.alt_text(&s, &alt, link, w, h, &mut items);
                }
                (w, h)
            }
        };
        if link != 0 {
            items.push(Item::Link {
                r: Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                },
                link,
            });
        }
        ops.push(Op::Box(boxes.len()));
        boxes.push(BoxInfo {
            w,
            h,
            items,
            va: s.valign,
        });
        *visible = true;
    }

    /// Alt text centred in a placeholder, only when it fits.
    fn alt_text(&mut self, s: &Style, alt: &str, link: u32, w: f32, h: f32, items: &mut Vec<Item>) {
        let mut ts = s.clone();
        ts.nowrap = false;
        ts.bold = false;
        ts.italic = false;
        ts.underline = false;
        ts.font_size = s.font_size.min(13.0);
        ts.line_height = LineH::Normal;
        ts.color[3] = (ts.color[3] / 3).saturating_mul(2);
        let mut b = self.lcx.tree_builder(
            self.fcx,
            1.0,
            true,
            &text_style(&ts, brush(&ts, link), false),
        );
        b.push_text(alt);
        let (mut l, text) = b.build();
        l.break_all_lines(Some((w - 8.0).max(1.0)));
        l.align(align_of(Align::Center), AlignmentOptions::default());
        let widest = l
            .lines()
            .map(|ln| ln.metrics().advance)
            .fold(0.0f32, f32::max);
        let first_fits = l
            .lines()
            .next()
            .is_some_and(|ln| ln.metrics().line_height <= h + 1.0);
        if widest <= w - 4.0 && (l.height() <= h + 1.0 || first_fits) {
            let ty = ((h - l.height()) / 2.0).max(0.0);
            let layout = self.push_layout(l, text);
            items.push(Item::Text {
                layout,
                x: 4.0,
                y: ty,
                clip_h: h - ty,
                selectable: false,
            });
        }
    }

    /// The link id of `<a>` element `n` (0 when its `href` is inert).
    fn link_id(&mut self, n: Id, href: &str) -> u32 {
        if let Some(&i) = self.link_ids.get(&n) {
            return i;
        }
        let i = match links::sanitize(href) {
            Some(href) if self.doc.links.len() <= MAX_LINKS => {
                let text = self.link_text(n);
                let i = u32::try_from(self.doc.links.len()).unwrap_or(0);
                self.doc.links.push(Link { href, text });
                i
            }
            _ => 0,
        };
        self.link_ids.insert(n, i);
        i
    }

    /// The visible text of element `n`, whitespace collapsed.
    fn link_text(&self, n: Id) -> String {
        let mut raw = String::new();
        let mut stack: Vec<Id> = self.dom.children(n).iter().rev().copied().collect();
        let mut seen = 0u32;
        while let Some(c) = stack.pop() {
            seen = seen.saturating_add(1);
            if seen > 4096 || raw.len() > MAX_LINK_TEXT.saturating_mul(16) {
                break;
            }
            let s = self.style(c);
            if s.display == Display::None || s.color[3] == 0 || s.font_size < 0.5 {
                continue;
            }
            match self.dom.node(c).map(|x| &x.kind) {
                Some(Kind::Text(t)) => raw.push_str(t),
                Some(Kind::Elem { .. }) => {
                    stack.extend(self.dom.children(c).iter().rev().copied());
                }
                None => {}
            }
        }
        let words: Vec<&str> = raw.split_whitespace().collect();
        words.join(" ").chars().take(MAX_LINK_TEXT).collect()
    }

    /// Mark where element `id` starts, if a `#fragment` link points at it.
    fn anchor(&mut self, id: Id, y: f32) {
        if let Some(&anchor) = self.anchor_of.get(&id) {
            self.out.push(Item::Anchor { y, anchor });
        }
    }

    /// Mark the anchors among the inline `nodes` of a group placed at `y`
    /// (inline-blocks mark their own in [`Self::box_at`]).
    fn inline_anchors(&mut self, nodes: &[Id], y: f32) {
        if self.anchor_of.is_empty() {
            return;
        }
        let deep = self.depth >= MAX_LAYOUT_DEPTH;
        let mut stack: Vec<Id> = nodes.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if !self.dom.is_elem(n) || self.style(n).display == Display::None {
                continue;
            }
            self.anchor(n, y);
            if deep || self.style(n).display == Display::Inline {
                stack.extend(self.dom.children(n).iter().rev().copied());
            }
        }
    }

    /// The background image of `s`; a remote one on a tracking-sized or
    /// hidden box is never loaded, as for `<img>`.
    pub fn load_bg(&mut self, s: &Style) -> Option<usize> {
        let src = s.bg_image.clone()?;
        if super::css::is_remote(&src) && is_tracker(s) {
            return None;
        }
        self.load_image(&src, 0)
    }

    pub fn load_image(&mut self, src: &str, target_w: u32) -> Option<usize> {
        if let Some(r) = self.img_cache.get(src) {
            return *r;
        }
        let r = self
            .src
            .load(src)
            .and_then(|bytes| img::decode(&bytes, target_w, &mut self.budget))
            .map(|d| {
                self.doc.images.push(d);
                self.doc.loaded.push(src.to_string());
                self.doc.images.len().saturating_sub(1)
            });
        self.img_cache.insert(src.to_string(), r);
        r
    }

    fn build_group(&mut self, parent: Id, nodes: &[Id], avail: f32, emergency: bool) -> Group {
        let pct0 = self.pct_seen;
        let mut ops = vec![];
        let mut boxes = vec![];
        let mut visible = false;
        self.collect(nodes, 0, avail, &mut ops, &mut boxes, &mut visible);
        let ps = self.style(parent).clone();
        let root = text_style(&ps, brush(&ps, 0), emergency);
        let mut b = self.lcx.tree_builder(self.fcx, 1.0, true, &root);
        b.set_white_space_mode(WhiteSpaceCollapse::Collapse);
        let mut room = MAX_GROUP_TEXT;
        let mut capped = false;
        for op in &ops {
            match op {
                // Past the cap only the spans are still opened and closed.
                Op::Text(..) | Op::Br | Op::Box(_) if capped => {}
                Op::Text(t, pre) if t.len() > room => {
                    let kept = cut_text(t, room);
                    if *pre {
                        preserved(&mut b, kept);
                    } else {
                        collapsible(&mut b, kept);
                    }
                    preserved(&mut b, "\n");
                    b.push_text(CAPPED_MARKER);
                    capped = true;
                    self.text_capped = true;
                }
                Op::Open(id, link) => {
                    let s = self.st.get(*id as usize).unwrap_or(&self.default);
                    b.push_style_span(text_style(s, brush(s, *link), emergency));
                }
                Op::Close => b.pop_style_span(),
                Op::Text(t, false) => {
                    room = room.saturating_sub(t.len());
                    collapsible(&mut b, t);
                }
                // parley applies the white-space mode when text is
                // committed (at the next span boundary), so preserved text
                // gets its own span, committed while Preserve is set.
                Op::Text(t, true) => {
                    room = room.saturating_sub(t.len());
                    preserved(&mut b, t);
                }
                Op::Br => preserved(&mut b, "\n"),
                Op::Box(i) => {
                    let (w, h) = boxes.get(*i).map_or((0.0, 0.0), |bi| (bi.w, bi.h));
                    b.push_inline_box(InlineBox {
                        id: *i as u64,
                        kind: InlineBoxKind::InFlow,
                        index: 0,
                        width: w,
                        height: h,
                    });
                }
            }
        }
        let (layout, text) = b.build();
        let dep = !boxes.is_empty() || self.pct_seen != pct0;
        Group {
            layout,
            text,
            boxes,
            visible,
            built_avail: avail,
            dep,
        }
    }

    pub fn group_widths(&mut self, parent: Id, nodes: &[Id]) -> (f32, f32) {
        let Some(&first) = nodes.first() else {
            return (0.0, 0.0);
        };
        let key = (parent, first);
        if !self.groups.contains_key(&key) {
            let was = std::mem::replace(&mut self.measuring, 2);
            let g = self.build_group(parent, nodes, 10000.0, false);
            if g.dep {
                // Inline boxes: min-content needs the boxes at their min-content widths.
                self.measuring = 1;
                let gm = self.build_group(parent, nodes, 10000.0, false);
                self.group_min
                    .insert(key, gm.layout.calculate_content_widths().min);
            }
            self.measuring = was;
            self.groups.insert(key, g);
        }
        let Some(g) = self.groups.get(&key).filter(|g| g.visible) else {
            return (0.0, 0.0);
        };
        let cw = g.layout.calculate_content_widths();
        let mn = self.group_min.get(&key).copied().unwrap_or(cw.min);
        (mn, cw.max)
    }

    pub fn layout_group(&mut self, parent: Id, nodes: &[Id], x: f32, y: f32, w: f32) -> f32 {
        let Some(&first) = nodes.first() else {
            return 0.0;
        };
        let w = fin(w);
        let key = (parent, first);
        let mut g = match self.groups.remove(&key) {
            // Width-dependent groups are rebuilt when the width changed.
            Some(g) if !g.dep || (g.built_avail - w).abs() < 0.5 => g,
            _ => self.build_group(parent, nodes, w, false),
        };
        if !g.visible {
            // An empty `<a name>` still marks its place.
            self.inline_anchors(nodes, y);
            return 0.0;
        }
        let align = align_of(self.style(parent).align);
        let wrap = if g.dep { w + FIT_TOLERANCE } else { w };
        g.layout.break_all_lines(Some(wrap));
        // Emergency break-word: a word wider than the line is split rather
        // than overflowing. Only groups that overflow pay for the rebuild.
        if g.layout.width() > wrap + FIT_TOLERANCE && !self.style(parent).nowrap {
            let mut e = self.build_group(parent, nodes, w, true);
            e.layout.break_all_lines(Some(wrap));
            g = e;
        }
        g.layout.align(align, AlignmentOptions::default());
        let h = g.layout.height();
        for line in g.layout.lines() {
            let lm = *line.metrics();
            for item in line.items() {
                match item {
                    PositionedLayoutItem::InlineBox(pb) => {
                        let Some(bi) = g.boxes.get_mut(pb.id as usize) else {
                            continue;
                        };
                        let mut items = std::mem::take(&mut bi.items);
                        let line_h = lm.block_max_coord - lm.block_min_coord;
                        let by = match bi.va {
                            VAlign::Top => lm.block_min_coord,
                            VAlign::Middle => lm.block_min_coord + (line_h - bi.h) / 2.0,
                            VAlign::Bottom => lm.block_max_coord - bi.h,
                            VAlign::Baseline => pb.y,
                        };
                        shift(&mut items, x + pb.x, y + by);
                        self.out.append(&mut items);
                    }
                    PositionedLayoutItem::GlyphRun(gr) => {
                        let link = gr.style().brush.link;
                        if link != 0 {
                            self.out.push(Item::Link {
                                r: Rect {
                                    x: x + gr.offset(),
                                    y: y + lm.block_min_coord,
                                    w: gr.advance(),
                                    h: lm.block_max_coord - lm.block_min_coord,
                                },
                                link,
                            });
                        }
                    }
                }
            }
        }
        let layout = self.push_layout(g.layout, g.text);
        self.out.push(Item::Text {
            layout,
            x,
            y,
            clip_h: f32::INFINITY,
            selectable: true,
        });
        self.inline_anchors(nodes, y);
        h
    }

    // ------------------------------------------------------------------ intrinsic widths

    /// (min-content, max-content) widths of `id`'s content box.
    pub fn content_widths(&mut self, id: Id) -> (f32, f32) {
        if let Some(&r) = self.widths.get(&id) {
            return r;
        }
        let r = if self.style(id).display == Display::Table && id != ROOT {
            self.table_intrinsic(id)
        } else {
            let kids = self.dom.children(id).to_vec();
            self.list_widths(id, &kids)
        };
        self.widths.insert(id, r);
        r
    }

    /// Intrinsic widths of a list of children of `parent`.
    pub fn list_widths(&mut self, parent: Id, kids: &[Id]) -> (f32, f32) {
        if self.depth >= MAX_LAYOUT_DEPTH || self.expired() {
            return (0.0, 0.0);
        }
        self.depth = self.depth.saturating_add(1);
        let (mut mn, mut mx) = (0.0f32, 0.0f32);
        let mut run: Vec<Id> = vec![];
        let mut fsum = 0.0f32;
        for &c in kids {
            match self.classify(c) {
                Cls::Skip => {}
                Cls::Inline => run.push(c),
                Cls::Float => {
                    let (a, b) = if self.dom.tag(c) == "img" {
                        let s = self.style(c);
                        let w = match s.width {
                            Len::Px(p) => p.min(self.maxw),
                            _ => 0.0,
                        } + px_of(s.margin[1])
                            + px_of(s.margin[3]);
                        (w, w)
                    } else {
                        self.outer_widths(c)
                    };
                    mn = mn.max(a);
                    fsum += b;
                }
                Cls::Block => {
                    if !run.is_empty() {
                        let (a, b) = self.group_widths(parent, &std::mem::take(&mut run));
                        mn = mn.max(a);
                        mx = mx.max(b);
                    }
                    let (a, b) = self.outer_widths(c);
                    mn = mn.max(a);
                    mx = mx.max(b);
                }
            }
        }
        if !run.is_empty() {
            let (a, b) = self.group_widths(parent, &run);
            mn = mn.max(a);
            mx = mx.max(b);
        }
        self.depth = self.depth.saturating_sub(1);
        (fin(mn).min(self.maxw), fin(mx + fsum).min(self.maxw))
    }

    /// Border box plus horizontal margins, honouring a specified width.
    pub fn outer_widths(&mut self, c: Id) -> (f32, f32) {
        let s = self.style(c).clone();
        let extra = s.padding[1] + s.padding[3] + s.border[1] + s.border[3];
        let marg = px_of(s.margin[1]) + px_of(s.margin[3]);
        if s.display == Display::Table {
            let (mn, mx) = self.table_intrinsic(c);
            return (mn + marg, mx + marg);
        }
        let (mut mn, mut mx) = self.content_widths(c);
        match s.width {
            Len::Px(w) => {
                mn = w.min(self.maxw);
                mx = mn;
            }
            // Percentage widths contribute min-content only.
            Len::Pct(_) => mx = mx.max(mn),
            Len::Auto => {}
        }
        if let Len::Px(m) = s.max_width {
            mx = mx.min(m);
            mn = mn.min(m);
        }
        if let Len::Px(m) = s.min_width {
            mn = mn.max(m);
            mx = mx.max(m);
        }
        (fin(mn + extra + marg), fin(mx + extra + marg))
    }
}

/// Push collapsible text `t`. parley trims each span's edges with
/// `str::trim`, which also drops no-break and other non-ASCII spaces that
/// CSS never collapses: Word's pseudo-bullets end in a span holding only
/// `&nbsp;`s, so the bullet touched its text ("•Ship"). A leading or
/// trailing white-space run holding such a space is committed preserved
/// instead, with other white-space runs squeezed to one space as collapsing
/// would leave them.
fn collapsible(b: &mut parley::TreeBuilder<'_, Brush>, t: &str) {
    let rest = t.trim_start();
    let mid = rest.trim_end();
    let lead = t.strip_suffix(rest).unwrap_or_default();
    let tail = rest.strip_prefix(mid).unwrap_or_default();
    // Line and paragraph separators would become hard breaks when
    // preserved; they collapse like ASCII white space instead.
    let collapses =
        |c: char| c.is_ascii_whitespace() || c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
    let kept = |run: &str| run.chars().any(|c| c.is_whitespace() && !collapses(c));
    for (run, edge) in [(lead, true), (mid, false), (tail, true)] {
        if run.is_empty() {
            continue;
        }
        if edge && kept(run) {
            let mut squeezed = String::with_capacity(run.len());
            for c in run.chars() {
                if !collapses(c) {
                    squeezed.push(c);
                } else if !squeezed.ends_with(' ') {
                    squeezed.push(' ');
                }
            }
            preserved(b, &squeezed);
        } else {
            b.push_text(run);
        }
    }
}

/// Characters of `alt` text laid out: it only has to fit a placeholder.
const MAX_ALT_CHARS: usize = 400;

/// Characters kept per grapheme cluster: parley counts a cluster's
/// characters in a `u8`, and stacks of combining marks ("zalgo") carry no
/// meaning past a few.
const MAX_CLUSTER_CHARS: usize = 32;

/// `t` with every grapheme cluster cut to [`MAX_CLUSTER_CHARS`]
/// characters (ASCII text has none that long, and passes untouched).
fn tame_clusters(t: String) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let long = |g: &&str| g.len() > MAX_CLUSTER_CHARS && g.chars().nth(MAX_CLUSTER_CHARS).is_some();
    if t.is_ascii() || !t.graphemes(true).any(|g| long(&g)) {
        return t;
    }
    let mut out = String::with_capacity(t.len());
    for g in t.graphemes(true) {
        if long(&g) {
            out.extend(g.chars().take(MAX_CLUSTER_CHARS));
        } else {
            out.push_str(g);
        }
    }
    out
}

/// The start of `t` at most `max` bytes long, ending at its last white
/// space when there is one in the second half (else at a character
/// boundary, inside a very long word).
fn cut_text(t: &str, max: usize) -> &str {
    let mut end = max.min(t.len());
    while end > 0 && !t.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let head = t.get(..end).unwrap_or_default();
    match head.rfind(char::is_whitespace) {
        Some(ws) if ws >= end / 2 => head.get(..ws).unwrap_or(head),
        _ => head,
    }
}

/// Commit `t` with white space preserved.
fn preserved(b: &mut parley::TreeBuilder<'_, Brush>, t: &str) {
    b.push_style_modification_span(&[]);
    b.set_white_space_mode(WhiteSpaceCollapse::Preserve);
    b.push_text(t);
    b.pop_style_span();
    b.set_white_space_mode(WhiteSpaceCollapse::Collapse);
}

fn align_of(a: Align) -> Alignment {
    match a {
        Align::Left => Alignment::Start,
        Align::Center => Alignment::Center,
        Align::Right => Alignment::End,
        Align::Justify => Alignment::Justify,
    }
}

/// The brush for text in style `s`; hidden text is transparent.
pub(super) fn brush(s: &Style, link: u32) -> Brush {
    Brush {
        color: if s.hidden { [0; 4] } else { s.color },
        link,
    }
}

fn text_style(s: &Style, brush: Brush, emergency: bool) -> TextStyle<'static, 'static, Brush> {
    TextStyle {
        font_family: FontFamily::Source(Cow::Owned(s.font_family.to_string())),
        font_size: s.font_size.max(0.0),
        font_weight: if s.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        font_style: if s.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        brush,
        has_underline: s.underline,
        has_strikethrough: s.strike,
        letter_spacing: s.letter_spacing,
        line_height: match s.line_height {
            LineH::Normal => LineHeight::MetricsRelative(1.0),
            LineH::Mult(m) => LineHeight::FontSizeRelative(m),
            LineH::Px(p) => LineHeight::Absolute(p),
        },
        text_wrap_mode: if s.nowrap {
            TextWrapMode::NoWrap
        } else {
            TextWrapMode::Wrap
        },
        word_break: if s.break_all {
            WordBreak::BreakAll
        } else {
            WordBreak::Normal
        },
        // parley 0.11's BreakWord also breaks where a normal break exists,
        // so it is opt-in (or an emergency for overflowing lines).
        overflow_wrap: if s.wrap_anywhere {
            OverflowWrap::Anywhere
        } else if emergency {
            OverflowWrap::BreakWord
        } else {
            OverflowWrap::Normal
        },
        ..TextStyle::default()
    }
}

/// Resample every drawn image once to its final device size so tile paints
/// are 1:1 copies. Huge targets keep the original; the pixel budget bounds
/// the copies.
fn prescale_images(doc: &mut Doc, scale: f32, budget: &mut u64) {
    let mut done: HashMap<(usize, u32, u32), usize> = HashMap::new();
    for i in 0..doc.items.len() {
        let Some(Item::Image { r, img, clip, .. }) = doc.items.get(i) else {
            continue;
        };
        let Some(im) = doc.images.get(*img) else {
            continue;
        };
        let (iw, ih) = (im.pix.width() as f32, im.pix.height() as f32);
        let (rw, rh) = (r.w * scale, r.h * scale);
        let (w, h) = if clip.is_some() {
            let s = (rw / iw).max(rh / ih);
            (iw * s, ih * s)
        } else {
            (rw, rh)
        };
        let (tw, th) = (
            fin(w).round().max(1.0) as u32,
            fin(h).round().max(1.0) as u32,
        );
        if (tw as f32 - iw).abs() < 1.0 && (th as f32 - ih).abs() < 1.0 {
            continue;
        }
        let key = (*img, tw, th);
        let ni = match done.get(&key) {
            Some(&k) => k,
            None => {
                let px = u64::from(tw).saturating_mul(u64::from(th));
                if px > *budget {
                    continue;
                }
                let Some(pix) = img::resample(&im.pix, tw, th) else {
                    continue;
                };
                *budget = budget.saturating_sub(px);
                let (nat_w, nat_h, dark_on_clear) = (im.nat_w, im.nat_h, im.dark_on_clear);
                doc.images.push(ImageData {
                    pix,
                    nat_w,
                    nat_h,
                    dark_on_clear,
                });
                let k = doc.images.len().saturating_sub(1);
                done.insert(key, k);
                k
            }
        };
        if let Some(Item::Image { img, .. }) = doc.items.get_mut(i) {
            *img = ni;
        }
    }
    // Drop originals no longer referenced.
    let mut used = vec![false; doc.images.len()];
    for it in &doc.items {
        if let Item::Image { img, .. } = it
            && let Some(u) = used.get_mut(*img)
        {
            *u = true;
        }
    }
    for (im, used) in doc.images.iter_mut().zip(used) {
        if !used && let Some(p) = tiny_skia::Pixmap::new(1, 1) {
            im.pix = p;
        }
    }
}
