//! Defensive HTML sanitizer + remote-content consent gate.
//!
//! Pure-Rust, dependency-free allowlist sanitizer for rendering untrusted email
//! HTML bodies in the GPUI reader. Not a full HTML parser: a byte-scanning
//! tag/attribute stripper that *preserves* safe HTML structure (unlike
//! [`super::mime::strip_html`], which collapses to plain text).
//!
//! # Security posture
//!
//! - **Allowlist of safe tags**: unknown tags are dropped (tag removed, text
//!   contents kept). Dangerous tags are dropped *with their contents*.
//! - **Dangerous tags dropped with contents**: `script`, `style`, `iframe`,
//!   `object`, `embed`, `form`, `input`, `button`, `meta`, `link`, `base`,
//!   `applet`, `frame`, `frameset`, `noscript`, `title`, `head`, `svg`.
//! - **Attribute allowlist**: only safe attributes per tag are kept; all `on*`
//!   event handlers and `style` are dropped.
//! - **URL scheme gating**: `javascript:`, `vbscript:`, `data:` URLs are
//!   stripped. `http(s)://` URLs are remote content (recorded; kept or blocked
//!   per consent).
//! - **Anchor hardening**: kept links get `rel="noopener noreferrer"
//!   target="_blank"`.
//! - **Entity decoding** happens before URL-scheme checks (so
//!   `javascript&#58;` is caught) and decoded `<`/`>` in text are re-encoded.

/// Safe, sanitized HTML plus remote-content metadata for the consent gate.
#[derive(Debug, Clone)]
pub struct SanitizedHtml {
    /// The sanitized, safe-to-render HTML string.
    pub html: String,
    /// `true` if the original referenced any remote (`http://`/`https://`)
    /// resource via `src` or `href`.
    pub has_remote_content: bool,
    /// Remote URLs found in the original (blocked or allowed).
    pub blocked_resources: Vec<String>,
}

/// Tags removed *with their contents* (script execution / CSS / frame surface).
const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "input", "button", "meta", "link",
    "base", "applet", "frame", "frameset", "noscript", "title", "head", "svg",
];

/// Tags safe to keep. Unknown tags are dropped but their text contents survive.
const ALLOWED_TAGS: &[&str] = &[
    "p",
    "br",
    "div",
    "span",
    "b",
    "strong",
    "i",
    "em",
    "u",
    "a",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
    "img",
];

/// Void (self-closing) allowed tags.
const VOID_TAGS: &[&str] = &["br", "hr", "img"];

/// Sanitize untrusted email HTML.
///
/// `allow_remote` is the consent flag: when `false` (privacy-first default),
/// remote `http(s)://` URLs are stripped from `src`/`href` (and `<img>` with a
/// remote-only `src` is dropped). When `true`, remote URLs pass through but are
/// still recorded in `blocked_resources` and `has_remote_content` is set.
pub fn sanitize(html: &str, allow_remote: bool) -> SanitizedHtml {
    let mut out = String::with_capacity(html.len());
    let mut blocked: Vec<String> = Vec::new();
    let mut has_remote = false;

    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];

        if c == b'<' {
            // Comments, CDATA, declarations: drop entirely.
            if html[i..].starts_with("<!--") {
                i = find_after(html, i + 4, "-->").unwrap_or(bytes.len());
                continue;
            }
            if html[i..].starts_with("<![CDATA[") {
                i = find_after(html, i + 9, "]]>").unwrap_or(bytes.len());
                continue;
            }
            if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
                i = html[i..]
                    .find('>')
                    .map(|e| i + e + 1)
                    .unwrap_or(bytes.len());
                continue;
            }

            // A real tag. Find its end (next '>' respecting quotes).
            let tag_end = match find_tag_end(&html[i..]) {
                Some(e) => i + e,
                None => break, // no closing '>': stop
            };
            let tag_slice = &html[i..tag_end];
            let name = tag_name(tag_slice);
            i = tag_end;

            let Some(name) = name else {
                out.push('&');
                out.push_str("lt;");
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let is_close = tag_slice.starts_with("</");

            if DROP_WITH_CONTENT.contains(&lower.as_str()) {
                // Record any remote URLs in the dropped tag's attributes first.
                record_remote_attrs(tag_slice, &mut blocked, &mut has_remote);
                // These dangerous tags are void-like (no close tag): drop just
                // the tag, not following content.
                if is_close
                    || tag_slice.ends_with("/>")
                    || matches!(lower.as_str(), "meta" | "link" | "base" | "input" | "embed")
                    || is_void_name(&lower)
                {
                    continue; // self-closing/void/close: nothing more to skip
                }
                // Skip contents up to the matching close tag.
                i = skip_until_close(html, i, &lower);
                continue;
            }

            if is_close {
                if ALLOWED_TAGS.contains(&lower.as_str()) {
                    out.push_str("</");
                    out.push_str(&lower);
                    out.push('>');
                }
                continue;
            }

            if ALLOWED_TAGS.contains(&lower.as_str()) {
                let void = is_void_name(&lower) || tag_slice.ends_with("/>");
                emit_allowed_tag(
                    &lower,
                    tag_slice,
                    void,
                    allow_remote,
                    &mut out,
                    &mut blocked,
                    &mut has_remote,
                );
                continue;
            }

            // Unknown non-dangerous tag: drop the tag, keep contents.
        } else if c == b'&' {
            let (decoded, len) = decode_entity(&html[i..]);
            // Re-encode `<` and `>` so decoded entities can't form tags.
            for ch in decoded.chars() {
                match ch {
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    _ => out.push(ch),
                }
            }
            i += len;
            continue;
        } else {
            // ASCII byte: push directly; non-ASCII (multibyte) is part of a UTF-8
            // sequence - copy the whole char from the source to avoid corruption.
            if c < 0x80 {
                out.push(c as char);
                i += 1;
            } else {
                // Walk one UTF-8 char from the source string.
                let rest = &html[i..];
                let ch = rest.chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
            continue;
        }
    }

    SanitizedHtml {
        html: out,
        has_remote_content: has_remote,
        blocked_resources: blocked,
    }
}

