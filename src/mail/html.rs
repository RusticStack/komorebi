//! HTML → readable text for the native reader.
//!
//! This converter does not render HTML. It keeps the structure a reader needs
//! (paragraphs, line breaks, lists, quotes and link targets) and drops
//! everything else. Scripts, styles and remote resources never run or load,
//! because the output is plain text.
//!
//! It is a single forward scan, linear in the input size. Named character
//! references use mail-parser's complete HTML5 entity table (a compile-time
//! hash map that is already in the binary); numeric references follow the
//! WHATWG rules.

use std::borrow::Cow;

/// Output cap, the same as the reader's display cap. Text past it is
/// dropped, so a hostile message can't blow up the output (deep quotes
/// repeat their prefix on every line).
const MAX_OUTPUT: usize = 300_000;

/// Elements whose contents are dropped entirely.
fn is_skipped(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "head" | "title" | "noscript" | "template" | "svg"
    )
}

/// Elements that start and end a block (separated by a blank line).
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "table"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "center"
            | "address"
            | "dl"
            | "dt"
            | "dd"
    )
}

pub fn html_to_text(html: &str) -> String {
    // ASCII lowercasing keeps byte offsets, so `lower` indexes like `html`.
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = Writer::default();
    let mut quote = 0usize;
    let mut pre = 0usize;
    let mut link: Option<(String, usize)> = None;
    // One entry per open list: None for <ul>, Some(next number) for <ol>.
    let mut lists: Vec<Option<u32>> = Vec::new();
    let mut quotes = QuoteMemo::default();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' && is_tag_start(bytes.get(i + 1).copied()) {
            if lower[i..].starts_with("<!--") {
                i = lower[i + 4..]
                    .find("-->")
                    .map_or(bytes.len(), |e| i + 4 + e + 3);
                continue;
            }
            let Some(end) = tag_end(bytes, i + 1, &mut quotes) else {
                // Unterminated tag: show the rest as text and stop.
                out.words(&decode_entities(&html[i..]), quote);
                break;
            };
            let tag = Tag::parse(&html[i + 1..end]);
            i = end + 1;
            let Some(tag) = tag else { continue };

            if !tag.closing && !tag.self_closing && is_skipped(&tag.name) {
                i = skip_element(&lower, i, &tag.name);
                continue;
            }
            match (tag.name.as_str(), tag.closing) {
                ("br", _) => out.newline(quote),
                ("ul" | "ol", false) => {
                    if lists.is_empty() {
                        out.block();
                    } else {
                        out.line(quote);
                    }
                    let start = tag.start.unwrap_or(1);
                    lists.push((tag.name == "ol").then_some(start));
                }
                ("ul" | "ol", true) => {
                    lists.pop();
                    if lists.is_empty() {
                        out.block();
                    } else {
                        out.line(quote);
                    }
                }
                ("li", false) => {
                    out.line(quote);
                    // Deep nesting stays readable; the real depth still counts.
                    out.text(&"  ".repeat(lists.len().saturating_sub(1).min(6)), quote);
                    match lists.last_mut() {
                        Some(Some(n)) => {
                            out.text(&format!("{n}. "), quote);
                            *n = n.saturating_add(1);
                        }
                        _ => out.text("• ", quote),
                    }
                }
                ("li", true) => out.line(quote),
                ("blockquote", false) => {
                    out.block();
                    quote += 1;
                }
                ("blockquote", true) => {
                    quote = quote.saturating_sub(1);
                    out.block();
                }
                ("pre", false) => {
                    out.block();
                    pre += 1;
                }
                ("pre", true) => {
                    pre = pre.saturating_sub(1);
                    out.block();
                }
                ("td" | "th", true) => out.text("  ", quote),
                ("a", false) => link = tag.href.map(|h| (h, out.buf.len())),
                ("a", true) => {
                    if let Some((href, start)) = link.take() {
                        // The buffer may have been trimmed below `start` since.
                        let label = out.buf.get(start..).unwrap_or("").trim();
                        let shown = href.trim_start_matches("mailto:").trim_end_matches('/');
                        if is_followable(&href)
                            && !label.is_empty()
                            && !label.trim_end_matches('/').ends_with(shown)
                        {
                            out.text(&format!(" <{href}>"), quote);
                        }
                    }
                }
                ("img", _) => {
                    if let Some(alt) = tag.alt.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                        out.text(&format!("[{alt}]"), quote);
                    }
                }
                (name, _) if is_block(name) => out.block(),
                _ => {}
            }
            continue;
        }

        // Text up to the next possible tag (a stray '<' is plain text).
        // Byte search: `i + 1` can fall inside a multi-byte character, but
        // '<' is ASCII, so the index found is always a character boundary.
        let next = bytes[i + 1..]
            .iter()
            .position(|&b| b == b'<')
            .map_or(bytes.len(), |e| i + 1 + e);
        let decoded = decode_entities(&html[i..next]);
        i = next;
        if pre > 0 {
            for (n, line) in decoded.split('\n').enumerate() {
                if n > 0 {
                    out.newline(quote);
                }
                out.text(line, quote);
            }
        } else {
            out.words(&decoded, quote);
        }
    }
    out.finish()
}

