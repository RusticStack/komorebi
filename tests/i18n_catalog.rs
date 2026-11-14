//! Guards the message catalog: every shown string of the GPUI app goes
//! through `tr!` or `trn!` (`komorebi_i18n`), `translations/komorebi.pot`
//! lists exactly the messages of those calls, and each language's `.po` file
//! is well formed, current, complete and free of Brazilian wording. No
//! dependencies: a small Rust lexer and the `.po` reader the catalog's build
//! script uses (`crates/komorebi-i18n/po.rs`) live here.
//!
//! Regenerate the template after changing a string:
//! `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`.

#[path = "../crates/komorebi-i18n/po.rs"]
mod po;

use po::{Entry, Msg, parse_po};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TRANSLATIONS_DIR: &str = "translations";
const POT: &str = "translations/komorebi.pot";
const ALLOWLIST: &str = "translations/untranslated-allowlist.txt";
/// Rust sources whose `tr!` and `trn!` calls are extracted: the library
/// and the GPUI app.
const RUST_DIRS: &[&str] = &["src", "crates/komorebi-gpui/src"];
/// Rust sources whose shown string literals must be messages: the GPUI app.
const GPUI_DIR: &str = "crates/komorebi-gpui/src";
/// The GPUI app's development-only component gallery, left out of the
/// shown-literal check.
const GALLERY_DIR: &str = "crates/komorebi-gpui/src/gallery/";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

// ---- the Rust scanner ------------------------------------------------------

/// A Rust token, as far as the scanner needs one.
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// A string literal, unescaped (byte and raw strings too).
    Str(String),
    Punct(char),
    /// Numbers, char literals, lifetimes.
    Other,
}

