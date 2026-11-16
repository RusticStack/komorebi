//! HTML read into a [`Doc`], in two modes.
//!
//! [`strict`] reads exactly what [`super::emit::html`] writes in
//! [`super::emit::Mode::Draft`] (the version tag, the exact subset of
//! elements, attributes and style properties) and rebuilds the same
//! document; anything else is refused. [`lossy`] reads any HTML (pasted
//! CF_HTML, "Edit quoted text", a draft another client saved): it is
//! sanitized first, then mapped to what the model can hold (bold, italic,
//! underline, strikethrough, the nearest font, size and colour, lists,
//! quotes, tables with their merged cells, column widths (px, or percent
//! of the table's width), vertical alignment and borders, pictures, links,
//! alignment, indent) and drops the rest. A picture is kept only when
//! [`lossy_with_images`] is given a resolver that has its bytes without the
//! network ([`super::images::Resolver`]).
//!
//! Text colours are kept exactly, except near black or white (Automatic).
//!
//! What a pasted document can lose (the model cannot hold it): tables
//! nested in a cell (each nested row becomes one line of the cell), more
//! than 10 rows or columns, cell padding and spacing (the editor's and the
//! sent cell padding stand in, so pictures in neighbouring cells never
//! touch) and background colours, border colours and single-side borders,
//! line height and letter spacing, any font or size outside the closed sets
//! (the nearest is taken), headings (bold text), floats, and remote
//! pictures (never fetched).
//!
//! The tokenizer is the reader's (`render::dom`): linear, with node and
//! depth caps. Nothing here fetches anything.

use super::emit::{IMG_STYLE, QUOTE_CLOSE, QUOTE_OPEN, TAG};
use super::images::Found;
use super::link_target;
use super::model::*;
use crate::mail::compose_html;
use crate::mail::render::{MAX_HTML_BYTES, dom};
use dom::{Dom, Id, Kind};

/// Nesting followed before the rest is read as text.
const MAX_WALK_DEPTH: usize = 64;
/// The width in CSS px a percent column width is taken of when its table
/// gives none: a typical message's.
const PERCENT_OF: f32 = 600.0;
/// Stored quotes in one document.
const MAX_QUOTES: usize = 8;
/// A picture's place in pasted HTML while it is sanitized: these two
/// private-use characters around its index.
const MARK_OPEN: char = '\u{E000}';
const MARK_CLOSE: char = '\u{E001}';

/// Read Komorebi's own stored HTML back into the document it came from.
pub fn strict(html: &str) -> Option<Doc> {
    if html.len() > MAX_HTML_BYTES || html.matches(TAG).count() != 1 {
        return None;
    }
    let (pre, quotes) = extract_quotes(html)?;
    let at = pre.find(TAG)?;
    let after = pre.get(at.checked_add(TAG.len())?..)?;
    if !after.starts_with("<div") {
        return None;
    }
    let dom = dom::parse(&pre);
    if dom.truncated {
        return None;
    }
    let wrapper = find_wrapper(&dom)?;
    let mut w = Walk {
        dom: &dom,
        strict: true,
        quotes,
        bad: false,
        pictures: Vec::new(),
    };
    let mut doc = Doc::default();
    let mut base = Ctx::new(true);
    if !w.attrs_only(wrapper, &["style"]) {
        return None;
    }
    if let Some(css) = dom.attr(wrapper, "style") {
        w.style(css, &mut base, StyleKind::Wrapper);
    }
    doc.font = base.style.font.take().unwrap_or_default();
    doc.size = base.style.size.take().unwrap_or_default();
    doc.color = base.style.color.take();
    let mut sink = Sink::new(false);
    w.kids(wrapper, &base, &mut sink);
    sink.flush();
    if w.bad {
        return None;
    }
    doc.blocks = sink.blocks;
    doc.normalize();
    Some(doc)
}

/// Read any HTML into the model, keeping what it can hold, pictures left
/// out. `None` if nothing readable is left (or the HTML is too large to
/// sanitize).
pub fn lossy(html: &str) -> Option<Doc> {
    lossy_with_images(html, &mut |_| None)
}

/// [`lossy`], each `<img>` kept where `resolve` gives it (pasted HTML:
/// a picture whose bytes are on this PC or in the HTML itself).
pub fn lossy_with_images(
    html: &str,
    resolve: &mut dyn FnMut(&Found) -> Option<InlineImage>,
) -> Option<Doc> {
    if html.len() > MAX_PASTE_BYTES {
        return None;
    }
    let (marked, pictures) = mark_images(html, resolve);
    let clean = compose_html::sanitize(&marked)?;
    let dom = dom::parse(&clean);
    let mut w = Walk {
        dom: &dom,
        strict: false,
        quotes: Vec::new(),
        bad: false,
        pictures,
    };
    let mut sink = Sink::new(true);
    w.kids(dom::ROOT, &Ctx::new(false), &mut sink);
    sink.flush();
    let mut doc = Doc {
        blocks: sink.blocks,
        ..Doc::default()
    };
    doc.normalize();
    (!doc.blocks.is_empty() && !doc.is_blank()).then_some(doc)
}

