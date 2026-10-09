//! A small, tolerant CSS parser for email. It is quote and paren aware,
//! handles `!important`, specificity and source order, type, `.class`,
//! `#id`, `[attr]`, `:first-child`, `:last-child` and `:root` selectors
//! with descendant, child, `+` and `~` combinators, and `@media`
//! (min/max-width, prefers-color-scheme) evaluated against the render
//! width. `@import`, `@font-face` and every other at-rule are skipped;
//! their remote URLs are only counted. Nothing here fetches anything.

use super::dom::{Dom, Id};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Decl {
    /// Lowercase.
    pub name: Box<str>,
    /// Keywords lowercased; `url(...)` and quoted strings kept verbatim.
    pub value: Box<str>,
    pub important: bool,
}

/// At most this many rules are kept per message.
pub const MAX_RULES: usize = 5_000;
/// Selectors with more compound parts than this never match (bounds the
/// matcher's recursion).
const MAX_PARTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comb {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AttrOp {
    Exists,
    Eq,
    Word,
    Prefix,
    Suffix,
    Contains,
    Dash,
}

#[derive(Clone, Debug)]
struct AttrSel {
    name: Box<str>,
    op: AttrOp,
    val: Box<str>,
    ci: bool,
}

#[derive(Clone, Debug, Default)]
struct Compound {
    tag: Option<Box<str>>,
    id: Option<Box<str>>,
    classes: Vec<Box<str>>,
    attrs: Vec<AttrSel>,
    first_child: bool,
    last_child: bool,
    root: bool,
}

#[derive(Clone, Debug)]
struct Selector {
    /// Right to left: `parts[0]` is the subject; `comb[i]` links `parts[i]`
    /// to `parts[i + 1]`.
    parts: Vec<Compound>,
    comb: Vec<Comb>,
    spec: u32,
}

struct Rule {
    sel: Selector,
    order: u32,
    decls: std::ops::Range<usize>,
}

#[derive(Default)]
pub struct Sheet {
    rules: Vec<Rule>,
    decls: Vec<Decl>,
    by_id: HashMap<Box<str>, Vec<u32>>,
    by_class: HashMap<Box<str>, Vec<u32>>,
    by_tag: HashMap<Box<str>, Vec<u32>>,
    universal: Vec<u32>,
    /// Remote URLs referenced by `@import`, `@font-face` and `url()`
    /// (refused, counted only).
    pub remote_urls: Vec<String>,
    /// The sheet has `prefers-color-scheme: dark` rules.
    pub has_dark_rules: bool,
}

pub struct Ctx {
    pub width: f32,
    pub dark: bool,
    /// Dim the resolved colours for a dark theme (see `style::dim_bg`).
    pub dim: bool,
}

/// Strip `/* */` comments and `<!-- -->` wrappers outside strings.
fn strip_comments(css: &str) -> String {
    let b = css.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut start = 0;
    let mut quote = 0u8;
    let keep = |out: &mut String, from: usize, to: usize| {
        out.push_str(css.get(from..to).unwrap_or(""));
        out.push(' ');
    };
    while i < b.len() {
        let c = at(i);
        if quote != 0 {
            if c == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if c == quote || c == b'\n' {
                quote = 0;
            }
            i = i.saturating_add(1);
            continue;
        }
        let rest = css.get(i..).unwrap_or("");
        if c == b'"' || c == b'\'' {
            quote = c;
        } else if rest.starts_with("/*") {
            keep(&mut out, start, i);
            i = rest
                .get(2..)
                .and_then(|r| r.find("*/"))
                .map_or(b.len(), |p| i.saturating_add(p).saturating_add(4));
            start = i;
            continue;
        } else if rest.starts_with("<!--") || rest.starts_with("-->") {
            keep(&mut out, start, i);
            i = i.saturating_add(if c == b'<' { 4 } else { 3 });
            start = i;
            continue;
        }
        i = i.saturating_add(1);
    }
    out.push_str(css.get(start.min(b.len())..).unwrap_or(""));
    out
}

/// Index of the `}` matching the `{` at `open` (quote aware), or the length
/// if unbalanced.
fn match_brace(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut depth = 0u32;
    let mut quote = 0u8;
    let mut i = open;
    while let Some(&c) = b.get(i) {
        if quote != 0 {
            if c == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'{' {
            depth = depth.saturating_add(1);
        } else if c == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return i;
            }
        }
        i = i.saturating_add(1);
    }
    b.len()
}

