//! HTML and plain text generated from a [`Doc`].
//!
//! The HTML is a small, fixed vocabulary (div, strong, em, u, s, span with
//! font-family, font-size and color only, a, ul, ol, li, blockquote, table)
//! with every value taken from a closed set or escaped, so nothing the user
//! typed can become markup. [`Mode::Draft`] adds what Komorebi needs to read
//! its own drafts back exactly ([`super::import::strict`]): a version tag, the
//! stored quote's boundaries and a signature's source and untouched mark.
//! [`Mode::Send`] leaves those out.

use super::model::*;

/// What the HTML is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The text/html part of a message that is sent.
    Send,
    /// A draft's stored form (and its copy in the server's Drafts folder).
    Draft,
}

/// The tag a draft's HTML starts with after the `<body>`.
pub const TAG: &str = "<!--komorebi:rich 1-->";
pub(super) const QUOTE_OPEN: &str = "<!--komorebi:quote";
pub(super) const QUOTE_CLOSE: &str = "<!--/komorebi:quote-->";
const BLOCKQUOTE: &str =
    r#"<blockquote style="margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex">"#;
const TABLE: &str = r#"<table style="border-collapse:collapse">"#;
const TD: &str = r#"<td style="border:1px solid #c8c8c8;padding:4px 8px;vertical-align:top">"#;
const HEAD: &str = "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body>";

/// The full HTML document of `doc`.
pub fn html(doc: &Doc, mode: Mode) -> String {
    let mut e = Emitter::new(mode);
    e.out.push_str(HEAD);
    if mode == Mode::Draft {
        e.out.push_str(TAG);
    }
    e.out.push_str("<div style=\"font-family:");
    e.out.push_str(doc.font.stack());
    e.out.push_str(";font-size:");
    e.out.push_str(&doc.size.get().to_string());
    e.out.push_str("pt");
    if let Some(c) = doc.color {
        e.out.push_str(";color:");
        e.out.push_str(c.hex());
    }
    e.out.push_str("\">");
    for b in &doc.blocks {
        e.block(b);
    }
    e.finish();
    e.out.push_str("</div></body></html>");
    e.out
}

/// Whether the message needs a text/html part. A document with no formatting
/// at all is sent as text/plain only.
pub fn needs_html(doc: &Doc) -> bool {
    doc.font != FontId::default()
        || doc.size != SizePt::DEFAULT
        || doc.color.is_some()
        || doc.blocks.iter().any(block_needs)
}

fn block_needs(b: &Block) -> bool {
    match b {
        Block::Para(p) => para_needs(p),
        Block::Table(_) | Block::Quote(_) => true,
        Block::Signature(s) => s.logo.is_some() || s.blocks.iter().any(block_needs),
    }
}

fn para_needs(p: &Para) -> bool {
    p.align != Align::Left
        || p.indent > 0
        || p.list.is_some()
        || p.quote > 0
        || p.runs.iter().any(|r| !r.style.is_plain())
}

/// A hash of a signature's content: what [`SignatureBlock::is_untouched`]
/// compares.
pub fn signature_hash(blocks: &[Block], logo: Option<&LogoRef>) -> u64 {
    let mut e = Emitter::new(Mode::Draft);
    for b in blocks {
        e.block(b);
    }
    e.finish();
    if let Some(l) = logo {
        e.out.push_str(&l.cid());
    }
    fnv(e.out.as_bytes())
}

struct Emitter {
    out: String,
    mode: Mode,
    /// Open lists, outermost first; each has its last `<li>` open.
    lists: Vec<ListKind>,
    /// Open blockquotes.
    quote: u8,
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

impl Emitter {
    fn new(mode: Mode) -> Emitter {
        Emitter {
            out: String::new(),
            mode,
            lists: Vec::new(),
            quote: 0,
        }
    }