/// Pasted HTML with `data:` pictures may be this large before they are
/// taken out.
const MAX_PASTE_BYTES: usize = 48 * 1024 * 1024;

/// Each `<img>` of `html` replaced by a mark the sanitizer keeps as text
/// (an index between [`MARK_OPEN`] and [`MARK_CLOSE`]) when `resolve` gives
/// its picture, else dropped; the pictures by index. Marks already in the
/// text are taken out first, so pasted text cannot forge one.
fn mark_images(
    html: &str,
    resolve: &mut dyn FnMut(&Found) -> Option<InlineImage>,
) -> (String, Vec<InlineImage>) {
    let mut out = String::with_capacity(html.len().min(MAX_HTML_BYTES));
    let mut pictures = Vec::new();
    let mut rest = html;
    loop {
        let Some(at) = find_img(rest) else {
            push_unmarked(rest, &mut out);
            break;
        };
        push_unmarked(rest.get(..at).unwrap_or(""), &mut out);
        let tail = rest.get(at..).unwrap_or("");
        let end = tag_end(tail).unwrap_or(tail.len());
        let tag = tail.get(..end).unwrap_or("");
        rest = tail.get(end..).unwrap_or("");
        if pictures.len() >= MAX_IMAGES {
            continue;
        }
        let found = img_found(tag);
        if let Some(img) = found.and_then(|f| resolve(&f)) {
            out.push(MARK_OPEN);
            out.push_str(&pictures.len().to_string());
            out.push(MARK_CLOSE);
            pictures.push(img);
        }
    }
    (out, pictures)
}

fn push_unmarked(s: &str, out: &mut String) {
    if s.contains([MARK_OPEN, MARK_CLOSE]) {
        out.extend(s.chars().filter(|c| *c != MARK_OPEN && *c != MARK_CLOSE));
    } else {
        out.push_str(s);
    }
}

/// The byte offset of the next `<img` tag (any case).
fn find_img(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while let Some(off) = b.get(i..)?.iter().position(|&c| c == b'<') {
        let at = i + off;
        let name = b.get(at + 1..at + 4)?;
        let next = b.get(at + 4).copied().unwrap_or(b'>');
        if name.eq_ignore_ascii_case(b"img")
            && (next.is_ascii_whitespace() || next == b'>' || next == b'/')
        {
            return Some(at);
        }
        i = at + 1;
    }
    None
}

/// The length of the tag at the start of `s`, up to its `>` outside quotes.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &c) in s.as_bytes().iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return Some(i + 1),
            None => {}
        }
    }
    None
}

/// What an `<img>` tag says about its picture: its source, width and
/// alternative text.
fn img_found(tag: &str) -> Option<Found> {
    let dom = dom::parse(tag);
    let id = dom
        .children(dom::ROOT)
        .iter()
        .copied()
        .find(|&c| dom.tag(c) == "img")?;
    let src = dom.attr(id, "src")?.trim().to_string();
    let css_w = dom
        .attr(id, "style")
        .and_then(|s| css_decl(s, "width"))
        .and_then(css_px);
    let width = dom
        .attr(id, "width")
        .and_then(|w| {
            w.trim()
                .strip_suffix("px")
                .unwrap_or(w.trim())
                .parse::<f32>()
                .ok()
        })
        .or(css_w)
        .filter(|w| w.is_finite() && *w > 0.0)
        .map(|w| w.round() as u32);
    Some(Found {
        src,
        width,
        alt: dom.attr(id, "alt").unwrap_or_default().to_string(),
    })
}

/// The value of the last `name` declaration of a `style` attribute.
fn css_decl<'s>(css: &'s str, name: &str) -> Option<&'s str> {
    css.split(';')
        .filter_map(|d| d.split_once(':'))
        .filter(|(p, _)| p.trim().eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().trim_end_matches("!important").trim())
        .next_back()
}

/// A CSS length in px (`px`, `pt`, or a bare number); `None` for percent
/// and anything else.
fn css_px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(n) = v.strip_suffix("px") {
        return n.trim().parse().ok();
    }
    if let Some(n) = v.strip_suffix("pt") {
        return n.trim().parse::<f32>().ok().map(|pt| pt * 96.0 / 72.0);
    }
    if let Some(n) = v.strip_suffix("in") {
        return n.trim().parse::<f32>().ok().map(|i| i * 96.0);
    }
    if let Some(n) = v.strip_suffix("cm") {
        return n.trim().parse::<f32>().ok().map(|c| c * 96.0 / 2.54);
    }
    v.parse().ok()
}

/// Replace each stored quote with a placeholder element and return them.
fn extract_quotes(html: &str) -> Option<(String, Vec<QuoteBlock>)> {
    let mut out = String::with_capacity(html.len());
    let mut quotes = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(QUOTE_OPEN) {
        out.push_str(rest.get(..i)?);
        let after = rest.get(i.checked_add(QUOTE_OPEN.len())?..)?;
        let flags_end = after.find("-->")?;
        let folded = match after.get(..flags_end)? {
            "" => false,
            " folded" => true,
            _ => return None,
        };
        let body = after.get(flags_end.checked_add(3)?..)?;
        let close = body.find(QUOTE_CLOSE)?;
        if quotes.len() >= MAX_QUOTES {
            return None;
        }
        quotes.push(QuoteBlock::new(body.get(..close)?, folded)?);
        out.push_str(&format!(
            "<komorebi-quote n=\"{}\"></komorebi-quote>",
            quotes.len() - 1
        ));
        rest = body.get(close.checked_add(QUOTE_CLOSE.len())?..)?;
    }
    out.push_str(rest);
    Some((out, quotes))
}

