//! The text/html alternative of outgoing mail. The composer stays plain
//! text: a few Markdown-lite markers typed there (`**bold**`, `*italic*` or
//! `_italic_`, `- ` and `1. ` lists, `> ` quotes, `[label](https://...)`
//! links) become conservative email HTML, and a reply or forward of an HTML
//! message quotes the sanitized original below the user's text. There are
//! no fonts or colours to pick, and nothing here fetches anything.
//!
//! A message gets HTML only when [`has_formatting`] finds markers or a
//! quote is attached; otherwise it goes out plain, byte for byte as typed.
//!
//! The input is the user's text and untrusted mail HTML, so this module
//! denies the lints below and every scan is linear.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use super::render::{MAX_HTML_BYTES, dom, links};

/// `> ` levels nested as blockquotes; deeper lines stay at this depth.
const MAX_QUOTE_DEPTH: usize = 8;
/// A sanitized original larger than this is not quoted as HTML.
const MAX_QUOTE_BYTES: usize = 1024 * 1024;
/// Nested inline formatting (bold inside italic ...) kept.
const MAX_INLINE_DEPTH: usize = 3;
/// The quote below a reply or forward, set off from the text above.
const QUOTE_DIV: &str = r#"<div class="komorebi_quote" style="margin-top:1em">"#;
/// The blockquote style of the user's `> ` lines and of a reply's quote.
const BLOCKQUOTE: &str =
    r#"<blockquote style="border-left:2px solid #ccc;margin:0 0 0 .8ex;padding-left:1ex">"#;

/// What a draft sends: the text/plain body and, when it needs one, the
/// text/html alternative. `quoted` is the draft's stored quote ([`attach`]):
/// used while the text from its boundary line on is still exactly as the
/// composer put it, and dropped otherwise (the whole text is converted, so
/// the edited text quote still goes out as blockquotes).
pub fn alternative(body: &str, quoted: Option<&str>) -> (String, Option<String>) {
    if let Some((user, block)) = quoted.and_then(|q| split(body, q)) {
        let mut html = to_html(user);
        html.push_str(block);
        return (plain(body), Some(document(&html)));
    }
    // A quote whose text the user edited: the whole text is converted.
    if quoted.is_some() || has_formatting(body) {
        return (plain(body), Some(document(&to_html(body))));
    }
    (body.to_string(), None)
}

/// The composer's Preview: the HTML part a draft sends ([`alternative`]),
/// or for plain text the same text as HTML, so Preview always shows how
/// the message reads.
pub fn preview_html(body: &str, quoted: Option<&str>) -> String {
    alternative(body, quoted)
        .1
        .unwrap_or_else(|| document(&to_html(body)))
}

/// Whether `text` uses a marker that needs HTML: bold, italic, a list or a
/// `[label](url)` link outside `> ` quoted lines. Bare URLs and quotes
/// alone don't count: every client shows those well in plain text.
pub fn has_formatting(text: &str) -> bool {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .any(|l| list_item(l, false).is_some() || inline(l, &mut String::new(), 0))
}

/// The text/plain part of a message that also has HTML, and the text of
/// a draft's previews: the inline marks the HTML part converts are taken
/// out (`**bold**` reads `bold`, `_it_` reads `it`) and `[label](url)`
/// links read `label (url)`, by the same scan as [`to_html`], so a lone
/// `*` or `snake_case` stays. Lists, `> ` quotes and line breaks stay as
/// typed: they read well as text.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        inline_to(body, &mut out, 0, Sink::Plain);
        out.push_str(end);
    }
    out
}

/// The full text/html part around converted `body` HTML.
fn document(body: &str) -> String {
    format!(
        "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head>\
         <body><div style=\"font-family:Segoe UI,Arial,sans-serif;font-size:14px\">{body}</div></body></html>"
    )
}

/// The quote below a reply: the attribution line and the sanitized
/// original ([`sanitize`]) in a blockquote.
pub fn reply_quote(attribution: &str, original: &str) -> String {
    let mut out = String::from(QUOTE_DIV);
    escape(attribution, &mut out);
    out.push_str(BLOCKQUOTE);
    out.push_str(original);
    out.push_str("</blockquote></div>");
    out
}