/// Emit a sanitized allowed tag. For `<img>` with a blocked remote `src`, the
/// whole tag is dropped (returns without emitting).
fn emit_allowed_tag(
    name: &str,
    tag: &str,
    void: bool,
    allow_remote: bool,
    out: &mut String,
    blocked: &mut Vec<String>,
    has_remote: &mut bool,
) {
    if name == "a" {
        emit_anchor(tag, allow_remote, out, blocked, has_remote);
        return;
    }
    if name == "img" {
        emit_img(tag, allow_remote, out, blocked, has_remote);
        return;
    }

    let mut attrs = String::new();
    for (k, v) in parse_attrs(tag) {
        let key = k.to_ascii_lowercase();
        if key.starts_with("on") || key == "style" {
            continue;
        }
        if is_safe_attr(name, &key) {
            push_attr(&mut attrs, &key, &v);
        }
    }
    push_tag(out, name, &attrs, void);
}

/// Emit a sanitized `<a>`. Always emitted (even if href stripped - text stays
/// readable). Navigational hrefs get `rel="noopener noreferrer" target="_blank"`.
fn emit_anchor(
    tag: &str,
    allow_remote: bool,
    out: &mut String,
    blocked: &mut Vec<String>,
    has_remote: &mut bool,
) {
    let mut attrs = String::new();
    let mut navigational = false;
    for (k, v) in parse_attrs(tag) {
        let key = k.to_ascii_lowercase();
        if key.starts_with("on") || key == "style" {
            continue;
        }
        if key == "href" {
            let decoded = decode_entities_in_string(&v);
            let scheme = url_scheme(&decoded);
            if scheme == "javascript" || scheme == "vbscript" || scheme == "data" {
                continue; // drop dangerous scheme
            }
            if scheme == "http" || scheme == "https" {
                *has_remote = true;
                if !blocked.contains(&decoded) {
                    blocked.push(decoded.clone());
                }
                if !allow_remote {
                    continue; // blocked: omit href
                }
                push_attr(&mut attrs, "href", &decoded);
                navigational = true;
            } else {
                // mailto, tel, cid, relative: safe, keep as-is.
                push_attr(&mut attrs, "href", &decoded);
                navigational = !matches!(scheme.as_str(), "mailto" | "tel" | "cid" | "");
            }
            continue;
        }
        if key == "alt" || key == "title" || key == "name" {
            push_attr(&mut attrs, &key, &v);
        }
    }
    if navigational {
        push_attr(&mut attrs, "rel", "noopener noreferrer");
        push_attr(&mut attrs, "target", "_blank");
    }
    push_tag(out, "a", &attrs, false);
}