    fn close_lists_to(&mut self, n: usize) {
        while self.lists.len() > n {
            match self.lists.pop() {
                Some(ListKind::Number) => self.out.push_str("</li></ol>"),
                Some(ListKind::Bullet) => self.out.push_str("</li></ul>"),
                None => break,
            }
        }
    }

    fn set_quote(&mut self, q: u8) {
        if q == self.quote {
            return;
        }
        self.close_lists_to(0);
        while self.quote > q {
            self.out.push_str("</blockquote>");
            self.quote -= 1;
        }
        while self.quote < q {
            self.out.push_str(BLOCKQUOTE);
            self.quote += 1;
        }
    }

    /// Close everything open: lists and blockquotes.
    fn finish(&mut self) {
        self.close_lists_to(0);
        self.set_quote(0);
    }

    fn block(&mut self, b: &Block) {
        match b {
            Block::Para(p) => self.para(p),
            Block::Table(t) => {
                self.finish();
                self.table(t);
            }
            Block::Signature(s) => {
                self.finish();
                self.signature(s);
            }
            Block::Quote(q) => {
                self.finish();
                if self.mode == Mode::Draft {
                    self.out.push_str(QUOTE_OPEN);
                    if q.folded {
                        self.out.push_str(" folded");
                    }
                    self.out.push_str("-->");
                }
                self.out.push_str(q.html());
                if self.mode == Mode::Draft {
                    self.out.push_str(QUOTE_CLOSE);
                }
            }
        }
    }

    fn para_style(p: &Para) -> String {
        let mut parts: Vec<String> = Vec::new();
        if p.align != Align::Left {
            parts.push(format!("text-align:{}", p.align.css()));
        }
        if p.indent > 0 {
            parts.push(format!("margin-left:{}px", u32::from(p.indent) * 40));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" style=\"{}\"", parts.join(";"))
        }
    }

    fn para(&mut self, p: &Para) {
        if p.quote != self.quote {
            self.set_quote(p.quote);
        }
        let style = Self::para_style(p);
        match p.list {
            None => {
                self.close_lists_to(0);
                self.out.push_str("<div");
                self.out.push_str(&style);
                self.out.push('>');
                self.runs(p);
                self.out.push_str("</div>");
            }
            Some((kind, level)) => {
                let want = usize::from(level.min(MAX_LIST_LEVEL)) + 1;
                self.close_lists_to(want);
                if self.lists.len() == want {
                    if self.lists.get(want - 1) == Some(&kind) {
                        self.out.push_str("</li>");
                    } else {
                        self.close_lists_to(want - 1);
                    }
                }
                while self.lists.len() < want {
                    self.out.push_str(match kind {
                        ListKind::Bullet => "<ul>",
                        ListKind::Number => "<ol>",
                    });
                    self.lists.push(kind);
                    if self.lists.len() < want {
                        self.out.push_str("<li>");
                    }
                }
                self.out.push_str("<li");
                self.out.push_str(&style);
                self.out.push('>');
                self.runs(p);
            }
        }
    }

    fn runs(&mut self, p: &Para) {
        if p.runs.is_empty() {
            self.out.push_str("<br>");
            return;
        }
        let total: usize = p.runs.iter().map(|r| r.text.chars().count()).sum();
        let mut seen = 0usize;
        // Collapsible whitespace is kept as no-break spaces where a browser
        // would drop it: at the edges and after another space.
        let mut prev_space = true;
        for r in &p.runs {
            let close = self.open_run(&r.style);
            for c in r.text.chars() {
                seen += 1;
                match c {
                    ' ' => {
                        if prev_space || seen == total {
                            self.out.push_str("&nbsp;");
                            prev_space = false;
                        } else {
                            self.out.push(' ');
                            prev_space = true;
                        }
                    }
                    '&' => {
                        self.out.push_str("&amp;");
                        prev_space = false;
                    }
                    '<' => {
                        self.out.push_str("&lt;");
                        prev_space = false;
                    }
                    '>' => {
                        self.out.push_str("&gt;");
                        prev_space = false;
                    }
                    '"' => {
                        self.out.push_str("&quot;");
                        prev_space = false;
                    }
                    c => {
                        self.out.push(c);
                        prev_space = false;
                    }
                }
            }
            self.out.push_str(&close);
        }
    }

