//! The formatting toolbar's edits on the composer body: the Markdown-lite
//! markers that `compose_html` turns into the HTML part. Offsets are the
//! UTF-8 byte offsets of the body's selection (anchor and cursor, in either
//! order); each edit returns the new text and the selection to restore, or
//! `None` when there is nothing to do. Offsets come from the window, so
//! they are checked, never trusted: off a char boundary means no edit.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::mail::render::links;

/// Bold and italic markers. Italic uses `_` so it can sit inside or around
/// bold (`**` is never mistaken for an italic `*`).
pub const BOLD: &str = "**";
pub const ITALIC: &str = "_";

/// A line-level format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    Bullet,
    Number,
    Quote,
}

/// A text and the selection to show in it (anchor, cursor).
pub type Edited = (String, usize, usize);

/// The selection as an ordered byte range, if both ends are char boundaries.
fn span(text: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    (text.is_char_boundary(s) && text.is_char_boundary(e)).then_some((s, e))
}

/// `text` with `start..end` replaced by `with`.
fn splice(text: &str, start: usize, end: usize, with: &str) -> Option<String> {
    Some([text.get(..start)?, with, text.get(end..)?].concat())
}

/// Bold or italic (`marker`) on the selection, or off when it is already
/// wrapped (markers inside or just outside the selection). Whitespace at
/// the selection's ends stays outside the markers and part of a word takes
/// the whole word, as the converter expects; a selection over several
/// lines wraps each line. With no
/// selection, an empty pair goes in with the caret between.
pub fn wrap_selection(text: &str, a: usize, b: usize, marker: &str) -> Option<Edited> {
    let (s, e) = span(text, a, b)?;
    let m = marker.len();
    let sel = text.get(s..e)?;
    if sel.contains('\n') {
        let lines = toggle_lines(sel, marker);
        let out = splice(text, s, e, &lines)?;
        let end = s + lines.len();
        return Some((out, s, end));
    }
    let (mut s, mut e) = if sel.trim().is_empty() {
        (e, e)
    } else {
        let lead = sel.len() - sel.trim_start().len();
        let trail = sel.len() - sel.trim_end().len();
        (s + lead, e - trail)
    };
    // Part of a word grows to the whole word: markers inside a word are
    // left as text by the converter.
    if s < e {
        while let Some(c) = text
            .get(..s)?
            .chars()
            .next_back()
            .filter(|c| c.is_alphanumeric())
        {
            s -= c.len_utf8();
        }
        while let Some(c) = text
            .get(e..)?
            .chars()
            .next()
            .filter(|c| c.is_alphanumeric())
        {
            e += c.len_utf8();
        }
    }
    let before = text.get(..s)?;
    let after = text.get(e..)?;
    let sel = text.get(s..e)?;
    if before.ends_with(marker) && after.starts_with(marker) {
        let inner = splice(text, e, e + m, "")?;
        let out = splice(&inner, s - m, s, "")?;
        return Some((out, s - m, e - m));
    }
    if s < e && sel.len() >= 2 * m && sel.starts_with(marker) && sel.ends_with(marker) {
        let inner = sel.get(m..sel.len() - m)?;
        let out = splice(text, s, e, inner)?;
        return Some((out, s, e - 2 * m));
    }
    let out = splice(text, s, e, &[marker, sel, marker].concat())?;
    Some((out, s + m, e + m))
}

/// Each non-blank line of `sel` wrapped in `marker` (its own surrounding
/// whitespace kept outside), or every line unwrapped if all already are.
fn toggle_lines(sel: &str, marker: &str) -> String {
    let m = marker.len();
    let wrapped = |t: &str| t.len() >= 2 * m && t.starts_with(marker) && t.ends_with(marker);
    let all = sel
        .split('\n')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .all(wrapped);
    let lines: Vec<String> = sel
        .split('\n')
        .map(|line| {
            let t = line.trim();
            if t.is_empty() {
                return line.to_string();
            }
            let lead = line.len() - line.trim_start().len();
            let (head, tail) = (
                line.get(..lead).unwrap_or(""),
                line.get(lead + t.len()..).unwrap_or(""),
            );
            let body = if all {
                t.get(m..t.len() - m).unwrap_or(t).to_string()
            } else if wrapped(t) {
                t.to_string()
            } else {
                [marker, t, marker].concat()
            };
            [head, &body, tail].concat()
        })
        .collect();
    lines.join("\n")
}