/// Emit a sanitized `<img>`. Dropped entirely if `src` is remote and not
/// allowed, or if `src` is a dangerous scheme.
fn emit_img(
    tag: &str,
    allow_remote: bool,
    out: &mut String,
    blocked: &mut Vec<String>,
    has_remote: &mut bool,
) {
    let mut attrs = String::new();
    let mut src_kept = false;
    for (k, v) in parse_attrs(tag) {
        let key = k.to_ascii_lowercase();
        if key.starts_with("on") || key == "style" {
            continue;
        }
        if key == "src" {
            let decoded = decode_entities_in_string(&v);
            let scheme = url_scheme(&decoded);
            if scheme == "javascript" || scheme == "vbscript" || scheme == "data" {
                continue; // drop dangerous scheme
            }
            if scheme == "http" || scheme == "https" {
                *has_remote = true;
                if !blocked.contains(&decoded) {
                    blocked.push(decoded.clone());
                }
                if !allow_remote {
                    continue; // blocked
                }
                push_attr(&mut attrs, "src", &decoded);
                src_kept = true;
            } else {
                // cid (inline attachment), relative, etc.: keep.
                push_attr(&mut attrs, "src", &decoded);
                src_kept = true;
            }
            continue;
        }
        if key == "alt" || key == "title" {
            push_attr(&mut attrs, &key, &v);
        }
    }
    if src_kept {
        push_tag(out, "img", &attrs, true);
    }
}

/// Record remote URLs in a dropped dangerous tag's attributes (so the consent
/// gate still fires even when the tag is removed with its contents).
fn record_remote_attrs(tag: &str, blocked: &mut Vec<String>, has_remote: &mut bool) {
    for (k, v) in parse_attrs(tag) {
        let key = k.to_ascii_lowercase();
        if key == "src" || key == "href" || key == "data" {
            let decoded = decode_entities_in_string(&v);
            let scheme = url_scheme(&decoded);
            if scheme == "http" || scheme == "https" {
                *has_remote = true;
                if !blocked.contains(&decoded) {
                    blocked.push(decoded);
                }
            }
        }
    }
}

/// Scan all `src="..."` / `href="..."` URLs and return deduplicated remote ones.
pub fn remote_content_urls(html: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for (key, val) in parse_all_attrs(html) {
        if key == "src" || key == "href" || key == "data" {
            let decoded = decode_entities_in_string(&val);
            let scheme = url_scheme(&decoded);
            if (scheme == "http" || scheme == "https") && !urls.contains(&decoded) {
                urls.push(decoded);
            }
        }
    }
    urls
}

// ---------------------------------------------------------------------------
// Low-level helpers
// ---------------------------------------------------------------------------

/// Find the index *after* the first occurrence of `needle` in `html` starting
/// at `from`. Returns None if not found.
fn find_after(html: &str, from: usize, needle: &str) -> Option<usize> {
    html[from..].find(needle).map(|p| from + p + needle.len())
}

/// Find the byte length of a tag starting at `<`, respecting quoted attribute
/// values. Returns the index past the closing `>` and whether it was self-closing.
fn find_tag_end(tag: &str) -> Option<usize> {
    let bytes = tag.as_bytes();
    let mut p = 1usize; // skip '<'
    let mut quote: Option<u8> = None;
    while p < bytes.len() {
        let c = bytes[p];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'>' => return Some(p + 1),
                _ => {}
            },
        }
        p += 1;
    }
    None
}

/// Extract the tag name from a slice like `<a href="...">` → `a`, `</p>` → `p`.
fn tag_name(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut start = 1usize; // skip '<'
    if start >= bytes.len() {
        return None;
    }
    if bytes[start] == b'/' {
        start += 1;
    }
    let mut end = start;
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_alphanumeric() || c == b'-' || c == b':' || c == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    if end == start {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..end]).to_string())
}

/// Skip from `after_open` to just past the matching `</name>` close tag,
/// handling nesting. If no close tag is found, consumes the rest.
fn skip_until_close(html: &str, after_open: usize, name: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{name}");
    let close_pat = format!("</{name}");
    let bytes = lower.as_bytes();
    let mut depth: usize = 1;
    let mut i = after_open;
    while i < bytes.len() {
        if lower[i..].starts_with(&close_pat) {
            depth -= 1;
            i = lower[i..]
                .find('>')
                .map(|e| i + e + 1)
                .unwrap_or(bytes.len());
            if depth == 0 {
                return i;
            }
            continue;
        }
        if lower[i..].starts_with(&open_pat) {
            let after = i + open_pat.len();
            // Only count as a nested open if it's a real tag name boundary AND
            // the tag is not void/self-closing (void tags have no close tag).
            if (after >= bytes.len() || !is_name_char(bytes[after]))
                && !is_void_or_self_closing(&lower[i..])
            {
                depth += 1;
            }
        }
        i += 1;
    }
    bytes.len()
}

