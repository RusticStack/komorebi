//! vCard (RFC 2426 3.0, RFC 6350 4.0, and 2.1 read best effort): one card
//! read into a [`Contact`], a new card written, and a stored card patched
//! with the fields Komorebi owns while every other line stays verbatim.
//!
//! The input comes from servers, so this module denies the lints below (as
//! `render/` and `ical.rs` do), refuses a card over [`MAX_BYTES`],
//! [`MAX_LINES`] lines or [`MAX_PROPS`] properties, and returns an error
//! for anything it can't read: the sync skips that card and counts it.
//! The content-line code (unfolding, parameters, escapes) is a copy of the
//! idea in `ical.rs`, not shared with it, so the two can change apart.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use crate::contacts::{Contact, Email, MAX_EMAILS, MAX_PHONES, Phone};

/// Larger cards are skipped.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Physical lines read at most.
pub const MAX_LINES: usize = 5_000;
/// Properties (unfolded lines) read at most.
pub const MAX_PROPS: usize = 2_000;
/// Members of one group read at most.
const MAX_MEMBERS: usize = 5_000;

/// Why a card was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    TooLarge,
    Malformed,
}

/// One card as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    /// 2, 3 or 4.
    pub version: u8,
    pub contact: Contact,
    /// `KIND:group` (4.0) or `X-ADDRESSBOOKSERVER-KIND:group` (3.0).
    pub group: bool,
    /// The members' UIDs (`urn:uuid:` removed), for a group.
    pub members: Vec<String>,
}

/// One unfolded property.
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The `TYPE` values, lower-cased (2.1 bare parameters count as types).
    fn types(&self) -> Vec<String> {
        self.params
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("type") || n.is_empty())
            .flat_map(|(_, v)| v.split(','))
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
}

/// The logical lines of `raw`, each with the byte range of its physical
/// lines, so a patch can keep the lines it does not own verbatim.
/// Unfolded lines, each with its physical bytes.
type Lines<'a> = Vec<(Vec<u8>, &'a [u8])>;

fn logical_lines(raw: &[u8]) -> Result<Lines<'_>, Skip> {
    if raw.len() > MAX_BYTES {
        return Err(Skip::TooLarge);
    }
    let mut out: Vec<(Vec<u8>, usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut count = 0usize;
    let mut qp_open = false;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        count = count.saturating_add(1);
        if count > MAX_LINES {
            return Err(Skip::TooLarge);
        }
        let end = start.saturating_add(line.len());
        let text = trim_eol(line);
        let continues = matches!(text.first(), Some(b' ' | b'\t'));
        match out.last_mut() {
            Some((buf, _, e)) if continues => {
                buf.extend_from_slice(text.get(1..).unwrap_or_default());
                *e = end;
            }
            Some((buf, _, e)) if qp_open => {
                // 2.1 quoted-printable soft break: `=` at the end joins.
                buf.pop();
                buf.extend_from_slice(text);
                *e = end;
            }
            _ if text.is_empty() => {}
            _ => out.push((text.to_vec(), start, end)),
        }
        qp_open = out.last().is_some_and(|(buf, _, _)| {
            buf.ends_with(b"=") && {
                let head = buf.iter().position(|b| *b == b':').unwrap_or(buf.len());
                buf.get(..head).is_some_and(|h| {
                    h.to_ascii_uppercase()
                        .windows(16)
                        .any(|w| w == b"QUOTED-PRINTABLE")
                })
            }
        });
        start = end;
        if out.len() > MAX_PROPS {
            return Err(Skip::TooLarge);
        }
    }
    Ok(out
        .into_iter()
        .map(|(buf, s, e)| (buf, raw.get(s..e).unwrap_or_default()))
        .collect())
}

