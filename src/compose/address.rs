//! Addresses in the composer: who a reply goes to, and the address being
//! typed at the caret of an address field (for suggestions).

use crate::db;
use crate::mail::smtp;

/// To and Cc for a reply to `m`. A reply to one's own message (a Sent item)
/// goes to its original To. Addresses compare by bare address, so excluding
/// `al@x.org` leaves `val@x.org` alone.
pub fn reply_recipients(
    m: &db::Message,
    reply_to: &str,
    me: &str,
    all: bool,
) -> (Vec<String>, Vec<String>) {
    let me = bare_address(me);
    let not_me = |a: &String| me.is_empty() || bare_address(a) != me;
    let mut to = if !me.is_empty() && bare_address(&m.from_addr) == me {
        smtp::split_addresses(&m.to)
    } else if reply_to.is_empty() {
        vec![m.from_addr.clone()]
    } else {
        vec![reply_to.to_string()]
    };
    let mut cc = Vec::new();
    if all {
        let listed =
            |list: &[String], a: &String| list.iter().any(|t| bare_address(t) == bare_address(a));
        for a in smtp::split_addresses(&m.to) {
            if not_me(&a) && !listed(&to, &a) {
                to.push(a);
            }
        }
        for a in smtp::split_addresses(&m.cc) {
            if not_me(&a) && !listed(&to, &a) && !listed(&cc, &a) {
                cc.push(a);
            }
        }
    }
    (to, cc)
}

/// Where the caret most likely is after an edit turned `old` into `new`:
/// just past the changed span. A line field reports its text, not its
/// caret; within a run of equal characters this picks the run's end.
pub fn caret_after_edit(old: &str, new: &str) -> usize {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = 0;
    for (a, b) in old.chars().rev().zip(new.chars().rev()) {
        if a != b || suffix + b.len_utf8() > room {
            break;
        }
        suffix += b.len_utf8();
    }
    new.len() - suffix
}

/// The address around byte `cursor` of an address field, as byte offsets
/// (start, cursor, end), with `cursor` moved onto the text. Addresses end
/// at a `,` or `;` outside quotes and angle brackets, as in
/// `smtp::split_addresses`.
pub fn address_at(text: &str, cursor: usize) -> (usize, usize, usize) {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let (mut start, mut end) = (0, text.len());
    let (mut quoted, mut angle, mut escaped) = (false, false, false);
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                if i < cursor {
                    start = i + 1;
                } else {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    (start, cursor, end)
}

/// What is typed of the address at `cursor`, up to the caret.
pub fn typed_address(text: &str, cursor: usize) -> &str {
    let (start, cursor, _) = address_at(text, cursor);
    text[start..cursor]
        .trim()
        .trim_start_matches(['"', '<'])
        .trim_start()
}

/// Replace the address at `cursor` with `mailbox` and a `, ` separator
/// (taking the place of the one after it, if any). Returns the new text
/// and the caret, just past the separator.
pub fn complete_address(text: &str, cursor: usize, mailbox: &str) -> (String, usize) {
    let (start, _, end) = address_at(text, cursor);
    let head = text[..start].trim_end();
    let tail = &text[end..];
    let tail = tail.strip_prefix([',', ';']).unwrap_or(tail).trim_start();
    let mut out = String::with_capacity(text.len() + mailbox.len() + 3);
    out.push_str(head);
    if !head.is_empty() {
        out.push(' ');
    }
    out.push_str(mailbox);
    out.push_str(", ");
    let caret = out.len();
    out.push_str(tail);
    (out, caret)
}

/// `Name <addr>` or `addr` to lowercase `addr`.
pub fn bare_address(a: &str) -> String {
    let a = a.trim();
    let addr = match (a.rfind('<'), a.rfind('>')) {
        (Some(l), Some(r)) if l < r => &a[l + 1..r],
        _ => a,
    };
    addr.trim().to_lowercase()
}