/// The length of a line's list marker (`- ` or `12. `) after its indent.
fn list_marker(line: &str) -> Option<usize> {
    if line.starts_with("- ") {
        return Some(2);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = line.get(digits..)?;
    ((1..=9).contains(&digits) && rest.starts_with(". ")).then_some(digits + 2)
}

/// A line's indent and the rest of it.
fn split(line: &str) -> (usize, &str) {
    let body = line.trim_start_matches(' ');
    (line.len() - body.len(), body)
}

/// Bullets, numbers or a quote level on every line the selection touches,
/// or off when every non-blank line already has it. Turning a list into
/// the other kind replaces its markers; numbers count from 1 in each run
/// of lines (a blank line ends a list, as in the converter). The caret
/// keeps its place in its line; a selection grows to the whole lines.
pub fn toggle_line_prefix(text: &str, a: usize, b: usize, prefix: Prefix) -> Option<Edited> {
    let (s, e) = span(text, a, b)?;
    let start = text.get(..s)?.rfind('\n').map_or(0, |i| i + 1);
    // A selection ending at the start of a line leaves that line alone.
    let probe = if e > s && text.get(..e)?.ends_with('\n') {
        e - 1
    } else {
        e
    };
    let end = text
        .get(probe..)?
        .find('\n')
        .map_or(text.len(), |i| probe + i);
    let block = text.get(start..end)?;

    let has = |line: &str| {
        let (_, body) = split(line);
        match prefix {
            Prefix::Bullet => body.starts_with("- "),
            Prefix::Number => list_marker(body).is_some_and(|_| !body.starts_with("- ")),
            Prefix::Quote => body.starts_with('>'),
        }
    };
    let blank = |line: &str| line.trim().is_empty();
    // A blank caret line starts a list or quote of its own.
    let fresh = blank(block) && !block.contains('\n');
    let remove = !fresh && block.split('\n').filter(|l| !blank(l)).all(has);

    let mut number = 0;
    let lines: Vec<String> = block
        .split('\n')
        .map(|line| {
            if blank(line) && !fresh {
                // A blank line ends a list: the next one counts from 1.
                number = 0;
                return line.to_string();
            }
            let (indent, body) = split(line);
            let head = line.get(..indent).unwrap_or("");
            let body = if fresh { "" } else { body };
            let new_body = match prefix {
                Prefix::Quote if remove => {
                    let rest = body.strip_prefix('>').unwrap_or(body);
                    rest.strip_prefix(' ').unwrap_or(rest).to_string()
                }
                Prefix::Quote if has(line) => body.to_string(),
                Prefix::Quote => format!("> {body}"),
                _ => {
                    let bare = list_marker(body)
                        .and_then(|n| body.get(n..))
                        .unwrap_or(body);
                    if remove {
                        bare.to_string()
                    } else if prefix == Prefix::Bullet {
                        format!("- {bare}")
                    } else {
                        number += 1;
                        format!("{number}. {bare}")
                    }
                }
            };
            [head, &new_body].concat()
        })
        .collect();
    let new_block = lines.join("\n");
    let out = splice(text, start, end, &new_block)?;
    if s < e {
        return Some((out, start, start + new_block.len()));
    }
    // The caret's line is the whole block; markers are ASCII, so moving
    // by the length change stays on a boundary unless it lands in them.
    let column = (s - start).saturating_add_signed(new_block.len() as isize - block.len() as isize);
    let column = column.min(new_block.len());
    let caret = if out.is_char_boundary(start + column) {
        start + column
    } else {
        start
    };
    Some((out, caret, caret))
}

/// The selection as a `[label](url)` link, or the address itself with no
/// selection (web addresses are linked as typed). An address without a
/// scheme gets `https://` (or `mailto:` for an e-mail address); one the
/// reader would not follow, or a label the converter can't read, is no
/// edit. The caret goes after the link.
pub fn insert_link(text: &str, a: usize, b: usize, url: &str) -> Option<Edited> {
    let (s, e) = span(text, a, b)?;
    let target = link_target(url)?;
    let sel = text.get(s..e)?;
    let label = sel.trim();
    let markup = if label.is_empty() {
        match target.strip_prefix("mailto:") {
            Some(address) => format!("[{address}]({target})"),
            None => target,
        }
    } else if label.contains(['[', ']', '\n']) {
        return None;
    } else {
        format!("[{label}]({target})")
    };
    let (s, e) = if label.is_empty() {
        (s, e)
    } else {
        let lead = sel.len() - sel.trim_start().len();
        (s + lead, s + lead + label.len())
    };
    let out = splice(text, s, e, &markup)?;
    let caret = s + markup.len();
    Some((out, caret, caret))
}

/// What a typed link address becomes: `http`, `https` and `mailto` kept,
/// a bare host or path given `https://`, an e-mail address `mailto:`,
/// any other scheme refused. Brackets are percent-encoded so the link's
/// markup stays readable to the converter.
pub fn link_target(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphabetic()));
    let full = match scheme {
        Some(_) => url.to_string(),
        None if url.contains('@') && !url.contains('/') => format!("mailto:{url}"),
        None => format!("https://{url}"),
    };
    links::sanitize(&full)?;
    let mut out = String::with_capacity(full.len());
    for c in full.chars() {
        match c {
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::mail::compose_html::to_html;

    /// The text with `[` and `]` around the selection (anchor first).
    fn show((text, a, c): Edited) -> String {
        let (s, e) = (a.min(c), a.max(c));
        format!("{}[{}]{}", &text[..s], &text[s..e], &text[e..])
    }

    /// The offsets of `[...]` in a marked-up text, and the bare text.
    fn sel(marked: &str) -> (String, usize, usize) {
        let s = marked.find('[').unwrap();
        let e = marked.find(']').unwrap() - 1;
        (marked.replacen('[', "", 1).replacen(']', "", 1), s, e)
    }

    fn wrap(marked: &str, marker: &str) -> String {
        let (t, s, e) = sel(marked);
        show(wrap_selection(&t, s, e, marker).unwrap())
    }

    fn prefix(marked: &str, p: Prefix) -> String {
        let (t, s, e) = sel(marked);
        show(toggle_line_prefix(&t, s, e, p).unwrap())
    }

    #[test]
    fn bold_and_italic_wrap_and_unwrap() {
        assert_eq!(wrap("say [hello] now", BOLD), "say **[hello]** now");
        assert_eq!(wrap("say **[hello]** now", BOLD), "say [hello] now");
        assert_eq!(wrap("say [**hello**] now", BOLD), "say [hello] now");
        assert_eq!(wrap("say [hello ]now", ITALIC), "say _[hello]_ now");
        assert_eq!(wrap("a[]b", BOLD), "a**[]**b");
        assert_eq!(wrap("a**[]**b", BOLD), "a[]b");
        assert_eq!(wrap("[  ]", BOLD), "  **[]**");
        // Multi-byte text: offsets stay on char boundaries.
        assert_eq!(wrap("a [ação] 😀", BOLD), "a **[ação]** 😀");
        assert_eq!(wrap("a ação [😀]", ITALIC), "a ação _[😀]_");
        // Part of a word takes the whole word.
        assert_eq!(wrap("the re[port]s", BOLD), "the **[reports]**");
        assert_eq!(wrap("a[çã]o!", ITALIC), "_[ação]_!");
        assert_eq!(wrap("**re[po]rt**", BOLD), "[report]");
        assert_eq!(wrap("[ação\n\n😀 x]", BOLD), "[**ação**\n\n**😀 x**]");
        assert_eq!(wrap("[**ação**\n\n**😀 x**]", BOLD), "[ação\n\n😀 x]");
        // Bold inside italic and italic inside bold.
        assert_eq!(wrap("_[word]_", BOLD), "_**[word]**_");
        assert_eq!(wrap("[**word**]", ITALIC), "_[**word**]_");
    }

    #[test]
    fn offsets_off_a_char_boundary_or_past_the_end_are_no_edit() {
        let t = "ação";
        assert!(wrap_selection(t, 0, 2, BOLD).is_none());
        assert!(wrap_selection(t, 0, 99, BOLD).is_none());
        assert!(toggle_line_prefix(t, 2, 2, Prefix::Quote).is_none());
        assert!(insert_link(t, 2, 2, "example.org").is_none());
        // Anchor after the cursor is the same selection.
        assert_eq!(wrap_selection(t, 6, 0, BOLD).unwrap().0, "**ação**");
    }

    #[test]
    fn toggles_reverse() {
        let texts = [
            "",
            "plain",
            "a ação b 😀 c",
            "one\ntwo\n\nthree 😀",
            "  - x\n> y",
        ];
        for t in texts {
            let ends: Vec<usize> = (0..=t.len()).filter(|&i| t.is_char_boundary(i)).collect();
            for &s in &ends {
                for &e in ends.iter().filter(|&&e| e >= s) {
                    for marker in [BOLD, ITALIC] {
                        let (once, a, b) = wrap_selection(t, s, e, marker).unwrap();
                        let (back, ..) = wrap_selection(&once, a, b, marker).unwrap();
                        assert_eq!(back, t, "{marker} on {t:?} {s}..{e}");
                    }
                    for p in [Prefix::Bullet, Prefix::Number, Prefix::Quote] {
                        let (once, a, b) = toggle_line_prefix(t, s, e, p).unwrap();
                        let (back, ..) = toggle_line_prefix(&once, a, b, p).unwrap();
                        let plain = !t.contains("- ") && !t.contains('>');
                        if plain {
                            assert_eq!(back, t, "{p:?} on {t:?} {s}..{e}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn lists_and_quotes() {
        assert_eq!(prefix("o[ne\ntw]o", Prefix::Bullet), "[- one\n- two]");
        assert_eq!(prefix("[- one\n- two]", Prefix::Bullet), "[one\ntwo]");
        assert_eq!(
            prefix("[- one\n\n- two]", Prefix::Number),
            "[1. one\n\n1. two]"
        );
        assert_eq!(prefix("[1. one\ntwo]", Prefix::Number), "[1. one\n2. two]");
        assert_eq!(prefix("[1. one\n2. two]", Prefix::Number), "[one\ntwo]");
        assert_eq!(prefix("x\n[ação\n]y", Prefix::Quote), "x\n[> ação]\ny");
        assert_eq!(prefix("[> > deep]", Prefix::Quote), "[> deep]");
        // The caret keeps its place in the line.
        assert_eq!(prefix("ab[]c", Prefix::Bullet), "- ab[]c");
        assert_eq!(prefix("- ab[]c", Prefix::Bullet), "ab[]c");
        assert_eq!(prefix("-[] abc", Prefix::Bullet), "[]abc");
        assert_eq!(prefix("😀[]", Prefix::Quote), "> 😀[]");
        assert_eq!(prefix("[]", Prefix::Quote), "> []");
        assert_eq!(
            prefix(
                "a
[]",
                Prefix::Bullet
            ),
            "a
- []"
        );
        assert_eq!(
            prefix(
                "a
[]",
                Prefix::Number
            ),
            "a
1. []"
        );
    }

    #[test]
    fn links() {
        let link = |marked: &str, url: &str| {
            let (t, s, e) = sel(marked);
            insert_link(&t, s, e, url).map(show)
        };
        assert_eq!(
            link("see [ação ]now", "example.org/a(b)").unwrap(),
            "see [ação](https://example.org/a%28b%29)[] now"
        );
        assert_eq!(
            link("at []", "https://x.org").unwrap(),
            "at https://x.org[]"
        );
        assert_eq!(
            link("[]", "ana@example.org").unwrap(),
            "[ana@example.org](mailto:ana@example.org)[]"
        );
        assert_eq!(link("[x]", "javascript:alert(1)"), None);
        assert_eq!(link("[x]", "two words"), None);
        assert_eq!(link("[x]", ""), None);
        assert_eq!(link("[a\nb]", "x.org"), None);
        assert_eq!(insert_link("a]b", 0, 3, "x.org"), None);
        assert_eq!(link_target("HTTP://x.org").as_deref(), Some("HTTP://x.org"));
        assert_eq!(
            link_target("x.org:8080/p").as_deref(),
            Some("https://x.org:8080/p")
        );
    }

    /// What the toolbar writes is what the HTML part shows.
    #[test]
    fn toolbar_output_converts() {
        let cases: [(Edited, &str); 6] = [
            (wrap_selection("a b", 0, 1, BOLD).unwrap(), "<b>a</b>"),
            (wrap_selection("a b", 2, 3, ITALIC).unwrap(), "<i>b</i>"),
            (
                toggle_line_prefix("x\ny", 0, 3, Prefix::Bullet).unwrap(),
                "<ul>",
            ),
            (
                toggle_line_prefix("x\ny", 0, 3, Prefix::Number).unwrap(),
                "<ol>",
            ),
            (
                toggle_line_prefix("x", 0, 0, Prefix::Quote).unwrap(),
                "<blockquote",
            ),
            (
                insert_link("go", 0, 2, "x.org").unwrap(),
                "<a href=\"https://x.org/\">go</a>",
            ),
        ];
        for ((text, ..), want) in cases {
            let html = to_html(&text);
            assert!(html.contains(want), "{text:?} -> {html}");
        }
        let (both, ..) = wrap_selection("_word_", 1, 5, BOLD).unwrap();
        assert!(
            to_html(&both).contains("<i><b>word</b></i>"),
            "{}",
            to_html(&both)
        );
        let (both, ..) = wrap_selection("**word**", 0, 8, ITALIC).unwrap();
        assert!(
            to_html(&both).contains("<i><b>word</b></i>"),
            "{}",
            to_html(&both)
        );
    }
}