/// True if the tag at the start of `tag` is void or self-closing (no close tag).
fn is_void_or_self_closing(tag: &str) -> bool {
    if tag.starts_with("</") {
        return false; // close tag
    }
    // Find the end of this tag to check for "/>".
    let end = find_tag_end(tag).unwrap_or(tag.len());
    if tag[..end].ends_with("/>") {
        return true;
    }
    if let Some(name) = tag_name(&tag[..end]) {
        let n = name.to_ascii_lowercase();
        if is_void_name(&n) {
            return true;
        }
        // input/meta/base/link are dangerous void-like tags (no close tag).
        if matches!(n.as_str(), "input" | "meta" | "base" | "link") {
            return true;
        }
    }
    false
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b':' || b == b'_'
}

fn is_void_name(name: &str) -> bool {
    VOID_TAGS.contains(&name)
}

/// Parse attributes from a single tag slice (e.g. `<a href="x" title="y">`).
/// Returns (key, value) pairs with values entity-decoded.
fn parse_attrs(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Skip the tag name: find the first whitespace or end.
    let bytes = tag.as_bytes();
    let mut p = 1usize; // skip '<'
    if p < bytes.len() && bytes[p] == b'/' {
        p += 1;
    }
    while p < bytes.len() && !bytes[p].is_ascii_whitespace() && bytes[p] != b'>' && bytes[p] != b'/'
    {
        p += 1;
    }
    // Now parse attrs.
    while p < bytes.len() {
        // Skip whitespace and stray slashes.
        while p < bytes.len() && (bytes[p].is_ascii_whitespace() || bytes[p] == b'/') {
            p += 1;
        }
        if p >= bytes.len() || bytes[p] == b'>' {
            break;
        }
        // Read attr name.
        let name_start = p;
        while p < bytes.len()
            && !bytes[p].is_ascii_whitespace()
            && bytes[p] != b'='
            && bytes[p] != b'>'
            && bytes[p] != b'/'
        {
            p += 1;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..p]).to_string();
        // Skip whitespace.
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        if p >= bytes.len() || bytes[p] != b'=' {
            // Valueless attribute (e.g. `disabled`).
            if !name.is_empty() {
                out.push((name.to_ascii_lowercase(), String::new()));
            }
            continue;
        }
        p += 1; // skip '='
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        if p >= bytes.len() {
            break;
        }
        let quote = bytes[p];
        let value = if quote == b'"' || quote == b'\'' {
            p += 1;
            let start = p;
            while p < bytes.len() && bytes[p] != quote {
                p += 1;
            }
            String::from_utf8_lossy(&bytes[start..p]).to_string()
        } else {
            let start = p;
            while p < bytes.len()
                && !bytes[p].is_ascii_whitespace()
                && bytes[p] != b'>'
                && bytes[p] != b'/'
            {
                p += 1;
            }
            String::from_utf8_lossy(&bytes[start..p]).to_string()
        };
        if !name.is_empty() {
            out.push((name.to_ascii_lowercase(), value));
        }
        if p < bytes.len() && (quote == b'"' || quote == b'\'') {
            p += 1; // skip closing quote
        }
    }
    out
}

