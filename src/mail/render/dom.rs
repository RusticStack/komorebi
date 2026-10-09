//! Tolerant HTML tokenizer and arena tree for the email subset, in one
//! forward pass. Raw-text elements (`script`, `style`, `title`, `textarea`)
//! are skipped; `<style>` text is kept for the cascade. Conditional MSO
//! comments (`<!--[if mso]>...<![endif]-->`) are comments, so Outlook-only
//! markup is dropped and `<!--[if !mso]><!-->` content is kept.

use crate::mail::html::decode_entities;

pub type Id = u32;

pub enum Kind {
    Elem {
        tag: Box<str>,
        attrs: Vec<(Box<str>, Box<str>)>,
    },
    Text(String),
}

pub struct Node {
    pub kind: Kind,
    pub parent: Id,
    pub children: Vec<Id>,
}

pub struct Dom {
    pub nodes: Vec<Node>,
    /// `<style>` contents, at most [`MAX_STYLE_BYTES`] in total.
    pub styles: Vec<String>,
    /// The node cap was hit and the rest of the document dropped.
    pub truncated: bool,
}

/// Elements plus text runs.
pub const MAX_NODES: usize = 50_000;
/// Open elements nested deeper than this become siblings at this depth.
pub const MAX_DEPTH: usize = 256;
/// Total `<style>` text kept.
pub const MAX_STYLE_BYTES: usize = 256 * 1024;
/// Attributes kept per element (the duplicate check is quadratic).
const MAX_ATTRS: usize = 64;

pub const ROOT: Id = 0;

impl Dom {
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.get(id as usize)
    }
    pub fn tag(&self, id: Id) -> &str {
        match self.node(id).map(|n| &n.kind) {
            Some(Kind::Elem { tag, .. }) => tag,
            _ => "",
        }
    }
    pub fn attr(&self, id: Id, name: &str) -> Option<&str> {
        match self.node(id).map(|n| &n.kind) {
            Some(Kind::Elem { attrs, .. }) => {
                attrs.iter().find(|(k, _)| &**k == name).map(|(_, v)| &**v)
            }
            _ => None,
        }
    }
    pub fn is_elem(&self, id: Id) -> bool {
        matches!(self.node(id).map(|n| &n.kind), Some(Kind::Elem { .. }))
    }
    pub fn children(&self, id: Id) -> &[Id] {
        self.node(id).map_or(&[], |n| &n.children)
    }
    pub fn parent(&self, id: Id) -> Id {
        self.node(id).map_or(ROOT, |n| n.parent)
    }
}

