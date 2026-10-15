//! The document model: closed sets for fonts, sizes and colours, runs,
//! paragraphs, simple tables, signatures and an atomic stored quote, and
//! the caps that keep a document bounded.

use super::link_target;

/// Text of one document (a signature or a message), in bytes.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Blocks in a document, counting paragraphs, tables and signature blocks.
pub const MAX_BLOCKS: usize = 20_000;
/// Runs in one paragraph.
pub const MAX_RUNS: usize = 2048;
/// List levels (0 to this).
pub const MAX_LIST_LEVEL: u8 = 3;
/// Nested quote levels.
pub const MAX_QUOTE_LEVEL: u8 = 8;
/// Indent steps of 40 px.
pub const MAX_INDENT: u8 = 4;
/// Rows and columns of a table.
pub const MAX_TABLE: usize = 10;
/// A stored quote's HTML (the same as the composer's `MAX_QUOTE_BYTES`).
pub const MAX_QUOTE_BYTES: usize = 1024 * 1024;
/// A link address.
pub const MAX_LINK_BYTES: usize = 2048;

/// The ten email-safe font stacks. The label is the first name of the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontId {
    #[default]
    SegoeUi,
    Arial,
    Calibri,
    Verdana,
    Tahoma,
    TrebuchetMs,
    Georgia,
    TimesNewRoman,
    CourierNew,
    Consolas,
}

impl FontId {
    pub const ALL: [FontId; 10] = [
        FontId::SegoeUi,
        FontId::Arial,
        FontId::Calibri,
        FontId::Verdana,
        FontId::Tahoma,
        FontId::TrebuchetMs,
        FontId::Georgia,
        FontId::TimesNewRoman,
        FontId::CourierNew,
        FontId::Consolas,
    ];

    /// The name shown in the font box (a proper noun, not translated).
    pub fn label(self) -> &'static str {
        match self {
            FontId::SegoeUi => "Segoe UI",
            FontId::Arial => "Arial",
            FontId::Calibri => "Calibri",
            FontId::Verdana => "Verdana",
            FontId::Tahoma => "Tahoma",
            FontId::TrebuchetMs => "Trebuchet MS",
            FontId::Georgia => "Georgia",
            FontId::TimesNewRoman => "Times New Roman",
            FontId::CourierNew => "Courier New",
            FontId::Consolas => "Consolas",
        }
    }

    /// The CSS `font-family` value sent.
    pub fn stack(self) -> &'static str {
        match self {
            FontId::SegoeUi => "'Segoe UI','Helvetica Neue',Arial,sans-serif",
            FontId::Arial => "Arial,Helvetica,sans-serif",
            FontId::Calibri => "Calibri,Carlito,Arial,sans-serif",
            FontId::Verdana => "Verdana,Geneva,sans-serif",
            FontId::Tahoma => "Tahoma,Verdana,sans-serif",
            FontId::TrebuchetMs => "'Trebuchet MS',Helvetica,sans-serif",
            FontId::Georgia => "Georgia,'Times New Roman',serif",
            FontId::TimesNewRoman => "'Times New Roman',Times,serif",
            FontId::CourierNew => "'Courier New',Courier,monospace",
            FontId::Consolas => "Consolas,'Courier New',monospace",
        }
    }

    /// The font whose stack is exactly `s` (what this module wrote).
    pub fn from_stack(s: &str) -> Option<FontId> {
        Self::ALL.into_iter().find(|f| f.stack() == s)
    }

    /// The font a pasted `font-family` list names: the first family that
    /// is one of ours (or a close relative), else a generic family.
    pub fn nearest(list: &str) -> Option<FontId> {
        let mut generic = None;
        for name in list.split(',') {
            let n = name
                .trim()
                .trim_matches(['\'', '"'])
                .trim()
                .to_ascii_lowercase();
            let hit = match n.as_str() {
                "segoe ui" | "segoe ui variable" | "system-ui" | "-apple-system" => FontId::SegoeUi,
                "arial" | "helvetica" | "helvetica neue" | "arial unicode ms" => FontId::Arial,
                "calibri" | "carlito" | "cambria" => FontId::Calibri,
                "verdana" | "geneva" => FontId::Verdana,
                "tahoma" => FontId::Tahoma,
                "trebuchet ms" => FontId::TrebuchetMs,
                "georgia" => FontId::Georgia,
                "times new roman" | "times" => FontId::TimesNewRoman,
                "courier new" | "courier" | "lucida console" => FontId::CourierNew,
                "consolas" | "menlo" | "monaco" => FontId::Consolas,
                "monospace" => {
                    generic = generic.or(Some(FontId::CourierNew));
                    continue;
                }
                "serif" => {
                    generic = generic.or(Some(FontId::TimesNewRoman));
                    continue;
                }
                _ => continue,
            };
            return Some(hit);
        }
        generic
    }
}