/// Parse all attributes from all tags in an HTML string (for remote_content_urls).
fn parse_all_attrs(html: &str) -> Vec<(String, String)> {
    let mut all = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = match find_tag_end(&html[i..]) {
                Some(e) => i + e,
                None => break,
            };
            let tag = &html[i..end];
            if let Some(name) = tag_name(tag) {
                let lower = name.to_ascii_lowercase();
                if !lower.starts_with('/') && !html[i..].starts_with("<!--") {
                    all.extend(parse_attrs(tag));
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    all
}

fn is_safe_attr(tag: &str, attr: &str) -> bool {
    matches!(
        (tag, attr),
        ("a", "href")
            | ("a", "alt")
            | ("a", "title")
            | ("a", "name")
            | ("img", "src")
            | ("img", "alt")
            | ("img", "title")
            | ("td", "colspan")
            | ("td", "rowspan")
            | ("th", "colspan")
            | ("th", "rowspan")
    )
}

fn push_tag(out: &mut String, name: &str, attrs: &str, void: bool) {
    out.push('<');
    out.push_str(name);
    if !attrs.is_empty() {
        out.push_str(attrs);
    }
    if void {
        out.push_str(" />");
    } else {
        out.push('>');
    }
}

fn push_attr(attrs: &mut String, key: &str, value: &str) {
    attrs.push(' ');
    attrs.push_str(key);
    attrs.push_str("=\"");
    // Escape the value for safe attribute inclusion.
    for ch in value.chars() {
        match ch {
            '"' => attrs.push_str("&quot;"),
            '&' => attrs.push_str("&amp;"),
            '<' => attrs.push_str("&lt;"),
            _ => attrs.push(ch),
        }
    }
    attrs.push('"');
}

/// Extract the URL scheme (lowercase, before the first `:`), or empty if none.
fn url_scheme(url: &str) -> String {
    let trimmed = url.trim();
    // Strip leading whitespace/newlines (attackers hide the scheme).
    let stripped = trimmed.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{0000}');
    if let Some(colon) = stripped.find(':') {
        let scheme = stripped[..colon].to_ascii_lowercase();
        // Scheme must be alphanumeric (no slashes/colons inside).
        if scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && !scheme.is_empty()
        {
            return scheme;
        }
    }
    String::new()
}

/// Decode HTML entities in a string (for attribute values before scheme check).
fn decode_entities_in_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let (decoded, len) = decode_entity(&s[i..]);
            out.push_str(&decoded);
            i += len;
        } else {
            // Copy one byte at a time for ASCII; for multibyte, copy the char.
            if bytes[i] < 0x80 {
                out.push(bytes[i] as char);
                i += 1;
            } else {
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Decode a single HTML entity at the start of `rest`. Returns (decoded, len).
fn decode_entity(rest: &str) -> (String, usize) {
    let bytes = rest.as_bytes();
    if bytes.is_empty() || bytes[0] != b'&' {
        return (String::new(), 0);
    }
    let end = rest.find(';').unwrap_or(0);
    if end == 0 || end > 12 {
        return ("&".to_string(), 1);
    }
    let entity = &rest[..=end];
    let decoded: String = match entity {
        "&amp;" => "&".into(),
        "&lt;" => "<".into(),
        "&gt;" => ">".into(),
        "&quot;" => "\"".into(),
        "&apos;" => "'".into(),
        "&#39;" => "'".into(),
        "&nbsp;" => " ".into(),
        _ => {
            let inner = &entity[1..entity.len() - 1];
            if let Some(num) = inner
                .strip_prefix("#x")
                .or_else(|| inner.strip_prefix("#X"))
            {
                u32::from_str_radix(num, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
            } else if let Some(num) = inner.strip_prefix('#') {
                num.parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
            } else {
                None
            }
            .unwrap_or_else(|| entity.to_string())
        }
    };
    (decoded, entity.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(s: &SanitizedHtml) -> &[String] {
        &s.blocked_resources
    }

    #[test]
    fn empty_input() {
        let s = sanitize("", false);
        assert!(s.html.is_empty());
        assert!(!s.has_remote_content);
    }

    #[test]
    fn drops_script_tag_and_contents() {
        let s = sanitize("<p>hi</p><script>alert(1)</script><p>bye</p>", false);
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("alert"));
        assert!(s.html.contains("<p>hi</p>"));
        assert!(s.html.contains("<p>bye</p>"));
        assert!(!s.has_remote_content);
        assert!(blocked(&s).is_empty());
    }

    #[test]
    fn drops_style_and_iframe_with_contents() {
        let s = sanitize(
            "<style>body { color: red }</style><iframe src=\"http://x\"></iframe>ok",
            false,
        );
        assert!(!s.html.contains("color"));
        assert!(!s.html.contains("iframe"));
        assert!(s.has_remote_content);
        assert!(blocked(&s).iter().any(|u| u == "http://x"));
        assert!(s.html.contains("ok"));
    }

    #[test]
    fn drops_object_embed_form_input_button() {
        let s = sanitize(
            "<object data=\"http://x\"></object><embed src=\"http://y\"><form><input></form>z",
            false,
        );
        for t in ["object", "embed", "form", "input", "button"] {
            assert!(!s.html.contains(t), "leaked {t}");
        }
        assert!(s.html.contains("z"));
        assert!(s.has_remote_content);
    }

    #[test]
    fn drops_meta_link_base_title_head_svg() {
        let s = sanitize(
            "<head><title>Secret</title></head><meta http-equiv=\"refresh\" content=\"0;url=http://x\"><link rel=\"stylesheet\" href=\"http://y\"><base href=\"http://z\"><svg onload=\"alert(1)\"></svg>visible",
            false,
        );
        for t in [
            "head", "title", "meta", "link", "base", "svg", "onload", "Secret",
        ] {
            assert!(!s.html.contains(t), "leaked {t}");
        }
        assert!(s.html.contains("visible"));
        assert!(s.has_remote_content);
    }

    #[test]
    fn strips_javascript_urls_from_href_and_src() {
        let s = sanitize(
            "<a href=\"javascript:alert(1)\">x</a><img src=\"javascript:alert(2)\">",
            false,
        );
        assert!(!s.html.contains("javascript"));
        assert!(!s.html.contains("alert"));
        assert!(!s.html.contains("<img"));
        // anchor kept as inert text (href stripped)
        assert!(s.html.contains(">x<"));
    }

    #[test]
    fn strips_data_and_vbscript_urls() {
        let s = sanitize(
            "<a href=\"data:text/html,<script>alert(1)</script>\">x</a><img src=\"vbscript:msgbox\">",
            false,
        );
        assert!(!s.html.contains("data:"));
        assert!(!s.html.contains("vbscript:"));
        assert!(!s.html.contains("<img"));
    }

    #[test]
    fn strips_on_event_handlers() {
        let s = sanitize("<p onclick=\"evil()\" onmouseover=\"x\">text</p>", false);
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("onmouseover"));
        assert!(s.html.contains("<p>"));
        assert!(s.html.contains("text"));
    }

    #[test]
    fn style_attribute_dropped() {
        let s = sanitize("<p style=\"color:expression(alert(1))\">text</p>", false);
        assert!(!s.html.contains("style"));
        assert!(!s.html.contains("expression"));
        assert!(s.html.contains("text"));
    }

    #[test]
    fn img_remote_src_dropped_when_disallowed() {
        let s = sanitize("<img src=\"http://evil/x.png\" alt=\"hi\">", false);
        assert!(!s.html.contains("<img"));
        assert!(!s.html.contains("evil"));
        assert!(s.has_remote_content);
        assert!(blocked(&s).contains(&"http://evil/x.png".to_string()));
    }

    #[test]
    fn img_remote_src_kept_when_allowed() {
        let s = sanitize("<img src=\"https://ok/x.png\" alt=\"hi\">", true);
        assert!(s.html.contains("<img"));
        assert!(s.html.contains("https://ok/x.png"));
        assert!(s.has_remote_content);
    }

    #[test]
    fn img_cid_attachment_kept_without_remote_flag() {
        let s = sanitize("<img src=\"cid:attach-1\" alt=\"inline\">", false);
        assert!(s.html.contains("<img"));
        assert!(s.html.contains("cid:attach-1"));
        assert!(!s.has_remote_content);
    }

    #[test]
    fn anchor_remote_href_blocked_when_disallowed() {
        let s = sanitize("<a href=\"http://evil\">click</a>", false);
        assert!(!s.html.contains("http://evil"));
        assert!(s.html.contains(">click<"));
        assert!(s.has_remote_content);
    }

    #[test]
    fn anchor_remote_href_allowed_when_consent_given() {
        let s = sanitize("<a href=\"https://safe\">link</a>", true);
        assert!(s.html.contains("href=\"https://safe\""));
        assert!(s.html.contains("rel=\"noopener noreferrer\""));
        assert!(s.html.contains("target=\"_blank\""));
        assert!(s.has_remote_content);
    }

    #[test]
    fn mailto_link_preserved_and_not_remote() {
        let s = sanitize("<a href=\"mailto:me@example.com\">email</a>", false);
        assert!(s.html.contains("mailto:me@example.com"));
        assert!(!s.has_remote_content);
        // mailto is not navigational: no rel/target forced.
        assert!(!s.html.contains("target=\"_blank\""));
    }

    #[test]
    fn relative_url_preserved() {
        let s = sanitize("<a href=\"/inbox/1\">open</a>", false);
        assert!(s.html.contains("href=\"/inbox/1\""));
        assert!(!s.has_remote_content);
    }

    #[test]
    fn allowed_tags_pass_through_with_safe_attrs() {
        let s = sanitize("<div><p class=\"x\">text</p></div>", false);
        assert!(s.html.contains("<div>"));
        assert!(s.html.contains("<p>"));
        assert!(s.html.contains("text"));
        // class is not in the allowlist → dropped
        assert!(!s.html.contains("class"));
    }

    #[test]
    fn preserves_paragraph_structure() {
        let s = sanitize("<p>one</p><p>two</p><br><p>three</p>", false);
        assert_eq!(s.html.matches("<p>").count(), 3);
        assert!(s.html.contains("<br"));
        assert!(s.html.contains("one"));
        assert!(s.html.contains("two"));
        assert!(s.html.contains("three"));
    }

    #[test]
    fn table_colspan_rowspan_preserved() {
        let s = sanitize(
            "<td colspan=\"2\" rowspan=\"3\" style=\"x\">cell</td>",
            false,
        );
        assert!(s.html.contains("colspan=\"2\""));
        assert!(s.html.contains("rowspan=\"3\""));
        assert!(!s.html.contains("style"));
        assert!(s.html.contains("cell"));
    }

    #[test]
    fn comments_dropped() {
        let s = sanitize("<p>before</p><!-- hidden comment --><p>after</p>", false);
        assert!(!s.html.contains("hidden"));
        assert!(!s.html.contains("<!--"));
        assert!(s.html.contains("before"));
        assert!(s.html.contains("after"));
    }

    #[test]
    fn case_insensitive_tag_matching() {
        let s = sanitize("<P>Text</P><SCRIPT>alert(1)</SCRIPT>", false);
        assert!(s.html.contains("Text"));
        assert!(!s.html.contains("alert"));
        assert!(!s.html.contains("SCRIPT"));
    }

    #[test]
    fn unknown_tag_dropped_but_contents_kept() {
        let s = sanitize("<font color=\"red\">visible</font>", false);
        assert!(!s.html.contains("font"));
        assert!(s.html.contains("visible"));
    }

    #[test]
    fn nested_dangerous_tag_fully_removed() {
        let s = sanitize("<div><script>a</script>kept</div>", false);
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("a"));
        assert!(s.html.contains("kept"));
        assert!(s.html.contains("<div>"));
    }

    #[test]
    fn self_closing_void_tags() {
        let s = sanitize("<p>text<br>more</p>", false);
        assert!(s.html.contains("<br"));
        assert!(s.html.contains("text"));
        assert!(s.html.contains("more"));
    }

    #[test]
    fn entity_decoding_in_text() {
        let s = sanitize(
            "<p>5 &amp; 6 &lt; 7 &gt; 4 &quot;hi&quot; &#39;x&#39;</p>",
            false,
        );
        // Decoded text survives; `<` and `>` are re-encoded so no tag forms.
        assert!(s.html.contains("5"));
        assert!(s.html.contains("6"));
        assert!(!s.html.contains("< 7"));
        // The decoded `>` must not appear raw.
        assert!(!s.html.contains("> 4"));
    }

    #[test]
    fn entity_decoding_in_attr_value() {
        // An entity-encoded javascript: scheme must be caught after decoding.
        let s = sanitize("<a href=\"javascript&#58;alert(1)\">x</a>", false);
        assert!(!s.html.contains("javascript"));
        assert!(!s.html.contains("alert"));
    }

    #[test]
    fn remote_content_urls_extracts_and_dedupes() {
        let html = r#"<img src="http://a/1.png"><a href="https://b">l</a><img src='http://a/1.png'><a href="mailto:local">m</a>"#;
        let urls = remote_content_urls(html);
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"http://a/1.png".to_string()));
        assert!(urls.contains(&"https://b".to_string()));
        assert!(!urls.iter().any(|u| u.contains("mailto")));
    }

    #[test]
    fn unterminated_tag_does_not_panic() {
        let s = sanitize("<p>text without close", false);
        assert!(s.html.contains("text"));
    }

    #[test]
    fn quote_inside_attr_value_does_not_terminate_tag_early() {
        let s = sanitize(r#"<a href="http://x?a='b'">link</a>"#, true);
        assert!(s.html.contains("http://x?a='b'"));
        assert!(s.html.contains(">link<"));
    }
}