/// A '<' starts a tag only when followed by a letter, '/', '!' or '?'
/// (WHATWG tag open state); otherwise it is text, as in "a < b".
fn is_tag_start(next: Option<u8>) -> bool {
    next.is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?'))
}

/// Where each quote character was last found missing: once no " exists
/// after position p, there is none after any later position either. This
/// keeps tags with unclosed quotes from rescanning the document each time.
struct QuoteMemo {
    none_after: [usize; 2],
}

impl Default for QuoteMemo {
    fn default() -> Self {
        Self {
            none_after: [usize::MAX; 2],
        }
    }
}

/// Index of the '>' closing the tag that starts at `from`. A '>' inside a
/// quoted attribute value does not count. A quote only opens right after
/// '=', so `<a title=it's>` still ends at its '>'. An unclosed quote falls
/// back to the first '>'.
fn tag_end(bytes: &[u8], from: usize, memo: &mut QuoteMemo) -> Option<usize> {
    let first_gt = from + bytes[from..].iter().position(|&b| b == b'>')?;
    let mut prev = 0u8;
    let mut j = from;
    while j < bytes.len() {
        match bytes[j] {
            b'>' => return Some(j),
            q @ (b'"' | b'\'') if prev == b'=' => {
                let slot = usize::from(q == b'\'');
                let close = if j >= memo.none_after[slot] {
                    None
                } else {
                    bytes[j + 1..].iter().position(|&b| b == q)
                };
                match close {
                    Some(close) => j += close + 1,
                    None => {
                        memo.none_after[slot] = memo.none_after[slot].min(j);
                        return Some(first_gt);
                    }
                }
            }
            _ => {}
        }
        if !bytes[j].is_ascii_whitespace() {
            prev = bytes[j];
        }
        j += 1;
    }
    Some(first_gt)
}

/// Skip the contents of a dropped element and return the index after its end
/// tag. `<head>` and `<title>` also end at `<body`, so an unclosed head can't
/// hide the whole message.
fn skip_element(lower: &str, from: usize, name: &str) -> usize {
    let rest = &lower[from..];
    let close = rest.find(&format!("</{name}"));
    let body = matches!(name, "head" | "title")
        .then(|| rest.find("<body"))
        .flatten();
    match (close, body) {
        (Some(c), Some(b)) if b < c => from + b,
        (Some(c), _) => from + c + rest[c..].find('>').map_or(rest.len() - c, |e| e + 1),
        (None, Some(b)) => from + b,
        (None, None) => lower.len(),
    }
}

fn is_followable(href: &str) -> bool {
    let h = href.trim().to_ascii_lowercase();
    h.starts_with("http://") || h.starts_with("https://") || h.starts_with("mailto:")
}

#[derive(Default)]
struct Writer {
    buf: String,
    /// Whether the last thing written was whitespace (or the line start).
    space: bool,
    at_line_start: bool,
    /// Output reached `MAX_OUTPUT`; further writes are dropped.
    truncated: bool,
}

impl Writer {
    fn full(&mut self) -> bool {
        self.truncated |= self.buf.len() >= MAX_OUTPUT;
        self.truncated
    }

    fn ensure_prefix(&mut self, quote: usize) {
        if self.at_line_start || self.buf.is_empty() {
            // Only the printed depth is clamped; the caller keeps the count.
            for _ in 0..quote.min(6) {
                self.buf.push_str("> ");
            }
            self.at_line_start = false;
        }
    }

    fn text(&mut self, s: &str, quote: usize) {
        if s.is_empty() || self.full() {
            return;
        }
        self.ensure_prefix(quote);
        self.buf.push_str(s);
        self.space = s.ends_with(' ');
    }