/// The forwarded message: `heading`, a table of the original's headers
/// (label and value pairs) and the sanitized original ([`sanitize`]).
pub fn forward_quote(heading: &str, fields: &[(&str, &str)], original: &str) -> String {
    let mut out = String::from(QUOTE_DIV);
    escape(heading, &mut out);
    out.push_str(r#"<table cellpadding="0" cellspacing="0" border="0">"#);
    for (label, value) in fields {
        out.push_str(r#"<tr><td style="padding-right:1ex"><b>"#);
        escape(label, &mut out);
        out.push_str(":</b></td><td>");
        escape(value, &mut out);
        out.push_str("</td></tr>");
    }
    out.push_str("</table><br>");
    out.push_str(original);
    out.push_str("</div>");
    out
}

/// A quote `block` stored with a draft whose `body` holds the text quote
/// from the line `boundary` on (the attribution or forward heading). It
/// records that tail's line count and hash, so [`alternative`] uses the
/// block only while the user left the text quote alone; an edited quote
/// (a trimmed thread, answers typed between quoted lines) sends the text
/// as edited instead. `None` if `body` has no such line.
pub fn attach(body: &str, boundary: &str, block: &str) -> Option<String> {
    let mut start = 0usize;
    let at = body.split('\n').find_map(|line| {
        let here = start;
        start = start.saturating_add(line.len()).saturating_add(1);
        (line == boundary).then_some(here)
    })?;
    let tail = body.get(at..)?;
    Some(format!(
        "<!--komorebi-quote {} {:016x}-->{block}",
        tail.split('\n').count(),
        fnv(tail)
    ))
}

/// `body` split before its text quote, and the quote block, if the tail
/// still matches what [`attach`] recorded.
fn split<'a, 'b>(body: &'a str, stored: &'b str) -> Option<(&'a str, &'b str)> {
    let rest = stored.strip_prefix("<!--komorebi-quote ")?;
    let (marker, block) = rest.split_once("-->")?;
    let (lines, hash) = marker.split_once(' ')?;
    let lines: usize = lines.parse().ok()?;
    let hash = u64::from_str_radix(hash, 16).ok()?;
    // The tail is the last `lines` lines: it starts after the `lines`-th
    // newline from the end, or at the start with one newline fewer.
    let newlines = lines.checked_sub(1)?;
    let start = match body.rmatch_indices('\n').nth(newlines) {
        Some((i, _)) => i.saturating_add(1),
        None if body.matches('\n').count() == newlines => 0,
        None => return None,
    };
    let tail = body.get(start..)?;
    (fnv(tail) == hash).then_some((body.get(..start)?, block))
}

/// FNV-1a: stable across builds, so a saved draft matches after updates.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

// ---- Markdown-lite to HTML ----------------------------------------------

/// Convert typed text to an HTML fragment: paragraphs on blank lines (line
/// breaks kept), lists, `> ` quote levels, the signature (from a `-- `
/// line) in `<div class="sig">`, inline markers and links. Everything
/// else is escaped.
pub fn to_html(text: &str) -> String {
    let mut b = Blocks::default();
    for line in text.lines() {
        let (depth, line) = quote_depth(line);
        b.set_depth(depth);
        if depth == 0 && line == "-- " {
            b.close_all();
            b.out.push_str(r#"<div class="sig">-- "#);
            b.sig = true;
        } else if line.trim().is_empty() {
            b.close_all();
        } else if b.sig {
            b.out.push_str("<br>");
            inline(line, &mut b.out, 0);
        } else if let Some((indent, ordered, item)) = list_item(line, b.lists.contains(&true)) {
            b.item(indent, ordered, item);
        } else if !b.lists.is_empty() && line.starts_with("  ") {
            // A wrapped list item goes on in it.
            b.out.push_str("<br>");
            inline(line.trim_start(), &mut b.out, 0);
        } else {
            b.close_lists();
            b.out.push_str(if b.para { "<br>" } else { "<p>" });
            b.para = true;
            inline(line, &mut b.out, 0);
        }
    }
    b.set_depth(0);
    b.close_all();
    b.out
}

#[derive(Default)]
struct Blocks {
    out: String,
    depth: usize,
    /// A `<p>` is open.
    para: bool,
    /// The signature `<div>` is open.
    sig: bool,
    /// Open lists (`true`: ordered), outermost first; at most two.
    lists: Vec<bool>,
}

impl Blocks {
    fn close_lists(&mut self) {
        while let Some(ordered) = self.lists.pop() {
            self.out
                .push_str(if ordered { "</li></ol>" } else { "</li></ul>" });
        }
    }

    fn close_all(&mut self) {
        if self.para {
            self.out.push_str("</p>");
            self.para = false;
        }
        if self.sig {
            self.out.push_str("</div>");
            self.sig = false;
        }
        self.close_lists();
    }

    fn set_depth(&mut self, depth: usize) {
        if depth == self.depth {
            return;
        }
        self.close_all();
        while self.depth < depth {
            self.out.push_str(BLOCKQUOTE);
            self.depth = self.depth.saturating_add(1);
        }
        while self.depth > depth {
            self.out.push_str("</blockquote>");
            self.depth = self.depth.saturating_sub(1);
        }
    }

    /// A list item at nesting `level` (0 or 1, by indent).
    fn item(&mut self, indent: usize, ordered: bool, text: &str) {
        if self.para {
            self.out.push_str("</p>");
            self.para = false;
        }
        let level = usize::from(indent >= 2 && !self.lists.is_empty());
        while self.lists.len() > level.saturating_add(1) {
            self.close_one();
        }
        if self.lists.len() == level.saturating_add(1) && self.lists.last() != Some(&ordered) {
            self.close_one();
        }
        if self.lists.len() == level.saturating_add(1) {
            self.out.push_str("</li>");
        } else {
            // A new list (nested inside the open item for level 1).
            self.out.push_str(if ordered { "<ol>" } else { "<ul>" });
            self.lists.push(ordered);
        }
        self.out.push_str("<li>");
        inline(text, &mut self.out, 0);
    }

    fn close_one(&mut self) {
        if let Some(ordered) = self.lists.pop() {
            self.out
                .push_str(if ordered { "</li></ol>" } else { "</li></ul>" });
        }
    }
}

/// The `> ` quote level of a line (capped) and the line without markers.
fn quote_depth(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    loop {
        let t = if depth == 0 {
            rest.trim_start_matches(' ')
        } else {
            rest
        };
        let Some(r) = t.strip_prefix('>') else { break };
        depth += 1;
        rest = r.strip_prefix(' ').unwrap_or(r);
        if !rest.trim_start_matches(' ').starts_with('>') {
            break;
        }
        rest = rest.trim_start_matches(' ');
    }
    (depth.min(MAX_QUOTE_DEPTH), rest)
}

/// A `- ` or `1. ` list item: its indent, whether it is ordered and its
/// text. An ordered list starts at `1. `; other numbers continue one
/// (`in_ordered`), so prose like `2026. A good year` stays text.
fn list_item(line: &str, in_ordered: bool) -> Option<(usize, bool, &str)> {
    let rest = line.trim_start_matches(' ');
    let indent = line.len().saturating_sub(rest.len());
    if indent > 4 {
        return None;
    }
    if let Some(item) = rest.strip_prefix("- ") {
        return (!item.trim().is_empty()).then_some((indent, false, item));
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let item = rest.get(digits..)?.strip_prefix(". ")?;
    (1..=9).contains(&digits).then_some(())?;
    (in_ordered || rest.starts_with("1. ")).then_some(())?;
    (!item.trim().is_empty()).then_some((indent, true, item))
}

/// Where [`inline_to`] writes: HTML, or text with the marks taken out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sink {
    Html,
    Plain,
}

/// Append `s` with its inline markers converted; true if it had any
/// (bold, italic or a `[label](url)` link; bare URLs don't count).
fn inline(s: &str, out: &mut String, depth: usize) -> bool {
    inline_to(s, out, depth, Sink::Html)
}

/// [`inline`] into `sink`: HTML tags and escapes, or for [`Sink::Plain`]
/// the same text without the marks (links as `label (url)`).
fn inline_to(s: &str, out: &mut String, depth: usize, sink: Sink) -> bool {
    let html = sink == Sink::Html;
    let b = s.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(b' ');
    let mut found = false;
    let mut lit = 0;
    let mut i = 0;
    // Markers with no closer from here on (each search fails only once).
    let (mut no_bold, mut no_star, mut no_under) = (false, false, false);
    let flush = |out: &mut String, from: usize, to: usize| {
        let text = s.get(from..to).unwrap_or("");
        if html {
            escape(text, out);
        } else {
            out.push_str(text);
        }
    };
    while i < b.len() {
        let c = at(i);
        let prev = if i == 0 { b' ' } else { at(i - 1) };
        let mut done: Option<usize> = None;
        if c == b'[' {
            if let Some((label, url, len)) = s.get(i..).and_then(md_link) {
                flush(out, lit, i);
                if html {
                    out.push_str("<a href=\"");
                    escape(&url, out);
                    out.push_str("\">");
                    escape(label, out);
                    out.push_str("</a>");
                } else {
                    out.push_str(label);
                    out.push_str(" (");
                    out.push_str(&url);
                    out.push(')');
                }
                found = true;
                done = Some(i.saturating_add(len));
            }
        } else if (c == b'h' || c == b'H') && !prev.is_ascii_alphanumeric() {
            if let Some(len) = s.get(i..).and_then(bare_url) {
                let end = i.saturating_add(len);
                let raw = s.get(i..end).unwrap_or("");
                if let Some(url) = links::sanitize(raw) {
                    flush(out, lit, i);
                    if html {
                        out.push_str("<a href=\"");
                        escape(&url, out);
                        out.push_str("\">");
                        escape(raw, out);
                        out.push_str("</a>");
                    } else {
                        out.push_str(raw);
                    }
                    done = Some(end);
                } else {
                    // Not a link: text, never scanned again.
                    i = end;
                    continue;
                }
            }
        } else if (c == b'*' || c == b'_')
            && depth < MAX_INLINE_DEPTH
            && !prev.is_ascii_alphanumeric()
        {
            let bold = c == b'*' && at(i + 1) == b'*';
            let width = if bold { 2 } else { 1 };
            let next = at(i + width);
            let skip = if bold {
                no_bold
            } else if c == b'*' {
                no_star
            } else {
                no_under
            };
            // The other marker may follow (`**_x_**`, `_**x**_`), not the same.
            let opens = !skip && !next.is_ascii_whitespace() && next != c;
            if opens && prev != c {
                let from = i.saturating_add(width);
                match closer(b, from, c, bold) {
                    Some(end) => {
                        flush(out, lit, i);
                        let tag = if bold { "b" } else { "i" };
                        let inner = s.get(from..end).unwrap_or("");
                        if html {
                            out.push('<');
                            out.push_str(tag);
                            out.push('>');
                        }
                        inline_to(inner, out, depth + 1, sink);
                        if html {
                            out.push_str("</");
                            out.push_str(tag);
                            out.push('>');
                        }
                        found = true;
                        done = Some(end.saturating_add(width));
                    }
                    None if bold => no_bold = true,
                    None if c == b'*' => no_star = true,
                    None => no_under = true,
                }
            }
        }
        match done {
            Some(next) => {
                i = next;
                lit = next;
            }
            None => i += 1,
        }
    }
    flush(out, lit, b.len());
    found
}

/// Where the closing marker of an emphasis opened before `from` starts: a
/// non-space before it and no letter or digit after it.
fn closer(b: &[u8], from: usize, c: u8, bold: bool) -> Option<usize> {
    let at = |i: usize| b.get(i).copied().unwrap_or(b' ');
    let width = if bold { 2 } else { 1 };
    let mut i = from.saturating_add(1);
    while i < b.len() {
        let here = at(i) == c && (!bold || at(i + 1) == c);
        let prev = at(i - 1);
        let after = at(i + width);
        if here
            && !prev.is_ascii_whitespace()
            && prev != c
            && after != c
            && !after.is_ascii_alphanumeric()
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// A `[label](url)` link at the start of `s` whose URL is followable
/// (http, https or mailto): its label, the link and the length of the
/// markup. The label holds no brackets, the URL no spaces.
fn md_link(s: &str) -> Option<(&str, String, usize)> {
    let rest = s.strip_prefix('[')?;
    let close = rest.find([']', '[', '\n'])?;
    let label = rest.get(..close)?;
    let after = rest.get(close..)?.strip_prefix("](")?;
    let end = after.find(|c: char| matches!(c, ')' | '[' | ']') || c.is_whitespace())?;
    let url = after.get(..end)?;
    if label.trim().is_empty() || !after.get(end..)?.starts_with(')') || url.starts_with('#') {
        return None;
    }
    let link = links::sanitize(url)?;
    // `[`, label, `](`, url, `)`.
    Some((label, link, label.len() + url.len() + 4))
}

/// The length of a bare `http://` or `https://` URL at the start of `s`,
/// without trailing punctuation (or an unmatched closing parenthesis).
fn bare_url(s: &str) -> Option<usize> {
    let head = s.as_bytes().get(..8).unwrap_or(s.as_bytes());
    let scheme = |p: &str| {
        head.get(..p.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(p.as_bytes()))
    };
    if !scheme("http://") && !scheme("https://") {
        return None;
    }
    let mut url = s
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .map_or(s, |end| s.get(..end).unwrap_or(s));
    // Parentheses in `url`, counted once.
    let open = url.matches('(').count();
    let mut close = url.matches(')').count();
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '*', '_']);
        let trimmed = match trimmed.strip_suffix(')') {
            Some(t) if open == 0 || close > open => {
                close -= 1;
                t
            }
            _ => trimmed,
        };
        if trimmed.len() == url.len() {
            break;
        }
        url = trimmed;
    }
    (url.len() > "https://".len()).then_some(url.len())
}

/// Append `s` HTML-escaped (`&`, `<`, `>` and `"`).
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

// ---- Quoting received HTML ----------------------------------------------

/// What the sanitizer does with an element.
enum Treat {
    Keep,
    /// Drop the tag, keep its content.
    Unwrap,
    /// Drop the element and its content.
    Drop,
}

fn treat(tag: &str) -> Treat {
    match tag {
        "a" | "abbr" | "b" | "bdi" | "bdo" | "big" | "blockquote" | "br" | "caption" | "center"
        | "cite" | "code" | "col" | "colgroup" | "dd" | "del" | "dfn" | "div" | "dl" | "dt"
        | "em" | "font" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "i" | "ins" | "kbd"
        | "li" | "mark" | "ol" | "p" | "pre" | "q" | "s" | "samp" | "small" | "span" | "strike"
        | "strong" | "sub" | "sup" | "table" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
        | "tt" | "u" | "ul" | "var" | "wbr" => Treat::Keep,
        "head" | "title" | "script" | "style" | "template" | "iframe" | "frame" | "frameset"
        | "object" | "embed" | "applet" | "svg" | "math" | "select" | "option" | "textarea"
        | "button" | "input" | "img" | "image" | "picture" | "video" | "audio" | "source"
        | "track" | "canvas" | "map" | "area" | "noscript" | "link" | "meta" | "base" => {
            Treat::Drop
        }
        _ => Treat::Unwrap,
    }
}

fn is_void(tag: &str) -> bool {
    matches!(tag, "br" | "hr" | "wbr" | "col")
}

/// Attributes kept as they are (besides a filtered `style` and a
/// followable `href` on links).
const ATTRS: &[&str] = &[
    "align",
    "valign",
    "dir",
    "lang",
    "width",
    "height",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "colspan",
    "rowspan",
    "color",
    "face",
    "size",
    "start",
    "type",
    "span",
    "nowrap",
];

/// The original HTML of a reply or forward, made safe to quote: nothing
/// that loads anything (images, cid images included, CSS `url()`,
/// `<style>`, `<link>`, frames, objects), no scripts, forms or event
/// handlers, no classes or ids that could meet the reader's styles. Links
/// keep only followable targets (http, https, mailto). `None` if nothing
/// readable is left, or the input or the result is too large.
pub fn sanitize(html: &str) -> Option<String> {
    if html.len() > MAX_HTML_BYTES {
        return None;
    }
    enum Step {
        Open(dom::Id),
        Close(dom::Id),
    }
    let doc = dom::parse(html);
    let mut out = String::new();
    let mut readable = false;
    let mut stack: Vec<Step> = doc
        .children(dom::ROOT)
        .iter()
        .rev()
        .map(|&c| Step::Open(c))
        .collect();
    while let Some(step) = stack.pop() {
        let id = match step {
            Step::Close(id) => {
                out.push_str("</");
                out.push_str(doc.tag(id));
                out.push('>');
                continue;
            }
            Step::Open(id) => id,
        };
        let Some(node) = doc.node(id) else { continue };
        let (tag, attrs) = match &node.kind {
            dom::Kind::Text(t) => {
                readable |= !t.trim().is_empty();
                escape(t, &mut out);
                continue;
            }
            dom::Kind::Elem { tag, attrs } => (&**tag, attrs),
        };
        let href = (tag == "a")
            .then(|| doc.attr(id, "href").and_then(links::sanitize))
            .flatten()
            .filter(|h| !h.starts_with('#'));
        let keep = match treat(tag) {
            Treat::Drop => continue,
            Treat::Keep => tag != "a" || href.is_some(),
            Treat::Unwrap => false,
        };
        if keep {
            out.push('<');
            out.push_str(tag);
            if let Some(href) = &href {
                out.push_str(" href=\"");
                escape(href, &mut out);
                out.push('"');
            }
            for (k, v) in attrs {
                let v = match &**k {
                    "style" => clean_style(v),
                    k if ATTRS.contains(&k) => v.to_string(),
                    _ => continue,
                };
                if !v.is_empty() {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    escape(&v, &mut out);
                    out.push('"');
                }
            }
            out.push('>');
            if is_void(tag) {
                continue;
            }
            stack.push(Step::Close(id));
        }
        stack.extend(doc.children(id).iter().rev().map(|&c| Step::Open(c)));
        if out.len() > MAX_QUOTE_BYTES {
            return None;
        }
    }
    (readable && out.len() <= MAX_QUOTE_BYTES).then_some(out)
}

/// A `style` attribute without declarations that could load anything or
/// escape the quote: `url()` and image functions, `expression`, `@import`,
/// behaviours and bindings, CSS escapes and comments that could hide
/// those, and positioning.
fn clean_style(style: &str) -> String {
    const BAD: &[&str] = &[
        "url(",
        "image(",
        "image-set(",
        "src(",
        "expression",
        "@import",
        "behavior",
        "binding",
        "position",
        "\\",
        "/*",
    ];
    style
        .split(';')
        .map(str::trim)
        .filter(|d| {
            let lower = d.to_ascii_lowercase();
            d.contains(':') && !BAD.iter().any(|b| lower.contains(b))
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn html(text: &str) -> String {
        to_html(text)
    }

    #[test]
    fn plain_text_needs_no_html() {
        for text in [
            "Hi Ana,\n\nSee you at 5.\n\nBob",
            "Prices: 5 * 3 * 2, a_b_c and snake_case_name",
            "Visit https://example.com/a_b_ today.",
            "\n\nOn Monday, Bob wrote:\n> **old** and - text\n> - item",
            "[not a link](javascript:alert(1)) and [x](#top)",
            "2 ** 8 is 256; *nix and _ alone",
            "2026. A good year",
            "Call me at\n15. Then lunch",
            "",
        ] {
            assert!(!has_formatting(text), "{text}");
            assert_eq!(alternative(text, None), (text.to_string(), None));
        }
    }

    #[test]
    fn markers_need_html() {
        for text in [
            "This is **important**.",
            "Really *soon*",
            "Really _soon_",
            "- one\n- two",
            "1. one",
            "See [the doc](https://example.com/doc).",
        ] {
            assert!(has_formatting(text), "{text}");
        }
    }

    #[test]
    fn converts_inline_markers_and_escapes() {
        assert_eq!(
            html("A **bold** and *it* and _it_ <b>&\"x\""),
            "<p>A <b>bold</b> and <i>it</i> and <i>it</i> &lt;b&gt;&amp;&quot;x&quot;</p>"
        );
        assert_eq!(
            html("**bold _and it_**"),
            "<p><b>bold <i>and it</i></b></p>"
        );
        assert_eq!(
            html("**_both_** _**both**_"),
            "<p><b><i>both</i></b> <i><b>both</b></i></p>"
        );
        assert_eq!(html("snake_case_name **"), "<p>snake_case_name **</p>");
        assert_eq!(
            html("[doc](https://x.org/a?b=1&c=2) or https://x.org/p."),
            "<p><a href=\"https://x.org/a?b=1&amp;c=2\">doc</a> or \
             <a href=\"https://x.org/p\">https://x.org/p</a>.</p>"
        );
        assert_eq!(
            html("[mail me](mailto:a@x.org) (https://x.org/q)"),
            "<p><a href=\"mailto:a@x.org\">mail me</a> (<a href=\"https://x.org/q\">https://x.org/q</a>)</p>"
        );
    }

    #[test]
    fn bare_urls_end_before_punctuation_and_stray_parentheses() {
        let link = |t: &str| bare_url(t).map(|n| t[..n].to_string());
        assert_eq!(link("https://x.org/a)."), Some("https://x.org/a".into()));
        assert_eq!(
            link("HTTP://x.org/a_(b)),"),
            Some("HTTP://x.org/a_(b)".into())
        );
        assert_eq!(link("https://"), None);
        assert_eq!(link("httpx://x.org"), None);
    }

    #[test]
    fn javascript_links_stay_text() {
        let out = html("[click](javascript:alert(1)) [d](data:text/html,x) javascript:x");
        assert!(!out.contains("<a"), "{out}");
        assert!(out.contains("[click](javascript:alert(1))"), "{out}");
        assert_eq!(plain("[click](javascript:x)"), "[click](javascript:x)");
        assert_eq!(
            plain("**Hi** see [the doc](https://x.org/d) [x](ftp://y)"),
            "Hi see the doc (https://x.org/d) [x](ftp://y)"
        );
    }

    #[test]
    fn text_plain_has_no_markers() {
        let body = "Hi Ana,\n\nThe **report** is _ready_ and *final*. Notes:\n\n\
                    - **totals** checked\n1. [the sheet](https://example.org/sheet)\n\n\
                    > _Can_ you send it today?\n\n-- \n**Ana**";
        let (text, html) = alternative(body, None);
        assert!(html.is_some(), "formatting needs the HTML part");
        assert_eq!(
            text,
            "Hi Ana,\n\nThe report is ready and final. Notes:\n\n\
             - totals checked\n1. the sheet (https://example.org/sheet)\n\n\
             > Can you send it today?\n\n-- \nAna"
        );
        assert!(!text.contains("**") && !text.contains("_r"), "{text}");
        // Only marks the HTML part converts go: a lone star, an
        // unclosed mark, snake_case and a product name keep theirs.
        assert_eq!(
            plain("2 * 3 = 6, snake_case_name, **open, a_b_c, C*"),
            "2 * 3 = 6, snake_case_name, **open, a_b_c, C*"
        );
        // Line breaks and a last line without one stay as typed.
        assert_eq!(plain("**a**\n\n_b_"), "a\n\nb");
        // Plain text with no marks goes out byte for byte.
        assert_eq!(alternative("2 * 3\n", None), ("2 * 3\n".to_string(), None));
    }

    #[test]
    fn paragraphs_lists_and_quotes() {
        assert_eq!(html("a\nb\n\n\nc"), "<p>a<br>b</p><p>c</p>");
        assert_eq!(
            html("- a\n  - b\n  - c\n- d\n1. e"),
            "<ul><li>a<ul><li>b</li><li>c</li></ul></li><li>d</li></ul><ol><li>e</li></ol>"
        );
        assert_eq!(html("- a\n  wrapped"), "<ul><li>a<br>wrapped</li></ul>");
        assert_eq!(html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
        assert_eq!(html("2026. A good year"), "<p>2026. A good year</p>");
        let q = BLOCKQUOTE;
        assert_eq!(
            html("me\n> one\n>> two\n> > two too\n> back"),
            format!(
                "<p>me</p>{q}<p>one</p>{q}<p>two<br>two too</p></blockquote><p>back</p></blockquote>"
            )
        );
        let deep = format!("{}x", ">".repeat(100));
        assert_eq!(html(&deep).matches("<blockquote").count(), MAX_QUOTE_DEPTH);
    }

    #[test]
    fn signature_gets_its_own_div() {
        assert_eq!(
            html("Hi\n\n-- \nAna *Silva*\nACME\n\nOn Monday, Bob wrote:"),
            "<p>Hi</p><div class=\"sig\">-- <br>Ana <i>Silva</i><br>ACME</div><p>On Monday, Bob wrote:</p>"
        );
    }

    #[test]
    fn quote_is_used_only_while_the_text_quote_is_untouched() {
        let body = "\n\n-- \nAna\n\nOn Monday, Bob wrote:\n> hello\n> there";
        let block = reply_quote("On Monday, Bob wrote:", "<p>hello there</p>");
        let stored = attach(body, "On Monday, Bob wrote:", &block).unwrap();
        assert!(attach(body, "On Tuesday, Bob wrote:", &block).is_none());

        let typed = format!("Thanks!{body}");
        let (text, html) = alternative(&typed, Some(&stored));
        assert_eq!(text, typed);
        let html = html.unwrap();
        assert!(html.contains("<p>Thanks!</p><div class=\"sig\">-- <br>Ana</div><div class=\"komorebi_quote\" style=\"margin-top:1em\">On Monday, Bob wrote:"), "{html}");
        assert!(
            html.contains("<p>hello there</p>") && !html.contains("&gt; hello"),
            "{html}"
        );

        // An answer typed into the quote, or a deleted attribution line:
        // the text goes as edited, quote markers as blockquotes.
        for edited in [
            typed.replace("> hello\n", "> hello\nYes!\n"),
            typed.replace("On Monday, Bob wrote:\n", ""),
            typed.replace("\n> there", ""),
        ] {
            let html = alternative(&edited, Some(&stored)).1.unwrap();
            assert!(!html.contains("komorebi_quote"), "{html}");
            assert!(html.contains("<blockquote"), "{html}");
        }
        // A garbled stored value is ignored (the text is converted).
        let garbled = alternative("plain", Some("<p>x</p>")).1.unwrap();
        assert!(
            garbled.contains("<p>plain</p>") && !garbled.contains("<p>x"),
            "{garbled}"
        );
        // The whole body is the quote.
        let only = "On Monday, Bob wrote:\n> hi";
        let stored = attach(only, "On Monday, Bob wrote:", &block).unwrap();
        assert!(
            alternative(only, Some(&stored))
                .1
                .unwrap()
                .contains("komorebi_quote")
        );
    }

    #[test]
    fn sanitizer_drops_remote_content_and_scripts() {
        let dirty = r##"<html><head><title>T</title><style>p{color:red}</style>
            <link rel=stylesheet href="https://x.org/a.css"></head>
            <body class="b" onload="evil()"><p id="p" style="color:#123; background:url(https://t.org/p.gif); font-weight:bold">Hi <b>there</b></p>
            <img src="https://t.org/pixel.gif"><img src="cid:logo"><script>alert(1)</script>
            <a href="javascript:alert(1)">bad</a> <a href="https://ok.org/" onclick="x()">good</a>
            <table background="https://t.org/bg.png" width="600" bgcolor="#fff"><tr><td style="background-image:url('x')">cell</td></tr></table>
            <form action="https://x.org"><input value="v">form text</form><iframe src="https://x.org"></iframe>
            <div style="po\sition:fixed">esc</div><p>&lt;tag&gt; &amp; "q"</p></body></html>"##;
        let out = sanitize(dirty).unwrap();
        for bad in [
            "url(",
            "pixel",
            "cid:",
            "script",
            "alert",
            "<img",
            "<link",
            "<style",
            "color:red",
            "onload",
            "onclick",
            "class=",
            "id=",
            "background=",
            "<form",
            "<input",
            "<iframe",
            "javascript",
            "fixed",
            "<title",
        ] {
            assert!(!out.contains(bad), "{bad} in {out}");
        }
        for good in [
            r#"<p style="color:#123;font-weight:bold">Hi <b>there</b></p>"#,
            r#"<a href="https://ok.org/">good</a>"#,
            r##"<table width="600" bgcolor="#fff"><tr><td>cell</td></tr></table>"##,
            "form text",
            "&lt;tag&gt; &amp; &quot;q&quot;",
            " bad ",
        ] {
            assert!(out.contains(good), "{good} not in {out}");
        }
        assert_eq!(sanitize("<img src='https://x'><p> </p>"), None);
        assert_eq!(sanitize(&"x".repeat(MAX_HTML_BYTES + 1)), None);
    }

    #[test]
    fn deep_nesting_is_not_recursive() {
        let deep = "<div><b>".repeat(20_000) + "x";
        let out = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || sanitize(&deep))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert!(out.contains('x'));
        assert_eq!(out.matches("<div>").count(), out.matches("</div>").count());
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Mutated Markdown-lite and HTML never panic, and a long hostile line
    /// stays linear.
    #[test]
    fn mutants_never_panic() {
        const PIECES: &[&str] = &[
            "**",
            "*",
            "_",
            "[",
            "]",
            "(",
            ")",
            "](",
            "https://",
            "http://x.org/",
            "mailto:a@b",
            "javascript:",
            "> ",
            ">",
            "- ",
            "1. ",
            "  ",
            "\n",
            "\n\n",
            "-- ",
            "<",
            ">",
            "&",
            "\"",
            "é",
            "😀",
            "a",
            "word ",
            "<p>",
            "</p>",
            "<a href='",
            "<!--",
            "-->",
            "<img src=",
            "style='",
            "url(",
            "\r\n",
            "\u{0}",
            "On x wrote:",
        ];
        let seed = 0x9e37_79b9_7f4a_7c15;
        let mut rng = Rng(seed);
        for iter in 0..3000 {
            let n = rng.below(60);
            let s: String = (0..n).map(|_| PIECES[rng.below(PIECES.len())]).collect();
            let _ = to_html(&s);
            let _ = plain(&s);
            let _ = has_formatting(&s);
            let _ = sanitize(&s);
            let stored = attach(&s, "On x wrote:", "<p>q</p>");
            let _ = alternative(&s, stored.as_deref());
            // Cut at every char boundary of a short mutant.
            if iter % 50 == 0 {
                for (i, _) in s.char_indices() {
                    let _ = alternative(&s[..i], stored.as_deref());
                }
            }
        }
        let t = std::time::Instant::now();
        for hostile in [
            "*a ".repeat(100_000),
            "**a ".repeat(100_000),
            "_a ".repeat(100_000),
            "[a".repeat(100_000),
            "[a](".repeat(100_000),
            "https://x".repeat(50_000),
            "hé".repeat(60_000),
            format!("http://x{}", ")".repeat(100_000)),
            format!("http://x({}", ")".repeat(100_000)),
        ] {
            let _ = to_html(&hostile);
            let _ = plain(&hostile);
        }
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
    }
}