/// The one `div` inside `html` and `body` that holds the message.
fn find_wrapper(dom: &Dom) -> Option<Id> {
    let mut cur = dom::ROOT;
    for _ in 0..4 {
        let mut elems = Vec::new();
        for &c in dom.children(cur) {
            match dom.node(c).map(|n| &n.kind) {
                Some(Kind::Text(t)) if t.trim().is_empty() => {}
                Some(Kind::Text(_)) => return None,
                Some(Kind::Elem { tag, .. }) if &**tag == "head" => {}
                Some(Kind::Elem { .. }) => elems.push(c),
                None => return None,
            }
        }
        let [only] = elems.as_slice() else {
            return None;
        };
        match dom.tag(*only) {
            "html" | "body" => cur = *only,
            "div" => return Some(*only),
            _ => return None,
        }
    }
    None
}

#[derive(Clone)]
struct Ctx {
    style: Style,
    align: Align,
    indent: u8,
    list: Option<(ListKind, u8)>,
    quote: u8,
    /// Only text and inline elements may appear (a paragraph's content).
    inline_only: bool,
    in_cell: bool,
    /// Where a signature or a stored quote may stand.
    top: bool,
    in_sig: bool,
    depth: usize,
}

impl Ctx {
    fn new(top: bool) -> Ctx {
        Ctx {
            style: Style::default(),
            align: Align::Left,
            indent: 0,
            list: None,
            quote: 0,
            inline_only: false,
            in_cell: false,
            top,
            in_sig: false,
            depth: 0,
        }
    }

    fn para(&self) -> Para {
        Para {
            runs: Vec::new(),
            align: self.align,
            indent: self.indent,
            list: self.list,
            quote: self.quote,
        }
    }
}

/// Collects blocks and the paragraph being filled.
struct Sink {
    blocks: Vec<Block>,
    cur: Option<Para>,
    lossy: bool,
}

impl Sink {
    fn new(lossy: bool) -> Sink {
        Sink {
            blocks: Vec::new(),
            cur: None,
            lossy,
        }
    }

    fn last_char(&self) -> Option<char> {
        self.cur
            .as_ref()
            .and_then(|p| p.runs.last())
            .and_then(|r| r.text.chars().next_back())
    }

    fn push_text(&mut self, text: &str, ctx: &Ctx) {
        let mut buf = String::with_capacity(text.len());
        if self.lossy {
            // Whitespace collapses as in a browser; no-break spaces stay.
            let mut prev = self.last_char();
            for c in text.chars() {
                if c == '\u{a0}' {
                    buf.push(' ');
                    prev = Some(' ');
                } else if c.is_whitespace() {
                    if prev.is_some_and(|p| p != ' ') {
                        buf.push(' ');
                        prev = Some(' ');
                    }
                } else {
                    buf.push(c);
                    prev = Some(c);
                }
            }
        } else {
            buf.extend(text.chars().map(|c| if c == '\u{a0}' { ' ' } else { c }));
        }
        if buf.is_empty() {
            return;
        }
        let p = self.cur.get_or_insert_with(|| ctx.para());
        match p.runs.last_mut() {
            Some(last) if last.image.is_none() && last.style == ctx.style => {
                last.text.push_str(&buf)
            }
            _ => p.runs.push(Run::styled(buf, ctx.style.clone())),
        }
    }

    /// A picture in the paragraph being filled (its link from `ctx`).
    fn push_image(&mut self, img: InlineImage, ctx: &Ctx) {
        let style = Style {
            link: ctx.style.link.clone(),
            ..Style::default()
        };
        let p = self.cur.get_or_insert_with(|| ctx.para());
        p.runs.push(Run::picture(img, style));
    }

    /// In lossy mode, the trailing collapsed space and empty runs go.
    fn trim(&self, mut p: Para) -> Para {
        if self.lossy {
            if let Some(last) = p.runs.last_mut() {
                let t = last.text.trim_end_matches(' ').len();
                last.text.truncate(t);
            }
            p.runs.retain(|r| !r.text.is_empty());
        }
        p
    }

    /// End the paragraph being filled. One that held only collapsed
    /// whitespace is no paragraph.
    fn flush(&mut self) {
        if let Some(p) = self.cur.take() {
            let p = self.trim(p);
            if !(self.lossy && p.runs.is_empty()) {
                self.blocks.push(Block::Para(p));
            }
        }
    }

    /// A line break: the paragraph so far ends (an empty one if none began).
    fn br(&mut self, ctx: &Ctx) {
        let p = self.cur.take().unwrap_or_else(|| ctx.para());
        let p = self.trim(p);
        self.blocks.push(Block::Para(p));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleKind {
    Wrapper,
    Inline,
    Block,
}

struct Walk<'a> {
    dom: &'a Dom,
    strict: bool,
    quotes: Vec<QuoteBlock>,
    bad: bool,
    /// Lossy: the pasted pictures, by the index their marks hold.
    pictures: Vec<InlineImage>,
}

impl Walk<'_> {
    fn fail(&mut self) {
        if self.strict {
            self.bad = true;
        }
    }