/// A font size in points, one of twelve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SizePt(u8);

impl SizePt {
    pub const ALL: [u8; 12] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36];
    pub const DEFAULT: SizePt = SizePt(11);

    pub fn new(pt: u8) -> Option<SizePt> {
        Self::ALL.contains(&pt).then_some(SizePt(pt))
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// The closest allowed size to `pt` (a pasted size, possibly odd).
    pub fn nearest(pt: f32) -> SizePt {
        let mut best = SizePt::DEFAULT;
        let mut gap = f32::MAX;
        for &s in &Self::ALL {
            let d = (f32::from(s) - pt).abs();
            if d < gap {
                gap = d;
                best = SizePt(s);
            }
        }
        best
    }

    /// The next larger (`up`) or smaller allowed size, staying at the end.
    pub fn step(self, up: bool) -> SizePt {
        let at = Self::ALL.iter().position(|&s| s == self.0).unwrap_or(3);
        let to = if up {
            at.saturating_add(1)
        } else {
            at.saturating_sub(1)
        };
        Self::ALL.get(to).map_or(self, |&s| SizePt(s))
    }
}

impl Default for SizePt {
    fn default() -> Self {
        SizePt::DEFAULT
    }
}

/// The ten named colours (Fluent's shared colours). "Automatic" is `None`
/// where a colour is optional: no colour is sent, so the recipient's dark
/// mode still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorId {
    Black,
    Grey,
    DarkRed,
    Red,
    Orange,
    Gold,
    Green,
    Teal,
    Blue,
    Purple,
}

impl ColorId {
    pub const ALL: [ColorId; 10] = [
        ColorId::Black,
        ColorId::Grey,
        ColorId::DarkRed,
        ColorId::Red,
        ColorId::Orange,
        ColorId::Gold,
        ColorId::Green,
        ColorId::Teal,
        ColorId::Blue,
        ColorId::Purple,
    ];

    /// The `#RRGGBB` value sent.
    pub fn hex(self) -> &'static str {
        match self {
            ColorId::Black => "#000000",
            ColorId::Grey => "#595959",
            ColorId::DarkRed => "#A4262C",
            ColorId::Red => "#D13438",
            ColorId::Orange => "#CA5010",
            ColorId::Gold => "#986F0B",
            ColorId::Green => "#107C10",
            ColorId::Teal => "#038387",
            ColorId::Blue => "#0F6CBD",
            ColorId::Purple => "#8764B8",
        }
    }

    /// The English name (the UI translates it).
    pub fn name(self) -> &'static str {
        match self {
            ColorId::Black => "Black",
            ColorId::Grey => "Grey",
            ColorId::DarkRed => "Dark red",
            ColorId::Red => "Red",
            ColorId::Orange => "Orange",
            ColorId::Gold => "Gold",
            ColorId::Green => "Green",
            ColorId::Teal => "Teal",
            ColorId::Blue => "Blue",
            ColorId::Purple => "Purple",
        }
    }

    pub fn rgb(self) -> (u8, u8, u8) {
        parse_hex(self.hex()).unwrap_or((0, 0, 0))
    }

    /// The colour with exactly this `#RRGGBB` value.
    pub fn from_hex(s: &str) -> Option<ColorId> {
        Self::ALL
            .iter()
            .copied()
            .find(|c| c.hex().eq_ignore_ascii_case(s))
    }

    /// The closest named colour to a pasted CSS colour, or `None`
    /// (Automatic) for a colour that is black, white or too far from every
    /// name: plain text colour is the recipient's to choose.
    pub fn nearest(css: &str) -> Option<ColorId> {
        let (r, g, b) = parse_css_color(css)?;
        let lum = u32::from(r) + u32::from(g) + u32::from(b);
        if lum <= 3 * 40 || lum >= 3 * 235 {
            return None;
        }
        let mut best = None;
        let mut gap = u32::MAX;
        for c in Self::ALL {
            let (cr, cg, cb) = c.rgb();
            let d = sq(r, cr) + sq(g, cg) + sq(b, cb);
            if d < gap {
                gap = d;
                best = Some(c);
            }
        }
        best.filter(|_| gap <= 120 * 120)
    }
}

