//! Guards the message catalog: every user-facing string in `ui/*.slint` goes
//! through `@tr(...)`, `translations/komorebi.pot` lists exactly those
//! strings, and each language's `.po` file is well formed, current and free
//! of Brazilian wording. No dependencies: a small `@tr` scanner and `.po`
//! parser live here.
//!
//! Regenerate the template after changing a string:
//! `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const UI_DIR: &str = "ui";
const TRANSLATIONS_DIR: &str = "translations";
const POT: &str = "translations/komorebi.pot";
const ALLOWLIST: &str = "translations/untranslated-allowlist.txt";

/// One `@tr(...)` call: `@tr("ctx" => "id" | "plural" % n, args...)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Msg {
    ctx: Option<String>,
    id: String,
    plural: Option<String>,
}

/// What a scan of one `.slint` file found.
#[derive(Default)]
struct Scan {
    /// The `@tr` calls with the line they start on.
    msgs: Vec<(Msg, usize)>,
    /// String literals outside any `@tr`, unescaped, with their line.
    bare: Vec<(String, usize)>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

fn slint_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(root().join(UI_DIR))
        .expect("ui directory")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "slint"))
        .collect();
    files.sort();
    files
}

/// The unescaped text of a Slint string literal starting at `chars[i]` (a
/// quote); returns it with the index after the closing quote.
fn string_literal(chars: &[char], mut i: usize, what: &str) -> (String, usize) {
    assert_eq!(chars[i], '"', "{what}: expected a string literal");
    i += 1;
    let mut out = String::new();
    while chars[i] != '"' {
        if chars[i] == '\\' {
            i += 1;
            match chars[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'u' => {
                    assert_eq!(chars[i + 1], '{', "{what}: bad \\u escape");
                    let end = (i..chars.len())
                        .find(|&j| chars[j] == '}')
                        .expect("\\u{..}");
                    let hex: String = chars[i + 2..end].iter().collect();
                    let code = u32::from_str_radix(&hex, 16).expect("hex in \\u{..}");
                    out.push(char::from_u32(code).expect("a scalar value in \\u{..}"));
                    i = end;
                }
                '{' => panic!("{what}: a string interpolation inside a string is not supported"),
                other => panic!("{what}: unknown escape \\{other}"),
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    (out, i + 1)
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn scan_slint(text: &str, name: &str) -> Scan {
    let chars: Vec<char> = text.chars().collect();
    let mut scan = Scan::default();
    let (mut i, mut line) = (0, 1);
    // One entry per open parenthesis: whether it belongs to an `@tr(`.
    let mut parens: Vec<bool> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\n' => line += 1,
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if next == Some('*') => {
                i += 2;
                while !(chars[i] == '*' && chars[i + 1] == '/') {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
                continue;
            }
            '@' if chars[i..].starts_with(&['@', 't', 'r', '(']) => {
                let at = line;
                let what = format!("{name}:{at}");
                i = skip_space(&chars, i + 4);
                let (first, after) = string_literal(&chars, i, &what);
                i = skip_space(&chars, after);
                let (ctx, id) = if chars[i] == '=' && chars[i + 1] == '>' {
                    i = skip_space(&chars, i + 2);
                    let (id, after) = string_literal(&chars, i, &what);
                    i = skip_space(&chars, after);
                    (Some(first), id)
                } else {
                    (None, first)
                };
                let mut plural = None;
                if chars[i] == '|' {
                    i = skip_space(&chars, i + 1);
                    let (p, after) = string_literal(&chars, i, &what);
                    i = skip_space(&chars, after);
                    plural = Some(p);
                }
                // The count and the arguments follow; their closing
                // parenthesis is found by the main loop.
                parens.push(true);
                scan.msgs.push((Msg { ctx, id, plural }, at));
                continue;
            }
            '(' => parens.push(false),
            ')' => {
                parens.pop();
            }
            '"' => {
                let (lit, after) = string_literal(&chars, i, &format!("{name}:{line}"));
                if !parens.contains(&true) {
                    scan.bare.push((lit.clone(), line));
                }
                line += lit.matches('\n').count();
                i = after;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    scan
}

/// Every `@tr` call of the UI, deduplicated, in source order, with the
/// files that use each.
fn ui_messages() -> Vec<(Msg, Vec<String>)> {
    let mut order: Vec<(Msg, Vec<String>)> = Vec::new();
    for file in slint_files() {
        let name = format!("{UI_DIR}/{}", file.file_name().unwrap().to_string_lossy());
        for (msg, _) in scan_slint(&read(&file), &name).msgs {
            match order.iter_mut().find(|(m, _)| *m == msg) {
                Some((_, files)) => {
                    if !files.contains(&name) {
                        files.push(name.clone());
                    }
                }
                None => order.push((msg, vec![name.clone()])),
            }
        }
    }
    order
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn template() -> String {
    let mut out = String::from(
        "# Komorebi message catalog (template). Generated from the @tr() calls in\n\
         # ui/*.slint: run `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`.\n\
         # Add a language as translations/<tag>/LC_MESSAGES/komorebi.po.\n\
         msgid \"\"\n\
         msgstr \"\"\n\
         \"Project-Id-Version: komorebi\\n\"\n\
         \"MIME-Version: 1.0\\n\"\n\
         \"Content-Type: text/plain; charset=UTF-8\\n\"\n\
         \"Content-Transfer-Encoding: 8bit\\n\"\n",
    );
    for (msg, files) in ui_messages() {
        out.push_str(&format!("\n#: {}\n", files.join(" ")));
        if let Some(ctx) = &msg.ctx {
            out.push_str(&format!("msgctxt {}\n", quote(ctx)));
        }
        out.push_str(&format!("msgid {}\n", quote(&msg.id)));
        match &msg.plural {
            Some(plural) => {
                out.push_str(&format!("msgid_plural {}\n", quote(plural)));
                out.push_str("msgstr[0] \"\"\nmsgstr[1] \"\"\n");
            }
            None => out.push_str("msgstr \"\"\n"),
        }
    }
    out
}

// ---- the .po reader -------------------------------------------------------

#[derive(Default, Debug)]
struct Entry {
    msg: Option<Msg>,
    ctx: Option<String>,
    id: Option<String>,
    plural: Option<String>,
    strs: BTreeMap<usize, String>,
    fuzzy: bool,
    line: usize,
}

struct Po {
    header: String,
    entries: Vec<Entry>,
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

fn parse_po(text: &str) -> Po {
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

fn po_files() -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for dir in fs::read_dir(root().join(TRANSLATIONS_DIR)).expect("translations directory") {
        let dir = dir.expect("directory entry").path();
        let po = dir.join("LC_MESSAGES").join("komorebi.po");
        if po.is_file() {
            found.push((dir.file_name().unwrap().to_string_lossy().into_owned(), po));
        }
    }
    found.sort();
    assert!(
        !found.is_empty(),
        "no translations/<tag>/LC_MESSAGES/komorebi.po"
    );
    found
}

/// The placeholders of a format string, as tokens: `{}`, `{0}`, `{n}`, and
/// the escapes `{{` and `}}`.
fn placeholders(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                out.push("{{".into());
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                out.push("}}".into());
                i += 2;
            }
            '{' => {
                let end = (i..chars.len())
                    .find(|&j| chars[j] == '}')
                    .unwrap_or(chars.len() - 1);
                out.push(chars[i..=end].iter().collect());
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    out.sort();
    out
}

fn label(msg: &Msg) -> String {
    match &msg.ctx {
        Some(ctx) => format!("[{ctx}] {:?}", msg.id),
        None => format!("{:?}", msg.id),
    }
}

// ---- tests ----------------------------------------------------------------

#[test]
fn template_is_current() {
    let want = template();
    if std::env::var_os("KOMOREBI_UPDATE_POT").is_some() {
        fs::write(root().join(POT), &want).expect("writing the template");
        return;
    }
    let have = read(&root().join(POT));
    assert!(
        have == want,
        "{POT} is out of date with the @tr() calls in {UI_DIR}/*.slint; run\n  KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template"
    );
}

#[test]
fn every_slint_string_is_translatable() {
    let allowed: BTreeSet<String> = read(&root().join(ALLOWLIST))
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.replace("\\u{a0}", "\u{a0}"))
        .collect();
    let mut used = BTreeSet::new();
    let mut unwrapped = Vec::new();
    for file in slint_files() {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        for (lit, line) in scan_slint(&read(&file), &name).bare {
            // Only text: it has a run of two letters (not "b", "993", "|" or
            // an icon code point); import paths are not shown.
            let chars: Vec<char> = lit.chars().collect();
            let text = chars
                .windows(2)
                .any(|w| w[0].is_alphabetic() && w[1].is_alphabetic());
            if !text || lit.ends_with(".slint") {
                continue;
            }
            if allowed.contains(&lit) {
                used.insert(lit);
            } else {
                unwrapped.push(format!("{name}:{line}: {lit:?}"));
            }
        }
    }
    assert!(
        unwrapped.is_empty(),
        "string literals outside @tr() (wrap them, or list ids and names in {ALLOWLIST}):\n{}",
        unwrapped.join("\n")
    );
    let stale: Vec<_> = allowed.difference(&used).collect();
    assert!(
        stale.is_empty(),
        "{ALLOWLIST} lists literals no .slint file has any more: {stale:?}"
    );
}

#[test]
fn messages_are_whole_and_distinct() {
    let msgs = ui_messages();
    assert!(
        msgs.len() > 100,
        "the scanner found only {} messages",
        msgs.len()
    );
    for (msg, files) in &msgs {
        assert!(!msg.id.is_empty(), "an empty @tr() in {files:?}");
        assert_eq!(
            msg.id.trim(),
            msg.id,
            "{} has leading or trailing space; keep layout out of the text",
            label(msg)
        );
        if let Some(plural) = &msg.plural {
            assert!(
                msg.id.contains("{n}") && plural.contains("{n}"),
                "{}: a plural form needs {{n}}",
                label(msg)
            );
        }
    }
}

#[test]
fn catalogs_are_well_formed() {
    for (tag, path) in po_files() {
        let po = parse_po(&read(&path));
        assert!(
            po.header.contains("charset=UTF-8"),
            "{tag}: the header needs `Content-Type: text/plain; charset=UTF-8`"
        );
        assert!(
            po.header.contains("Plural-Forms:") && po.header.contains("plural="),
            "{tag}: the header needs Plural-Forms"
        );
        assert!(
            po.header.contains(&format!("Language: {tag}")),
            "{tag}: the header's Language must be the folder name"
        );

        let known: BTreeSet<Msg> = ui_messages().into_iter().map(|(m, _)| m).collect();
        let mut seen = BTreeSet::new();
        for e in &po.entries {
            let msg = e.msg.clone().unwrap();
            assert!(
                seen.insert(msg.clone()),
                "{tag}: {} appears twice",
                label(&msg)
            );
            assert!(
                known.contains(&msg),
                "{tag}: stale entry {} (no @tr() in ui/*.slint has it; line {})",
                label(&msg),
                e.line
            );
            assert!(
                !e.fuzzy,
                "{tag}: {} is fuzzy; review it and drop the flag",
                label(&msg)
            );
            let want = placeholders(msg.plural.as_deref().unwrap_or(&msg.id));
            let forms = if msg.plural.is_some() { 2 } else { 1 };
            for form in 0..forms {
                let s = e
                    .strs
                    .get(&form)
                    .unwrap_or_else(|| panic!("{tag}: {} lacks msgstr[{form}]", label(&msg)));
                // Untranslated entries are the completeness test's business.
                if s.is_empty() {
                    continue;
                }
                let mut have = placeholders(s);
                let mut want = want.clone();
                if form == 0 && msg.plural.is_some() {
                    // The singular may say "uma" instead of "{n}".
                    have.retain(|p| p != "{n}");
                    want.retain(|p| p != "{n}");
                }
                assert_eq!(
                    have,
                    want,
                    "{tag}: placeholders of {} differ from the source in msgstr[{form}]: {s:?}",
                    label(&msg)
                );
            }
        }
    }
}

/// Brazilian wording that does not belong in the pt-PT catalog.
const BRAZILIAN: &[&str] = &[
    "tela",
    "senha",
    "senhas",
    "configuração",
    "configurações",
    "você",
    "vocês",
    "baixar",
    "usuário",
    "usuários",
    "deletar",
    "arquivo",
    "arquivos",
    // A tab is a "separador" in Portugal.
    "aba",
    "abas",
];

#[test]
fn portuguese_has_no_brazilian_wording() {
    for (tag, path) in po_files() {
        if !tag.starts_with("pt") {
            continue;
        }
        assert_eq!(
            tag, "pt-PT",
            "only European Portuguese is shipped (never pt-BR)"
        );
        for e in parse_po(&read(&path)).entries {
            let msg = e.msg.clone().unwrap();
            // "Arquivo" is the Archive folder (the Windows pt-PT name); a file is a "ficheiro".
            let archive = msg.id.contains("Archive");
            for s in e.strs.values() {
                let lower = s.to_lowercase();
                for word in lower
                    .split(|c: char| !c.is_alphabetic())
                    .filter(|w| !w.is_empty())
                {
                    if BRAZILIAN.contains(&word)
                        && !(archive && (word == "arquivo" || word == "arquivos"))
                    {
                        panic!("{tag}: {} uses the Brazilian {word:?}: {s:?}", label(&msg));
                    }
                }
            }
        }
    }
}

/// Every message has a translation in every shipped language.
#[test]
fn catalogs_are_complete() {
    let msgs = ui_messages();
    for (tag, path) in po_files() {
        let po = parse_po(&read(&path));
        let have: BTreeMap<Msg, &Entry> = po
            .entries
            .iter()
            .map(|e| (e.msg.clone().unwrap(), e))
            .collect();
        let mut missing = Vec::new();
        for (msg, _) in &msgs {
            let forms = if msg.plural.is_some() { 2 } else { 1 };
            let done = have
                .get(msg)
                .is_some_and(|e| (0..forms).all(|f| e.strs.get(&f).is_some_and(|s| !s.is_empty())));
            if !done {
                missing.push(label(msg));
            }
        }
        assert!(
            missing.is_empty(),
            "{tag}: {} messages are untranslated:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }
}