    /// Strict: only `allowed` attributes.
    fn attrs_only(&mut self, id: Id, allowed: &[&str]) -> bool {
        if !self.strict {
            return true;
        }
        let ok = match self.dom.node(id).map(|n| &n.kind) {
            Some(Kind::Elem { attrs, .. }) => attrs.iter().all(|(k, _)| allowed.contains(&&**k)),
            _ => false,
        };
        if !ok {
            self.bad = true;
        }
        ok
    }

    fn kids(&mut self, id: Id, ctx: &Ctx, sink: &mut Sink) {
        if ctx.depth >= MAX_WALK_DEPTH {
            if self.strict {
                self.bad = true;
            } else {
                let mut text = String::new();
                self.text_of(id, &mut text);
                sink.push_text(&text, ctx);
            }
            return;
        }
        let mut c = ctx.clone();
        c.depth += 1;
        let dom = self.dom;
        for &ch in dom.children(id) {
            self.flow(ch, &c, sink);
            if self.bad {
                return;
            }
        }
    }

    fn text_of(&self, id: Id, out: &mut String) {
        let mut stack = vec![id];
        let mut budget = 100_000usize;
        while let Some(n) = stack.pop() {
            budget = budget.saturating_sub(1);
            if budget == 0 {
                return;
            }
            if let Some(Kind::Text(t)) = self.dom.node(n).map(|x| &x.kind) {
                out.push_str(t);
            }
            stack.extend(self.dom.children(n).iter().rev());
        }
    }

    fn flow(&mut self, id: Id, ctx: &Ctx, sink: &mut Sink) {
        match self.dom.node(id).map(|n| &n.kind) {
            Some(Kind::Text(t)) => {
                if self.strict && !ctx.inline_only {
                    self.bad = true;
                    return;
                }
                if self.strict || !t.contains(MARK_OPEN) {
                    sink.push_text(t, ctx);
                    return;
                }
                // Pasted pictures, at their marks.
                let mut rest = t.as_str();
                while let Some(at) = rest.find(MARK_OPEN) {
                    sink.push_text(rest.get(..at).unwrap_or(""), ctx);
                    let after = rest.get(at + MARK_OPEN.len_utf8()..).unwrap_or("");
                    let Some(end) = after.find(MARK_CLOSE) else {
                        rest = after;
                        continue;
                    };
                    let picture = after
                        .get(..end)
                        .and_then(|n| n.parse::<usize>().ok())
                        .and_then(|n| self.pictures.get(n))
                        .cloned();
                    if let Some(img) = picture {
                        sink.push_image(img, ctx);
                    }
                    rest = after.get(end + MARK_CLOSE.len_utf8()..).unwrap_or("");
                }
                sink.push_text(rest, ctx);
            }
            Some(Kind::Elem { tag, .. }) => self.elem(id, tag, ctx, sink),
            None => {}
        }
    }

    fn only_child(&self, id: Id) -> bool {
        self.dom.children(self.dom.parent(id)).len() == 1
    }

