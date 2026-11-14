//! A small reader of gettext `.po` files, shared by this crate's build
//! script (which turns the catalogs into a static table) and the root
//! package's `tests/i18n_catalog.rs` (which enforces the catalog rules), so
//! both read a catalog the same way. No dependencies.
//!
//! Supported: `msgctxt`, `msgid`, `msgid_plural`, `msgstr`, `msgstr[i]`,
//! continuation lines, the escapes `\n \t \" \\`, the `#, fuzzy` flag.
//! Obsolete entries (`#~`) are refused.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// One message: `ctx` (msgctxt), `id` (msgid) and, for counts, `plural`
/// (msgid_plural).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Msg {
    pub ctx: Option<String>,
    pub id: String,
    pub plural: Option<String>,
}

#[derive(Default, Debug)]
pub struct Entry {
    pub msg: Option<Msg>,
    pub ctx: Option<String>,
    pub id: Option<String>,
    pub plural: Option<String>,
    /// `msgstr` (index 0) or `msgstr[i]`.
    pub strs: BTreeMap<usize, String>,
    pub fuzzy: bool,
    /// The line of its first keyword.
    pub line: usize,
}

pub struct Po {
    /// The header entry's `msgstr`.
    pub header: String,
    pub entries: Vec<Entry>,
}

fn unquote(s: &str, line: usize) -> String {
    let s = s.trim();
    assert!(
        s.starts_with('"') && s.ends_with('"') && s.len() >= 2,
        "po line {line}: not a quoted string: {s}"
    );
    let mut out = String::new();
    let mut it = s[1..s.len() - 1].chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => panic!("po line {line}: unknown escape {other:?}"),
        }
    }
    out
}

/// Parses a whole `.po` file (line ends already `\n`). Panics with the line
/// on anything malformed.
pub fn parse_po(text: &str) -> Po {
    #[derive(Clone, Copy)]
    enum Field {
        Ctx,
        Id,
        Plural,
        Str(usize),
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut cur = Entry::default();
    let mut field: Option<Field> = None;
    let mut started = false;
    let mut flush = |cur: &mut Entry, started: &mut bool| {
        if *started {
            let mut done = std::mem::take(cur);
            done.msg = Some(Msg {
                ctx: done.ctx.clone(),
                id: done.id.clone().expect("an entry without msgid"),
                plural: done.plural.clone(),
            });
            entries.push(done);
        }
        *started = false;
    };
    for (n, raw) in text.lines().enumerate() {
        let n = n + 1;
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut cur, &mut started);
            field = None;
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            if comment.starts_with(',') && comment.contains("fuzzy") {
                cur.fuzzy = true;
                started = true;
            }
            assert!(
                !comment.starts_with('~'),
                "po line {n}: obsolete entries (#~) are not kept"
            );
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let target = match key {
            "msgctxt" => Some(Field::Ctx),
            "msgid" => Some(Field::Id),
            "msgid_plural" => Some(Field::Plural),
            "msgstr" => Some(Field::Str(0)),
            k if k.starts_with("msgstr[") => {
                let idx = k["msgstr[".len()..k.len() - 1]
                    .parse()
                    .expect("msgstr index");
                Some(Field::Str(idx))
            }
            _ => None,
        };
        let value = match target {
            Some(t) => {
                field = Some(t);
                started = true;
                cur.line = if cur.line == 0 { n } else { cur.line };
                unquote(rest, n)
            }
            None => unquote(line, n),
        };
        match field.expect("a continuation line before any keyword") {
            Field::Ctx => cur.ctx.get_or_insert_with(String::new).push_str(&value),
            Field::Id => cur.id.get_or_insert_with(String::new).push_str(&value),
            Field::Plural => cur.plural.get_or_insert_with(String::new).push_str(&value),
            Field::Str(i) => cur.strs.entry(i).or_default().push_str(&value),
        }
    }
    flush(&mut cur, &mut started);
    assert!(
        !entries.is_empty(),
        "a .po file without even its header entry"
    );
    let header = entries.remove(0);
    assert_eq!(
        header.id.as_deref(),
        Some(""),
        "the first entry must be the header (empty msgid)"
    );
    Po {
        header: header.strs.get(&0).cloned().unwrap_or_default(),
        entries,
    }
}

/// The plural rule of a header's `Plural-Forms`, as the name of a
/// `komorebi_i18n::Plural` variant. Only the rules Komorebi's languages need
/// are known; another is an error that names the header.
pub fn plural_rule(header: &str) -> Result<&'static str, String> {
    let line = header
        .lines()
        .find_map(|l| l.trim().strip_prefix("Plural-Forms:"))
        .ok_or("no Plural-Forms header")?;
    let squeezed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let expr = squeezed
        .split(';')
        .find_map(|part| part.strip_prefix("plural="))
        .ok_or_else(|| format!("no plural= in Plural-Forms: {line}"))?;
    let expr = expr.trim_start_matches('(').trim_end_matches(')');
    match expr {
        "n!=1" => Ok("NotOne"),
        "n>1" => Ok("MoreThanOne"),
        "0" => Ok("One"),
        _ => Err(format!(
            "unknown plural rule {expr:?} (known: n != 1, n > 1, 0); add it to po.rs and komorebi_i18n::Plural"
        )),
    }
}