    fn words(&mut self, s: &str, quote: usize) {
        let leading = s.starts_with(char::is_whitespace);
        let trailing = s.ends_with(char::is_whitespace);
        let mut first = true;
        for word in s.split_whitespace() {
            if self.full() {
                return;
            }
            if (!first || leading) && !self.space && !self.at_line_start && !self.buf.is_empty() {
                self.buf.push(' ');
            }
            self.ensure_prefix(quote);
            self.buf.push_str(word);
            self.space = false;
            first = false;
        }
        if trailing && !first && !self.space {
            self.buf.push(' ');
            self.space = true;
        } else if first && (leading || trailing) && !self.space && !self.at_line_start {
            // whitespace-only chunk between inline elements
            if !self.buf.is_empty() {
                self.buf.push(' ');
                self.space = true;
            }
        }
    }

    fn newline(&mut self, quote: usize) {
        if self.full() {
            return;
        }
        self.trim_trailing_space();
        if self.at_line_start {
            // keep quote markers on blank lines inside quotes
            self.ensure_prefix(quote);
            self.trim_trailing_space();
        }
        self.buf.push('\n');
        self.at_line_start = true;
        self.space = true;
    }

    /// Start a new line unless already at the start of one.
    fn line(&mut self, quote: usize) {
        if !self.buf.is_empty() && !self.at_line_start {
            self.newline(quote);
        }
    }

    /// End the current block: at most one blank line between blocks.
    fn block(&mut self) {
        self.trim_trailing_space();
        if self.buf.is_empty() || self.full() {
            return;
        }
        let trailing_newlines = self.buf.len() - self.buf.trim_end_matches('\n').len();
        for _ in trailing_newlines..2 {
            self.buf.push('\n');
        }
        self.at_line_start = true;
        self.space = true;
    }

    fn trim_trailing_space(&mut self) {
        while self.buf.ends_with(' ') {
            self.buf.pop();
        }
    }

    fn finish(self) -> String {
        let mut out = String::with_capacity(self.buf.len());
        let mut blank = 0;
        for line in self.buf.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.chars().all(|c| c == '>' || c == ' ') {
                blank += 1;
                if blank > 1 {
                    continue;
                }
            } else {
                blank = 0;
            }
            out.push_str(line);
            out.push('\n');
        }
        let mut out = out.trim().to_string();
        // A single long run (one huge <pre> line) can pass the cap unflagged.
        if self.truncated || out.len() > MAX_OUTPUT {
            out.truncate(out.floor_char_boundary(MAX_OUTPUT));
            out.push_str("\n[Message shortened]");
        }
        out
    }
}

struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
    href: Option<String>,
    alt: Option<String>,
    /// `<ol start=N>`.
    start: Option<u32>,
}

impl Tag {
    fn parse(inner: &str) -> Option<Self> {
        let inner = inner.trim();
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(r) => (true, r),
            None => (false, inner),
        };
        if inner.starts_with('!') || inner.starts_with('?') {
            return None;
        }
        let self_closing = inner.ends_with('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        if name.is_empty() {
            return None;
        }
        let attrs = &inner[name_end..];
        Some(Self {
            href: (name == "a").then(|| attr(attrs, "href")).flatten(),
            alt: (name == "img").then(|| attr(attrs, "alt")).flatten(),
            start: (name == "ol")
                .then(|| attr(attrs, "start").and_then(|s| s.parse().ok()))
                .flatten(),
            name,
            closing,
            self_closing,
        })
    }
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let at = from + pos;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = attrs[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let value = match rest.chars().next()? {
            q @ ('"' | '\'') => rest[1..].split(q).next()?,
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next()?,
        };
        return Some(decode_entities(value).trim().to_string());
    }
    None
}

