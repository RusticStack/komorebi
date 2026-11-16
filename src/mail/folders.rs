//! Folder names for CREATE, RENAME and DELETE: where a new folder goes on
//! the server (its hierarchy delimiter, and the `INBOX.` prefix of servers
//! whose personal namespace is under the inbox, RFC 2342),
//! how a name the user typed is written there (modified UTF-7, RFC 3501
//! §5.1.3) and which names are refused before anything is sent.
//!
//! Pure: the session calls are in `imap.rs` ([`super::imap::Imap::hierarchy`]
//! and the folder commands), the queued jobs in `mail::Worker`.

/// How folder names nest on a server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hierarchy {
    /// The hierarchy delimiter (`/`, `.`); `None`: a flat server.
    pub delimiter: Option<String>,
    /// The personal namespace's prefix, where top-level folders go
    /// (`INBOX.` on Courier and older Cyrus; usually empty).
    pub prefix: String,
}

/// Why a typed name can't be a folder name, before the server is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadName {
    /// Nothing but spaces.
    Empty,
    /// Longer than [`MAX_NAME`] characters.
    TooLong,
    /// A line break, a control character, or a LIST wildcard (`*`, `%`).
    Character(char),
    /// The server's hierarchy delimiter (it would nest the folder).
    Delimiter(String),
}

impl std::fmt::Display for BadName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BadName::Empty => f.write_str("the folder name is empty"),
            BadName::TooLong => write!(f, "folder names are at most {MAX_NAME} characters"),
            BadName::Character(c) => write!(f, "folder names can't contain {c:?}"),
            BadName::Delimiter(d) => write!(f, "folder names can't contain {d:?} on this server"),
        }
    }
}

impl std::error::Error for BadName {}

/// The longest name a folder may be given here (servers allow more; a
/// sidebar line shows far less).
pub const MAX_NAME: usize = 100;

/// A typed name trimmed and checked for what no server takes. The
/// delimiter is the server's business ([`Hierarchy::child`]).
pub fn clean(name: &str) -> Result<&str, BadName> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BadName::Empty);
    }
    if name.chars().count() > MAX_NAME {
        return Err(BadName::TooLong);
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || matches!(c, '*' | '%'))
    {
        return Err(BadName::Character(c));
    }
    Ok(name)
}

impl Hierarchy {
    /// The hierarchy of a server with this delimiter and these folders
    /// (server names): top-level folders go under `INBOX` and the delimiter
    /// when every folder but the inbox is there (Courier, Cyrus without the
    /// alternate namespace), else at the top.
    pub fn judged(delimiter: Option<String>, names: &[String]) -> Self {
        let prefix = delimiter.as_deref().map_or_else(String::new, |d| {
            let under = format!("INBOX{d}");
            let mut others = names
                .iter()
                .filter(|n| !n.eq_ignore_ascii_case("INBOX"))
                .peekable();
            let nested = others.peek().is_some()
                && others.all(|n| {
                    n.get(..under.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(&under))
                });
            if nested {
                // As the server spells it.
                names
                    .iter()
                    .find_map(|n| {
                        n.get(..under.len())
                            .filter(|p| p.eq_ignore_ascii_case(&under))
                    })
                    .unwrap_or(&under)
                    .to_string()
            } else {
                String::new()
            }
        });
        Hierarchy { delimiter, prefix }
    }

    /// The server name of a new folder `leaf` (as typed) under the folder
    /// named `parent` (its server name), or at the top of the personal
    /// namespace.
    pub fn child(&self, parent: Option<&str>, leaf: &str) -> Result<String, BadName> {
        let leaf = self.leaf(leaf)?;
        Ok(match (parent, &self.delimiter) {
            (Some(p), Some(d)) => format!("{p}{d}{leaf}"),
            // A flat server has no subfolders: the new folder goes at the
            // top (the app never offers it; the worker stays safe).
            _ => format!("{}{leaf}", self.prefix),
        })
    }

    /// The server name for `name` renamed to `leaf`, under the same parent.
    pub fn renamed(&self, name: &str, leaf: &str) -> Result<String, BadName> {
        let leaf = self.leaf(leaf)?;
        Ok(match self.parent_of(name) {
            Some(parent) => format!("{parent}{}{leaf}", self.delimiter.as_deref().unwrap_or("")),
            None => {
                // At the top: keep the namespace prefix the folder had.
                let prefix = if !self.prefix.is_empty() && name.starts_with(&self.prefix) {
                    self.prefix.as_str()
                } else {
                    ""
                };
                format!("{prefix}{leaf}")
            }
        })
    }

    /// The parent's server name, if `name` is nested (`INBOX.Work` is
    /// under `INBOX`).
    pub fn parent_of<'a>(&self, name: &'a str) -> Option<&'a str> {
        let d = self.delimiter.as_deref()?;
        let (parent, _) = name.rsplit_once(d)?;
        (!parent.is_empty()).then_some(parent)
    }

    /// Whether `name` is `folder` or one of its subfolders.
    pub fn within(&self, name: &str, folder: &str) -> bool {
        name == folder
            || self
                .delimiter
                .as_deref()
                .is_some_and(|d| name.strip_prefix(folder).is_some_and(|r| r.starts_with(d)))
    }

    /// A typed leaf, checked and in modified UTF-7.
    fn leaf(&self, typed: &str) -> Result<String, BadName> {
        let leaf = clean(typed)?;
        if let Some(d) = self.delimiter.as_deref().filter(|d| leaf.contains(*d)) {
            return Err(BadName::Delimiter(d.to_string()));
        }
        Ok(encode_mutf7(leaf))
    }
}