fn sq(a: u8, b: u8) -> u32 {
    let d = u32::from(a.abs_diff(b));
    d * d
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    let v = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    match h.len() {
        6 => Some((v(0)?, v(2)?, v(4)?)),
        3 => {
            let d = |i: usize| {
                h.get(i..i + 1)
                    .and_then(|x| u8::from_str_radix(x, 16).ok())
                    .map(|n| n * 17)
            };
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

/// `#rgb`, `#rrggbb`, `rgb(r, g, b)` and a few names.
fn parse_css_color(css: &str) -> Option<(u8, u8, u8)> {
    let c = css.trim().to_ascii_lowercase();
    if c.starts_with('#') {
        return parse_hex(&c);
    }
    if let Some(args) = c.strip_prefix("rgb(").or_else(|| c.strip_prefix("rgba(")) {
        let args = args.split(')').next()?;
        let mut it = args.split([',', ' ', '/']).filter(|p| !p.is_empty());
        let mut chan = || it.next()?.trim_end_matches('%').parse::<f32>().ok();
        let (r, g, b) = (chan()?, chan()?, chan()?);
        let to = |v: f32| v.clamp(0.0, 255.0) as u8;
        return Some((to(r), to(g), to(b)));
    }
    Some(match c.as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "gray" | "grey" => (128, 128, 128),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "orange" => (255, 165, 0),
        "purple" => (128, 0, 128),
        "teal" => (0, 128, 128),
        "gold" => (255, 215, 0),
        "darkred" => (139, 0, 0),
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
        }
    }

    pub fn from_css(s: &str) -> Option<Align> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => Some(Align::Left),
            "center" | "-webkit-center" => Some(Align::Center),
            "right" | "end" => Some(Align::Right),
            "justify" => Some(Align::Justify),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Number,
}

/// Character formatting of a run. `font`, `size` and `color` are `None`
/// where the message default applies (see [`Doc::normalize`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Style {
    pub b: bool,
    pub i: bool,
    pub u: bool,
    pub s: bool,
    pub font: Option<FontId>,
    pub size: Option<SizePt>,
    pub color: Option<ColorId>,
    /// A followable address ([`link_target`]).
    pub link: Option<String>,
}

impl Style {
    pub fn is_plain(&self) -> bool {
        *self == Style::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

impl Run {
    pub fn new(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            style: Style::default(),
        }
    }

    pub fn styled(text: impl Into<String>, style: Style) -> Run {
        Run {
            text: text.into(),
            style,
        }
    }
}

/// A paragraph: runs of text with one alignment, indent, list position and
/// quote level.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Para {
    pub runs: Vec<Run>,
    pub align: Align,
    /// Indent steps, 0 to [`MAX_INDENT`].
    pub indent: u8,
    /// Kind and level (0 to [`MAX_LIST_LEVEL`]) of a list item.
    pub list: Option<(ListKind, u8)>,
    /// Quote depth, 0 to [`MAX_QUOTE_LEVEL`].
    pub quote: u8,
}

impl Para {
    pub fn text(s: &str) -> Para {
        Para {
            runs: if s.is_empty() {
                vec![]
            } else {
                vec![Run::new(s)]
            },
            ..Para::default()
        }
    }

    /// The paragraph's text without formatting.
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.is_empty())
    }
}

/// A table cell: paragraphs of text (no lists, quotes or indent).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub paras: Vec<Para>,
}

impl Cell {
    pub fn text(s: &str) -> Cell {
        Cell {
            paras: vec![Para::text(s)],
        }
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for p in &self.paras {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&p.plain_text());
        }
        out
    }
}

/// A rectangular table up to 10 x 10, no merged cells, no nesting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
}

/// A signature's logo: the picture's bytes live with the signature (its
/// row, or the draft's inline attachment); this names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoRef {
    /// FNV-1a of the image bytes.
    pub hash: u64,
    pub width: u16,
    pub height: u16,
}

impl LogoRef {
    /// The Content-ID the logo is sent under (`cid:` without the scheme).
    pub fn cid(&self) -> String {
        format!("sig-{:016x}@komorebi", self.hash)
    }

    pub fn of(bytes: &[u8], width: u16, height: u16) -> LogoRef {
        LogoRef {
            hash: fnv(bytes),
            width,
            height,
        }
    }
}