struct Token {
    tok: Tok,
    line: usize,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The string literal at `chars[i]` (its prefix, if any, already read up to
/// `body`, the index of the `"` or the first `#` of a raw string): its text
/// and the index after it.
fn rust_string(chars: &[char], body: usize, raw: bool, what: &str) -> (String, usize) {
    let mut i = body;
    if raw {
        let mut hashes = 0;
        while chars[i] == '#' {
            hashes += 1;
            i += 1;
        }
        assert_eq!(chars[i], '"', "{what}: a raw string without its quote");
        let start = i + 1;
        let mut j = start;
        loop {
            if chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) {
                return (chars[start..j].iter().collect(), j + 1 + hashes);
            }
            j += 1;
        }
    }
    assert_eq!(chars[i], '"', "{what}: expected a string literal");
    i += 1;
    let mut out = String::new();
    while chars[i] != '"' {
        if chars[i] != '\\' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        match chars[i] {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\\' => out.push('\\'),
            'x' => {
                let hex: String = chars[i + 1..i + 3].iter().collect();
                let code = u32::from_str_radix(&hex, 16).expect("hex in \\x..");
                out.push(char::from_u32(code).expect("an ASCII \\x.."));
                i += 2;
            }
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
            '\n' => {
                // A line continuation: the line end and the next line's
                // leading space are not part of the string.
                while chars[i + 1].is_whitespace() {
                    i += 1;
                }
            }
            other => panic!("{what}: unknown escape \\{other}"),
        }
        i += 1;
    }
    (out, i + 1)
}

/// The tokens of a Rust file, comments dropped.
fn lex_rust(text: &str, name: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut toks = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < n {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && next == Some('/') {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while depth > 0 {
                if chars[i] == '/' && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' {
            let (s, end) = rust_string(&chars, i, false, &format!("{name}:{line}"));
            toks.push(Token {
                tok: Tok::Str(s),
                line,
            });
            line += chars[i..end].iter().filter(|&&c| c == '\n').count();
            i = end;
        } else if c == '\'' {
            if next == Some('\\') {
                // An escaped char literal: '\n', '\'', '\u{..}'.
                let mut j = i + 3;
                while chars[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                // A lifetime or a loop label.
                i += 1;
                while i < n && is_ident(chars[i]) {
                    i += 1;
                }
            }
            toks.push(Token {
                tok: Tok::Other,
                line,
            });
        } else if c.is_ascii_digit() {
            while i < n && (is_ident(chars[i])) {
                i += 1;
            }
            toks.push(Token {
                tok: Tok::Other,
                line,
            });
        } else if is_ident(c) {
            let start = i;
            while i < n && is_ident(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let quote = chars.get(i) == Some(&'"');
            // `r"`, `r#"` (not the raw identifier `r#type`).
            let raw_start = {
                let mut j = i;
                while chars.get(j) == Some(&'#') {
                    j += 1;
                }
                chars.get(j) == Some(&'"')
            };
            if matches!(word.as_str(), "b" | "c") && quote {
                let (s, end) = rust_string(&chars, i, false, &format!("{name}:{line}"));
                toks.push(Token {
                    tok: Tok::Str(s),
                    line,
                });
                line += chars[i..end].iter().filter(|&&c| c == '\n').count();
                i = end;
            } else if matches!(word.as_str(), "r" | "br" | "cr") && raw_start {
                let (s, end) = rust_string(&chars, i, true, &format!("{name}:{line}"));
                toks.push(Token {
                    tok: Tok::Str(s),
                    line,
                });
                line += chars[i..end].iter().filter(|&&c| c == '\n').count();
                i = end;
            } else {
                toks.push(Token {
                    tok: Tok::Ident(word),
                    line,
                });
            }
        } else {
            toks.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        }
    }
    toks
}

fn is_open(t: &Tok) -> bool {
    matches!(t, Tok::Punct('(' | '[' | '{'))
}

fn is_close(t: &Tok) -> bool {
    matches!(t, Tok::Punct(')' | ']' | '}'))
}

/// The index of the delimiter closing the one at `open`.
fn matching(toks: &[Token], open: usize) -> usize {
    let mut depth = 0;
    for (k, t) in toks.iter().enumerate().skip(open) {
        if is_open(&t.tok) {
            depth += 1;
        } else if is_close(&t.tok) {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    panic!("an unclosed delimiter at token {open}");
}

/// The index after the item that starts at `i` (after its attributes): up
/// to its `;`, or its braced body.
fn skip_item(toks: &[Token], mut i: usize) -> usize {
    while i < toks.len() {
        match &toks[i].tok {
            Tok::Punct(';') => return i + 1,
            Tok::Punct('{') => return matching(toks, i) + 1,
            t if is_open(t) => i = matching(toks, i) + 1,
            _ => i += 1,
        }
    }
    i
}

/// One `tr!` or `trn!` call: its message, line and number of arguments
/// (the count of `trn!` not included).
struct Call {
    msg: Msg,
    line: usize,
    args: usize,
}

/// What a scan of one Rust file found.
#[derive(Default)]
struct RustScan {
    calls: Vec<Call>,
    /// String literals outside `tr!` that look shown, with their line.
    shown: BTreeSet<(usize, String)>,
}

/// Macros whose literals are never shown: messages for developers, logs,
/// compile-time inputs.
const QUIET_MACROS: &[&str] = &[
    "panic",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "unreachable",
    "todo",
    "unimplemented",
    "eprintln",
    "println",
    "eprint",
    "print",
    "write",
    "writeln",
    "concat",
    "env",
    "option_env",
    "include",
    "include_str",
    "include_bytes",
    "anyhow",
    "bail",
    "ensure",
];

/// Methods whose literal argument is never shown (a panic message, an
/// error's context).
const QUIET_METHODS: &[&str] = &["expect", "context", "with_context"];

/// Methods whose literal argument reaches the screen or the accessibility
/// tree (GPUI elements, Kit components, the window).
const SINK_METHODS: &[&str] = &[
    "child",
    "aria_label",
    "label",
    "tooltip",
    "placeholder",
    "title",
    "set_window_title",
    "description",
    "message",
    "text",
    "header",
    "caption",
    "summary",
    "detail",
];

/// Text, not an id: a run of two letters.
fn has_words(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    chars
        .windows(2)
        .any(|w| w[0].is_alphabetic() && w[1].is_alphabetic())
}

/// A sentence for a person: a capital, three words or more, and a full
/// stop (or a question or an exclamation) at its end or inside.
fn sentence_like(s: &str) -> bool {
    let capital = s.chars().next().is_some_and(char::is_uppercase);
    let words = s
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .count();
    let stop = s.trim_end().ends_with(['.', '!', '?']) || s.contains(". ");
    capital && words >= 3 && stop
}

fn str_at(toks: &[Token], i: usize) -> Option<&str> {
    match toks.get(i).map(|t| &t.tok) {
        Some(Tok::Str(s)) => Some(s),
        _ => None,
    }
}

fn punct_at(toks: &[Token], i: usize, c: char) -> bool {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Punct(p)) if *p == c)
}

/// The `tr!(...)`/`trn!(...)` whose `(` is at `open`.
fn parse_call(toks: &[Token], open: usize, plural: bool, what: &str) -> Call {
    let line = toks[open].line;
    let lit = |i: usize| -> String {
        str_at(toks, i)
            .unwrap_or_else(|| {
                panic!(
                    "{what}:{line}: tr! and trn! take string literals (they are extracted into the template)"
                )
            })
            .to_string()
    };
    let mut j = open + 1;
    let first = lit(j);
    j += 1;
    let (ctx, id) = if punct_at(toks, j, '=') && punct_at(toks, j + 1, '>') {
        j += 2;
        let id = lit(j);
        j += 1;
        (Some(first), id)
    } else {
        (None, first)
    };
    let plural = if plural {
        assert!(punct_at(toks, j, ','), "{what}:{line}: trn!(one, many, n)");
        j += 1;
        let p = lit(j);
        j += 1;
        Some(p)
    } else {
        None
    };
    // The arguments: top-level commas before the closing parenthesis (a
    // trailing comma adds none).
    let close = matching(toks, open);
    let mut depth = 0;
    let mut commas = 0;
    for (k, token) in toks.iter().enumerate().take(close).skip(j) {
        let t = &token.tok;
        if is_open(t) {
            depth += 1;
        } else if is_close(t) {
            depth -= 1;
        } else if depth == 0 && *t == Tok::Punct(',') && k + 1 < close {
            commas += 1;
        }
    }
    let args = if plural.is_some() {
        assert!(commas >= 1, "{what}:{line}: trn! needs its count");
        commas - 1
    } else {
        commas
    };
    Call {
        msg: Msg { ctx, id, plural },
        line,
        args,
    }
}

fn scan_rust(text: &str, name: &str) -> RustScan {
    #[derive(Clone, Copy, PartialEq)]
    enum Group {
        Plain,
        /// Inside `tr!`/`trn!`: its literals are the message.
        Tr,
        /// Inside a quiet macro or method: never shown.
        Quiet,
    }
    let toks = lex_rust(text, name);
    let mut scan = RustScan::default();
    let mut stack: Vec<Group> = Vec::new();
    let checked = |stack: &[Group]| stack.iter().all(|g| *g == Group::Plain);
    let mut i = 0;
    while i < toks.len() {
        match &toks[i].tok {
            Tok::Punct('#') => {
                // An attribute; `#[cfg(test)]` drops the item it is on.
                let mut j = i + 1;
                if punct_at(&toks, j, '!') {
                    j += 1;
                }
                if punct_at(&toks, j, '[') {
                    let close = matching(&toks, j);
                    let inner: Vec<&Tok> = toks[j + 1..close].iter().map(|t| &t.tok).collect();
                    let cfg_test = inner
                        == [
                            &Tok::Ident("cfg".into()),
                            &Tok::Punct('('),
                            &Tok::Ident("test".into()),
                            &Tok::Punct(')'),
                        ];
                    i = close + 1;
                    if cfg_test {
                        i = skip_item(&toks, i);
                    }
                    continue;
                }
            }
            Tok::Ident(word)
                if punct_at(&toks, i + 1, '!')
                    && toks.get(i + 2).is_some_and(|t| is_open(&t.tok)) =>
            {
                let open = i + 2;
                if word == "tr" || word == "trn" {
                    scan.calls
                        .push(parse_call(&toks, open, word == "trn", name));
                    stack.push(Group::Tr);
                    i = open + 1;
                    continue;
                }
                if QUIET_MACROS.contains(&word.as_str()) {
                    stack.push(Group::Quiet);
                    i = open + 1;
                    continue;
                }
            }
            Tok::Punct('.') => {
                if let Some(Tok::Ident(method)) = toks.get(i + 1).map(|t| &t.tok)
                    && punct_at(&toks, i + 2, '(')
                {
                    if QUIET_METHODS.contains(&method.as_str()) {
                        stack.push(Group::Quiet);
                        i += 3;
                        continue;
                    }
                    if SINK_METHODS.contains(&method.as_str())
                        && let Some(s) = str_at(&toks, i + 3)
                        && checked(&stack)
                        && has_words(s)
                    {
                        scan.shown.insert((toks[i + 3].line, s.to_string()));
                    }
                }
            }
            Tok::Ident(word) if word == "title" => {
                // `title: Some("...")` (TitlebarOptions, Kit's dialogs).
                if punct_at(&toks, i + 1, ':')
                    && matches!(toks.get(i + 2).map(|t| &t.tok), Some(Tok::Ident(s)) if s == "Some")
                    && punct_at(&toks, i + 3, '(')
                    && let Some(s) = str_at(&toks, i + 4)
                    && checked(&stack)
                    && has_words(s)
                {
                    scan.shown.insert((toks[i + 4].line, s.to_string()));
                }
            }
            Tok::Ident(word) if word == "SharedString" => {
                // `SharedString::from("...")`, `SharedString::new_static("...")`.
                if punct_at(&toks, i + 1, ':')
                    && punct_at(&toks, i + 2, ':')
                    && punct_at(&toks, i + 4, '(')
                    && let Some(s) = str_at(&toks, i + 5)
                    && checked(&stack)
                    && has_words(s)
                {
                    scan.shown.insert((toks[i + 5].line, s.to_string()));
                }
            }
            Tok::Str(s) if checked(&stack) && sentence_like(s) => {
                scan.shown.insert((toks[i].line, s.clone()));
            }
            t if is_open(t) => stack.push(Group::Plain),
            t if is_close(t) => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    scan
}

/// The `.rs` files under `dir`, sorted, as (path from the root with `/`,
/// path).
fn rust_files(dir: &str) -> Vec<(String, PathBuf)> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
            let p = entry.expect("directory entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root().join(dir), &mut files);
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(root()).expect("under the root");
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (name, p)
        })
        .collect();
    named.sort();
    named
}

/// Every `tr!`/`trn!` call of the scanned Rust sources, with its file.
fn rust_calls() -> Vec<(String, Call)> {
    let mut calls = Vec::new();
    for dir in RUST_DIRS {
        for (name, path) in rust_files(dir) {
            for call in scan_rust(&read(&path), &name).calls {
                calls.push((name.clone(), call));
            }
        }
    }
    calls
}

/// Every message of the UI, deduplicated, in source order, with the files
/// that use each.
fn ui_messages() -> Vec<(Msg, Vec<String>)> {
    let mut order: Vec<(Msg, Vec<String>)> = Vec::new();
    let mut add = |msg: Msg, name: &str| match order.iter_mut().find(|(m, _)| *m == msg) {
        Some((_, files)) => {
            if !files.iter().any(|f| f == name) {
                files.push(name.to_string());
            }
        }
        None => order.push((msg, vec![name.to_string()])),
    };
    for (name, call) in rust_calls() {
        add(call.msg, &name);
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
        "# Komorebi message catalog (template). Generated from the\n\
         # tr!/trn! calls in src/ and crates/komorebi-gpui/src:\n\
         # run `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`.\n\
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

// ---- the catalogs (read by po.rs) ---------------------------------------

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
        "{POT} is out of date with the tr!/trn! calls in {RUST_DIRS:?}; run\n  KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template"
    );
}

#[test]
fn every_shown_string_is_translatable() {
    let allowed: BTreeSet<String> = read(&root().join(ALLOWLIST))
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.replace("\\u{a0}", "\u{a0}"))
        .collect();
    let mut used = BTreeSet::new();
    let mut unwrapped = Vec::new();
    // The GPUI app: literals given to an element's text, label or title,
    // and sentences anywhere outside tr!, logs and panics (tests skipped).
    for (name, path) in rust_files(GPUI_DIR) {
        // The component gallery (feature `gallery`) is a development tool
        // with sample mail, never in a release build.
        if name.contains(GALLERY_DIR) {
            continue;
        }
        for (line, lit) in scan_rust(&read(&path), &name).shown {
            if allowed.contains(&lit) {
                used.insert(lit);
            } else {
                unwrapped.push(format!("{name}:{line}: {lit:?} (use tr!)"));
            }
        }
    }
    assert!(
        unwrapped.is_empty(),
        "string literals outside tr! (wrap them, or list ids and names in {ALLOWLIST}):\n{}",
        unwrapped.join("\n")
    );
    let stale: Vec<_> = allowed.difference(&used).collect();
    assert!(
        stale.is_empty(),
        "{ALLOWLIST} lists literals no GPUI source has any more: {stale:?}"
    );
}

/// The placeholders a message's text asks for: how many `{}` and the
/// highest `{i}` (`{n}` is the count, not an argument).
fn wanted_args(text: &str) -> (usize, Option<usize>) {
    let mut auto = 0;
    let mut highest = None;
    for p in placeholders(text) {
        match p.as_str() {
            "{}" => auto += 1,
            "{n}" | "{{" | "}}" => {}
            other => {
                let i: usize = other[1..other.len() - 1]
                    .parse()
                    .unwrap_or_else(|_| panic!("{text:?}: unknown placeholder {other}"));
                highest = Some(highest.map_or(i, |h: usize| h.max(i)));
            }
        }
    }
    (auto, highest)
}

#[test]
fn placeholders_count_as_arguments() {
    assert_eq!(wanted_args("no placeholder"), (0, None));
    assert_eq!(wanted_args("{} and {}"), (2, None));
    assert_eq!(wanted_args("{1} before {0}"), (0, Some(1)));
    assert_eq!(wanted_args("{n} of {0}, {{literal}}"), (0, Some(0)));
}

#[test]
fn tr_calls_pass_their_arguments() {
    let calls = rust_calls();
    for (name, call) in &calls {
        let at = format!("{name}:{}: {}", call.line, label(&call.msg));
        let (auto, highest) = wanted_args(&call.msg.id);
        if let Some(plural) = &call.msg.plural {
            assert_eq!(
                wanted_args(plural),
                (auto, highest),
                "{at}: the plural form asks for other arguments"
            );
        }
        assert!(
            auto == 0 || highest.is_none(),
            "{at}: mixes {{}} and {{0}}; use one style"
        );
        let want = highest.map_or(auto, |h| h + 1);
        assert_eq!(
            call.args, want,
            "{at}: passes {} arguments, the text has {want} placeholders",
            call.args
        );
    }
}

/// The Rust scanner on a sample: what it extracts and what it flags.
#[test]
fn the_rust_scanner_reads_macros_and_shown_literals() {
    let sample = r####"
        // tr!("in a comment") is not a call.
        /* nor tr!("in /* a nested */ block") */
        fn view() {
            let a = tr!("Plain");
            let b = tr!("ctx" => "With {0}", x.name(), );
            let c = trn!("{n} item", "{n} items", count, );
            let d = trn!("ctx" => "{n} of {0}", "{n} of {0}", n, tr!("Inner"));
            let e = tr!("Escapes \"q\" \u{a0}and\
                         continued");
            let r = r#"A raw "string". Not a call."#;
            let ch = '"'; let lt: &'static str = "lowercase text, not shown.";
            eprintln!("Komorebi could not start the thing. Logged only.");
            x.expect("This is a panic message, never shown.");
            div().child("Shown label").aria_label("Close the pane");
            let t = TitlebarOptions { title: Some("Window title".into()) };
            let s = SharedString::from("Shared text");
            let f = format!("A sentence built in code, then shown.");
        }
        #[cfg(test)]
        mod tests {
            fn t() { tr!("Only in a test"); div().child("Test label. With words."); }
        }
        fn after() { tr!("After the tests"); }
    "####;
    let scan = scan_rust(sample, "sample.rs");
    let got: Vec<(Option<&str>, &str, Option<&str>, usize)> = scan
        .calls
        .iter()
        .map(|c| {
            (
                c.msg.ctx.as_deref(),
                c.msg.id.as_str(),
                c.msg.plural.as_deref(),
                c.args,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (None, "Plain", None, 0),
            (Some("ctx"), "With {0}", None, 1),
            (None, "{n} item", Some("{n} items"), 0),
            (Some("ctx"), "{n} of {0}", Some("{n} of {0}"), 1),
            (None, "Inner", None, 0),
            (None, "Escapes \"q\" \u{a0}andcontinued", None, 0),
            (None, "After the tests", None, 0),
        ]
    );
    let shown: Vec<&str> = scan.shown.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(
        shown,
        [
            "A raw \"string\". Not a call.",
            // One line: in text order.
            "Close the pane",
            "Shown label",
            "Window title",
            "Shared text",
            "A sentence built in code, then shown.",
        ]
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
        assert!(!msg.id.is_empty(), "an empty tr! in {files:?}");
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
                "{tag}: stale entry {} (no tr! in the Rust sources has it; line {})",
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

/// GPUI Kit's own strings (the vendored `locales/ui.yml`, whose `pt-PT`
/// column Komorebi adds; third_party/gpui-component/KOMOREBI-PATCH.md): the
/// `(key, English, pt-PT)` of every key that has both.
fn kit_strings() -> Vec<(String, String, String)> {
    let table = read(&root().join("third_party/gpui-component/locales/ui.yml"));
    let value = |s: &str| s.trim().trim_matches('"').to_owned();
    let mut out = Vec::new();
    let (mut section, mut key) = (String::new(), String::new());
    let (mut en, mut pt): (Option<String>, Option<String>) = (None, None);
    let mut flush = |key: &str, en: &mut Option<String>, pt: &mut Option<String>| {
        if let (Some(e), Some(p)) = (en.take(), pt.take()) {
            out.push((key.to_owned(), e, p));
        }
    };
    for line in table.lines() {
        if line.starts_with('#') || line.starts_with('_') {
            continue;
        }
        if !line.starts_with(' ') && line.ends_with(':') {
            flush(&key, &mut en, &mut pt);
            section = line.trim_end_matches(':').to_owned();
        } else if let Some(k) = line
            .strip_prefix("  ")
            .filter(|k| !k.starts_with(' '))
            .and_then(|k| k.strip_suffix(':'))
        {
            flush(&key, &mut en, &mut pt);
            key = format!("{section}.{k}");
        } else if let Some(v) = line.strip_prefix("    en:") {
            en = Some(value(v));
        } else if let Some(v) = line.strip_prefix("    pt-PT:") {
            pt = Some(value(v));
        }
    }
    flush(&key, &mut en, &mut pt);
    out
}

/// Kit's pt-PT words are the catalog's where the app has the same English
/// text (a word in one place is the same word in the other), and they are
/// European, not Brazilian. Words where Kit's sense differs from the
/// catalog's entry of the same text are listed with the reason.
#[test]
fn kit_strings_agree_with_the_catalog() {
    // Kit keys whose English word means something else than the catalog's
    // entry of the same text: the catalog's "Collapse" is the folder tree's
    // (Fechar) and Kit's dock "Collapse" shrinks a panel (Recolher); the
    // chart's Open and Close are the opening and closing price.
    const DIFFERENT_SENSE: &[&str] = &["Dock.Collapse", "Chart.open", "Chart.close"];
    let kit = kit_strings();
    assert!(
        kit.len() >= 119,
        "{} Kit strings with both languages",
        kit.len()
    );

    let catalog: BTreeMap<String, String> = po_files()
        .into_iter()
        .filter(|(tag, _)| tag == "pt-PT")
        .flat_map(|(_, path)| parse_po(&read(&path)).entries)
        .filter(|e| {
            e.msg
                .as_ref()
                .is_some_and(|m| m.ctx.is_none() && m.plural.is_none())
        })
        .filter_map(|e| {
            let id = e.msg.as_ref()?.id.to_lowercase();
            Some((id, e.strs.get(&0)?.clone()))
        })
        .collect();

    let mut shared = 0;
    for (key, en, pt) in &kit {
        if let Some(have) = catalog.get(&en.to_lowercase())
            && !DIFFERENT_SENSE.contains(&key.as_str())
        {
            shared += 1;
            assert_eq!(
                pt.to_lowercase(),
                have.to_lowercase(),
                "{key}: Kit says {pt:?} for {en:?}, the catalog {have:?}"
            );
        }
        for word in pt
            .to_lowercase()
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
        {
            assert!(
                !BRAZILIAN.contains(&word),
                "{key}: {pt:?} uses the Brazilian {word:?}"
            );
        }
        assert!(
            !pt.contains(['\u{2013}', '\u{2014}']),
            "{key}: no en or em dash"
        );
    }
    assert!(
        shared >= 10,
        "only {shared} Kit strings are checked against the catalog"
    );
}
