//! The token lint (Skua's `xtask check` lint, as a test): view code takes
//! every colour, size and duration from `src/theme/`, so a literal in any
//! other source file fails. A line may opt out with a trailing
//! `// token-exempt: <reason>`. Code after a file's `#[cfg(test)]` is not
//! scanned (test windows have literal sizes), nor is `test_support.rs`.

use std::fs;
use std::path::{Path, PathBuf};

const EXEMPT: &str = "// token-exempt:";
/// Where the literals live.
const TOKEN_DIR: &str = "theme";

/// The pattern a line breaks, if any.
fn violation(line: &str) -> Option<&'static str> {
    if let Some(at) = line.find(EXEMPT)
        && !line[at + EXEMPT.len()..].trim().is_empty()
    {
        return None;
    }
    let code = line.split("//").next().unwrap_or(line);
    for (needle, name) in [
        ("rgb(0x", "rgb(0x"),
        ("rgba(0x", "rgba(0x"),
        ("rgb(", "rgb("),
        ("hsla(", "hsla("),
        ("Rgba {", "Rgba {"),
    ] {
        if code.contains(needle) {
            return Some(name);
        }
    }
    for (needle, name) in [
        ("px(", "px(<digit>"),
        (".size(", ".size(<digit>"),
        ("from_millis(", "from_millis(<digit>"),
        ("opacity(", "opacity(<digit>"),
        ("rems(", "rems(<digit>"),
    ] {
        if followed_by_digit(code, needle) {
            return Some(name);
        }
    }
    if hex_colour(code) {
        return Some("0xRRGGBB");
    }
    None
}

fn followed_by_digit(code: &str, needle: &str) -> bool {
    code.match_indices(needle).any(|(i, _)| {
        code[i + needle.len()..]
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit())
    })
}

/// A six or eight digit hex literal (a colour).
fn hex_colour(code: &str) -> bool {
    code.match_indices("0x").any(|(i, _)| {
        let digits: String = code[i + 2..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '_')
            .filter(|c| *c != '_')
            .collect();
        matches!(digits.len(), 6 | 8)
    })
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("list src") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != TOKEN_DIR) {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs")
            // Test-only as a whole (`#[cfg(test)] mod test_support`), as
            // `cargo xtask lint` treats it.
            && path.file_name().is_some_and(|n| n != "test_support.rs")
        {
            out.push(path);
        }
    }
}

#[test]
fn view_code_has_no_colour_or_size_literals() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(files.len() > 5, "the scan found the sources");
    let mut hits = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if let Some(pattern) = violation(line) {
                let rel = file.strip_prefix(&src).unwrap_or(&file);
                hits.push(format!(
                    "{}:{}: `{pattern}`: {}",
                    rel.display(),
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "token lint: use crate::theme tokens or end the line with `{EXEMPT} <reason>`:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_lint_catches_literals_and_allows_tokens() {
    assert_eq!(violation("div().w(px(13.))"), Some("px(<digit>"));
    assert_eq!(violation("let c = rgb(0x2a2a2a);"), Some("rgb(0x"));
    assert_eq!(violation("const INK: u32 = 0x171717;"), Some("0xRRGGBB"));
    assert_eq!(violation(".opacity(0.5)"), Some("opacity(<digit>"));
    assert_eq!(
        violation("Duration::from_millis(250)"),
        Some("from_millis(<digit>")
    );
    assert_eq!(violation("svg().size(px(layout::ICON))"), None);
    assert_eq!(violation(".opacity(motion::DIM)"), None);
    assert_eq!(violation("let mask = 0x1;"), None);
    assert_eq!(violation("px(0.) // token-exempt: zero offset"), None);
    assert_eq!(violation("px(0.) // token-exempt:"), Some("px(<digit>"));
    assert_eq!(violation("// px(3.) in a comment"), None);
}