/// The signature block of a message. `source` is the stored signature it
/// came from; `untouched_hash` hashes its content when it was put in, so
/// "untouched" (swappable with another account's default) is a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SignatureBlock {
    pub blocks: Vec<Block>,
    pub logo: Option<LogoRef>,
    pub source: Option<i64>,
    pub untouched_hash: u64,
}

impl SignatureBlock {
    /// A signature block with its untouched mark set to its content.
    pub fn new(blocks: Vec<Block>, logo: Option<LogoRef>, source: Option<i64>) -> SignatureBlock {
        let mut s = SignatureBlock {
            blocks,
            logo,
            source,
            untouched_hash: 0,
        };
        s.untouched_hash = s.content_hash();
        s
    }

    pub fn content_hash(&self) -> u64 {
        super::emit::signature_hash(&self.blocks, self.logo.as_ref())
    }

    pub fn is_untouched(&self) -> bool {
        self.untouched_hash == self.content_hash()
    }
}

/// The quote below a reply or forward, kept whole: its HTML and the text
/// read from it.
///
/// The HTML is always the output of `compose_html::sanitize` (nothing that
/// loads anything, no scripts, forms or event handlers, only followable
/// links): [`QuoteBlock::new`] is the only way to make one, so a quote read
/// back from a stored or server draft is as safe as one made from a
/// received message. (The sanitizer drops the `komorebi_quote` class; the
/// quote's `style` stays.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteBlock {
    html: String,
    text: String,
    pub folded: bool,
}

impl QuoteBlock {
    /// `None` if nothing readable is left after sanitizing, or the HTML is
    /// too large. Line ends are read as `\n` (a server may have changed them).
    pub fn new(html: &str, folded: bool) -> Option<QuoteBlock> {
        if html.len() > MAX_QUOTE_BYTES {
            return None;
        }
        let unix;
        let html = if html.contains('\r') {
            unix = html.replace("\r\n", "\n").replace('\r', "\n");
            unix.as_str()
        } else {
            html
        };
        let clean = crate::mail::compose_html::sanitize(html)?;
        let mut text = crate::mail::html::html_to_text(&clean);
        cap_str(&mut text, MAX_TEXT_BYTES);
        Some(QuoteBlock {
            html: clean,
            text: text.trim_end().to_string(),
            folded,
        })
    }

    /// The sanitized HTML.
    pub fn html(&self) -> &str {
        &self.html
    }

    /// The quote as plain text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Para(Para),
    Table(Table),
    Signature(SignatureBlock),
    Quote(QuoteBlock),
}

/// A message or signature: defaults for its text, then blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doc {
    pub font: FontId,
    pub size: SizePt,
    /// `None`: Automatic.
    pub color: Option<ColorId>,
    pub blocks: Vec<Block>,
}

impl Doc {
    /// A document of one paragraph per line of `text`, unformatted.
    pub fn from_plain(text: &str) -> Doc {
        let mut doc = Doc {
            blocks: text.lines().map(|l| Block::Para(Para::text(l))).collect(),
            ..Doc::default()
        };
        doc.normalize();
        doc
    }

    /// Bytes of text in the document.
    pub fn text_bytes(&self) -> usize {
        fn blocks(bs: &[Block]) -> usize {
            bs.iter().fold(0usize, |n, b| {
                n.saturating_add(match b {
                    Block::Para(p) => para(p),
                    Block::Table(t) => t
                        .rows
                        .iter()
                        .flatten()
                        .flat_map(|c| &c.paras)
                        .fold(0usize, |n, p| n.saturating_add(para(p))),
                    Block::Signature(s) => blocks(&s.blocks),
                    Block::Quote(q) => q.html().len(),
                })
            })
        }
        fn para(p: &Para) -> usize {
            p.runs
                .iter()
                .fold(0usize, |n, r| n.saturating_add(r.text.len()))
        }
        blocks(&self.blocks)
    }

    /// Whether the document is empty: no text, table, logo or quote.
    pub fn is_blank(&self) -> bool {
        self.blocks.iter().all(|b| match b {
            Block::Para(p) => p.is_empty(),
            Block::Signature(s) => {
                s.logo.is_none()
                    && s.blocks
                        .iter()
                        .all(|b| matches!(b, Block::Para(p) if p.is_empty()))
            }
            _ => false,
        })
    }

    /// The canonical form every function here produces and expects:
    /// control characters out, no-break spaces as spaces, empty runs gone,
    /// equal neighbours merged, styles equal to the defaults reset, limits
    /// applied, tables rectangular. `import(emit(doc)) == doc` holds for a
    /// normalized document.
    pub fn normalize(&mut self) {
        let mut budget = Budget {
            text: MAX_TEXT_BYTES,
            blocks: MAX_BLOCKS,
        };
        let defaults = (self.font, self.size, self.color);
        let blocks = std::mem::take(&mut self.blocks);
        self.blocks = norm_blocks(blocks, defaults, &mut budget, true);
    }