    fn elem(&mut self, id: Id, tag: &str, ctx: &Ctx, sink: &mut Sink) {
        let dom = self.dom;
        match tag {
            "br" => {
                // A paragraph's only child, or an empty list item's first
                // (a nested list may follow it).
                let parent = dom.parent(id);
                let first = dom.children(parent).first() == Some(&id);
                let lone = self.only_child(id) || (first && dom.tag(parent) == "li");
                if self.strict && !(ctx.inline_only && lone && self.attrs_only(id, &[])) {
                    self.bad = true;
                    return;
                }
                sink.br(ctx);
            }
            "strong" | "b" | "em" | "i" | "u" | "ins" | "s" | "strike" | "del" => {
                if self.strict
                    && (!ctx.inline_only
                        || !matches!(tag, "strong" | "em" | "u" | "s")
                        || !self.attrs_only(id, &[]))
                {
                    self.bad = true;
                    return;
                }
                let mut c = ctx.clone();
                match tag {
                    "strong" | "b" => c.style.b = true,
                    "em" | "i" => c.style.i = true,
                    "u" | "ins" => c.style.u = true,
                    _ => c.style.s = true,
                }
                self.kids(id, &c, sink);
            }
            "a" => {
                if self.strict && (!ctx.inline_only || !self.attrs_only(id, &["href"])) {
                    self.bad = true;
                    return;
                }
                let mut c = ctx.clone();
                if let Some(href) = dom.attr(id, "href") {
                    c.style.link = if self.strict {
                        Some(href.to_string())
                    } else {
                        link_target(href).filter(|l| !l.starts_with('#'))
                    };
                } else if self.strict {
                    self.bad = true;
                    return;
                }
                self.kids(id, &c, sink);
            }
            "span" | "font" => {
                if self.strict
                    && (tag != "span" || !ctx.inline_only || !self.attrs_only(id, &["style"]))
                {
                    self.bad = true;
                    return;
                }
                let mut c = ctx.clone();
                if let Some(css) = dom.attr(id, "style") {
                    self.style(css, &mut c, StyleKind::Inline);
                }
                if tag == "font" {
                    if let Some(face) = dom.attr(id, "face") {
                        c.style.font = FontId::nearest(face).or(c.style.font);
                    }
                    if let Some(color) = dom.attr(id, "color") {
                        c.style.color = ColorId::nearest(color);
                    }
                    if let Some(n) = dom
                        .attr(id, "size")
                        .and_then(|s| s.trim().parse::<i32>().ok())
                    {
                        let pt = [8, 10, 12, 14, 18, 24, 36];
                        let at = usize::try_from(n.clamp(1, 7) - 1).unwrap_or(2);
                        c.style.size = pt.get(at).and_then(|&p| SizePt::new(p));
                    }
                }
                self.kids(id, &c, sink);
            }
            "div" => self.div(id, ctx, sink),
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "center" | "section"
            | "article" | "header" | "footer" | "address" | "figure" | "dl" | "dd" | "dt"
            | "caption" | "main" | "nav" | "aside" | "details" | "summary" | "form"
            | "fieldset" | "hr"
                if !self.strict =>
            {
                sink.flush();
                let mut c = ctx.clone();
                if let Some(css) = dom.attr(id, "style") {
                    self.style(css, &mut c, StyleKind::Block);
                }
                if let Some(a) = dom.attr(id, "align").and_then(Align::from_css) {
                    c.align = a;
                }
                if tag == "center" {
                    c.align = Align::Center;
                }
                if tag.len() == 2 && tag.starts_with('h') {
                    c.style.b = true;
                }
                self.kids(id, &c, sink);
                sink.flush();
            }
            "blockquote" => {
                if self.strict && (ctx.inline_only || !self.attrs_only(id, &["style"])) {
                    self.bad = true;
                    return;
                }
                sink.flush();
                let mut c = ctx.clone();
                if self.strict && c.quote >= MAX_QUOTE_LEVEL {
                    self.bad = true;
                    return;
                }
                c.quote = c.quote.saturating_add(1).min(MAX_QUOTE_LEVEL);
                self.kids(id, &c, sink);
                sink.flush();
            }
            "ul" | "ol" => {
                if self.strict
                    && ((ctx.inline_only && dom.tag(dom.parent(id)) != "li")
                        || !self.attrs_only(id, &[]))
                {
                    self.bad = true;
                    return;
                }
                sink.flush();
                let level = ctx.list.map_or(0, |(_, l)| l.saturating_add(1));
                if self.strict && level > MAX_LIST_LEVEL {
                    self.bad = true;
                    return;
                }
                let kind = if tag == "ol" {
                    ListKind::Number
                } else {
                    ListKind::Bullet
                };
                let mut c = ctx.clone();
                c.list = Some((kind, level.min(MAX_LIST_LEVEL)));
                c.inline_only = false;
                self.kids(id, &c, sink);
                sink.flush();
            }
            "li" => {
                if self.strict && !matches!(dom.tag(dom.parent(id)), "ul" | "ol") {
                    self.bad = true;
                    return;
                }
                if !self.attrs_only(id, &["style"]) {
                    return;
                }
                sink.flush();
                let mut c = ctx.clone();
                c.inline_only = true;
                if self.strict {
                    c.align = Align::Left;
                    c.indent = 0;
                }
                if let Some(css) = dom.attr(id, "style") {
                    self.style(css, &mut c, StyleKind::Block);
                }
                self.kids(id, &c, sink);
                sink.flush();
            }
            // A table in a cell (no nesting in the model): each of its rows
            // one line of the cell, its cells side by side.
            "table" if ctx.in_cell && !self.strict => {
                sink.flush();
                self.kids(id, ctx, sink);
                sink.flush();
            }
            "tr" if ctx.in_cell && !self.strict => {
                sink.flush();
                self.kids(id, ctx, sink);
                sink.flush();
            }
            "td" | "th" if ctx.in_cell && !self.strict => {
                if sink.cur.as_ref().is_some_and(|p| !p.runs.is_empty()) {
                    sink.push_text(" ", ctx);
                }
                self.kids(id, ctx, sink);
            }
            "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" if !self.strict => {
                sink.flush();
                self.kids(id, ctx, sink);
                sink.flush();
            }
            "table" => {
                if self.strict
                    && (ctx.inline_only
                        || ctx.in_cell
                        || !self.attrs_only(id, &["style", "border", "cellpadding", "cellspacing"]))
                {
                    self.bad = true;
                    return;
                }
                sink.flush();
                if let Some(t) = self.table(id, ctx) {
                    sink.blocks.push(Block::Table(t));
                }
            }
            "komorebi-quote" if self.strict => {
                if ctx.inline_only || !ctx.top || !self.attrs_only(id, &["n"]) {
                    self.bad = true;
                    return;
                }
                let q = dom
                    .attr(id, "n")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| self.quotes.get(n))
                    .cloned();
                match q {
                    Some(q) => {
                        sink.flush();
                        sink.blocks.push(Block::Quote(q));
                    }
                    None => self.bad = true,
                }
            }
            "img" if self.strict && ctx.inline_only => {
                let num = |k: &str| dom.attr(id, k).and_then(|v| v.parse::<u32>().ok());
                let img = (|| {
                    if !self.attrs_only(
                        id,
                        &[
                            "src", "width", "height", "alt", "data-iw", "data-ih", "style",
                        ],
                    ) || dom.attr(id, "style") != Some(IMG_STYLE)
                    {
                        return None;
                    }
                    let cid = dom.attr(id, "src")?.strip_prefix("cid:")?;
                    let width = u16::try_from(num("width")?).ok()?;
                    let img = InlineImage {
                        cid: cid.to_string(),
                        iw: num("data-iw")?,
                        ih: num("data-ih")?,
                        width,
                        alt: dom.attr(id, "alt")?.to_string(),
                    };
                    (InlineImage::valid_cid(cid)
                        && img.height() == u16::try_from(num("height")?).ok()?)
                    .then_some(img)
                })();
                match img {
                    Some(img) => sink.push_image(img, ctx),
                    None => self.bad = true,
                }
            }
            // A signature logo as Komorebi 1.0.2 and before wrote it, at the
            // end of the signature: the same picture, now in a paragraph of
            // its own (its Content-ID names the same bytes, `image_cid`).
            "img" if self.strict && ctx.in_sig => {
                let logo = (|| {
                    if !self.attrs_only(id, &["src", "alt", "width", "height"]) {
                        return None;
                    }
                    let hex = dom
                        .attr(id, "src")?
                        .strip_prefix("cid:sig-")?
                        .strip_suffix("@komorebi")?;
                    let hash = u64::from_str_radix(hex, 16).ok()?;
                    let w: u32 = dom.attr(id, "width")?.parse().ok()?;
                    let h: u32 = dom.attr(id, "height")?.parse().ok()?;
                    Some(InlineImage::new(
                        format!("img-{hash:016x}@komorebi"),
                        w,
                        h,
                        "",
                    ))
                })();
                match logo {
                    Some(img) => {
                        sink.flush();
                        sink.push_image(img, ctx);
                        sink.flush();
                    }
                    None => self.bad = true,
                }
            }
            "head" | "title" | "script" | "style" | "meta" | "link" | "img" | "svg" | "object"
            | "iframe" | "video" | "audio" | "canvas" | "template" | "noscript" | "select"
            | "textarea" | "button" | "input"
                if !self.strict => {}
            _ if self.strict => self.bad = true,
            // Unknown and inline-only elements (small, code, sup ...) pass
            // their content through.
            _ => self.kids(id, ctx, sink),
        }
    }

    fn div(&mut self, id: Id, ctx: &Ctx, sink: &mut Sink) {
        let dom = self.dom;
        if self.strict && ctx.inline_only {
            self.bad = true;
            return;
        }
        if dom.attr(id, "class") == Some("sig") && self.strict {
            self.signature(id, ctx, sink);
            return;
        }
        if !self.attrs_only(id, &["style"]) {
            return;
        }
        sink.flush();
        let mut c = ctx.clone();
        c.inline_only = true;
        if self.strict {
            c.align = Align::Left;
            c.indent = 0;
        } else {
            c.inline_only = false;
            if let Some(a) = dom.attr(id, "align").and_then(Align::from_css) {
                c.align = a;
            }
        }
        if let Some(css) = dom.attr(id, "style") {
            self.style(css, &mut c, StyleKind::Block);
        }
        self.kids(id, &c, sink);
        sink.flush();
    }

    fn signature(&mut self, id: Id, ctx: &Ctx, sink: &mut Sink) {
        if !ctx.top || ctx.in_sig || !self.attrs_only(id, &["class", "data-id", "data-h"]) {
            self.bad = true;
            return;
        }
        let dom = self.dom;
        let source = match dom.attr(id, "data-id") {
            Some(s) => match s.parse::<i64>() {
                Ok(n) => Some(n),
                Err(_) => {
                    self.bad = true;
                    return;
                }
            },
            None => None,
        };
        let Some(hash) = dom
            .attr(id, "data-h")
            .and_then(|h| u64::from_str_radix(h, 16).ok())
        else {
            self.bad = true;
            return;
        };
        sink.flush();
        let mut c = Ctx::new(false);
        c.in_sig = true;
        c.depth = ctx.depth;
        let mut inner = Sink::new(false);
        self.kids(id, &c, &mut inner);
        inner.flush();
        sink.blocks.push(Block::Signature(SignatureBlock {
            blocks: inner.blocks,
            source,
            untouched_hash: hash,
        }));
    }

    fn table(&mut self, id: Id, ctx: &Ctx) -> Option<Table> {
        let dom = self.dom;
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut trs: Vec<Id> = Vec::new();
        let mut stack: Vec<Id> = dom.children(id).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            match dom.tag(n) {
                "tr" => trs.push(n),
                "tbody" | "thead" | "tfoot" if !self.strict => {
                    stack.extend(dom.children(n).iter().rev());
                }
                "" => {
                    if self.strict && matches!(dom.node(n).map(|x| &x.kind), Some(Kind::Text(_))) {
                        self.bad = true;
                    }
                }
                _ => self.fail(),
            }
            if trs.len() > MAX_TABLE && self.strict {
                self.bad = true;
            }
            if self.bad {
                return None;
            }
        }
        // Borders: strict, as written (`border="0"` on a layout table);
        // pasted, any `border` attribute or declaration that draws one.
        let draws = |css: Option<&str>| {
            css.and_then(|s| css_decl(s, "border")).is_some_and(|v| {
                let v = v.to_ascii_lowercase();
                !(v.contains("none") || v.starts_with('0') || v.contains("hidden"))
            })
        };
        let mut bordered = if self.strict {
            dom.attr(id, "border") != Some("0")
        } else {
            dom.attr(id, "border")
                .and_then(|b| b.trim().parse::<u32>().ok())
                .is_some_and(|b| b > 0)
                || draws(dom.attr(id, "style"))
        };
        // Percent widths are taken against the table's own width in px, or
        // a typical message's width.
        let table_w = dom
            .attr(id, "width")
            .and_then(|w| css_px(w.trim()))
            .or_else(|| {
                dom.attr(id, "style")
                    .and_then(|s| css_decl(s, "width"))
                    .and_then(css_px)
            })
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(PERCENT_OF);
        let strict = self.strict;
        let length = |v: &str| -> Option<f32> {
            let v = v.trim();
            match v.strip_suffix('%') {
                Some(p) if !strict => p.trim().parse::<f32>().ok().map(|p| p * table_w / 100.0),
                Some(_) => None,
                None => css_px(v.strip_suffix("px").map_or(v, |n| n.trim())),
            }
        };
        let span = |v: Option<&str>, room: usize| -> usize {
            v.and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, room.max(1))
        };
        // Cells placed on a grid: each in the first slot of its row a span
        // from above has not taken.
        let mut grid: Vec<Vec<Option<Cell>>> = Vec::new();
        let mut taken = [[false; MAX_TABLE]; MAX_TABLE];
        let mut widths = [0u16; MAX_TABLE];
        for (r, tr) in trs.into_iter().take(MAX_TABLE).enumerate() {
            if !self.attrs_only(tr, &[]) {
                return None;
            }
            let row_valign = dom.attr(tr, "valign").and_then(VAlign::from_css);
            let mut row: Vec<Option<Cell>> = Vec::new();
            let mut col = 0usize;
            for &td in dom.children(tr) {
                match dom.tag(td) {
                    "td" | "th" => {}
                    _ => {
                        self.fail();
                        continue;
                    }
                }
                while taken
                    .get(r)
                    .and_then(|x| x.get(col))
                    .copied()
                    .unwrap_or(false)
                {
                    col = col.saturating_add(1);
                }
                if col >= MAX_TABLE {
                    self.fail();
                    break;
                }
                if !self.attrs_only(td, &["style", "width", "colspan", "rowspan"]) {
                    return None;
                }
                let span_c = span(dom.attr(td, "colspan"), MAX_TABLE - col);
                let span_r = span(dom.attr(td, "rowspan"), MAX_TABLE - r);
                for line in taken.iter_mut().skip(r).take(span_r) {
                    for flag in line.iter_mut().skip(col).take(span_c) {
                        *flag = true;
                    }
                }
                let css = dom.attr(td, "style");
                if !self.strict && draws(css) {
                    bordered = true;
                }
                let valign = css
                    .and_then(|s| css_decl(s, "vertical-align"))
                    .and_then(VAlign::from_css)
                    .or_else(|| dom.attr(td, "valign").and_then(VAlign::from_css))
                    .or(row_valign)
                    .unwrap_or(if self.strict {
                        VAlign::Top
                    } else {
                        // A browser centres a cell's content by default.
                        VAlign::Middle
                    });
                let w = dom
                    .attr(td, "width")
                    .and_then(length)
                    .or_else(|| css.and_then(|s| css_decl(s, "width")).and_then(length))
                    .filter(|w| w.is_finite() && *w > 0.0)
                    .map_or(0, |w| w.round().min(f32::from(MAX_IMAGE_W)) as u16);
                // A column's width from a cell of its own (a merged cell's
                // is shared by the columns it spans).
                if span_c == 1
                    && let Some(slot) = widths.get_mut(col)
                    && *slot == 0
                {
                    *slot = w;
                }
                let mut c = Ctx::new(false);
                c.in_cell = true;
                c.depth = ctx.depth + 1;
                if !self.strict {
                    c.style = ctx.style.clone();
                    if let Some(css) = css {
                        self.style(css, &mut c, StyleKind::Block);
                    }
                    if let Some(a) = dom.attr(td, "align").and_then(Align::from_css) {
                        c.align = a;
                    }
                }
                let mut cs = Sink::new(!self.strict);
                self.kids(td, &c, &mut cs);
                cs.flush();
                let paras = cs
                    .blocks
                    .into_iter()
                    .filter_map(|b| match b {
                        Block::Para(p) => Some(p),
                        _ => None,
                    })
                    .collect();
                if row.len() <= col {
                    row.resize_with(col.saturating_add(1), || None);
                }
                if let Some(slot) = row.get_mut(col) {
                    *slot = Some(Cell {
                        paras,
                        valign,
                        colspan: if span_c > 1 { span_c as u8 } else { 0 },
                        rowspan: if span_r > 1 { span_r as u8 } else { 0 },
                    });
                }
                col = col.saturating_add(span_c);
                if self.bad {
                    return None;
                }
            }
            grid.push(row);
        }
        // Rows a span reaches past the last `tr` are not added; a row with
        // no cell of its own and none reaching it is dropped.
        let cols = taken
            .iter()
            .take(grid.len())
            .filter_map(|line| line.iter().rposition(|t| *t))
            .max()
            .map_or(0, |c| c.saturating_add(1));
        for (r, row) in grid.into_iter().enumerate() {
            let reached = taken.get(r).is_some_and(|line| line.iter().any(|t| *t));
            if row.iter().all(Option::is_none) && !reached {
                continue;
            }
            let mut out: Vec<Cell> = row.into_iter().map(Option::unwrap_or_default).collect();
            out.resize_with(cols.max(out.len()), Cell::default);
            rows.push(out);
        }
        let widths: Vec<u16> = if widths.iter().all(|w| *w == 0) {
            Vec::new()
        } else {
            widths.iter().take(cols).copied().collect()
        };
        (!rows.is_empty()).then_some(Table {
            rows,
            borderless: !bordered,
            widths,
        })
    }

    /// Apply the declarations of a `style` attribute. Strict: only the
    /// properties this module writes for `kind`, with exactly its values.
    fn style(&mut self, css: &str, ctx: &mut Ctx, kind: StyleKind) {
        for decl in css.split(';') {
            let Some((prop, val)) = decl.split_once(':') else {
                if !decl.trim().is_empty() {
                    self.fail();
                }
                continue;
            };
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim().trim_end_matches("!important").trim();
            let font_like = matches!(kind, StyleKind::Wrapper | StyleKind::Inline);
            match prop.as_str() {
                "font-family" => {
                    if self.strict {
                        match FontId::from_stack(val) {
                            Some(f) if font_like => ctx.style.font = Some(f),
                            _ => self.bad = true,
                        }
                    } else if let Some(f) = FontId::nearest(val) {
                        ctx.style.font = Some(f);
                    }
                }
                "font-size" => {
                    if self.strict {
                        let pt = val
                            .strip_suffix("pt")
                            .and_then(|n| n.parse::<u8>().ok())
                            .and_then(SizePt::new);
                        match pt {
                            Some(s) if font_like => ctx.style.size = Some(s),
                            _ => self.bad = true,
                        }
                    } else if let Some(pt) = css_pt(val) {
                        ctx.style.size = Some(SizePt::nearest(pt));
                    }
                }
                "color" => {
                    if self.strict {
                        match ColorId::from_hex(val) {
                            Some(c) if font_like => ctx.style.color = Some(c),
                            _ => self.bad = true,
                        }
                    } else {
                        ctx.style.color = ColorId::nearest(val);
                    }
                }
                "text-align" => match Align::from_css(val) {
                    Some(a) if !self.strict || kind == StyleKind::Block => ctx.align = a,
                    _ => self.fail(),
                },
                "margin-left" => {
                    let px = val
                        .strip_suffix("px")
                        .and_then(|n| n.trim().parse::<f32>().ok());
                    match px {
                        Some(px) if self.strict => {
                            let steps = (px / 40.0).round();
                            if kind == StyleKind::Block
                                && px >= 0.0
                                && (px % 40.0) == 0.0
                                && steps <= f32::from(MAX_INDENT)
                            {
                                ctx.indent = steps as u8;
                            } else {
                                self.bad = true;
                            }
                        }
                        Some(px) => {
                            ctx.indent =
                                (px / 40.0).round().clamp(0.0, f32::from(MAX_INDENT)) as u8;
                        }
                        None => self.fail(),
                    }
                }
                "font-weight" if !self.strict => {
                    let v = val.to_ascii_lowercase();
                    ctx.style.b =
                        v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|n| n >= 600);
                }
                "font-style" if !self.strict => {
                    let v = val.to_ascii_lowercase();
                    ctx.style.i = v == "italic" || v == "oblique";
                }
                "text-decoration" | "text-decoration-line" if !self.strict => {
                    let v = val.to_ascii_lowercase();
                    if v.contains("underline") {
                        ctx.style.u = true;
                    }
                    if v.contains("line-through") {
                        ctx.style.s = true;
                    }
                }
                _ => self.fail(),
            }
        }
    }
}

/// A CSS font size as points: `pt`, `px` and the absolute keywords.
fn css_pt(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(n) = v.strip_suffix("pt") {
        return n.trim().parse().ok();
    }
    if let Some(n) = v.strip_suffix("px") {
        return n.trim().parse::<f32>().ok().map(|px| px * 0.75);
    }
    Some(match v.as_str() {
        "xx-small" => 8.0,
        "x-small" => 9.0,
        "small" => 10.0,
        "medium" => 12.0,
        "large" => 14.0,
        "x-large" => 18.0,
        "xx-large" => 24.0,
        _ => return None,
    })
}