/// The first top-level occurrence of any byte in `set` (quote, paren and
/// bracket aware).
fn find_top(s: &str, set: &[u8]) -> Option<usize> {
    let b = s.as_bytes();
    let (mut quote, mut depth) = (0u8, 0i32);
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if quote != 0 {
            if c == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'(' || c == b'[' {
            depth = depth.saturating_add(1);
        } else if c == b')' || c == b']' {
            depth = depth.saturating_sub(1);
        } else if depth <= 0 && set.contains(&c) {
            return Some(i);
        }
        i = i.saturating_add(1);
    }
    None
}

fn split_top_on(s: &str, sep: u8) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(p) = find_top(rest, &[sep]) {
        out.push(rest.get(..p).unwrap_or(""));
        rest = rest.get(p.saturating_add(1)..).unwrap_or("");
    }
    out.push(rest);
    out
}

/// Whether a URL would be fetched from the network.
pub fn is_remote(u: &str) -> bool {
    let u = u.trim().trim_matches(['\'', '"']).trim_start();
    let head = u.get(..8).unwrap_or(u).to_ascii_lowercase();
    head.starts_with("http:")
        || head.starts_with("https:")
        || head.starts_with("//")
        || head.starts_with("ftp:")
}

/// Every `url(...)` target in a value.
pub fn urls_in(v: &str, out: &mut Vec<String>) {
    let lower = v.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower.get(from..).and_then(|r| r.find("url(")) {
        let s = from.saturating_add(p).saturating_add(4);
        let e = lower
            .get(s..)
            .and_then(|r| r.find(')'))
            .map_or(lower.len(), |q| s.saturating_add(q));
        out.push(
            v.get(s..e)
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"'])
                .to_string(),
        );
        from = e;
    }
}

fn remote_urls_in(v: &str, out: &mut Vec<String>) {
    if v.contains("url(") || v.contains("URL(") {
        let mut u = vec![];
        urls_in(v, &mut u);
        out.extend(u.into_iter().filter(|x| is_remote(x)));
    }
}

/// Lowercase a declaration value except inside `url(...)` and quoted
/// strings.
fn lower_value(v: &str) -> Box<str> {
    if !v.bytes().any(|c| c.is_ascii_uppercase()) {
        return v.into();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(c) = rest.chars().next() {
        let verbatim = if c == '"' || c == '\'' {
            // The quoted string with both quotes.
            rest.get(1..)
                .and_then(|r| r.find(c))
                .map_or(rest.len(), |p| p.saturating_add(2))
        } else if rest
            .get(..4)
            .is_some_and(|s| s.eq_ignore_ascii_case("url("))
        {
            out.push_str("url(");
            rest = rest.get(4..).unwrap_or("");
            rest.find(')').map_or(rest.len(), |p| p.saturating_add(1))
        } else {
            out.push(c.to_ascii_lowercase());
            rest = rest.get(c.len_utf8()..).unwrap_or("");
            continue;
        };
        out.push_str(rest.get(..verbatim).unwrap_or(rest));
        rest = rest.get(verbatim..).unwrap_or("");
    }
    out.into()
}

/// Parse a declaration block body (`a:b; c:d !important`).
pub fn parse_decls(body: &str, out: &mut Vec<Decl>) {
    for seg in split_top_on(body, b';') {
        let Some((name, val)) = seg.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '{' || c == '}') {
            continue;
        }
        let mut val = val.trim();
        let mut important = false;
        if let Some((v, bang)) = val.rsplit_once('!')
            && bang.trim().eq_ignore_ascii_case("important")
        {
            important = true;
            val = v.trim();
        }
        out.push(Decl {
            name: name.to_ascii_lowercase().into(),
            value: lower_value(val),
            important,
        });
    }
}

// ---------------------------------------------------------------- selectors

fn ident_end(b: &[u8], mut i: usize) -> usize {
    while let Some(&c) = b.get(i) {
        if !(c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80 || c == b'\\') {
            break;
        }
        i = i.saturating_add(if c == b'\\' { 2 } else { 1 });
    }
    i.min(b.len())
}

/// The text between byte offsets, or "" when they are not char boundaries.
fn sub(s: &str, from: usize, to: usize) -> &str {
    s.get(from..to).unwrap_or("")
}