/// A name in IMAP's modified UTF-7 (RFC 3501 §5.1.3): printable ASCII as
/// it is (`&` as `&-`), anything else as `&` base64 of UTF-16 `-`.
pub fn encode_mutf7(s: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    fn flush(units: &mut Vec<u16>, out: &mut String) {
        if units.is_empty() {
            return;
        }
        out.push('&');
        let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_be_bytes()).collect();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            let digits = chunk.len() + 1;
            for i in 0..digits {
                let v = (n >> (18 - 6 * i)) & 63;
                out.push(char::from(ALPHABET[v as usize]));
            }
        }
        out.push('-');
        units.clear();
    }
    let mut out = String::with_capacity(s.len());
    let mut units = Vec::new();
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut units, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut units, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::imap::decode_mutf7;

    fn h(delimiter: Option<&str>, prefix: &str) -> Hierarchy {
        Hierarchy {
            delimiter: delimiter.map(str::to_string),
            prefix: prefix.to_string(),
        }
    }

    #[test]
    fn modified_utf7_round_trips() {
        for s in [
            "Work",
            "Entwürfe",
            "日本語",
            "a&b",
            "&",
            "Résumé 2024",
            "😀 smile",
            "~ tilde",
        ] {
            assert_eq!(decode_mutf7(&encode_mutf7(s)), s, "{s}");
        }
        assert_eq!(encode_mutf7("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(encode_mutf7("a&b"), "a&-b");
        assert_eq!(encode_mutf7("日本語"), "&ZeVnLIqe-");
    }

    #[test]
    fn new_names_follow_the_delimiter_and_the_namespace() {
        let slash = h(Some("/"), "");
        assert_eq!(slash.child(None, " Work ").unwrap(), "Work");
        assert_eq!(slash.child(Some("Work"), "2024").unwrap(), "Work/2024");
        assert_eq!(
            slash.child(None, "a/b"),
            Err(BadName::Delimiter("/".into()))
        );
        // A dotted server takes a slash, refuses a dot.
        let dotted = h(Some("."), "INBOX.");
        assert_eq!(dotted.child(None, "Work").unwrap(), "INBOX.Work");
        assert_eq!(
            dotted.child(Some("INBOX.Work"), "a/b").unwrap(),
            "INBOX.Work.a/b"
        );
        assert_eq!(
            dotted.child(None, "v1.2"),
            Err(BadName::Delimiter(".".into()))
        );
        // Flat: always at the top.
        let flat = h(None, "");
        assert_eq!(flat.child(Some("Work"), "x").unwrap(), "x");
        assert_eq!(slash.child(None, "Café").unwrap(), "Caf&AOk-");
    }

    #[test]
    fn typed_names_are_checked() {
        assert_eq!(clean("   "), Err(BadName::Empty));
        assert_eq!(clean("a\nb"), Err(BadName::Character('\n')));
        assert_eq!(clean("a*"), Err(BadName::Character('*')));
        assert_eq!(clean("50%"), Err(BadName::Character('%')));
        assert_eq!(clean(&"x".repeat(MAX_NAME + 1)), Err(BadName::TooLong));
        assert_eq!(clean(" Receipts "), Ok("Receipts"));
    }

    #[test]
    fn renames_keep_the_parent_and_the_prefix() {
        let slash = h(Some("/"), "");
        assert_eq!(slash.renamed("Work/2024", "2025").unwrap(), "Work/2025");
        assert_eq!(slash.renamed("Work", "Jobs").unwrap(), "Jobs");
        let dotted = h(Some("."), "INBOX.");
        assert_eq!(dotted.renamed("INBOX.Work", "Jobs").unwrap(), "INBOX.Jobs");
        assert_eq!(dotted.renamed("INBOX.Work.A", "B").unwrap(), "INBOX.Work.B");
        assert_eq!(dotted.parent_of("INBOX.Work"), Some("INBOX"));
        assert_eq!(slash.parent_of("Work"), None);
        assert_eq!(dotted.parent_of("INBOX.Work.A"), Some("INBOX.Work"));
        assert!(slash.within("Work/2024", "Work"));
        assert!(slash.within("Work", "Work"));
        assert!(!slash.within("Workshop", "Work"));
    }

    #[test]
    fn the_inbox_prefix_is_judged_from_the_folders() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let dot = Some(".".to_string());
        let courier = names(&["INBOX", "INBOX.Sent", "INBOX.Trash", "INBOX.Work"]);
        assert_eq!(Hierarchy::judged(dot.clone(), &courier).prefix, "INBOX.");
        let mixed = names(&["INBOX", "INBOX.Receipts", "Sent", "Work"]);
        assert_eq!(Hierarchy::judged(dot.clone(), &mixed).prefix, "");
        assert_eq!(Hierarchy::judged(dot, &names(&["INBOX"])).prefix, "");
        assert_eq!(Hierarchy::judged(None, &courier).prefix, "");
    }
}