/// Decode character references. Unknown or malformed ones stay literal.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // The longest HTML5 entity name is 31 characters plus '&' and ';'.
        let end = rest[1..]
            .find(|c: char| c == ';' || c == '&' || c.is_whitespace())
            .map(|e| e + 1)
            .filter(|&e| e <= 34 && rest.as_bytes()[e] == b';');
        if let Some(end) = end
            && let Some(decoded) = reference(&rest[..=end])
        {
            out.extend(decoded);
            rest = &rest[end + 1..];
            continue;
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decode one complete reference like `&amp;` or `&#8211;`. `None` if it is
/// not a reference; `Some(None)` if it decodes to an invisible character
/// that is dropped (soft hyphens and zero-width spacing used as padding).
fn reference(token: &str) -> Option<Option<char>> {
    let name = &token[1..token.len() - 1];
    let c = match name.strip_prefix('#') {
        Some(num) => numeric(num)?,
        None => {
            let mut buf = String::new();
            mail_parser::decoders::html::add_html_token(&mut buf, token.as_bytes(), false);
            // Unknown names come back unchanged.
            if buf == token {
                return None;
            }
            buf.chars().next()?
        }
    };
    Some(
        (!matches!(
            c,
            '\u{AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'
        ))
        .then_some(c),
    )
}

/// A numeric character reference body (after `#`), per the WHATWG rules:
/// NUL, surrogates and out-of-range values become U+FFFD, and the C1 range
/// is read as Windows-1252, as browsers do.
fn numeric(num: &str) -> Option<char> {
    let (digits, radix) = match num.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (num, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = u32::from_str_radix(digits, radix).unwrap_or(u32::MAX);
    Some(match n {
        0 | 0xD800..=0xDFFF | 0x11_0000.. => '\u{FFFD}',
        0x80..=0x9F => CP1252_C1[(n - 0x80) as usize],
        n => char::from_u32(n).unwrap_or('\u{FFFD}'),
    })
}

/// Windows-1252 meanings of 0x80..=0x9F (WHATWG numeric reference table);
/// bytes without a meaning keep their C1 code point.
const CP1252_C1: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '\u{2013}', '\u{2014}', '˜', '™', 'š', '›', 'œ', '\u{9D}',
    'ž', 'Ÿ',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_and_breaks() {
        assert_eq!(
            html_to_text("<p>Hello <b>world</b>!</p><p>Line one<br>Line two</p>"),
            "Hello world!\n\nLine one\nLine two"
        );
    }

    #[test]
    fn drops_scripts_styles_and_comments() {
        assert_eq!(
            html_to_text(
                "<head><title>T</title><style>p{}</style></head><!-- c --><script>if (a<b) x()</script>Body"
            ),
            "Body"
        );
    }

    #[test]
    fn unclosed_head_ends_at_body() {
        assert_eq!(html_to_text("<html><head><title>T<body>Hi"), "Hi");
    }

    #[test]
    fn lists() {
        assert_eq!(
            html_to_text("<ul><li>One</li><li>Two</li></ul>"),
            "• One\n• Two"
        );
        assert_eq!(
            html_to_text("<p>Steps</p><ol><li>a</li><li>b</li></ol><p>Done</p>"),
            "Steps\n\n1. a\n2. b\n\nDone"
        );
        assert_eq!(
            html_to_text("<ul><li>A<ul><li>A1</li></ul></li><li>B</li></ul>"),
            "• A\n  • A1\n• B"
        );
        assert_eq!(html_to_text("<ol start=\"3\"><li>c</li></ol>"), "3. c");
    }

    #[test]
    fn quotes_and_links() {
        assert_eq!(
            html_to_text("<blockquote><p>Quoted</p></blockquote><p>Reply</p>"),
            "> Quoted\n\nReply"
        );
        assert_eq!(
            html_to_text(r#"<a href="https://example.com/x">Click</a>"#),
            "Click <https://example.com/x>"
        );
        // Link text that already shows the URL is not repeated.
        assert_eq!(
            html_to_text(r#"<a href="https://example.com/">https://example.com</a>"#),
            "https://example.com"
        );
        // Script URLs are never shown as targets.
        assert_eq!(html_to_text(r#"<a href="javascript:x()">Go</a>"#), "Go");
    }

    #[test]
    fn stray_angle_brackets_and_quoted_attributes() {
        assert_eq!(html_to_text("a < b and c > d"), "a < b and c > d");
        assert_eq!(
            html_to_text(r#"<a title="a>b" href="https://x.org/">t</a>"#),
            "t <https://x.org/>"
        );
        assert_eq!(html_to_text("<a title=it's>x</a> y"), "x y");
        assert_eq!(html_to_text("<p title=\"open>text"), "text");
        assert_eq!(html_to_text("tail <b"), "tail <b");
    }

    #[test]
    fn text_starting_with_multibyte_chars() {
        // Regression: a real newsletter crashed the converter when a text run
        // began with a multi-byte character right after a tag.
        assert_eq!(html_to_text("<b>›</b> ok"), "› ok");
        assert_eq!(
            html_to_text("<p>📦 envio</p><p>ação ›› fim</p>"),
            "📦 envio

ação ›› fim"
        );
        for s in [
            "›",
            "<",
            "<›",
            "›<",
            "a<›>b",
            "<a href=\"›\">›</a>",
            "&›;",
            "<!--›",
            "<p ›",
        ] {
            html_to_text(s);
        }
    }

    #[test]
    fn hostile_input_is_linear() {
        let start = std::time::Instant::now();
        html_to_text(&"<a".repeat(200_000));
        html_to_text(&"<".repeat(1_000_000));
        html_to_text(&"<a x=\">".repeat(100_000));
        assert!(start.elapsed().as_secs() < 2);
    }

    #[test]
    fn link_label_after_trim_does_not_panic() {
        // `</td>` and the block start trim the buffer below the link start.
        html_to_text("</td><a href=e><ol></a>");
    }

    #[test]
    fn deep_nesting_is_clamped_and_capped() {
        let html = "<blockquote>".repeat(20_000) + &"x<br>".repeat(30_000);
        let out = html_to_text(&html);
        assert!(out.len() <= MAX_OUTPUT + 64, "{}", out.len());
        assert!(out.starts_with("> > > > > > x\n"), "{}", &out[..40]);
        assert!(out.ends_with("[Message shortened]"));
        assert!(html_to_text(&"x ".repeat(200_000)).len() <= MAX_OUTPUT + 64);
        let pre = format!("<pre>{}", "é".repeat(200_000));
        assert!(html_to_text(&pre).len() <= MAX_OUTPUT + 64);
        let html = "<ul><li>".repeat(50) + "deep";
        assert!(html_to_text(&html).ends_with(&format!("{}• deep", "  ".repeat(6))));
        // Closing tags still pop the real depth.
        let html = "<blockquote>".repeat(10) + "a" + &"</blockquote>".repeat(10) + "b";
        assert_eq!(html_to_text(&html), "> > > > > > a\n\nb");
    }

    /// Seeded token fuzz: hostile tag soup must never panic or overrun the cap.
    #[test]
    fn token_fuzz() {
        const TOKENS: &[&str] = &[
            "<p>",
            "</p>",
            "<br>",
            "<li>",
            "</li>",
            "<ul>",
            "</ul>",
            "<ol>",
            "</ol>",
            "<ol start=4294967295>",
            "<ol start=99999999999>",
            "<td>",
            "</td>",
            "<th>",
            "</tr>",
            "<a href=",
            "<a href=\"https://x.org/\">",
            "</a>",
            "<blockquote>",
            "</blockquote>",
            "<pre>",
            "</pre>",
            "<img alt=\"›\">",
            "<!--",
            "-->",
            "<script>",
            "</script>",
            "<head>",
            "<body>",
            "&#",
            "&#x",
            "&#150;",
            "&amp;",
            "&",
            ";",
            "<",
            ">",
            "\"",
            "'",
            "=",
            " ",
            "\n",
            "x",
            "word ",
            "›",
            "📦",
            "ação",
            "é",
            "\u{200B}",
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5_000 {
            let len = next() % 200;
            let doc: String = (0..len)
                .map(|_| TOKENS[(next() % TOKENS.len() as u64) as usize])
                .collect();
            let out = html_to_text(&doc);
            assert!(out.len() <= MAX_OUTPUT + 64, "{doc:?}");
        }
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &bogus; &"),
            "a & b <c> AB &bogus; &"
        );
        assert_eq!(
            decode_entities("&rarr; &hearts; &CounterClockwiseContourIntegral;"),
            "→ ♥ ∳"
        );
        assert_eq!(
            decode_entities("&#150; &#+65; &#0; &#x110000;"),
            "\u{2013} &#+65; \u{FFFD} \u{FFFD}"
        );
        assert_eq!(decode_entities("a&shy;b&zwnj;c"), "abc");
        assert_eq!(decode_entities("AT&T and &amp; more;"), "AT&T and & more;");
        assert_eq!(html_to_text("Fish&nbsp;&amp;&nbsp;chips"), "Fish & chips");
        assert!(matches!(decode_entities("plain"), Cow::Borrowed(_)));
    }

    #[test]
    fn inline_whitespace() {
        assert_eq!(html_to_text("<span>a</span> <span>b</span>"), "a b");
        assert_eq!(html_to_text("<div>\n   text\n</div>"), "text");
        assert_eq!(html_to_text("<pre>a\n  b</pre>"), "a\n  b");
    }
}