fn is_void(t: &str) -> bool {
    matches!(
        t,
        "br" | "hr"
            | "img"
            | "input"
            | "meta"
            | "link"
            | "area"
            | "base"
            | "col"
            | "embed"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn closes_p(t: &str) -> bool {
    matches!(
        t,
        "div"
            | "p"
            | "table"
            | "ul"
            | "ol"
            | "dl"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
            | "blockquote"
            | "hr"
            | "form"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "address"
            | "li"
            | "dd"
            | "dt"
            | "figure"
    )
}

struct Builder {
    dom: Dom,
    /// Open elements; `stack[0]` is the root.
    stack: Vec<Id>,
    /// Tags opened past [`MAX_DEPTH`], not on `stack`, so their end tags
    /// close them instead of an outer element.
    overflow: Vec<Box<str>>,
    style_bytes: usize,
}

pub fn parse(html: &str) -> Dom {
    let mut p = Builder {
        dom: Dom {
            nodes: vec![Node {
                kind: Kind::Elem {
                    tag: "#root".into(),
                    attrs: vec![],
                },
                parent: ROOT,
                children: vec![],
            }],
            styles: vec![],
            truncated: false,
        },
        stack: vec![ROOT],
        overflow: vec![],
        style_bytes: 0,
    };
    let b = html.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let n = b.len();
    let mut i = 0;
    let mut text_start = 0;
    while i < n {
        if at(i) != b'<' {
            i = i.saturating_add(1);
            continue;
        }
        let rest = html.get(i..).unwrap_or("");
        if rest.starts_with("<!--") {
            p.text(html, text_start, i);
            i = rest
                .get(4..)
                .and_then(|r| r.find("-->"))
                .map_or(n, |e| i.saturating_add(e).saturating_add(7));
            text_start = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            p.text(html, text_start, i);
            i = rest
                .find('>')
                .map_or(n, |e| i.saturating_add(e).saturating_add(1));
            text_start = i;
            continue;
        }
        let closing = rest.starts_with("</");
        let name_start = i.saturating_add(if closing { 2 } else { 1 });
        if !at(name_start).is_ascii_alphabetic() {
            i = i.saturating_add(1);
            continue;
        }
        let mut j = name_start;
        while j < n && !at(j).is_ascii_whitespace() && !matches!(at(j), b'>' | b'/') {
            j = j.saturating_add(1);
        }
        let tag = html.get(name_start..j).unwrap_or("").to_ascii_lowercase();
        p.text(html, text_start, i);
        if closing {
            i = html
                .get(j..)
                .and_then(|r| r.find('>'))
                .map_or(n, |e| j.saturating_add(e).saturating_add(1));
            p.close(&tag);
            text_start = i;
            continue;
        }
        let mut attrs: Vec<(Box<str>, Box<str>)> = vec![];
        let mut self_close = false;
        loop {
            while j < n && at(j).is_ascii_whitespace() {
                j = j.saturating_add(1);
            }
            if j >= n {
                break;
            }
            if at(j) == b'>' {
                j = j.saturating_add(1);
                break;
            }
            if at(j) == b'/' {
                self_close = true;
                j = j.saturating_add(1);
                continue;
            }
            let ks = j;
            while j < n && !at(j).is_ascii_whitespace() && !matches!(at(j), b'=' | b'>' | b'/') {
                j = j.saturating_add(1);
            }
            if ks == j {
                j = j.saturating_add(1);
                continue;
            }
            let key = html.get(ks..j).unwrap_or("").to_ascii_lowercase();
            while j < n && at(j).is_ascii_whitespace() {
                j = j.saturating_add(1);
            }
            let mut val = String::new();
            if at(j) == b'=' {
                j = j.saturating_add(1);
                while j < n && at(j).is_ascii_whitespace() {
                    j = j.saturating_add(1);
                }
                let q = at(j);
                if q == b'"' || q == b'\'' {
                    let vs = j.saturating_add(1);
                    let e = html
                        .get(vs..)
                        .and_then(|r| r.bytes().position(|c| c == q))
                        .map_or(n, |e| vs.saturating_add(e));
                    val = decode_entities(html.get(vs..e).unwrap_or("")).into_owned();
                    j = e.saturating_add(1).min(n);
                } else {
                    let vs = j;
                    while j < n && !at(j).is_ascii_whitespace() && at(j) != b'>' {
                        j = j.saturating_add(1);
                    }
                    val = decode_entities(html.get(vs..j).unwrap_or("")).into_owned();
                }
            }
            if attrs.len() < MAX_ATTRS && !attrs.iter().any(|(k, _)| **k == *key) {
                attrs.push((key.into(), val.into()));
            }
        }
        i = j;
        text_start = i;
        if matches!(tag.as_str(), "script" | "style" | "title" | "textarea") {
            let close = format!("</{tag}");
            let e = html
                .get(i..)
                .and_then(|r| find_ci(r, &close))
                .map_or(n, |e| i.saturating_add(e));
            if tag == "style" {
                p.style(html.get(i..e).unwrap_or(""));
            }
            i = html
                .get(e..)
                .and_then(|r| r.find('>'))
                .map_or(n, |x| e.saturating_add(x).saturating_add(1));
            text_start = i;
            continue;
        }
        p.open(tag, attrs, self_close);
    }
    p.text(html, text_start, n);
    p.dom
}

/// Position of `needle` (which starts with `<`) in `hay`, ASCII
/// case-insensitive.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let nd = needle.as_bytes();
    if nd.is_empty() {
        return None;
    }
    hay.as_bytes()
        .windows(nd.len())
        .position(|w| w.first() == Some(&b'<') && w.eq_ignore_ascii_case(nd))
}

impl Builder {
    fn top(&self) -> Id {
        self.stack.last().copied().unwrap_or(ROOT)
    }

    fn style(&mut self, css: &str) {
        let room = MAX_STYLE_BYTES.saturating_sub(self.style_bytes);
        let mut end = css.len().min(room);
        while !css.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let css = css.get(..end).unwrap_or("");
        if !css.is_empty() {
            self.style_bytes = self.style_bytes.saturating_add(css.len());
            self.dom.styles.push(css.to_string());
        }
    }

    fn text(&mut self, html: &str, start: usize, end: usize) {
        if end <= start {
            return;
        }
        let raw = html.get(start..end).unwrap_or("");
        let decoded = decode_entities(raw);
        // NUL and C0 controls other than whitespace never render.
        let t: std::borrow::Cow<str> = if decoded
            .chars()
            .any(|c| c.is_control() && !c.is_ascii_whitespace())
        {
            decoded
                .chars()
                .filter(|c| !c.is_control() || c.is_ascii_whitespace())
                .collect::<String>()
                .into()
        } else {
            decoded
        };
        if t.is_empty() {
            return;
        }
        let parent = self.top();
        // Merge with a preceding text sibling (text split by comments).
        let last = self.dom.children(parent).last().copied();
        if let Some(Node {
            kind: Kind::Text(s),
            ..
        }) = last.and_then(|l| self.dom.nodes.get_mut(l as usize))
        {
            s.push_str(&t);
            return;
        }
        self.push(Kind::Text(t.into_owned()), parent);
    }

    /// Append a node under `parent`; `None` once the node cap is hit.
    fn push(&mut self, kind: Kind, parent: Id) -> Option<Id> {
        if self.dom.nodes.len() >= MAX_NODES {
            self.dom.truncated = true;
            return None;
        }
        let id = Id::try_from(self.dom.nodes.len()).ok()?;
        self.dom.nodes.push(Node {
            kind,
            parent,
            children: vec![],
        });
        if let Some(p) = self.dom.nodes.get_mut(parent as usize) {
            p.children.push(id);
        }
        Some(id)
    }

    fn open(&mut self, tag: String, attrs: Vec<(Box<str>, Box<str>)>, self_close: bool) {
        let t = tag.as_str();
        if closes_p(t) && self.dom.tag(self.top()) == "p" {
            self.stack.pop();
        }
        match t {
            "li" => self.pop_within(&["li"], &["ul", "ol"]),
            "dt" | "dd" => self.pop_within(&["dt", "dd"], &["dl"]),
            "td" | "th" => self.pop_within(&["td", "th"], &["tr", "table"]),
            "tr" => self.pop_within(&["tr", "td", "th"], &["table", "tbody", "thead", "tfoot"]),
            "tbody" | "thead" | "tfoot" => {
                self.pop_within(&["tr", "td", "th", "tbody", "thead", "tfoot"], &["table"])
            }
            "option" => self.pop_within(&["option"], &["select"]),
            _ => {}
        }
        let void = is_void(t) || self_close;
        let tag: Box<str> = tag.into();
        let parent = self.top();
        let Some(id) = self.push(
            Kind::Elem {
                tag: tag.clone(),
                attrs,
            },
            parent,
        ) else {
            return;
        };
        if void {
            return;
        }
        if self.stack.len() > MAX_DEPTH {
            self.overflow.push(tag);
        } else {
            self.stack.push(id);
        }
    }

    /// Close the nearest open element named one of `names`, unless a
    /// `boundary` element is open above it.
    fn pop_within(&mut self, names: &[&str], boundary: &[&str]) {
        for k in (1..self.stack.len()).rev() {
            let t = self.dom.tag(self.stack.get(k).copied().unwrap_or(ROOT));
            if names.contains(&t) {
                self.stack.truncate(k);
                return;
            }
            if boundary.contains(&t) {
                return;
            }
        }
    }

    fn close(&mut self, tag: &str) {
        if let Some(pos) = self.overflow.iter().rposition(|t| &**t == tag) {
            self.overflow.truncate(pos);
            return;
        }
        let table_part = matches!(
            tag,
            "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" | "table"
        );
        for k in (1..self.stack.len()).rev() {
            let t = self.dom.tag(self.stack.get(k).copied().unwrap_or(ROOT));
            if t == tag {
                self.stack.truncate(k);
                return;
            }
            // An end tag never reaches out of a table cell or table.
            if matches!(t, "table" | "td" | "th" | "tr") && !table_part {
                return;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn tags(dom: &Dom, id: Id) -> Vec<&str> {
        dom.children(id).iter().map(|&c| dom.tag(c)).collect()
    }

    #[test]
    fn implied_end_tags_and_raw_text() {
        let d = parse("<p>a<p>b<table><tr><td>x<td>y</table><script>if(a<b)x()</script>");
        assert_eq!(tags(&d, ROOT), ["p", "p", "table"]);
        let table = d.children(ROOT)[2];
        let tr = d.children(table)[0];
        assert_eq!(tags(&d, tr), ["td", "td"]);
        assert!(
            !d.nodes
                .iter()
                .any(|n| matches!(&n.kind, Kind::Text(t) if t.contains("x()")))
        );
    }

    #[test]
    fn depth_is_capped_without_losing_content() {
        let html = "<div>".repeat(400) + "deep" + &"</div>".repeat(400) + "after";
        let d = parse(&html);
        let mut depth = 0;
        let mut id = ROOT;
        while let Some(&c) = d.children(id).iter().find(|&&c| d.is_elem(c)) {
            id = c;
            depth += 1;
        }
        assert!(depth <= MAX_DEPTH + 1);
        let texts: Vec<&str> = d
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                Kind::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"deep"));
        assert!(texts.contains(&"after"));
        // "after" lands back at the top level.
        assert!(
            d.children(ROOT)
                .iter()
                .any(|&c| matches!(&d.nodes[c as usize].kind, Kind::Text(t) if t == "after"))
        );
    }

    #[test]
    fn conditional_comments() {
        let d = parse(
            "<!--[if mso]><b>outlook</b><![endif]--><!--[if !mso]><!--><i>web</i><!--<![endif]-->",
        );
        assert_eq!(tags(&d, ROOT), ["i"]);
    }
}