fn parse_selector(s: &str) -> Option<Selector> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut parts: Vec<Compound> = vec![];
    let mut combs: Vec<Comb> = vec![];
    let mut cur = Compound::default();
    let mut have = false;
    let mut pending: Option<Comb> = None;
    let (mut a, mut bb, mut c) = (0u32, 0u32, 0u32);
    let mut i = 0;
    while let Some(&ch) = b.get(i) {
        if matches!(ch, b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'+' | b'~') {
            let mut comb = Comb::Descendant;
            while let Some(&x) = b.get(i) {
                match x {
                    b'>' => comb = Comb::Child,
                    b'+' => comb = Comb::Adjacent,
                    b'~' => comb = Comb::Sibling,
                    b' ' | b'\t' | b'\n' | b'\r' => {}
                    _ => break,
                }
                i = i.saturating_add(1);
            }
            if have {
                parts.push(std::mem::take(&mut cur));
                have = false;
                pending = Some(comb);
            } else if !parts.is_empty() || comb != Comb::Descendant {
                return None;
            }
            continue;
        }
        if let Some(p) = pending.take() {
            combs.push(p);
        }
        let next = i.saturating_add(1);
        match ch {
            b'*' => i = next,
            b'#' | b'.' => {
                let e = ident_end(b, next);
                if e == next {
                    return None;
                }
                let name: Box<str> = sub(s, next, e).to_ascii_lowercase().into();
                if ch == b'#' {
                    cur.id = Some(name);
                    a = a.saturating_add(1);
                } else {
                    cur.classes.push(name);
                    bb = bb.saturating_add(1);
                }
                i = e;
            }
            b'[' => {
                let e = s.get(i..)?.find(']')?.saturating_add(i);
                let inner = sub(s, next, e).trim();
                let (name, op, val, ci) = if let Some(p) = inner.find('=') {
                    let before = p.checked_sub(1);
                    let (op, nend) = match before.and_then(|q| inner.as_bytes().get(q)) {
                        Some(b'~') => (AttrOp::Word, before),
                        Some(b'^') => (AttrOp::Prefix, before),
                        Some(b'$') => (AttrOp::Suffix, before),
                        Some(b'*') => (AttrOp::Contains, before),
                        Some(b'|') => (AttrOp::Dash, before),
                        _ => (AttrOp::Eq, Some(p)),
                    };
                    let mut v = inner.get(p.saturating_add(1)..).unwrap_or("").trim();
                    let mut ci = false;
                    if let Some(stripped) = v.strip_suffix(" i").or_else(|| v.strip_suffix(" I")) {
                        v = stripped.trim();
                        ci = true;
                    }
                    let name = inner.get(..nend.unwrap_or(0)).unwrap_or("").trim();
                    (name, op, v.trim_matches(['"', '\'']), ci)
                } else {
                    (inner, AttrOp::Exists, "", false)
                };
                if name.is_empty() {
                    return None;
                }
                cur.attrs.push(AttrSel {
                    name: name.to_ascii_lowercase().into(),
                    op,
                    val: val.into(),
                    ci,
                });
                bb = bb.saturating_add(1);
                i = e.saturating_add(1);
            }
            b':' => {
                if b.get(next) == Some(&b':') {
                    return None; // pseudo-elements never match
                }
                let e = ident_end(b, next);
                match sub(s, next, e).to_ascii_lowercase().as_str() {
                    "first-child" => cur.first_child = true,
                    "last-child" => cur.last_child = true,
                    "root" => cur.root = true,
                    _ => return None, // :hover, :not(), ...: never matches
                }
                bb = bb.saturating_add(1);
                i = e;
            }
            _ if ch.is_ascii_alphabetic() || ch == b'_' || ch >= 0x80 => {
                let e = ident_end(b, i);
                // Namespaced type selectors (v|rect) never match.
                if b.get(e) == Some(&b'|') {
                    return None;
                }
                cur.tag = Some(sub(s, i, e).to_ascii_lowercase().into());
                c = c.saturating_add(1);
                i = e;
            }
            _ => return None,
        }
        have = true;
    }
    if !have {
        return None;
    }
    parts.push(cur);
    if combs.len().saturating_add(1) != parts.len() || parts.len() > MAX_PARTS {
        return None;
    }
    parts.reverse();
    combs.reverse();
    Some(Selector {
        parts,
        comb: combs,
        spec: (a.min(255) << 16) | (bb.min(255) << 8) | c.min(255),
    })
}

// ---------------------------------------------------------------- @media

