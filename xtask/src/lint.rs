//! The GPUI app's source lints (Skua's xtask lint, adapted;
//! docs/GPUI-TESTING.md "Lints"). Both scan `crates/komorebi-gpui/src`.
//!
//! - **Tokens:** views take every colour and size from the token layer
//!   (phase 1; `palette.rs` until then), so raw literals fail: `rgb(0x`,
//!   `rgba(0x`, `hsla(`, `px(<digit>`, `.size(<digit>`, and a token times a
//!   number (`layout::GAP * 2.0`: a derived size is a named token). The
//!   token modules themselves, `test_support.rs` and test modules (from a
//!   `#[cfg(test)]` at the start of a line to the end of the file) are not
//!   scanned. A line may opt out with a trailing `// token-exempt: <why>`.
//! - **No timers or sleeps** (Skua plan 2.4): the UI thread never blocks,
//!   sleeps or polls, and tests move GPUI's test clock instead of waiting.
//!   `thread::sleep`, `block_on`, `.timer(`, `Timer::after`, `tokio::` fail
//!   anywhere in the app, tests included, unless the line ends with
//!   `// timer-exempt: <why>` (a debounce or a reader delay the design asks
//!   for, as Skua's two allowed timers).

use std::fs;
use std::path::{Path, PathBuf};

const SCANNED: &str = "crates/komorebi-gpui/src";
/// The token layer and the test helpers: literals belong there.
const TOKEN_FILES: &[&str] = &["palette.rs", "theme.rs", "tokens.rs", "test_support.rs"];
/// Folders holding the token layer.
const TOKEN_DIRS: &[&str] = &["theme"];
/// Token modules whose constants must not be multiplied by a number.
const TOKEN_MODULES: &[&str] = &[
    "space::",
    "layout::",
    "radius::",
    "type_scale::",
    "motion::",
    "shadow::",
];
const TOKEN_EXEMPT: &str = "// token-exempt:";
const TIMER_EXEMPT: &str = "// timer-exempt:";
const TIMER_PATTERNS: &[&str] = &[
    "thread::sleep",
    "block_on",
    ".timer(",
    "Timer::after",
    "tokio::",
];