    pub fn normalized(mut self) -> Doc {
        self.normalize();
        self
    }
}

type Defaults = (FontId, SizePt, Option<ColorId>);

struct Budget {
    text: usize,
    blocks: usize,
}

fn norm_blocks(blocks: Vec<Block>, d: Defaults, budget: &mut Budget, top: bool) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len().min(1024));
    for b in blocks {
        if budget.blocks == 0 {
            break;
        }
        match b {
            Block::Para(mut p) => {
                budget.blocks -= 1;
                norm_para(&mut p, d, budget, false);
                out.push(Block::Para(p));
            }
            Block::Table(t) => {
                budget.blocks -= 1;
                if let Some(t) = norm_table(t, d, budget) {
                    out.push(Block::Table(t));
                }
            }
            Block::Signature(mut s) if top => {
                budget.blocks -= 1;
                let inner = std::mem::take(&mut s.blocks);
                let inner: Vec<Block> = inner
                    .into_iter()
                    .filter(|b| matches!(b, Block::Para(_) | Block::Table(_)))
                    .collect();
                s.blocks = norm_blocks(inner, d, budget, false);
                out.push(Block::Signature(s));
            }
            Block::Quote(q) if top => {
                budget.blocks -= 1;
                out.push(Block::Quote(q));
            }
            // A signature or quote anywhere but at the top level is dropped.
            Block::Signature(_) | Block::Quote(_) => {}
        }
    }
    out
}

fn norm_table(t: Table, d: Defaults, budget: &mut Budget) -> Option<Table> {
    let mut rows: Vec<Vec<Cell>> = t.rows.into_iter().take(MAX_TABLE).collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).min(MAX_TABLE);
    if rows.is_empty() || cols == 0 {
        return None;
    }
    for row in &mut rows {
        row.truncate(cols);
        while row.len() < cols {
            row.push(Cell::default());
        }
        for cell in row.iter_mut() {
            let mut paras = std::mem::take(&mut cell.paras);
            paras.truncate(1024);
            for p in &mut paras {
                norm_para(p, d, budget, true);
            }
            if paras.is_empty() {
                paras.push(Para::default());
            }
            cell.paras = paras;
        }
    }
    Some(Table { rows })
}

fn norm_para(p: &mut Para, d: Defaults, budget: &mut Budget, in_cell: bool) {
    p.indent = p.indent.min(MAX_INDENT);
    p.quote = p.quote.min(MAX_QUOTE_LEVEL);
    p.list = p.list.map(|(k, l)| (k, l.min(MAX_LIST_LEVEL)));
    if in_cell {
        p.indent = 0;
        p.quote = 0;
        p.list = None;
    }
    let runs = std::mem::take(&mut p.runs);
    let mut out: Vec<Run> = Vec::with_capacity(runs.len().min(64));
    for mut r in runs {
        if budget.text == 0 {
            break;
        }
        let mut text = clean_text(&r.text);
        cap_str(&mut text, budget.text);
        if text.is_empty() {
            continue;
        }
        budget.text = budget.text.saturating_sub(text.len());
        r.text = text;
        if r.style.font == Some(d.0) {
            r.style.font = None;
        }
        if r.style.size == Some(d.1) {
            r.style.size = None;
        }
        if r.style.color.is_some() && r.style.color == d.2 {
            r.style.color = None;
        }
        r.style.link = r
            .style
            .link
            .take()
            .and_then(|l| link_target(&l))
            .filter(|l| l.len() <= MAX_LINK_BYTES);
        match out.last_mut() {
            Some(last) if last.style == r.style => last.text.push_str(&r.text),
            _ => out.push(r),
        }
    }
    out.truncate(MAX_RUNS);
    p.runs = out;
}

/// Control characters out (a line break belongs to the paragraph
/// structure), no-break spaces as spaces.
fn clean_text(s: &str) -> String {
    if !s.chars().any(|c| c.is_control() || c == '\u{a0}') {
        return s.to_string();
    }
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '\u{a0}' { ' ' } else { c })
        .collect()
}

/// Cut `s` to at most `max` bytes at a character boundary.
pub(super) fn cap_str(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.truncate(end);
}

/// FNV-1a, 64 bit.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
