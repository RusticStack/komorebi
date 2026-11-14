//! Subjects, quotes and signature blocks of replies and forwards.

use super::{Heads, Kind};
use crate::db;
use crate::mail::{self, mime};

/// The line a forward's quoted message starts with in English (the apps
/// write it in the language shown, `Heads::forwarded`).
pub const FORWARDED: &str = "---------- Forwarded message ----------";

/// A signature longer than this many lines is taken for something else.
const MAX_SIGNATURE_LINES: usize = 10;

/// A forward's heading line in any language: `---------- ... ----------`.
pub(super) fn is_forwarded_line(line: &str) -> bool {
    let line = line.trim_end();
    line.len() > 22 && line.starts_with("---------- ") && line.ends_with(" ----------")
}

pub fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    if s.get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    {
        s.to_string()
    } else {
        format!("{prefix} {s}")
    }
}

/// The block a signature goes into the body as: two blank lines, the
/// RFC 3676 delimiter `-- ` (with its trailing space) and the signature.
/// Empty for no signature.
pub fn signature_block(signature: &str) -> String {
    let sig = signature.replace("\r\n", "\n");
    let sig = sig.trim_end();
    // A delimiter typed into the signature itself is not doubled.
    let sig = sig
        .strip_prefix("-- \n")
        .or_else(|| sig.strip_prefix("--\n"))
        .unwrap_or(sig);
    if sig.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n-- \n{sig}")
    }
}

/// `body` with signature block `old` swapped for `new` (either may be
/// empty). `None` leaves the body alone: the block was edited, or with no
/// signature before, text was typed where the new one would go (above the
/// quote of a reply or forward).
pub fn swap_signature(body: &str, kind: Kind, old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    if !old.is_empty() {
        let i = body.find(old)?;
        return Some(format!("{}{new}{}", &body[..i], &body[i + old.len()..]));
    }
    match kind {
        Kind::New => Some(format!("{}{new}", body.trim_end())),
        _ if body.is_empty() || body.starts_with("\n\n") => Some(format!("{new}{body}")),
        _ => None,
    }
}

/// A sender as a quote names it: the display name, else the address.
fn sender(name: &str, addr: &str) -> String {
    if name.trim().is_empty() {
        addr.to_string()
    } else {
        name.trim().to_string()
    }
}

/// The line a reply's quote starts with, in the language shown.
pub fn attribution(m: &db::Message, heads: &Heads) -> String {
    (heads.wrote)(
        &(heads.when)(heads.lang, m.date),
        &sender(&m.from_name, &m.from_addr),
    )
}

/// The quote below a reply or forward of `m` (`original`: its sanitized
/// HTML), and the line it starts with in the text quote. `None` for a new
/// message.
pub fn quote_block(
    kind: Kind,
    m: &db::Message,
    original: &str,
    heads: &Heads,
) -> Option<(String, String)> {
    use mail::compose_html;
    Some(match kind {
        Kind::New => return None,
        Kind::Reply | Kind::ReplyAll => {
            let line = attribution(m, heads);
            let block = compose_html::reply_quote(&line, original);
            (line, block)
        }
        Kind::Forward => {
            let from = format!("{} <{}>", m.from_name, m.from_addr);
            let date = (heads.when)(heads.lang, m.date);
            let fields = [
                (heads.from.as_str(), from.trim()),
                (heads.date.as_str(), date.as_str()),
                (heads.to.as_str(), m.to.as_str()),
                (heads.subject.as_str(), m.subject.as_str()),
            ];
            let block = compose_html::forward_quote(&heads.forwarded, &fields, original);
            (heads.forwarded.clone(), block)
        }
    })
}

/// Text as HTML that shows it as it is: escaped, each line break kept.
pub fn text_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push_str("<br>\n");
        }
        for c in line.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
    }
    out
}

/// The sanitized HTML body of raw message `raw`, for a reply or forward to
/// quote (`None`: a text message, or nothing safe to quote).
pub fn quotable(raw: Option<&[u8]>) -> Option<String> {
    mail::compose_html::sanitize(&mime::html_body(raw?)?)
}

/// `text` without its trailing signature (the last `-- ` line, RFC 3676,
/// and what follows, if that is at most a few lines), for quoting.
pub fn strip_signature(text: &str) -> &str {
    let mut cut = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "-- " {
            cut = Some(at);
        }
        at += line.len();
    }
    match cut {
        Some(i) if text[i..].lines().count() <= MAX_SIGNATURE_LINES + 1 => text[..i].trim_end(),
        _ => text,
    }
}

pub fn quote(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.starts_with('>') {
                format!(">{l}")
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
