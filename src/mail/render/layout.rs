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
use super::select::TextPos;
use super::style::{
    Align, BgSize, Color, Display, Float, Gradient, Len, LineH, LineStyle, Radius, Style, Styles,
    VAlign,
};
use super::{Deadline, links};
use parley::{
    Alignment, AlignmentOptions, BreakReason, FontContext, FontFamily, FontStyle, FontWeight,
    InlineBox, InlineBoxKind, Layout, LayoutContext, LineHeight, OverflowWrap,
    PositionedLayoutItem, TextStyle, TextWrapMode, WhiteSpaceCollapse, WordBreak,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Brush {
    pub color: Color,
    /// Index into [`Doc::links`]; 0 for none.
    pub link: u32,
    /// The inline element whose background and border this text sits on
    /// (its node id); 0 for none.
    pub deco: u32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush {
            color: [0, 0, 0, 255],
            link: 0,
            deco: 0,
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
        radius: Radius,
    },
    /// A rounded box's border, or a dashed or dotted one or one side of it (a
    /// rectangle as thin as `width`: the line along its length).
    Stroke {
        r: Rect,
        width: f32,
        color: Color,
        radius: Radius,
        style: LineStyle,
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
    /// Stretched to fill `r`, shown only within `clip`, with `alpha`.
    Image {
        r: Rect,
        img: usize,
        clip: Option<Rect>,
        radius: Radius,
        alpha: f32,
    },
    /// A background image over box `r`: `tile` is the first tile, relative to
    /// the page; it repeats along the axes that say so.
    Tiled {
        r: Rect,
        img: usize,
        tile: Rect,
        repeat: (bool, bool),
        radius: Radius,
    },
    /// A linear gradient from (`x0`, `y0`) to (`x1`, `y1`) on the page over `r`.
    Gradient {
        r: Rect,
        radius: Radius,
        line: [f32; 4],
        stops: std::sync::Arc<Gradient>,
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
    /// For each inline group whose layout does not depend on the width: a
    /// digest of everything the shaper was given, and the index of its
    /// layout in [`Doc::layouts`]. A second layout of the same message
    /// (another width) reuses these instead of shaping the text again.
    pub(super) shaped: Vec<(GroupKey, u32)>,
    /// Inline groups taken from the earlier layout instead of shaped.
    pub reused: usize,
}

/// Where a line of [`Doc::text_lines`] came from: what maps a text
/// position onto it.
#[derive(Debug, Clone)]
struct LineSource {
    item: usize,
    range: std::ops::Range<usize>,
    /// The visible byte ranges of the item's text the line was built from,
    /// in text order.
    parts: Vec<std::ops::Range<usize>>,
    /// Characters trimmed off the start of the line's text.
    lead: usize,
    /// Characters of the line's (trimmed) text.
    chars: usize,
}

impl LineSource {
    /// Characters of the line's (trimmed) text before byte `index` of its
    /// item's `text`.
    fn chars_before(&self, text: &str, index: usize) -> usize {
        let raw: usize = self
            .parts
            .iter()
            .filter(|p| p.start < index)
            .map(|p| {
                let end = p.end.min(index);
                text.get(p.start..end).map_or(0, |s| s.chars().count())
            })
            .sum();
        raw.saturating_sub(self.lead).min(self.chars)
    }
}

/// What maps a selection onto the lines of [`Doc::text_lines_upto`]
/// without walking the layout again: built with the lines, kept with the
/// page, asked once per selection change.
#[derive(Debug, Clone, Default)]
pub struct LineMap {
    lines: Vec<LineSource>,
    /// Whether lines were left out past the cap.
    capped: bool,
}

impl LineMap {
    /// Where the selection from `a` to `b` (either order) of `doc` starts
    /// and ends in the mapped lines, as (line, character in its text): what
    /// screen readers get as the page's text selection. A position in
    /// hidden text or between lines goes to the start of the next line; an
    /// end past the last mapped line goes to that line's end. `None` with
    /// no lines, or when the selection starts past the cap.
    pub fn positions(
        &self,
        doc: &Doc,
        a: TextPos,
        b: TextPos,
    ) -> Option<((usize, usize), (usize, usize))> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let last = self.lines.len().checked_sub(1)?;
        let end_of_last = (last, self.lines.last()?.chars);
        let at = |p: TextPos| -> Option<(usize, usize)> {
            let i = self.lines.partition_point(|l| {
                l.item < p.item || (l.item == p.item && l.range.end < p.index)
            });
            let l = self.lines.get(i)?;
            Some(if l.item == p.item && p.index >= l.range.start {
                (i, l.chars_before(doc.text_of(l.item), p.index))
            } else {
                (i, 0)
            })
        };
        let start = match at(a) {
            Some(s) => s,
            None if self.capped => return None,
            None => end_of_last,
        };
        Some((start, at(b).unwrap_or(end_of_last)))
    }
}

/// One visual line of text (see [`Doc::text_lines`]).
#[derive(Debug, Clone, Default)]
pub struct TextLine {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
    /// Each character of `text`: its left edge from `x`, and its width.
    pub chars: Vec<(f32, f32)>,
    /// Whether a paragraph (or a forced line break) ends with this line;
    /// otherwise its text wraps on to the next line.
    pub ends_paragraph: bool,
}

/// A visible cluster of a line: its text and where it is drawn.
struct LineCluster {
    range: std::ops::Range<usize>,
    /// Left edge from the line's start.
    x: f32,
    advance: f32,
}

/// The visible clusters of `line` within its text range `line_r`, in text
/// order, each with its left edge from the line's start (`offset`, the
/// line's own offset in its layout). A run split by style comes as several
/// glyph runs: each run's clusters are walked once, from where its first
/// glyph run starts.
fn line_clusters(
    line: &parley::Line<'_, Brush>,
    line_r: &std::ops::Range<usize>,
    offset: f32,
) -> Vec<LineCluster> {
    let mut out = Vec::new();
    let mut last_run = None;
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(gr) = item else {
            continue;
        };
        let run = gr.run();
        let run_r = run.text_range();
        if last_run.as_ref() == Some(&run_r) {
            continue;
        }
        last_run = Some(run_r);
        let mut x = gr.offset() - offset;
        for c in run.visual_clusters() {
            let advance = c.advance();
            let shown = c.first_style().brush.color[3] != 0 && run.font_size() >= 0.5;
            let r = c.text_range();
            // A run can span lines: keep this line's part.
            if shown && r.start >= line_r.start && r.end <= line_r.end {
                out.push(LineCluster {
                    range: r,
                    x,
                    advance,
                });
            }
            x += advance;
        }
    }
    out.sort_by_key(|c| c.range.start);
    out
}

impl Doc {
    /// Visible text, one entry per laid-out line in display-list order.
    /// Transparent (hidden) and zero-size text is left out.
    pub fn text_lines(&self) -> Vec<TextLine> {
        self.text_lines_upto(usize::MAX).0
    }

    /// The first `max_lines` of [`Doc::text_lines`], and what maps a
    /// selection onto them ([`LineMap::positions`]).
    pub fn text_lines_upto(&self, max_lines: usize) -> (Vec<TextLine>, LineMap) {
        let mut out = vec![];
        let mut map = LineMap::default();
        for (index, it) in self.items.iter().enumerate() {
            let Item::Text { layout, x, y, .. } = it else {
                continue;
            };
            let (Some(l), Some(text)) = (self.layouts.get(*layout), self.texts.get(*layout)) else {
                continue;
            };
            for line in l.lines() {
                if out.len() >= max_lines {
                    map.capped = true;
                    return (out, map);
                }
                let m = line.metrics();
                let line_r = line.text_range();
                let clusters = line_clusters(&line, &line_r, m.offset);
                let mut s = String::new();
                let mut chars = Vec::new();
                let mut parts: Vec<std::ops::Range<usize>> = Vec::new();
                for c in &clusters {
                    let t = text.get(c.range.clone()).unwrap_or("");
                    // A ligature's characters share its advance.
                    let n = t.chars().count().max(1) as f32;
                    let w = c.advance / n;
                    for (k, _) in t.chars().enumerate() {
                        chars.push((c.x + w * k as f32, w));
                    }
                    s.push_str(t);
                    match parts.last_mut() {
                        Some(p) if p.end == c.range.start => p.end = c.range.end,
                        _ => parts.push(c.range.clone()),
                    }
                }
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let lead = s
                    .chars()
                    .count()
                    .saturating_sub(s.trim_start().chars().count());
                let count = trimmed.chars().count();
                chars = chars.into_iter().skip(lead).take(count).collect();
                map.lines.push(LineSource {
                    item: index,
                    range: line_r,
                    parts,
                    lead,
                    chars: count,
                });
                out.push(TextLine {
                    x: *x + m.offset,
                    y: *y + m.block_min_coord,
                    w: m.advance,
                    h: m.block_max_coord - m.block_min_coord,
                    text: trimmed.to_string(),
                    chars,
                    ends_paragraph: !matches!(
                        line.break_reason(),
                        BreakReason::Regular | BreakReason::Emergency
                    ),
                });
            }
        }
        (out, map)
    }

    /// [`LineMap::positions`] over the first `max_lines` lines. It builds
    /// the map: a caller asking more than once keeps the one from
    /// [`Doc::text_lines_upto`].
    pub fn line_positions(
        &self,
        a: TextPos,
        b: TextPos,
        max_lines: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        self.text_lines_upto(max_lines).1.positions(self, a, b)
    }

    /// The text of text item `item` (empty for any other item).
    fn text_of(&self, item: usize) -> &str {
        match self.items.get(item) {
            Some(Item::Text { layout, .. }) => self.texts.get(*layout).map_or("", |t| t.as_str()),
            _ => "",
        }
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
pub const MAX_DOC_H: f32 = 200_000.0;
/// The narrowest column deep quotes indent their text to (CSS px), or half
/// the page if that is less.
const MIN_COLUMN: f32 = 320.0;
/// Block nesting past this is flattened.
pub const MAX_LAYOUT_DEPTH: u32 = 48;
/// The widest inline margin or padding honoured.
const MAX_INLINE_SPACE: f32 = 200.0;
/// Text laid out per inline group (one paragraph, in effect): the rest is
/// cut at a white-space boundary and replaced by [`CAPPED_MARKER`], so one
/// pathological paragraph cannot stall shaping and line breaking.
pub const MAX_GROUP_TEXT: usize = 64 * 1024;
/// A digest of the shaper's input for an inline group (two SipHash states).
pub(super) type GroupKey = (u64, u64);
const CAPPED_MARKER: &str = "[Message too long: View as plain text shows the rest]";
/// Inline content wider than its line by less than this still fits
/// (side-by-side columns whose widths add up to the container's exactly).
const FIT_TOLERANCE: f32 = 2.0;

pub(super) enum Op {
    /// An inline element: its node, link id and decorating element.
    Open(Id, u32, Id),
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
    /// The inline element whose background this box extends (its padding
    /// and border); 0 for none.
    pub deco: Id,
}

/// An empty inline box `w` wide (inline margins and padding), if visible.
fn spacer(w: f32, deco: Id, ops: &mut Vec<Op>, boxes: &mut Vec<BoxInfo>) {
    if w >= 0.5 && w.is_finite() {
        ops.push(Op::Box(boxes.len()));
        boxes.push(BoxInfo {
            w: w.min(MAX_INLINE_SPACE),
            h: 0.0,
            items: vec![],
            va: VAlign::Baseline,
            deco,
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
    /// The digest of the shaper's input (see [`Layouter::group_key`]); only
    /// for groups that do not depend on the width.
    key: Option<GroupKey>,
}

pub(super) struct BoxOut {
    pub w: f32,
    pub h: f32,
    /// The bottom margin of its last child, collapsed through it.
    pub trail: f32,
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
    /// The first layout of this message, whose shaped groups this one may
    /// reuse, and its groups by digest.
    prev: Option<&'a Doc>,
    donors: HashMap<GroupKey, u32>,
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
    /// The images decoded ahead of the layout, by source.
    decoded: &'a img::Decoded,
    /// Device pixels per CSS px.
    scale: f32,
    pub timed_out: bool,
    /// Pixels left for decoding and resampling images.
    pub budget: u64,
    /// Table cells seen so far (capped).
    pub cells: usize,
    /// Some inline group was cut at [`MAX_GROUP_TEXT`].
    text_capped: bool,
    /// The collapsed top margin of a block by element, and whether its whole
    /// box collapses through (see [`Layouter::top_margin`]).
    margins: HashMap<Id, (f32, bool)>,
    /// The bottom margin the block list just laid out left to its parent.
    trail: f32,
    /// Where layout stops (CSS px): [`MAX_DOC_H`], or less for a preview.
    cap: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout<'a>(
    dom: &'a Dom,
    styles: Styles,
    fcx: &'a mut FontContext,
    lcx: &'a mut LayoutContext<Brush>,
    src: &'a mut Sources<'a>,
    decoded: &'a img::Decoded,
    width: f32,
    scale: f32,
    deadline: &'a Deadline,
    prev: Option<&'a Doc>,
    cap: f32,
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
            shaped: vec![],
            reused: 0,
        },
        prev,
        donors: prev
            .map(|p| p.shaped.iter().copied().collect())
            .unwrap_or_default(),
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
        decoded,
        scale,
        timed_out: false,
        budget: decoded.pixels,
        cells: 0,
        text_capped: false,
        margins: HashMap::new(),
        trail: 0.0,
        cap: cap.clamp(1.0, MAX_DOC_H),
    };
    let kids = dom.children(ROOT).to_vec();
    let h = e.block_list(ROOT, &kids, 0.0, 0.0, width);
    if e.timed_out {
        return None;
    }
    e.doc.full_height = h;
    e.doc.truncated = h > e.cap || e.text_capped;
    e.doc.height = h.clamp(1.0, e.cap);
    e.doc.items = std::mem::take(&mut e.out);
    e.doc.natural_width = content_right(&e.doc);
    e.doc.remote_images = std::mem::take(&mut e.src.missing);
    prescale_images(&mut e.doc, scale, &mut e.budget);
    Some(e.doc)
}

/// The images a layout of `dom` is going to draw and has not got yet, each
/// with the widest it can be shown: its declared width, or the page's (an
/// image never exceeds its line); a background may be cut from a larger one.
pub(super) fn wanted_images(
    dom: &Dom,
    styles: &Styles,
    src: &mut Sources<'_>,
    width: f32,
    scale: f32,
    have: &mut img::Decoded,
) -> Vec<img::Wanted> {
    let mut out: Vec<img::Wanted> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut hidden = vec![false; dom.nodes.len()];
    let device = |css: f32| (css * scale).round().max(1.0) as u32;
    for (i, n) in dom.nodes.iter().enumerate() {
        let Some(s) = styles.v.get(i) else {
            break;
        };
        let gone =
            s.display == Display::None || hidden.get(n.parent as usize).copied().unwrap_or(false);
        if let Some(h) = hidden.get_mut(i) {
            *h = gone;
        }
        let Kind::Elem { tag, attrs } = &n.kind else {
            continue;
        };
        if gone {
            continue;
        }
        let mut ask = |source: &str, cap_w: u32| {
            let source = source.trim();
            // Decoded already, at the width a decode from scratch would give
            // for this pass (a picture is shrunk once, from its original,
            // when it is bigger than 1.5 times its display width): a second
            // pass at another width gets the pixels of a layout from
            // scratch, not a shrunk picture grown or shrunk again.
            let enough = have.absent.contains(source)
                || match have.images.get(source) {
                    None => false,
                    Some(None) => true,
                    Some(Some(d)) => {
                        let shrunk = u64::from(cap_w).saturating_mul(3) / 2 < u64::from(d.nat_w);
                        d.pix.width() == if shrunk { cap_w } else { d.nat_w }
                    }
                };
            if source.is_empty() || enough || (super::css::is_remote(source) && is_tracker(s)) {
                return;
            }
            if let Some(&k) = at.get(source) {
                if let Some(w) = out.get_mut(k) {
                    w.cap_w = w.cap_w.max(cap_w);
                }
            } else if let Some(bytes) = src.peek(source) {
                at.insert(source.to_string(), out.len());
                out.push(img::Wanted {
                    src: source.to_string(),
                    bytes,
                    cap_w,
                });
            } else {
                // Asked once: the layout does not ask the resolver again.
                have.absent.insert(source.to_string());
            }
        };
        if &**tag == "img"
            && let Some((_, v)) = attrs.iter().find(|(k, _)| &**k == "src")
        {
            let shown = match s.width {
                Len::Px(p) => fin(p).min(width * 4.0),
                _ => width,
            };
            ask(v, device(shown.max(1.0)));
        }
        if let Some(b) = &s.bg_image {
            ask(b, device(width * 2.0));
        }
    }
    out
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
            Item::Tiled { r, .. } | Item::Gradient { r, .. } => r.x + r.w,
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
            Item::Tiled { r, tile, .. } => {
                r.x += dx;
                r.y += dy;
                tile.x += dx;
                tile.y += dy;
            }
            Item::Gradient { r, line, .. } => {
                r.x += dx;
                r.y += dy;
                line[0] += dx;
                line[1] += dy;
                line[2] += dx;
                line[3] += dy;
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

/// Cut `items` at the page height `bottom`: boxes are shortened, images
/// clipped and text loses the lines that end below it.
fn clip_below(items: &mut [Item], bottom: f32) {
    let cut = |r: &mut Rect| r.h = fin(r.h.min(bottom - r.y));
    for it in items {
        match it {
            Item::Rect { r, .. }
            | Item::Stroke { r, .. }
            | Item::Link { r, .. }
            | Item::Tiled { r, .. }
            | Item::Gradient { r, .. } => cut(r),
            Item::Image { r, clip, .. } => {
                let mut c = clip.unwrap_or(*r);
                cut(&mut c);
                *clip = Some(c);
            }
            Item::Text { y, clip_h, .. } => *clip_h = clip_h.min(bottom - *y),
            Item::Anchor { .. } => {}
        }
    }
}

pub(super) fn px_of(l: Len) -> f32 {
    match l {
        Len::Px(p) => p,
        _ => 0.0,
    }
}

/// The first tile of the background image of `s` on box `r`, for an image of
/// its natural size (`nw`, `nh`).
fn bg_tile(s: &Style, r: Rect, nw: f32, nh: f32) -> Rect {
    let place = s.bg_place;
    let (nw, nh) = (nw.max(1.0), nh.max(1.0));
    let of = |l: Len, whole: f32| match l {
        Len::Px(p) => Some(p),
        Len::Pct(p) => Some(whole * p / 100.0),
        Len::Auto => None,
    };
    let (w, h) = match place.size {
        BgSize::Cover => {
            let k = (r.w / nw).max(r.h / nh);
            (nw * k, nh * k)
        }
        BgSize::Contain => {
            let k = (r.w / nw).min(r.h / nh);
            (nw * k, nh * k)
        }
        BgSize::Is(w, h) => match (of(w, r.w), of(h, r.h)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, nh * w / nw),
            (None, Some(h)) => (nw * h / nh, h),
            (None, None) => (nw, nh),
        },
    };
    let (w, h) = (fin(w).max(1.0), fin(h).max(1.0));
    // A percentage lines up that point of the tile with that point of the box.
    let at = |l: Len, whole: f32, size: f32| match l {
        Len::Pct(p) => (whole - size) * p / 100.0,
        Len::Px(p) => p,
        Len::Auto => 0.0,
    };
    Rect {
        x: r.x + at(place.x, r.w, w),
        y: r.y + at(place.y, r.h, h),
        w,
        h,
    }
}

/// The gradient line for an angle (clockwise from up) on box `r`: through its
/// centre, as long as the corners need (CSS Images 3.1.1).
fn gradient_line(r: Rect, degrees: f32) -> [f32; 4] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let half = (r.w * sin.abs() + r.h * cos.abs()) / 2.0;
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    [
        cx - sin * half,
        cy + cos * half,
        cx + sin * half,
        cy - cos * half,
    ]
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
        self.block_list_lifting(parent, kids, x, y, w, (false, false))
    }

    /// [`Self::block_list`] for a box whose top and bottom margins collapse
    /// with those of its first and last children (`lift`): those margins are
    /// its parent's to apply.
    fn block_list_lifting(
        &mut self,
        parent: Id,
        kids: &[Id],
        x: f32,
        y: f32,
        w: f32,
        lift: (bool, bool),
    ) -> f32 {
        let flat = self.depth >= MAX_LAYOUT_DEPTH;
        self.depth = self.depth.saturating_add(1);
        let h = self.block_list_inner(parent, kids, x, y, fin(w), flat, lift);
        self.depth = self.depth.saturating_sub(1);
        h
    }

    /// Whether inline node `n` makes no line of its own: white space, or an
    /// element with nothing else in it (`<a name=top></a>`).
    fn blank_inline(&self, n: Id, depth: u32) -> bool {
        let Some(node) = self.dom.node(n) else {
            return true;
        };
        let s = self.style(n);
        match &node.kind {
            // A no-break space is content.
            Kind::Text(t) => !s.pre && t.trim_matches(|c: char| c.is_ascii_whitespace()).is_empty(),
            Kind::Elem { tag, .. } => {
                s.display == Display::None
                    || (s.display == Display::Inline
                        && !matches!(&**tag, "br" | "img")
                        && depth < MAX_LAYOUT_DEPTH
                        && self
                            .dom
                            .children(n)
                            .iter()
                            .all(|&k| self.blank_inline(k, depth.saturating_add(1))))
            }
        }
    }

    /// The margin that a block `c` brings to its top once collapsed with the
    /// first children it does not separate from it (CSS 2 8.3.1), and whether
    /// the block is empty, so that its margins collapse through it.
    fn top_margin(&mut self, c: Id, depth: u32) -> (f32, bool) {
        if let Some(&m) = self.margins.get(&c) {
            return m;
        }
        let s = self.style(c);
        let own = px_of(s.margin[0]);
        let bottom = px_of(s.margin[2]);
        let solid_top = s.padding[0] + s.border[0] > 0.0;
        let solid = solid_top
            || s.padding[2] + s.border[2] > 0.0
            || s.overflow_hidden
            || s.bg_image.is_some()
            || matches!(s.height, Len::Px(h) if h > 0.0)
            || matches!(s.min_height, Len::Px(h) if h > 0.0)
            || s.display != Display::Block;
        let opens = !solid_top && !s.overflow_hidden && s.display != Display::Table;
        let mut top = own;
        let mut empty = !solid;
        if depth < MAX_LAYOUT_DEPTH {
            let dom = self.dom;
            let mut lifting = opens;
            for &k in dom.children(c) {
                let (kid_top, kid_empty) = match self.classify(k) {
                    Cls::Skip => continue,
                    // Laid out here, a float counts towards the box's height.
                    Cls::Float => {
                        empty = false;
                        continue;
                    }
                    Cls::Inline if self.blank_inline(k, depth) => continue,
                    Cls::Inline => (0.0, false),
                    Cls::Block => self.top_margin(k, depth.saturating_add(1)),
                };
                if lifting {
                    top = top.max(kid_top);
                }
                empty = empty && kid_empty;
                lifting = lifting && kid_empty;
                if !lifting && !empty {
                    break;
                }
            }
        } else {
            empty = false;
        }
        if empty {
            top = top.max(bottom);
        }
        self.margins.insert(c, (top, empty));
        (top, empty)
    }

    #[allow(clippy::too_many_arguments)]
    fn block_list_inner(
        &mut self,
        parent: Id,
        kids: &[Id],
        x: f32,
        y: f32,
        w: f32,
        flat: bool,
        (lift_top, lift_bottom): (bool, bool),
    ) -> f32 {
        let mut cy = 0.0f32;
        let mut pend = 0.0f32;
        // Nothing yet separates the top of this box from its next child.
        let mut at_top = lift_top;
        let mut run: Vec<Id> = vec![];
        let mut floats = super::float::Floats::default();
        let mut work: Vec<Work> = kids.iter().rev().map(|&k| Work::Node(k)).collect();
        // Lay out the pending inline run as one group.
        macro_rules! flush {
            () => {
                if !run.is_empty() {
                    let r = std::mem::take(&mut run);
                    let (at, l, rr) = floats.band(cy + pend, w);
                    let h = self.layout_group(parent, &r, x + l, y + at, w - l - rr);
                    if h > 0.0 {
                        cy = at + h;
                        pend = 0.0;
                        at_top = false;
                    }
                }
            };
        }
        while let Some(job) = work.pop() {
            if y + cy > self.cap + 64.0 || self.expired() {
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
            // A block image is laid out as one, on a line of its own.
            if cls == Cls::Inline
                && self.style(c).display == Display::Block
                && self.dom.tag(c) == "img"
            {
                flush!();
                run.push(c);
                flush!();
                continue;
            }
            let clear = self.style(c).clear;
            if clear != (false, false) && cls != Cls::Skip {
                if cls == Cls::Inline {
                    // `<br clear=all>`: the line it ends goes first.
                    run.push(c);
                }
                flush!();
                let to = floats.cleared(clear);
                if to > cy + pend {
                    cy = to;
                    pend = 0.0;
                    at_top = false;
                }
                if cls == Cls::Inline {
                    continue;
                }
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
                    let mb = px_of(self.style(c).margin[2]);
                    let (mt, empty) = self.top_margin(c, self.depth);
                    // A margin this box lifts is outside it already.
                    let gap = if at_top { 0.0 } else { pend.max(mt) };
                    if empty {
                        // Its margins collapse through it; it may still hold
                        // anchors or floats, at no height.
                        let (at, l, rr) = floats.band(cy + gap, w);
                        self.box_at(c, x + l, y + at, w - l - rr, false);
                        pend = if at_top { 0.0 } else { pend.max(mt) };
                        continue;
                    }
                    let (at, l, rr) = floats.band(cy + gap, w);
                    let out = self.box_at(c, x + l, y + at, w - l - rr, false);
                    cy = at + out.h;
                    pend = mb.max(out.trail);
                    at_top = false;
                }
            }
        }
        flush!();
        self.trail = 0.0;
        if lift_bottom && !at_top {
            // The last child's margin goes on past the end of this box.
            self.trail = pend;
            return cy.max(floats.bottom());
        }
        // A box with nothing in it: the margins went with the lift at the top.
        if lift_bottom {
            return floats.bottom();
        }
        (cy + pend).max(floats.bottom())
    }

    /// Lay out `id` with its margins zeroed (the caller applies them).
    pub fn box_at_nomargin(&mut self, id: Id, x: f32, y: f32, avail: f32) -> BoxOut {
        let Some(s) = self.st.get_mut(id as usize) else {
            return BoxOut {
                w: 0.0,
                h: 0.0,
                trail: 0.0,
            };
        };
        let saved = (s.margin, s.moved_by);
        s.margin = [Len::Px(0.0); 4];
        s.moved_by = Align::Left;
        let r = self.box_at(id, x, y, avail, true);
        if let Some(s) = self.st.get_mut(id as usize) {
            (s.margin, s.moved_by) = saved;
        }
        r
    }

    pub fn box_at(&mut self, id: Id, x: f32, y: f32, cb_w: f32, shrink: bool) -> BoxOut {
        self.anchor(id, y);
        let mut s = self.style(id).clone();
        if let Len::Px(p) = s.width {
            s.width = Len::Px(p.min(self.maxw));
        }
        let (mut pad, bd) = (s.padding, s.border);
        let (ml_s, mr_s) = (s.margin[3], s.margin[1]);
        let (mut ml_px, mut mr_px) = (px_of(ml_s), px_of(mr_s));
        // Deep quotes stop indenting once the text would get narrower than
        // a readable column: forty levels of Gmail's or Apple Mail's quote
        // indent leave a sliver of a word per line (and a page hundreds of
        // thousands of pixels tall) at a window's width.
        let floor = (self.doc.width * 0.5).min(MIN_COLUMN);
        let indent = ml_px + mr_px + pad[1] + pad[3] + bd[1] + bd[3];
        if s.width == Len::Auto && !shrink && indent > 0.0 && cb_w - indent < floor {
            (ml_px, mr_px, pad[1], pad[3]) = (0.0, 0.0, 0.0, 0.0);
        }
        let auto_margin = |free: f32, own_width: bool| match (ml_s, mr_s) {
            (Len::Auto, Len::Auto) => free / 2.0,
            (Len::Auto, _) => free,
            _ if own_width && ml_px == 0.0 && mr_px == 0.0 => match s.moved_by {
                Align::Center => free / 2.0,
                Align::Right => free,
                _ => 0.0,
            },
            _ => ml_px,
        };
        if s.display == Display::Table {
            let avail = fin(cb_w - ml_px - mr_px);
            let tw = self.table_width(id, avail);
            let ml = auto_margin(fin(cb_w - tw - ml_px - mr_px), true);
            let h = self.table_layout(id, x + ml, y, tw);
            return BoxOut {
                w: tw,
                h,
                trail: 0.0,
            };
        }
        let hb = pad[1] + pad[3] + bd[1] + bd[3];
        let (vb_top, vb_bot) = (pad[0] + bd[0], pad[2] + bd[2]);
        let avail = fin(cb_w - ml_px - mr_px - hb);
        // What a declared size leaves for the content.
        let content = |v: f32, sides: f32| if s.border_box { fin(v - sides) } else { v };
        let mut cw = match s.width {
            Len::Px(p) => content(p, hb),
            Len::Pct(p) => content((cb_w - ml_px - mr_px) * p / 100.0, hb),
            Len::Auto if shrink => {
                let (_, mx) = self.content_widths(id);
                mx.min(avail)
            }
            Len::Auto => avail,
        };
        match s.max_width {
            Len::Px(m) => cw = cw.min(content(m, hb)),
            Len::Pct(p) => cw = cw.min(content(cb_w * p / 100.0, hb)),
            Len::Auto => {}
        }
        if let Len::Px(m) = s.min_width {
            cw = cw.max(content(m, hb));
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
                radius: s.radius_on(outer),
            });
        }
        // Holds the place of the background, known once the height is.
        let img_idx = self.out.len();
        let has_back = s.gradient.is_some() || s.bg_image.is_some();
        if has_back {
            self.out.push(Item::Anchor { y, anchor: 0 });
        }
        let kids = self.dom.children(id).to_vec();
        let kids_at = self.out.len();
        // Only a block in flow lets its children's margins through.
        let opens = !shrink && !s.overflow_hidden && s.display != Display::Table;
        let lift = (
            opens && vb_top <= 0.0,
            opens
                && vb_bot <= 0.0
                && s.height == Len::Auto
                && matches!(s.min_height, Len::Auto | Len::Pct(_)),
        );
        let mut ch = self.block_list_lifting(id, &kids, bx + bd[3] + pad[3], y + vb_top, cw, lift);
        let trail = self.trail;
        // A declared height is the box's, whatever the content does: more
        // content overflows it, as in a browser.
        if let Len::Px(hh) = s.height {
            ch = content(hh, vb_top + vb_bot);
        }
        if let Len::Px(m) = s.max_height {
            ch = ch.min(content(m, vb_top + vb_bot));
        }
        if let Len::Px(m) = s.min_height {
            ch = ch.max(content(m, vb_top + vb_bot));
        }
        // What overflows a clipping box is not painted (or linked).
        if s.overflow_hidden {
            let bottom = y + vb_top + ch + vb_bot;
            clip_below(self.out.get_mut(kids_at..).unwrap_or_default(), bottom);
        }
        if s.display == Display::ListItem {
            self.marker(id, bx + bd[3] + pad[3], y + vb_top);
        }
        let h = vb_top + ch + vb_bot;
        if let Some(Item::Rect { r, .. }) = self.out.get_mut(bg_idx).filter(|_| s.bg[3] > 0) {
            r.h = h;
        }
        if has_back {
            let r = Rect {
                x: bx,
                y,
                w: outer,
                h,
            };
            let back = self.background(&s, r);
            if let Some(slot) = self.out.get_mut(img_idx) {
                *slot = back.unwrap_or(Item::Rect {
                    r,
                    color: [0; 4],
                    radius: Radius::NONE,
                });
            }
        }
        self.borders(bx, y, outer, h, &s);
        BoxOut { w: outer, h, trail }
    }

    pub fn borders(&mut self, x: f32, y: f32, w: f32, h: f32, s: &Style) {
        let b = s.border;
        if b.iter().all(|&v| v <= 0.0) {
            return;
        }
        let (c, st) = (s.border_color, s.border_style);
        let radius = s.radius_on(w);
        if radius.px > 0.0 && b[0] > 0.0 && b.iter().all(|&v| v == b[0]) {
            self.out.push(Item::Stroke {
                r: Rect { x, y, w, h },
                width: b[0],
                color: c[0],
                radius,
                style: st[0],
            });
            return;
        }
        let sides = [
            (Rect { x, y, w, h: b[0] }, c[0], st[0]),
            (
                Rect {
                    x,
                    y: y + h - b[2],
                    w,
                    h: b[2],
                },
                c[2],
                st[2],
            ),
            (Rect { x, y, w: b[3], h }, c[3], st[3]),
            (
                Rect {
                    x: x + w - b[1],
                    y,
                    w: b[1],
                    h,
                },
                c[1],
                st[1],
            ),
        ];
        for (r, color, style) in sides {
            if r.w <= 0.0 || r.h <= 0.0 || color[3] == 0 {
                continue;
            }
            let rect = |r: Rect| Item::Rect {
                r,
                color,
                radius: Radius::NONE,
            };
            match style {
                LineStyle::Dashed | LineStyle::Dotted => self.out.push(Item::Stroke {
                    r,
                    width: r.w.min(r.h),
                    color,
                    radius: Radius::NONE,
                    style,
                }),
                // Two lines a third of the width each, when that makes pixels.
                LineStyle::Double if r.w.min(r.h) >= 3.0 => {
                    let third = r.w.min(r.h) / 3.0;
                    if r.h <= r.w {
                        let edge = Rect { h: third, ..r };
                        self.out.push(rect(edge));
                        self.out.push(rect(Rect {
                            y: r.y + 2.0 * third,
                            ..edge
                        }));
                    } else {
                        let edge = Rect { w: third, ..r };
                        self.out.push(rect(edge));
                        self.out.push(rect(Rect {
                            x: r.x + 2.0 * third,
                            ..edge
                        }));
                    }
                }
                _ => self.out.push(rect(r)),
            }
        }
    }

    fn marker(&mut self, li: Id, x: f32, y: f32) {
        let parent = self.dom.parent(li);
        let s = self.style(li).clone();
        if s.list_none || self.style(parent).list_none {
            return;
        }
        let text = s.marker.text(self.item_number(parent, li));
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

    /// The number of item `li` of a list: from the list's `start`, each item
    /// one up, except those with a `value`.
    fn item_number(&self, list: Id, li: Id) -> i64 {
        let int = |id: Id, name: &str| {
            self.dom
                .attr(id, name)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map(|v| v.clamp(-1_000_000, 1_000_000))
        };
        let mut n = int(list, "start").unwrap_or(1);
        for &c in self.dom.children(list) {
            if self.dom.tag(c) == "li" {
                if let Some(v) = int(c, "value") {
                    n = v;
                }
                if c == li {
                    return n;
                }
                n = n.saturating_add(1);
            }
        }
        n
    }

    fn push_layout(&mut self, l: Layout<Brush>, text: String) -> usize {
        self.doc.layouts.push(l);
        self.doc.texts.push(text);
        self.doc.layouts.len().saturating_sub(1)
    }

    // ------------------------------------------------------------------ inline groups

    /// Collect inline ops for `nodes`. Inline-block contents are laid out
    /// here, into boxes.
    #[allow(clippy::too_many_arguments)]
    fn collect(
        &mut self,
        nodes: &[Id],
        link: u32,
        deco: Id,
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
                    if s.pre
                        || !txt
                            .trim_matches(|c: char| c.is_ascii_whitespace())
                            .is_empty()
                    {
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
                self.image_box(n, link, deco, avail, ops, boxes, visible);
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
                // element: empty boxes at its two ends. Those of the border
                // and padding are part of its painted box.
                let margin = |i: usize| s.margin.get(i).map_or(0.0, |&m| px_of(m)).max(0.0);
                let inner = |i: usize| {
                    let b = s.border.get(i).copied().unwrap_or(0.0);
                    let p = s.padding.get(i).copied().unwrap_or(0.0);
                    (b + p).max(0.0)
                };
                let own = if s.has_inline_box() { n } else { deco };
                ops.push(Op::Open(n, l, own));
                spacer(margin(3), deco, ops, boxes);
                spacer(inner(3), own, ops, boxes);
                self.collect(dom.children(n), l, own, avail, ops, boxes, visible);
                spacer(inner(1), own, ops, boxes);
                spacer(margin(1), deco, ops, boxes);
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
                    deco,
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
                deco,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn image_box(
        &mut self,
        n: Id,
        link: u32,
        deco: Id,
        avail: f32,
        ops: &mut Vec<Op>,
        boxes: &mut Vec<BoxInfo>,
        visible: &mut bool,
    ) {
        let s = self.style(n).clone();
        let src = self.dom.attr(n, "src").unwrap_or("").trim().to_string();
        // Around the content box of an image: padding and border.
        let (ox, oy) = (s.border[3] + s.padding[3], s.border[0] + s.padding[0]);
        let (hb, vb) = (
            ox + s.border[1] + s.padding[1],
            oy + s.border[2] + s.padding[2],
        );
        let avail = fin(avail.min(self.doc.width.max(1.0) * 4.0) - hb);
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
                let radius = s.radius_on(w);
                if s.bg[3] > 0 {
                    items.push(Item::Rect {
                        r,
                        color: s.bg,
                        radius,
                    });
                }
                // `object-fit: cover`: as large as it takes, centred, cut to
                // the box.
                let (shown, clip) = if s.cover && nw > 0.0 && nh > 0.0 {
                    let k = (w / nw).max(h / nh);
                    let (sw, sh) = (nw * k, nh * k);
                    (
                        Rect {
                            x: (w - sw) / 2.0,
                            y: (h - sh) / 2.0,
                            w: sw,
                            h: sh,
                        },
                        Some(r),
                    )
                } else {
                    (r, None)
                };
                items.push(Item::Image {
                    r: shown,
                    img: ix,
                    clip,
                    radius,
                    alpha: s.opacity,
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
                        ops.push(Op::Open(n, link, deco));
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
                let radius = Radius::all(s.radius_on(w).px.max(4.0).min(w.min(h) / 2.0));
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
                    style: LineStyle::Solid,
                });
                if !alt.is_empty() {
                    self.alt_text(&s, &alt, link, w, h, &mut items);
                }
                (w, h)
            }
        };
        let (w, h) = (w + hb, h + vb);
        // Margins (and `hspace`, `vspace`) keep their room around the box.
        let m = s.margin.map(|l| match l {
            Len::Px(p) => fin(p),
            _ => 0.0,
        });
        if hb > 0.0 || vb > 0.0 || m.iter().any(|&v| v > 0.0) {
            shift(&mut items, ox + m[3], oy + m[0]);
            let saved = std::mem::take(&mut self.out);
            self.borders(m[3], m[0], w, h, &s);
            items.append(&mut self.out);
            self.out = saved;
        }
        if link != 0 {
            items.push(Item::Link {
                r: Rect {
                    x: m[3],
                    y: m[0],
                    w,
                    h,
                },
                link,
            });
        }
        ops.push(Op::Box(boxes.len()));
        boxes.push(BoxInfo {
            w: w + m[1] + m[3],
            h: h + m[0] + m[2],
            items,
            va: s.valign,
            deco,
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

    /// The background of `s` on box `r`, beyond its plain colour: the image
    /// (a remote one on a tracking-sized or hidden box is never loaded, as
    /// for `<img>`), or else the gradient.
    pub fn background(&mut self, s: &Style, r: Rect) -> Option<Item> {
        if let Some(src) = s.bg_image.clone()
            && !(super::css::is_remote(&src) && is_tracker(s))
            && let Some(img) = self.load_image(&src, 0)
            && let Some(im) = self.doc.images.get(img)
        {
            let tile = bg_tile(s, r, im.nat_w as f32, im.nat_h as f32);
            return Some(Item::Tiled {
                r,
                img,
                tile,
                repeat: (s.bg_place.repeat_x, s.bg_place.repeat_y),
                radius: s.radius_on(r.w),
            });
        }
        let g = s.gradient.clone()?;
        Some(Item::Gradient {
            r,
            radius: s.radius_on(r.w),
            line: gradient_line(r, g.degrees),
            stops: g,
        })
    }

    pub fn load_image(&mut self, src: &str, target_w: u32) -> Option<usize> {
        if let Some(r) = self.img_cache.get(src) {
            return *r;
        }
        let key = src.trim();
        let image = match self.decoded.images.get(key) {
            _ if self.decoded.absent.contains(key) => {
                self.src.note_missing(src, false);
                None
            }
            Some(d) => d.clone(),
            None => {
                let device = (target_w as f32 * self.scale).round() as u32;
                self.src
                    .load(src)
                    .and_then(|bytes| img::decode(&bytes, device, &mut self.budget))
            }
        };
        let r = image.map(|d| {
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
        self.collect(nodes, 0, 0, avail, &mut ops, &mut boxes, &mut visible);
        let ps = self.style(parent).clone();
        let key = self.group_key(
            &ps,
            &ops,
            boxes.is_empty() && self.pct_seen == pct0,
            emergency,
        );
        if let Some((layout, text)) = key.and_then(|k| self.donor(k)) {
            self.doc.reused = self.doc.reused.saturating_add(1);
            return Group {
                layout,
                text,
                boxes,
                visible,
                built_avail: avail,
                dep: false,
                key,
            };
        }
        let root = text_style(&ps, brush(&ps, 0), emergency);
        let mut b = self.lcx.tree_builder(self.fcx, 1.0, true, &root);
        // White space is collapsed here, over the whole group: parley's own
        // collapsing trims it at the edge of every inline element, which
        // joins "<b>From:</b> Ana" into "From:Ana".
        b.set_white_space_mode(WhiteSpaceCollapse::Preserve);
        let texts = collapse_white_space(&ops);
        let mut room = MAX_GROUP_TEXT;
        let mut capped = false;
        for (k, op) in ops.iter().enumerate() {
            let text = match op {
                Op::Text(t, pre) => match texts.get(k) {
                    Some(Some(c)) if !*pre => c.as_str(),
                    _ => t.as_str(),
                },
                _ => "",
            };
            match op {
                // Past the cap only the spans are still opened and closed.
                Op::Text(..) | Op::Br | Op::Box(_) if capped => {}
                Op::Text(..) if text.len() > room => {
                    b.push_text(cut_text(text, room));
                    b.push_text("\n");
                    b.push_text(CAPPED_MARKER);
                    capped = true;
                    self.text_capped = true;
                }
                Op::Open(id, link, deco) => {
                    let s = self.st.get(*id as usize).unwrap_or(&self.default);
                    b.push_style_span(text_style(s, deco_brush(s, *link, *deco), emergency));
                }
                Op::Close => b.pop_style_span(),
                Op::Text(..) => {
                    room = room.saturating_sub(text.len());
                    b.push_text(text);
                }
                Op::Br => b.push_text("\n"),
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
            key: key.filter(|_| !dep),
        }
    }

    /// The backgrounds and borders of the inline elements of `g`, one box per
    /// line each: over the element's text and its padding and border spacers,
    /// from its font's ascent to its descent plus the vertical padding and
    /// border, which do not move the lines (CSS 2 10.6.1).
    fn inline_boxes(&mut self, g: &Group, x: f32, y: f32) {
        let mut boxes: Vec<(Id, f32, f32, f32, f32)> = vec![];
        for line in g.layout.lines() {
            boxes.clear();
            let mut extend = |deco: Id, x0: f32, x1: f32, top: f32, bottom: f32| {
                if let Some(b) = boxes.iter_mut().find(|b| b.0 == deco) {
                    b.1 = b.1.min(x0);
                    b.2 = b.2.max(x1);
                    b.3 = b.3.min(top);
                    b.4 = b.4.max(bottom);
                } else {
                    boxes.push((deco, x0, x1, top, bottom));
                }
            };
            for item in line.items() {
                match item {
                    PositionedLayoutItem::InlineBox(pb) => {
                        let spacer = g.boxes.get(pb.id as usize).filter(|b| b.h <= 0.0);
                        // Only a spacer widens the box; its height comes
                        // from the text next to it.
                        if let Some(deco) = spacer.map(|b| b.deco).filter(|&d| d != 0) {
                            extend(deco, pb.x, pb.x + pb.width, f32::MAX, f32::MIN);
                        }
                    }
                    PositionedLayoutItem::GlyphRun(gr) => {
                        let deco = gr.style().brush.deco;
                        if deco != 0 {
                            let m = gr.run().metrics();
                            extend(
                                deco,
                                gr.offset(),
                                gr.offset() + gr.advance(),
                                gr.baseline() - m.ascent,
                                gr.baseline() + m.descent,
                            );
                        }
                    }
                }
            }
            for &(deco, x0, x1, top, bottom) in &boxes {
                // Only spacers on this line: nothing to sit on.
                if top > bottom {
                    continue;
                }
                let s = self.style(deco).clone();
                let (t, b) = (s.padding[0] + s.border[0], s.padding[2] + s.border[2]);
                let (bx, by) = (x + x0, y + top - t);
                let (w, h) = (x1 - x0, bottom - top + t + b);
                if !s.hidden {
                    if s.bg[3] > 0 {
                        self.out.push(Item::Rect {
                            r: Rect { x: bx, y: by, w, h },
                            color: s.bg,
                            radius: s.radius_on(w),
                        });
                    }
                    self.borders(bx, by, w, h, &s);
                }
            }
        }
    }

    /// A digest of everything the shaper is given for a group: the root and
    /// span text styles and the text, as [`text_style`] reads them. `None`
    /// when the group depends on the width (inline boxes, %-sized images) or
    /// is long enough to be cut: those are always built.
    fn group_key(&self, ps: &Style, ops: &[Op], plain: bool, emergency: bool) -> Option<GroupKey> {
        if !plain {
            return None;
        }
        let mut len = 0usize;
        let mut lo = DefaultHasher::new();
        let mut hi = DefaultHasher::new();
        // Two independent SipHash states make a collision negligible.
        hi.write_u8(0x5a);
        for h in [&mut lo, &mut hi] {
            hash_text_style(h, ps, brush(ps, 0), emergency);
        }
        for op in ops {
            for h in [&mut lo, &mut hi] {
                match op {
                    Op::Open(id, link, deco) => {
                        let s = self.st.get(*id as usize).unwrap_or(&self.default);
                        h.write_u8(1);
                        hash_text_style(h, s, deco_brush(s, *link, *deco), emergency);
                    }
                    Op::Close => h.write_u8(2),
                    Op::Text(t, pre) => {
                        h.write_u8(3);
                        h.write_u8(u8::from(*pre));
                        t.hash(h);
                    }
                    Op::Br => h.write_u8(4),
                    Op::Box(_) => {}
                }
            }
            if let Op::Text(t, _) = op {
                len = len.saturating_add(t.len());
            }
        }
        (len <= MAX_GROUP_TEXT).then(|| (hi.finish(), lo.finish()))
    }

    /// The layout and text a previous layout of this message shaped for
    /// the group with this digest.
    fn donor(&self, key: GroupKey) -> Option<(Layout<Brush>, String)> {
        let i = *self.donors.get(&key)? as usize;
        let prev = self.prev?;
        Some((prev.layouts.get(i)?.clone(), prev.texts.get(i)?.clone()))
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
        let indent = match self.style(parent).indent {
            Len::Px(p) => p,
            Len::Pct(p) => w * p / 100.0,
            Len::Auto => 0.0,
        };
        if indent != 0.0 {
            g.layout
                .set_text_indent(indent, parley::IndentOptions::default());
        }
        let rtl = self.style(parent).rtl;
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
        // A line break that ends the text ends its last line; the empty one
        // after it is no line of the page. A box after the break (a block
        // inside an inline, `<span><p>..</p><br><p>..</p></span>`) has no
        // text, but its line is one.
        let mut h = g.layout.height();
        if g.text.ends_with('\n')
            && g.layout.len() > 1
            && let Some(last) = g.layout.get(g.layout.len().saturating_sub(1))
            && last.text_range().is_empty()
            && !last
                .items()
                .any(|i| matches!(i, PositionedLayoutItem::InlineBox(_)))
        {
            h = fin(h - last.metrics().line_height);
        }
        self.inline_boxes(&g, x, y);
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
                        // Right to left: the line's boxes run from the other end.
                        let bx = if rtl {
                            fin(wrap - pb.x - pb.width)
                        } else {
                            pb.x
                        };
                        shift(&mut items, x + bx, y + by);
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
        let key = g.key;
        let layout = self.push_layout(g.layout, g.text);
        // A justified layout has had its advances adjusted, which breaking
        // again undoes only approximately: it is never handed on.
        if let (Some(k), false) = (key, align == Alignment::Justify) {
            self.doc
                .shaped
                .push((k, u32::try_from(layout).unwrap_or(u32::MAX)));
        }
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
        let content = |v: f32| if s.border_box { fin(v - extra) } else { v };
        if s.display == Display::Table {
            let (mn, mx) = self.table_intrinsic(c);
            return (mn + marg, mx + marg);
        }
        let (mut mn, mut mx) = self.content_widths(c);
        match s.width {
            Len::Px(w) => {
                mn = content(w).min(self.maxw);
                mx = mn;
            }
            // Percentage widths contribute min-content only.
            Len::Pct(_) => mx = mx.max(mn),
            Len::Auto => {}
        }
        if let Len::Px(m) = s.max_width {
            mx = mx.min(content(m));
            mn = mn.min(content(m));
        }
        if let Len::Px(m) = s.min_width {
            mn = mn.max(content(m));
            mx = mx.max(content(m));
        }
        (fin(mn + extra + marg), fin(mx + extra + marg))
    }
}

/// The text of each collapsible `Op::Text` of a group once white space is
/// collapsed as CSS does it (`white-space: normal`): runs of it become one
/// space across inline elements, and none is left at the start, at the end
/// or around a line break. ASCII white space, controls and line separators
/// collapse; other spaces (no-break, em) are kept. `None` for other ops.
fn collapse_white_space(ops: &[Op]) -> Vec<Option<String>> {
    let collapses =
        |c: char| c.is_ascii_whitespace() || c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
    let mut out: Vec<Option<String>> = Vec::with_capacity(ops.len());
    // After a space (or at the start of a line): the next one goes.
    let mut at_space = true;
    // The op whose text ends with a space nothing has followed yet.
    let mut trailing: Option<usize> = None;
    let trim = |out: &mut Vec<Option<String>>, at: Option<usize>| {
        if let Some(Some(s)) = at.and_then(|i| out.get_mut(i)) {
            s.pop();
        }
    };
    for op in ops {
        let collapsed = match op {
            Op::Text(t, false) => {
                let mut s = String::with_capacity(t.len());
                for c in t.chars() {
                    if !collapses(c) {
                        s.push(c);
                        at_space = false;
                        trailing = None;
                    } else if !at_space {
                        s.push(' ');
                        at_space = true;
                        trailing = Some(out.len());
                    }
                }
                Some(s)
            }
            Op::Text(t, true) => {
                if !t.is_empty() {
                    at_space = t.ends_with('\n');
                    trailing = None;
                }
                None
            }
            Op::Br => {
                trim(&mut out, trailing.take());
                at_space = true;
                None
            }
            Op::Box(_) => {
                at_space = false;
                trailing = None;
                None
            }
            Op::Open(..) | Op::Close => None,
        };
        out.push(collapsed);
    }
    trim(&mut out, trailing);
    out
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
    deco_brush(s, link, 0)
}

/// [`brush`] for text on the background of inline element `deco`.
fn deco_brush(s: &Style, link: u32, deco: Id) -> Brush {
    Brush {
        color: if s.hidden { [0; 4] } else { s.color },
        link,
        deco,
    }
}

/// Feeds `h` every input of [`text_style`]: keep the two in step.
fn hash_text_style(h: &mut DefaultHasher, s: &Style, brush: Brush, emergency: bool) {
    s.font_family.hash(h);
    h.write_u32(s.font_size.max(0.0).to_bits());
    h.write_u32(s.letter_spacing.to_bits());
    h.write_u32(s.word_spacing.to_bits());
    h.write_u16(s.weight.unwrap_or(0));
    // Not the shaper's, but kept in the layout that a later one takes over.
    match s.indent {
        Len::Px(p) => h.write_u32(p.to_bits()),
        Len::Pct(p) => h.write_u32(!p.to_bits()),
        Len::Auto => {}
    }
    match s.line_height {
        LineH::Normal => h.write_u8(0),
        LineH::Mult(m) => {
            h.write_u8(1);
            h.write_u32(m.to_bits());
        }
        LineH::Px(p) => {
            h.write_u8(2);
            h.write_u32(p.to_bits());
        }
    }
    for flag in [
        s.bold,
        s.italic,
        s.underline,
        s.strike,
        s.nowrap,
        s.break_all,
        s.wrap_anywhere,
        emergency,
    ] {
        h.write_u8(u8::from(flag));
    }
    h.write(&brush.color);
    h.write_u32(brush.link);
    h.write_u32(brush.deco);
}

fn text_style(s: &Style, brush: Brush, emergency: bool) -> TextStyle<'static, 'static, Brush> {
    TextStyle {
        font_family: FontFamily::Source(Cow::Owned(s.font_family.to_string())),
        font_size: s.font_size.max(0.0),
        font_weight: match (s.weight, s.bold) {
            (Some(n), _) => FontWeight::new(f32::from(n)),
            (None, true) => FontWeight::BOLD,
            (None, false) => FontWeight::NORMAL,
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
        word_spacing: s.word_spacing,
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
        let (r, img) = match doc.items.get(i) {
            Some(Item::Image { r, img, .. }) => (r, img),
            Some(Item::Tiled { tile, img, .. }) => (tile, img),
            _ => continue,
        };
        let Some(im) = doc.images.get(*img) else {
            continue;
        };
        let (iw, ih) = (im.pix.width() as f32, im.pix.height() as f32);
        let (tw, th) = (
            fin(r.w * scale).round().max(1.0) as u32,
            fin(r.h * scale).round().max(1.0) as u32,
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
                    pix: std::sync::Arc::new(pix),
                    nat_w,
                    nat_h,
                    dark_on_clear,
                });
                let k = doc.images.len().saturating_sub(1);
                done.insert(key, k);
                k
            }
        };
        if let Some(Item::Image { img, .. } | Item::Tiled { img, .. }) = doc.items.get_mut(i) {
            *img = ni;
        }
    }
    // Drop originals no longer referenced.
    let mut used = vec![false; doc.images.len()];
    for it in &doc.items {
        if let Item::Image { img, .. } | Item::Tiled { img, .. } = it
            && let Some(u) = used.get_mut(*img)
        {
            *u = true;
        }
    }
    for (im, used) in doc.images.iter_mut().zip(used) {
        if !used && let Some(p) = tiny_skia::Pixmap::new(1, 1) {
            im.pix = std::sync::Arc::new(p);
        }
    }
}