pub fn run(root: &Path) -> Result<(), String> {
    let src = root.join(SCANNED);
    let mut hits = Vec::new();
    for file in rust_files(&src)? {
        let text =
            fs::read_to_string(&file).map_err(|e| format!("reading {}: {e}", file.display()))?;
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let token_file = is_token_file(&src, &file);
        let mut in_tests = false;
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("#[cfg(test)]") {
                in_tests = true;
            }
            if let Some(pattern) = timer_violation(line) {
                hits.push(format!(
                    "{rel}:{}: timer `{pattern}`: {}",
                    n + 1,
                    line.trim()
                ));
            }
            if !token_file
                && !in_tests
                && let Some(pattern) = token_violation(line)
            {
                hits.push(format!(
                    "{rel}:{}: token `{pattern}`: {}",
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    if hits.is_empty() {
        println!("lint: ok (tokens, timers)");
        return Ok(());
    }
    for hit in &hits {
        eprintln!("lint: {hit}");
    }
    Err(format!(
        "lint: {} finding(s); use the token layer or the test clock, or end the line with `{TOKEN_EXEMPT} <why>` / `{TIMER_EXEMPT} <why>`",
        hits.len()
    ))
}

fn is_token_file(src: &Path, file: &Path) -> bool {
    let rel = file.strip_prefix(src).unwrap_or(file);
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    TOKEN_FILES.contains(&name)
        || rel
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .is_some_and(|first| TOKEN_DIRS.contains(&first) && rel.components().count() > 1)
}

/// The code of a line: nothing for a comment line, and no trailing comment.
fn code(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    match line.find("//") {
        // Not inside a string such as "https://".
        Some(at) if line[..at].matches('"').count().is_multiple_of(2) => &line[..at],
        _ => line,
    }
}

fn exempt(line: &str, marker: &str) -> bool {
    line.find(marker)
        .is_some_and(|at| !line[at + marker.len()..].trim().is_empty())
}

/// The no-timers pattern a line uses, unless it is exempt.
pub fn timer_violation(line: &str) -> Option<&'static str> {
    if exempt(line, TIMER_EXEMPT) {
        return None;
    }
    let code = code(line);
    TIMER_PATTERNS.iter().copied().find(|p| code.contains(p))
}

/// The raw literal pattern a line uses, unless it is exempt.
pub fn token_violation(line: &str) -> Option<&'static str> {
    if exempt(line, TOKEN_EXEMPT) {
        return None;
    }
    let code = code(line);
    if code.contains("rgb(0x") {
        return Some("rgb(0x");
    }
    if code.contains("rgba(0x") {
        return Some("rgba(0x");
    }
    if code.contains("hsla(") {
        return Some("hsla(");
    }
    if followed_by_digit(code, "px(") {
        return Some("px(<digit>");
    }
    if followed_by_digit(code, ".size(") {
        return Some(".size(<digit>");
    }
    if token_times_number(code) {
        return Some("<TOKEN> * <number>");
    }
    None
}

fn followed_by_digit(code: &str, needle: &str) -> bool {
    code.match_indices(needle).any(|(i, _)| {
        // `px(` must start a word (not `phys(`); `.size(` starts itself.
        let starts_word = needle.starts_with('.')
            || code[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        starts_word
            && code[i + needle.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
    })
}

/// `module::TOKEN * <number>` or `<number> * module::TOKEN`.
fn token_times_number(code: &str) -> bool {
    TOKEN_MODULES.iter().any(|module| {
        code.match_indices(module).any(|(i, _)| {
            let starts_path = code[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            let rest = &code[i + module.len()..];
            let name_len = rest
                .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(rest.len());
            let is_const = rest.starts_with(|c: char| c.is_ascii_uppercase());
            let after = rest[name_len..].trim_start();
            let before = code[..i].trim_end();
            starts_path
                && is_const
                && (after
                    .strip_prefix('*')
                    .is_some_and(|r| r.trim_start().starts_with(|c: char| c.is_ascii_digit()))
                    || before.strip_suffix('*').is_some_and(|b| {
                        b.trim_end()
                            .ends_with(|c: char| c.is_ascii_digit() || c == '.')
                    }))
        })
    })
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|e| format!("listing {}: {e}", d.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_literals_fail() {
        assert_eq!(token_violation("div().w(px(13.))"), Some("px(<digit>"));
        assert_eq!(token_violation("let c = rgb(0x2a2a2a);"), Some("rgb(0x"));
        assert_eq!(
            token_violation("let c = rgba(0x2a2a2aff);"),
            Some("rgba(0x")
        );
        assert_eq!(token_violation("hsla(0., 0., 0., 1.)"), Some("hsla("));
        assert_eq!(token_violation("icon.size(16.)"), Some(".size(<digit>"));
    }

    #[test]
    fn tokens_comments_and_exemptions_pass() {
        assert_eq!(token_violation("div().w(px(layout::SIDEBAR_W))"), None);
        assert_eq!(token_violation(".h(px(TITLEBAR_H))"), None);
        assert_eq!(token_violation("let p = phys(48.);"), None);
        assert_eq!(token_violation("// px(12.) in a comment"), None);
        assert_eq!(token_violation("px(0.) // token-exempt: zero border"), None);
        assert_eq!(
            token_violation("px(0.) // token-exempt:"),
            Some("px(<digit>")
        );
    }

    #[test]
    fn token_arithmetic_fails() {
        let hit = Some("<TOKEN> * <number>");
        assert_eq!(token_violation(".w(px(space::SP_10 * 4.0))"), hit);
        assert_eq!(token_violation("let h = 2.0 * space::SP_8;"), hit);
        assert_eq!(
            token_violation("let w = layout::SIDEBAR_W + space::SP_2;"),
            None
        );
        assert_eq!(token_violation("let w = layout::SIDEBAR_W * scale;"), None);
        assert_eq!(token_violation("let x = mylayout::SIDEBAR_W * 2.0;"), None);
    }

    #[test]
    fn timers_and_sleeps_fail_unless_exempt() {
        assert_eq!(
            timer_violation("std::thread::sleep(Duration::from_millis(5));"),
            Some("thread::sleep")
        );
        assert_eq!(
            timer_violation("cx.background_executor().timer(SAVE_AFTER).await;"),
            Some(".timer(")
        );
        assert_eq!(
            timer_violation("futures::executor::block_on(f)"),
            Some("block_on")
        );
        assert_eq!(
            timer_violation(".timer(SAVE_AFTER).await; // timer-exempt: the save debounce"),
            None
        );
        assert_eq!(
            timer_violation(".timer(d).await; // timer-exempt:"),
            Some(".timer(")
        );
        assert_eq!(timer_violation("/// never `thread::sleep` here"), None);
        assert_eq!(timer_violation("cx.executor().advance_clock(d);"), None);
    }

    #[test]
    fn token_files_are_the_token_layer() {
        let src = Path::new("crates/komorebi-gpui/src");
        assert!(is_token_file(src, &src.join("palette.rs")));
        assert!(is_token_file(src, &src.join("theme").join("colors.rs")));
        assert!(is_token_file(src, &src.join("theme.rs")));
        assert!(!is_token_file(src, &src.join("window.rs")));
    }

    /// The app's sources pass both lints (the same check as `cargo xtask
    /// lint`, so `cargo test` catches a finding too).
    #[test]
    fn the_app_passes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the workspace root");
        assert_eq!(run(root), Ok(()));
    }
}