    /// Open the tags of a run's style; returns the closing tags.
    fn open_run(&mut self, st: &Style) -> String {
        let mut close = String::new();
        if let Some(link) = &st.link {
            self.out.push_str("<a href=\"");
            escape(link, &mut self.out);
            self.out.push_str("\">");
            close.insert_str(0, "</a>");
        }
        if st.font.is_some() || st.size.is_some() || st.color.is_some() {
            let mut parts: Vec<String> = Vec::new();
            if let Some(f) = st.font {
                parts.push(format!("font-family:{}", f.stack()));
            }
            if let Some(s) = st.size {
                parts.push(format!("font-size:{}pt", s.get()));
            }
            if let Some(c) = st.color {
                parts.push(format!("color:{}", c.hex()));
            }
            self.out.push_str("<span style=\"");
            self.out.push_str(&parts.join(";"));
            self.out.push_str("\">");
            close.insert_str(0, "</span>");
        }
        for (on, tag) in [(st.b, "strong"), (st.i, "em"), (st.u, "u"), (st.s, "s")] {
            if on {
                self.out.push('<');
                self.out.push_str(tag);
                self.out.push('>');
                close.insert_str(0, &format!("</{tag}>"));
            }
        }
        close
    }

    fn table(&mut self, t: &Table) {
        self.out.push_str(TABLE);
        for row in &t.rows {
            self.out.push_str("<tr>");
            for cell in row {
                self.out.push_str(TD);
                for p in &cell.paras {
                    let style = Self::para_style(p);
                    self.out.push_str("<div");
                    self.out.push_str(&style);
                    self.out.push('>');
                    self.runs(p);
                    self.out.push_str("</div>");
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>");
        }
        self.out.push_str("</table>");
    }

    fn signature(&mut self, s: &SignatureBlock) {
        self.out.push_str("<div class=\"sig\"");
        if self.mode == Mode::Draft {
            if let Some(id) = s.source {
                self.out.push_str(&format!(" data-id=\"{id}\""));
            }
            self.out
                .push_str(&format!(" data-h=\"{:016x}\"", s.untouched_hash));
        }
        self.out.push('>');
        let mut inner = Emitter::new(self.mode);
        for b in &s.blocks {
            inner.block(b);
        }
        inner.finish();
        self.out.push_str(&inner.out);
        if let Some(l) = &s.logo {
            self.out.push_str(&format!(
                "<img src=\"cid:{}\" alt=\"\" width=\"{}\" height=\"{}\">",
                l.cid(),
                l.width,
                l.height
            ));
        }
        self.out.push_str("</div>");
    }
}

// ---- plain text -----------------------------------------------------------

/// A table is laid out in aligned columns when its lines fit this.
const PLAIN_WIDTH: usize = 76;

/// The text/plain form of `doc`: no bold, italic, underline, font or colour;
/// lists as `- ` and `1. `, quotes as `> `, links as `label (address)`,
/// tables as aligned columns, and the signature after a `-- ` line.
pub fn plain(doc: &Doc) -> String {
    plain_blocks(&doc.blocks, true)
}

/// The plain text of signature blocks without the `-- ` line (a stored
/// signature's `body_text`).
pub fn signature_text(blocks: &[Block]) -> String {
    plain_blocks(blocks, false)
}

fn plain_blocks(blocks: &[Block], delimiter: bool) -> String {
    let mut p = Plain {
        lines: Vec::new(),
        levels: [None; 4],
        delimiter,
    };
    for b in blocks {
        p.block(b);
    }
    p.lines.join("\n")
}

struct Plain {
    lines: Vec<String>,
    /// The kind and last number of the open list at each level.
    levels: [Option<(ListKind, u32)>; 4],
    delimiter: bool,
}

impl Plain {
    fn blank_before(&mut self) {
        if self.lines.last().is_some_and(|l| !l.is_empty()) {
            self.lines.push(String::new());
        }
    }

    fn block(&mut self, b: &Block) {
        if !matches!(b, Block::Para(Para { list: Some(_), .. })) {
            self.levels = [None; 4];
        }
        match b {
            Block::Para(p) => self.para(p),
            Block::Table(t) => self.table(t),
            Block::Signature(s) => {
                let mut inner = Plain {
                    lines: Vec::new(),
                    levels: [None; 4],
                    delimiter: self.delimiter,
                };
                for b in &s.blocks {
                    inner.block(b);
                }
                if inner.lines.iter().all(String::is_empty) {
                    return;
                }
                if self.delimiter {
                    self.blank_before();
                    self.lines.push("-- ".to_string());
                }
                self.lines.append(&mut inner.lines);
            }
            Block::Quote(q) => {
                self.blank_before();
                for l in q.text().lines() {
                    self.lines.push(if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    });
                }
            }
        }
    }

    fn para(&mut self, p: &Para) {
        let mut prefix = "> ".repeat(usize::from(p.quote));
        match p.list {
            Some((kind, level)) => {
                let l = usize::from(level.min(MAX_LIST_LEVEL));
                for deeper in self.levels.iter_mut().skip(l + 1) {
                    *deeper = None;
                }
                let n = match self.levels.get(l).copied().flatten() {
                    Some((k, n)) if k == kind => n.saturating_add(1),
                    _ => 1,
                };
                if let Some(slot) = self.levels.get_mut(l) {
                    *slot = Some((kind, n));
                }
                prefix.push_str(&"  ".repeat(l));
                match kind {
                    ListKind::Bullet => prefix.push_str("- "),
                    ListKind::Number => prefix.push_str(&format!("{n}. ")),
                }
            }
            None => self.levels = [None; 4],
        }
        let text = runs_plain(p);
        let line = format!("{prefix}{text}");
        self.lines.push(if text.is_empty() {
            line.trim_end().to_string()
        } else {
            line
        });
    }

    fn table(&mut self, t: &Table) {
        let rows: Vec<Vec<String>> = t
            .rows
            .iter()
            .map(|r| r.iter().map(Cell::plain_text).collect())
            .collect();
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..cols)
            .map(|c| {
                rows.iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| s.chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let total: usize = widths.iter().sum::<usize>() + 2 * cols.saturating_sub(1);
        let aligned = total <= PLAIN_WIDTH;
        for r in &rows {
            let line = if aligned {
                let mut s = String::new();
                for (i, cell) in r.iter().enumerate() {
                    if i > 0 {
                        s.push_str("  ");
                    }
                    s.push_str(cell);
                    let w = widths.get(i).copied().unwrap_or(0);
                    for _ in cell.chars().count()..w {
                        s.push(' ');
                    }
                }
                s.trim_end().to_string()
            } else {
                r.join(" | ")
            };
            self.lines.push(line);
        }
    }
}

/// A paragraph's text, a link reading `label (address)` unless the label is
/// the address.
fn runs_plain(p: &Para) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    while let Some(r) = p.runs.get(i) {
        match &r.style.link {
            None => {
                out.push_str(&r.text);
                i += 1;
            }
            Some(url) => {
                let mut label = String::new();
                while let Some(next) = p.runs.get(i) {
                    if next.style.link.as_deref() != Some(url.as_str()) {
                        break;
                    }
                    label.push_str(&next.text);
                    i += 1;
                }
                out.push_str(&label);
                if !same_target(&label, url) {
                    out.push_str(" (");
                    out.push_str(url);
                    out.push(')');
                }
            }
        }
    }
    out
}

fn same_target(label: &str, url: &str) -> bool {
    let l = label.trim().trim_end_matches('/');
    let u = url.trim_end_matches('/');
    l == u || url.strip_prefix("mailto:") == Some(l)
}
