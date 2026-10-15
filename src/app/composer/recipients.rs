//! The To, Cc and Bcc fields as person tokens (`ui/chips.slint`): a view
//! of the field's text, which stays the draft's model. The entries before
//! the last separator (`,` or `;`, outside quotes and angle brackets) show
//! as chips; what follows it is the text being typed. Nothing here changes
//! the text but a removed chip or Enter, so drafts, sending and reply-all
//! keep the text form they always had.

use super::*;
use crate::mail::smtp;

/// Where the text being typed starts: past the last separator outside
/// quotes and angle brackets and the spaces after it (0: none).
pub(crate) fn tail_start(text: &str) -> usize {
    let (mut quoted, mut angle, mut escaped) = (false, false, false);
    let mut start = 0;
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
            ',' | ';' if !quoted && !angle => start = i + 1,
            _ => {}
        }
    }
    let rest = &text[start..];
    start + (rest.len() - rest.trim_start().len())
}

/// The chips' part of `text`: up to the text being typed.
pub(crate) fn head(text: &str) -> &str {
    &text[..tail_start(text)]
}

/// The text being typed (after the chips).
pub(crate) fn tail(text: &str) -> &str {
    &text[tail_start(text)..]
}

/// One chip: what it shows (the name, else the address), the address,
/// and whether the entry is a valid address.
#[derive(Debug, PartialEq)]
pub(crate) struct Token {
    pub label: String,
    pub addr: String,
    pub valid: bool,
}

/// The chips of `list`, one per entry.
pub(crate) fn tokens(list: &str) -> Vec<Token> {
    smtp::split_addresses(list)
        .iter()
        .map(|entry| {
            let (name, addr) = smtp::mailbox_parts(entry);
            Token {
                label: if name.is_empty() {
                    addr.to_string()
                } else {
                    name.clone()
                },
                addr: addr.to_string(),
                valid: smtp::first_invalid_address(entry).is_none(),
            }
        })
        .collect()
}

/// `text` without chip `index` of its head (the text being typed stays).
pub(crate) fn remove(text: &str, index: usize) -> String {
    let mut entries = smtp::split_addresses(head(text));
    if index >= entries.len() {
        return text.to_string();
    }
    entries.remove(index);
    let mut out = entries.join(", ");
    if !out.is_empty() {
        out.push_str(", ");
    }
    out.push_str(tail(text));
    out
}

/// Enter: the text being typed becomes a chip (a separator after it).
pub(crate) fn commit(text: &str) -> String {
    if tail(text).trim().is_empty() {
        return text.to_string();
    }
    let mut out = text.trim_end().to_string();
    out.push_str(", ");
    out
}

/// Entry `index` of the head as typed, for the clipboard.
pub(crate) fn entry(text: &str, index: usize) -> String {
    smtp::split_addresses(head(text))
        .into_iter()
        .nth(index)
        .unwrap_or_default()
}

/// The `Recipients` global's callbacks, on each window.
pub(crate) fn wire(ui: &AppWindow) {
    let g = ui.global::<crate::Recipients>();
    g.on_chips(|list| {
        let rows: Vec<crate::RecipientChip> = tokens(&list)
            .into_iter()
            .map(|t| crate::RecipientChip {
                label: t.label.into(),
                addr: t.addr.into(),
                valid: t.valid,
            })
            .collect();
        ModelRc::new(VecModel::from(rows))
    });
    g.on_head(|text| head(&text).into());
    g.on_tail(|text| tail(&text).into());
    g.on_bytes(|text| i32::try_from(text.len()).unwrap_or(i32::MAX));
    g.on_remove(|text, i| remove(&text, usize::try_from(i).unwrap_or(usize::MAX)).into());
    g.on_commit(|text| commit(&text).into());
    g.on_entry(|text, i| entry(&text, usize::try_from(i).unwrap_or(usize::MAX)).into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_and_tail_split_at_the_last_separator() {
        let t = "Ann <a@x.pt>, \"Doe, Jane\" <j@x.pt>;  bo";
        assert_eq!(head(t), "Ann <a@x.pt>, \"Doe, Jane\" <j@x.pt>;  ");
        assert_eq!(tail(t), "bo");
        assert_eq!(format!("{}{}", head(t), tail(t)), t);
        assert_eq!(head("a@x.pt"), "");
        assert_eq!(tail("a@x.pt"), "a@x.pt");
        assert_eq!(tail("a@x.pt, "), "");
        assert_eq!(tail(""), "");
        // A comma inside quotes or brackets is no separator.
        assert_eq!(tail("\"Doe, J"), "\"Doe, J");
        assert_eq!(tail("Ann <a,"), "Ann <a,");
    }

    #[test]
    fn a_pasted_list_makes_one_chip_per_address() {
        let t = "Name <a@b.pt>, c@d.pt";
        let all = tokens(t);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "Name");
        assert_eq!(all[0].addr, "a@b.pt");
        assert_eq!(all[1].label, "c@d.pt");
        assert!(all.iter().all(|c| c.valid));
        // Typing: the last one is still text until a separator follows.
        assert_eq!(tokens(head(t)).len(), 1);
        assert_eq!(tail(t), "c@d.pt");
    }

    #[test]
    fn quoted_names_stay_one_chip_and_bad_ones_are_marked() {
        let all = tokens("\"Doe, Jane\" <j@x.pt>, not an address, ");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "Doe, Jane");
        assert!(all[0].valid);
        assert_eq!(all[1].label, "not an address");
        assert!(!all[1].valid);
    }

    #[test]
    fn removing_keeps_the_rest_and_the_typed_text() {
        let t = "a@x.pt, Bo <b@x.pt>; c@x.pt, d";
        assert_eq!(remove(t, 0), "Bo <b@x.pt>, c@x.pt, d");
        assert_eq!(remove(t, 1), "a@x.pt, c@x.pt, d");
        assert_eq!(remove(t, 2), "a@x.pt, Bo <b@x.pt>, d");
        assert_eq!(remove(t, 3), t);
        assert_eq!(remove("a@x.pt, ", 0), "");
        // Backspace twice on an empty tail: the last chip goes.
        assert_eq!(remove("a@x.pt, b@x.pt, ", 1), "a@x.pt, ");
    }

    #[test]
    fn enter_commits_the_typed_address() {
        assert_eq!(commit("a@x.pt, b@x.pt "), "a@x.pt, b@x.pt, ");
        assert_eq!(commit("a@x.pt, "), "a@x.pt, ");
        assert_eq!(commit(""), "");
        assert_eq!(tokens(head(&commit("a@x.pt, b@x.pt"))).len(), 2);
    }

    #[test]
    fn entries_copy_as_typed() {
        assert_eq!(entry("Bo <b@x.pt>, c@x.pt, ", 0), "Bo <b@x.pt>");
        assert_eq!(entry("Bo <b@x.pt>, c", 1), "");
    }

    #[test]
    fn two_hundred_recipients_split_quickly() {
        let list: String = (0..200).map(|i| format!("P{i} <p{i}@x.pt>, ")).collect();
        let start = std::time::Instant::now();
        assert_eq!(tokens(head(&list)).len(), 200);
        assert_eq!(remove(&list, 199).matches('@').count(), 199);
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
    }
}