fn media_px(v: &str) -> Option<f32> {
    let v = v.trim();
    let n = if let Some(n) = v.strip_suffix("px") {
        n.trim().parse().ok()
    } else if let Some(n) = v.strip_suffix("em") {
        n.trim()
            .trim_end_matches('r')
            .parse::<f32>()
            .ok()
            .map(|x| x * 16.0)
    } else {
        v.parse().ok()
    };
    n.filter(|x: &f32| x.is_finite())
}

/// Evaluate a media query list; also notes whether it asks for the dark
/// colour scheme.
fn media_matches(q: &str, ctx: &Ctx, has_dark: &mut bool) -> bool {
    let q = q.to_ascii_lowercase();
    let mut any = false;
    for one in q.split(',') {
        let one = one.trim();
        if one.is_empty() || one.starts_with("not ") {
            continue;
        }
        let one = one.strip_prefix("only ").unwrap_or(one);
        let mut ok = true;
        for term in one.split(" and ").map(str::trim).filter(|t| !t.is_empty()) {
            let hit = if let Some(inner) = term.strip_prefix('(').and_then(|x| x.strip_suffix(')'))
            {
                let (f, v) = inner.split_once(':').unwrap_or((inner, ""));
                let v = v.trim();
                match f.trim() {
                    "min-width" => media_px(v).is_some_and(|p| ctx.width >= p),
                    "max-width" => media_px(v).is_some_and(|p| ctx.width <= p),
                    "prefers-color-scheme" => {
                        if v == "dark" {
                            *has_dark = true;
                        }
                        v == if ctx.dark { "dark" } else { "light" }
                    }
                    _ => false,
                }
            } else {
                matches!(term, "screen" | "all")
            };
            ok &= hit;
        }
        any |= ok;
    }
    any
}

// ---------------------------------------------------------------- sheet

impl Sheet {
    pub fn parse_more(&mut self, css: &str, ctx: &Ctx) {
        let css = strip_comments(css);
        self.parse_block(&css, ctx, 0);
    }