fn trim_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// `group.NAME;P=v;P2="a,b":value`, the name upper-cased without its group.
fn parse_line(line: &[u8]) -> Option<Prop> {
    let text = String::from_utf8_lossy(line);
    let mut in_quote = false;
    let mut colon = None;
    for (i, c) in text.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ':' if !in_quote => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = text.get(..colon)?;
    let value = text.get(colon.checked_add(1)?..)?.to_string();
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?;
    let name = name
        .rsplit('.')
        .next()
        .unwrap_or(&name)
        .to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .map(|p| match p.split_once('=') {
            Some((n, v)) => (
                n.trim().to_ascii_uppercase(),
                caret_unescape(v.trim().trim_matches('"')),
            ),
            None => (String::new(), p.trim().to_string()),
        })
        .collect();
    Some(Prop {
        name,
        params,
        value,
    })
}

fn split_unquoted(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == sep && !quoted {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// RFC 6868 parameter escapes.
fn caret_unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c == '^' {
            match it.peek() {
                Some('n' | 'N') => out.push('\n'),
                Some('^') => out.push('^'),
                Some('\'') => out.push('"'),
                _ => {
                    out.push('^');
                    continue;
                }
            }
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// Text value escapes (`\n`, `\,`, `\;`, `\\`), split on unescaped `sep`.
fn unescape_split(v: &str, sep: Option<char>) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n' | 'N') => cur.push('\n'),
                Some(o) => cur.push(o),
                None => {}
            },
            c if Some(c) == sep => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unescape(v: &str) -> String {
    unescape_split(v, None).concat()
}

/// A 2.1 value decoded per its ENCODING and CHARSET parameters.
fn decoded(p: &Prop) -> String {
    let qp = p
        .param("ENCODING")
        .is_some_and(|e| e.eq_ignore_ascii_case("quoted-printable"))
        || p.params
            .iter()
            .any(|(n, v)| n.is_empty() && v.eq_ignore_ascii_case("quoted-printable"));
    if !qp {
        return p.value.clone();
    }
    let mut bytes = Vec::with_capacity(p.value.len());
    let src = p.value.as_bytes();
    let mut i = 0usize;
    while let Some(&b) = src.get(i) {
        let hex = src
            .get(i.saturating_add(1)..i.saturating_add(3))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'=', Some(v)) => {
                bytes.push(v);
                i = i.saturating_add(3);
            }
            _ => {
                bytes.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    let latin1 = p.param("CHARSET").is_some_and(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("8859-1") || c.contains("1252") || c == "latin1"
    });
    if latin1 {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

const LABELS: [&str; 6] = ["work", "home", "cell", "mobile", "fax", "other"];

fn label_of(types: &[String]) -> String {
    types
        .iter()
        .find(|t| LABELS.contains(&t.as_str()))
        .cloned()
        .unwrap_or_default()
}

fn is_pref(p: &Prop, types: &[String]) -> bool {
    types.iter().any(|t| t == "pref") || p.param("PREF").is_some_and(|v| v.trim() == "1")
}

/// Read one card.
pub fn parse(raw: &[u8]) -> Result<Card, Skip> {
    let lines = logical_lines(raw)?;
    let mut card = Card::default();
    let mut began = false;
    let mut depth = 0u8;
    for (line, _) in &lines {
        let Some(p) = parse_line(line) else {
            continue;
        };
        match p.name.as_str() {
            "BEGIN" if p.value.trim().eq_ignore_ascii_case("vcard") => {
                if began {
                    // A nested or second card: read only the first.
                    depth = depth.saturating_add(1);
                }
                began = true;
                continue;
            }
            "END" if p.value.trim().eq_ignore_ascii_case("vcard") => {
                if depth == 0 {
                    break;
                }
                depth = depth.saturating_sub(1);
                continue;
            }
            _ if !began || depth > 0 => continue,
            _ => {}
        }
        read_prop(&mut card, &p);
    }
    if !began {
        return Err(Skip::Malformed);
    }
    if card.version == 0 {
        card.version = 3;
    }
    if card.contact.uid.is_empty() && card.contact.display().is_empty() {
        return Err(Skip::Malformed);
    }
    Ok(card)
}

fn read_prop(card: &mut Card, p: &Prop) {
    let c = &mut card.contact;
    let text = || unescape(decoded(p).trim());
    match p.name.as_str() {
        "VERSION" => {
            card.version = match p.value.trim() {
                "4.0" => 4,
                "2.1" => 2,
                _ => 3,
            }
        }
        "UID" => c.uid = p.value.trim().to_string(),
        "FN" => c.name = text(),
        "N" => {
            let parts = unescape_split(decoded(p).trim(), Some(';'));
            let mut it = parts.into_iter();
            c.family = it.next().unwrap_or_default().trim().to_string();
            c.given = it.next().unwrap_or_default().trim().to_string();
        }
        "ORG" => {
            c.org = unescape_split(decoded(p).trim(), Some(';'))
                .into_iter()
                .next()
                .unwrap_or_default();
        }
        "TITLE" => c.title = text(),
        "NOTE" => c.note = text(),
        "EMAIL" if c.emails.len() < MAX_EMAILS => {
            let addr = text().to_lowercase();
            if !addr.is_empty() {
                let types = p.types();
                c.emails.push(Email {
                    pref: is_pref(p, &types),
                    label: label_of(&types),
                    addr,
                });
            }
        }
        "TEL" if c.phones.len() < MAX_PHONES => {
            let number = text();
            let number = number.strip_prefix("tel:").unwrap_or(&number).to_string();
            if !number.is_empty() {
                c.phones.push(Phone {
                    label: label_of(&p.types()),
                    number,
                });
            }
        }
        "CATEGORIES" => {
            for g in unescape_split(decoded(p).trim(), Some(',')) {
                let g = g.trim();
                if !g.is_empty() && !c.groups.iter().any(|o| o == g) {
                    c.groups.push(g.to_string());
                }
            }
        }
        "KIND" | "X-ADDRESSBOOKSERVER-KIND" => {
            card.group = p.value.trim().eq_ignore_ascii_case("group");
        }
        "MEMBER" | "X-ADDRESSBOOKSERVER-MEMBER" if card.members.len() < MAX_MEMBERS => {
            let v = p.value.trim();
            let uid = v
                .strip_prefix("urn:uuid:")
                .or_else(|| v.strip_prefix("URN:UUID:"))
                .unwrap_or(v);
            if !uid.is_empty() {
                card.members.push(uid.to_string());
            }
        }
        _ => {}
    }
}

/// The properties a patch replaces; every other line is kept verbatim.
const OWNED: [&str; 9] = [
    "FN",
    "N",
    "EMAIL",
    "TEL",
    "ORG",
    "TITLE",
    "NOTE",
    "CATEGORIES",
    "REV",
];

fn escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// `line` folded at 75 octets (on character boundaries) with CRLF.
fn push_folded(out: &mut String, line: &str) {
    let mut width = 0usize;
    for c in line.chars() {
        let w = c.len_utf8();
        if width.saturating_add(w) > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width = width.saturating_add(w);
    }
    out.push_str("\r\n");
}

/// The owned properties of `c`, in `version`'s syntax.
fn owned_lines(c: &Contact, version: u8, rev: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("FN:{}", escape(&c.display())));
    out.push(format!("N:{};{};;;", escape(&c.family), escape(&c.given)));
    if !c.org.is_empty() {
        out.push(format!("ORG:{}", escape(&c.org)));
    }
    if !c.title.is_empty() {
        out.push(format!("TITLE:{}", escape(&c.title)));
    }
    for e in &c.emails {
        let mut params = String::new();
        if !e.label.is_empty() {
            params.push_str(";TYPE=");
            params.push_str(&e.label);
        }
        if e.pref {
            params.push_str(if version >= 4 {
                ";PREF=1"
            } else {
                ";TYPE=pref"
            });
        }
        if version < 4 {
            params.insert_str(0, ";TYPE=INTERNET");
        }
        out.push(format!("EMAIL{params}:{}", escape(&e.addr)));
    }
    for p in &c.phones {
        let params = if p.label.is_empty() {
            String::new()
        } else {
            format!(";TYPE={}", p.label)
        };
        out.push(format!("TEL{params}:{}", escape(&p.number)));
    }
    if !c.note.is_empty() {
        out.push(format!("NOTE:{}", escape(&c.note)));
    }
    if !c.groups.is_empty() {
        let list: Vec<String> = c.groups.iter().map(|g| escape(g)).collect();
        out.push(format!("CATEGORIES:{}", list.join(",")));
    }
    if !rev.is_empty() {
        out.push(format!("REV:{rev}"));
    }
    out
}

/// A new card for `c` in vCard `version` (3 or 4).
pub fn write(c: &Contact, version: u8, rev: &str) -> String {
    let version = if version >= 4 { 4 } else { 3 };
    let mut out = String::new();
    push_folded(&mut out, "BEGIN:VCARD");
    push_folded(
        &mut out,
        if version == 4 {
            "VERSION:4.0"
        } else {
            "VERSION:3.0"
        },
    );
    push_folded(&mut out, &format!("UID:{}", c.uid));
    for l in owned_lines(c, version, rev) {
        push_folded(&mut out, &l);
    }
    push_folded(&mut out, "END:VCARD");
    out
}

/// `raw` with the owned properties replaced by `c`'s: every other line,
/// `PHOTO` and `X-` lines included, stays byte for byte, and the version
/// stays the card's own. `None` when `raw` can't be read.
pub fn patch(raw: &[u8], c: &Contact, rev: &str) -> Option<String> {
    let card = parse(raw).ok()?;
    let version = if card.version >= 4 { 4 } else { 3 };
    let lines = logical_lines(raw).ok()?;
    let mut out = String::new();
    let mut depth = 0u8;
    for (line, physical) in &lines {
        let p = parse_line(line);
        let name = p.as_ref().map(|p| p.name.as_str()).unwrap_or("");
        let value = p.as_ref().map(|p| p.value.trim()).unwrap_or("");
        if name == "BEGIN" && value.eq_ignore_ascii_case("vcard") {
            depth = depth.saturating_add(1);
        }
        if name == "END" && value.eq_ignore_ascii_case("vcard") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                for l in owned_lines(c, version, rev) {
                    push_folded(&mut out, &l);
                }
            }
        }
        if depth == 1 && OWNED.contains(&name) {
            continue;
        }
        let text = String::from_utf8_lossy(physical);
        out.push_str(&text);
        if !text.ends_with('\n') {
            out.push_str("\r\n");
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    const APPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iOS//EN\r\nN:Concei\
ção;Ana;;;\r\nFN:Ana Conceição\r\nORG:Mutual;Dev\r\nitem1.EMAIL;type=INTERNET;type=WORK;type=pref:\
ana@example.pt\r\nEMAIL;TYPE=HOME:ana.home@example.org\r\nTEL;TYPE=CELL:+351 912 345 678\r\nNOTE:Line one\\nline\\, two\r\n\
CATEGORIES:Family,Work\r\nPHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\n BBBB\r\nX-CUSTOM;PARAM=\"a^'b\":keep me\r\nUID:abc-1\r\nEND:VCARD\r\n";

    #[test]
    fn reads_apple_3() {
        let c = parse(APPLE.as_bytes()).unwrap();
        assert_eq!(c.version, 3);
        let p = &c.contact;
        assert_eq!(p.name, "Ana Conceição");
        assert_eq!((p.family.as_str(), p.given.as_str()), ("Conceição", "Ana"));
        assert_eq!(p.org, "Mutual");
        assert_eq!(p.emails.len(), 2);
        assert!(p.emails[0].pref);
        assert_eq!(p.emails[0].label, "work");
        assert_eq!(p.phones[0].label, "cell");
        assert_eq!(p.note, "Line one\nline, two");
        assert_eq!(p.groups, ["Family", "Work"]);
        assert_eq!(p.uid, "abc-1");
    }

    #[test]
    fn reads_groups_4_and_apple() {
        let v4 = "BEGIN:VCARD\nVERSION:4.0\nKIND:group\nFN:Team\nUID:g1\nMEMBER:urn:uuid:a\nMEMBER:urn:uuid:b\nEND:VCARD\n";
        let c = parse(v4.as_bytes()).unwrap();
        assert!(c.group);
        assert_eq!(c.members, ["a", "b"]);
        let apple = "BEGIN:VCARD\nVERSION:3.0\nX-ADDRESSBOOKSERVER-KIND:group\nFN:Team\nUID:g2\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:a\nEND:VCARD\n";
        let c = parse(apple.as_bytes()).unwrap();
        assert!(c.group);
        assert_eq!(c.members, ["a"]);
    }

    #[test]
    fn reads_folded_multibyte_and_qp() {
        // A fold in the middle of "ç" (two bytes) joins back before UTF-8.
        let mut raw = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Concei\xc3".to_vec();
        raw.extend_from_slice(b"\r\n \xa7\xc3\xa3o\r\nEND:VCARD\r\n");
        assert_eq!(parse(&raw).unwrap().contact.name, "Conceição");
        let qp = "BEGIN:VCARD\r\nVERSION:2.1\r\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Jo=C3=A3o =\r\nSilva\r\nEMAIL;INTERNET;WORK:j@x.pt\r\nEND:VCARD\r\n";
        let c = parse(qp.as_bytes()).unwrap();
        assert_eq!(c.version, 2);
        assert_eq!(c.contact.name, "João Silva");
        assert_eq!(c.contact.emails[0].label, "work");
        let caret = "BEGIN:VCARD\nVERSION:4.0\nFN:x\nEMAIL;TYPE=\"wo^'rk\":a@b.pt\nEND:VCARD\n";
        assert!(parse(caret.as_bytes()).is_ok());
    }

    #[test]
    fn caps_and_garbage() {
        let mut big = String::from("BEGIN:VCARD\nFN:x\nPHOTO:");
        big.push_str(&"A".repeat(MAX_BYTES));
        assert_eq!(parse(big.as_bytes()), Err(Skip::TooLarge));
        assert_eq!(parse(b"hello"), Err(Skip::Malformed));
        let many = format!(
            "BEGIN:VCARD\n{}END:VCARD\n",
            "X-A:1\n".repeat(MAX_PROPS + 1)
        );
        assert_eq!(parse(many.as_bytes()), Err(Skip::TooLarge));
        // Deterministic mutations never panic.
        let base = APPLE.as_bytes();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..10_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut m = base.to_vec();
            let at = (seed as usize) % m.len();
            match seed % 4 {
                0 => m[at] = (seed >> 8) as u8,
                1 => {
                    m.remove(at);
                }
                2 => m.insert(at, b"\n:;\\=\"^"[(seed >> 16) as usize % 7]),
                _ => m.truncate(at),
            }
            let _ = parse(&m);
            if let Ok(c) = parse(&m) {
                let _ = patch(&m, &c.contact, "");
            }
        }
    }

    #[test]
    fn write_and_patch_round_trip() {
        let mut card = parse(APPLE.as_bytes()).unwrap();
        card.contact.title = "Engineer; lead".into();
        card.contact.emails.truncate(1);
        card.contact.groups = vec!["Work".into()];
        let out = patch(APPLE.as_bytes(), &card.contact, "20261015T120000Z").unwrap();
        // Unknown lines survive byte for byte, the owned ones are replaced.
        assert!(out.contains("PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\n BBBB\r\n"));
        assert!(out.contains("X-CUSTOM;PARAM=\"a^'b\":keep me\r\n"));
        assert!(out.contains("PRODID:-//Apple Inc.//iOS//EN\r\n"));
        assert!(!out.contains("ana.home@example.org"));
        let again = parse(out.as_bytes()).unwrap();
        assert_eq!(again.version, 3);
        assert_eq!(again.contact.title, "Engineer; lead");
        assert_eq!(again.contact.groups, ["Work"]);
        assert_eq!(again.contact.emails.len(), 1);

        let fresh = write(&card.contact, 4, "");
        let back = parse(fresh.as_bytes()).unwrap();
        assert_eq!(back.version, 4);
        assert_eq!(back.contact.name, card.contact.name);
        assert_eq!(back.contact.note, card.contact.note);
        assert!(fresh.lines().all(|l| l.len() <= 76));
    }
}