    fn parse_block(&mut self, css: &str, ctx: &Ctx, depth: u32) {
        if depth > 8 {
            return;
        }
        let mut rest = css;
        loop {
            rest = rest.trim_start();
            if rest.is_empty() || self.rules.len() >= MAX_RULES {
                break;
            }
            if rest.starts_with('}') || rest.starts_with(';') {
                rest = rest.get(1..).unwrap_or("");
                continue;
            }
            if rest.starts_with('@') {
                let Some(p) = find_top(rest, b";{") else {
                    break;
                };
                if rest.as_bytes().get(p) == Some(&b';') {
                    // @import / @charset / @namespace: skipped, remote imports counted.
                    let stmt = rest.get(..p).unwrap_or("");
                    if let Some(target) = stmt
                        .get(..7)
                        .filter(|k| k.eq_ignore_ascii_case("@import"))
                        .and_then(|_| stmt.get(7..))
                    {
                        let target = target.trim();
                        let mut u = vec![];
                        urls_in(target, &mut u);
                        if u.is_empty() {
                            let first = target.split_whitespace().next().unwrap_or("");
                            u.push(first.trim_matches(['"', '\'']).to_string());
                        }
                        self.remote_urls
                            .extend(u.into_iter().filter(|x| is_remote(x)));
                    }
                    rest = rest.get(p.saturating_add(1)..).unwrap_or("");
                } else {
                    let prelude = rest.get(..p).unwrap_or("").trim();
                    let end = match_brace(rest, p);
                    let body = rest.get(p.saturating_add(1)..end).unwrap_or("");
                    let kw_end = prelude
                        .find(|c: char| c.is_whitespace() || c == '(')
                        .unwrap_or(prelude.len());
                    let kw = prelude.get(1..kw_end).unwrap_or("").to_ascii_lowercase();
                    if kw == "media" {
                        let query = prelude.get(kw_end..).unwrap_or("");
                        if media_matches(query, ctx, &mut self.has_dark_rules) {
                            self.parse_block(body, ctx, depth.saturating_add(1));
                        }
                    } else {
                        // @font-face, @page, @supports, @keyframes, ...: skipped.
                        remote_urls_in(body, &mut self.remote_urls);
                    }
                    rest = rest.get(end.saturating_add(1)..).unwrap_or("");
                }
                continue;
            }
            let Some(p) = find_top(rest, b"{") else {
                break;
            };
            let prelude = rest.get(..p).unwrap_or("");
            let end = match_brace(rest, p);
            let body = rest.get(p.saturating_add(1)..end).unwrap_or("");
            let start = self.decls.len();
            parse_decls(body, &mut self.decls);
            let stop = self.decls.len();
            for d in self.decls.get(start..).unwrap_or(&[]) {
                remote_urls_in(&d.value, &mut self.remote_urls);
            }
            if stop > start {
                for sel in split_top_on(prelude, b',') {
                    if self.rules.len() >= MAX_RULES {
                        break;
                    }
                    let Some(sel) = parse_selector(sel) else {
                        continue;
                    };
                    let Ok(idx) = u32::try_from(self.rules.len()) else {
                        break;
                    };
                    let index = match sel.parts.first() {
                        Some(Compound { id: Some(id), .. }) => {
                            self.by_id.entry(id.clone()).or_default()
                        }
                        Some(Compound { classes, .. }) if !classes.is_empty() => self
                            .by_class
                            .entry(classes.first().cloned().unwrap_or_default())
                            .or_default(),
                        Some(Compound { tag: Some(t), .. }) => {
                            self.by_tag.entry(t.clone()).or_default()
                        }
                        _ => &mut self.universal,
                    };
                    index.push(idx);
                    self.rules.push(Rule {
                        sel,
                        order: idx,
                        decls: start..stop,
                    });
                }
            }
            rest = rest.get(end.saturating_add(1)..).unwrap_or("");
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Matching declarations for an element, lowest priority first: normal
    /// author declarations by (specificity, order), then important ones.
    pub fn matched<'s>(
        &'s self,
        dom: &Dom,
        id: Id,
        normal: &mut Vec<&'s Decl>,
        important: &mut Vec<&'s Decl>,
    ) {
        if self.rules.is_empty() {
            return;
        }
        let mut cand: Vec<u32> = Vec::new();
        if let Some(r) = dom
            .attr(id, "id")
            .and_then(|v| self.by_id.get(v.trim().to_ascii_lowercase().as_str()))
        {
            cand.extend_from_slice(r);
        }
        if let Some(cls) = dom.attr(id, "class") {
            for c in cls.split_ascii_whitespace() {
                if let Some(r) = self.by_class.get(c.to_ascii_lowercase().as_str()) {
                    cand.extend_from_slice(r);
                }
            }
        }
        if let Some(r) = self.by_tag.get(dom.tag(id)) {
            cand.extend_from_slice(r);
        }
        cand.extend_from_slice(&self.universal);
        if cand.is_empty() {
            return;
        }
        cand.sort_unstable();
        cand.dedup();
        // One matching budget per element, shared by all its candidate
        // rules, so hostile selector sets cost a bounded time per node.
        let mut budget = MATCH_BUDGET;
        let mut hits: Vec<&Rule> = cand
            .iter()
            .filter_map(|&i| self.rules.get(i as usize))
            .filter(|r| matches_from(dom, id, &r.sel, 0, &mut budget))
            .collect();
        hits.sort_by_key(|r| (r.sel.spec, r.order));
        for r in hits {
            for d in self.decls.get(r.decls.clone()).unwrap_or(&[]) {
                if d.important {
                    important.push(d)
                } else {
                    normal.push(d)
                }
            }
        }
    }
}

fn elem_parent(dom: &Dom, id: Id) -> Option<Id> {
    let p = dom.parent(id);
    (p != 0 && p != id).then_some(p)
}

fn prev_elem(dom: &Dom, id: Id) -> Option<Id> {
    let ch = dom.children(dom.parent(id));
    // Children are appended in id order, so this stays cheap on huge lists.
    let pos = ch.binary_search(&id).ok()?;
    ch.get(..pos)?
        .iter()
        .rev()
        .find(|&&c| dom.is_elem(c))
        .copied()
}

fn compound_matches(dom: &Dom, id: Id, c: &Compound) -> bool {
    if c.tag.as_deref().is_some_and(|t| dom.tag(id) != t) {
        return false;
    }
    if let Some(want) = &c.id
        && !dom
            .attr(id, "id")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case(want))
    {
        return false;
    }
    if !c.classes.is_empty() {
        let Some(cls) = dom.attr(id, "class") else {
            return false;
        };
        let has = |want: &str| {
            cls.split_ascii_whitespace()
                .any(|x| x.eq_ignore_ascii_case(want))
        };
        if !c.classes.iter().all(|w| has(w)) {
            return false;
        }
    }
    for a in &c.attrs {
        let Some(v) = dom.attr(id, &a.name) else {
            return false;
        };
        let (v, w) = if a.ci {
            (v.to_ascii_lowercase(), a.val.to_ascii_lowercase())
        } else {
            (v.to_string(), a.val.to_string())
        };
        let ok = match a.op {
            AttrOp::Exists => true,
            AttrOp::Eq => v == w,
            AttrOp::Word => v.split_ascii_whitespace().any(|x| x == w),
            AttrOp::Prefix => !w.is_empty() && v.starts_with(&w),
            AttrOp::Suffix => !w.is_empty() && v.ends_with(&w),
            AttrOp::Contains => !w.is_empty() && v.contains(&w),
            AttrOp::Dash => v == w || v.strip_prefix(&w).is_some_and(|r| r.starts_with('-')),
        };
        if !ok {
            return false;
        }
    }
    if c.root && dom.tag(id) != "html" {
        return false;
    }
    if c.first_child || c.last_child {
        let kids = dom.children(dom.parent(id));
        if c.first_child && kids.iter().find(|&&k| dom.is_elem(k)) != Some(&id) {
            return false;
        }
        if c.last_child && kids.iter().rev().find(|&&k| dom.is_elem(k)) != Some(&id) {
            return false;
        }
    }
    true
}

/// Compound-selector tests allowed per element across all its rules.
const MATCH_BUDGET: u32 = 50_000;

/// Match `sel.parts[k..]` with `parts[k]` on `id`. `budget` bounds the
/// work of descendant and sibling combinators; every test is charged,
/// matching or not.
fn matches_from(dom: &Dom, id: Id, sel: &Selector, k: usize, budget: &mut u32) -> bool {
    let Some(part) = sel.parts.get(k) else {
        return false;
    };
    if *budget == 0 {
        return false;
    }
    *budget = budget.saturating_sub(1);
    if !compound_matches(dom, id, part) {
        return false;
    }
    let Some(&comb) = sel.comb.get(k) else {
        return true;
    };
    let k = k.saturating_add(1);
    match comb {
        Comb::Child => elem_parent(dom, id).is_some_and(|p| matches_from(dom, p, sel, k, budget)),
        Comb::Adjacent => prev_elem(dom, id).is_some_and(|p| matches_from(dom, p, sel, k, budget)),
        Comb::Descendant | Comb::Sibling => {
            let step = |x| {
                if comb == Comb::Descendant {
                    elem_parent(dom, x)
                } else {
                    prev_elem(dom, x)
                }
            };
            let mut cur = step(id);
            while let Some(p) = cur {
                if matches_from(dom, p, sel, k, budget) {
                    return true;
                }
                cur = step(p);
            }
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn quoted_font_does_not_drop_rest() {
        let mut v = vec![];
        parse_decls(
            "font-family:'Helvetica Neue',Arial;color:#FF0000;PADDING:0 !important;background:URL(Cid:X)",
            &mut v,
        );
        assert_eq!(v.len(), 4);
        assert_eq!(&*v[0].value, "'Helvetica Neue',arial");
        assert_eq!(&*v[1].value, "#ff0000");
        assert!(v[2].important);
        assert_eq!(&*v[3].value, "url(Cid:X)");
    }

    #[test]
    fn media() {
        let light = Ctx {
            width: 600.0,
            dark: false,
            dim: false,
        };
        let dark = Ctx {
            width: 600.0,
            dark: true,
            dim: false,
        };
        let mut d = false;
        assert!(media_matches(
            "only screen and (max-width: 600px)",
            &light,
            &mut d
        ));
        assert!(!media_matches(
            "screen and (min-width:601px)",
            &light,
            &mut d
        ));
        assert!(media_matches(
            "(prefers-color-scheme: light)",
            &light,
            &mut d
        ));
        assert!(!d);
        assert!(!media_matches(
            "(prefers-color-scheme: dark)",
            &light,
            &mut d
        ));
        assert!(d);
        assert!(media_matches("(prefers-color-scheme: dark)", &dark, &mut d));
        assert!(!media_matches("print", &light, &mut d));
    }

    #[test]
    fn spec() {
        let s = parse_selector("#a .b > td:first-child").unwrap();
        assert_eq!(s.spec, (1 << 16) | (2 << 8) | 1);
        assert!(parse_selector("a:hover").is_none());
        assert!(parse_selector("u + .body .x").is_some());
        assert!(parse_selector(&"a ".repeat(100)).is_none());
    }
}
