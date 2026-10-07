//! Folder roles. One table holds everything the app knows about each role:
//! how it is stored, its icon, its sidebar position, and how servers mark
//! or name it.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    #[default]
    Other,
}

struct Info {
    /// Value stored in the `folders.role` column.
    key: &'static str,
    /// Segoe Fluent Icons / MDL2 glyph.
    icon: &'static str,
    /// RFC 6154 special-use attribute.
    attr: &'static str,
    /// Common folder names (lowercase) for servers without special-use.
    names: &'static [&'static str],
    /// Whether the sidebar shows an unread count.
    shows_unread: bool,
}

/// Indexed by `Role as usize`; the order is also the sidebar order.
const ROLES: [Info; 7] = [
    Info {
        key: "inbox",
        icon: "\u{E715}",
        attr: "",
        // Only the top-level INBOX is the inbox (RFC 3501); see `from_name`.
        names: &[],
        shows_unread: true,
    },
    Info {
        key: "drafts",
        icon: "\u{E70B}",
        attr: "\\Drafts",
        names: &["drafts", "draft"],
        shows_unread: false,
    },
    Info {
        key: "sent",
        icon: "\u{E724}",
        attr: "\\Sent",
        names: &["sent", "sent items", "sent messages", "sent mail"],
        shows_unread: false,
    },
    Info {
        key: "archive",
        icon: "\u{E7B8}",
        attr: "\\Archive",
        names: &["archive", "archives"],
        shows_unread: true,
    },
    Info {
        key: "junk",
        icon: "\u{E8C9}",
        attr: "\\Junk",
        names: &["junk", "spam", "junk email", "junk e-mail"],
        shows_unread: true,
    },
    Info {
        key: "trash",
        icon: "\u{E74D}",
        attr: "\\Trash",
        names: &["trash", "deleted items", "deleted messages", "bin"],
        shows_unread: false,
    },
    Info {
        key: "",
        icon: "\u{E8B7}",
        attr: "",
        names: &[],
        shows_unread: true,
    },
];

const ALL: [Role; 7] = [
    Role::Inbox,
    Role::Drafts,
    Role::Sent,
    Role::Archive,
    Role::Junk,
    Role::Trash,
    Role::Other,
];

impl Role {
    fn info(self) -> &'static Info {
        &ROLES[self as usize]
    }

    pub fn key(self) -> &'static str {
        self.info().key
    }

    pub fn icon(self) -> &'static str {
        self.info().icon
    }

    /// Sidebar sort position.
    pub fn rank(self) -> u8 {
        self as u8
    }

    pub fn shows_unread(self) -> bool {
        self.info().shows_unread
    }

    /// Folders holding our own outgoing mail (the list shows recipients).
    pub fn is_outgoing(self) -> bool {
        matches!(self, Role::Sent | Role::Drafts)
    }

    pub fn from_key(key: &str) -> Role {
        ALL.into_iter().find(|r| r.key() == key).unwrap_or_default()
    }

    /// Role for a special-use attribute such as `\Sent`.
    pub fn from_attr(attr: &str) -> Option<Role> {
        ALL.into_iter()
            .find(|r| !r.info().attr.is_empty() && r.info().attr.eq_ignore_ascii_case(attr))
    }

    /// Role from a folder's full server name and decoded leaf name.
    pub fn from_name(full: &str, leaf: &str) -> Option<Role> {
        if full.eq_ignore_ascii_case("INBOX") {
            return Some(Role::Inbox);
        }
        ALL.into_iter()
            .find(|r| r.info().names.iter().any(|n| n.eq_ignore_ascii_case(leaf)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_enum() {
        for (i, role) in ALL.into_iter().enumerate() {
            assert_eq!(role as usize, i);
            assert_eq!(Role::from_key(role.key()), role);
            assert!(!role.icon().is_empty());
        }
        assert!(Role::Inbox.rank() < Role::Sent.rank() && Role::Trash.rank() < Role::Other.rank());
    }

    #[test]
    fn lookups() {
        assert_eq!(Role::from_attr("\\sent"), Some(Role::Sent));
        assert_eq!(Role::from_attr("\\All"), None);
        assert_eq!(Role::from_name("INBOX", "INBOX"), Some(Role::Inbox));
        assert_eq!(Role::from_name("Work/Inbox", "Inbox"), None);
        assert_eq!(
            Role::from_name("INBOX.Sent Items", "Sent Items"),
            Some(Role::Sent)
        );
        assert_eq!(
            Role::from_name("Junk E-mail", "Junk E-mail"),
            Some(Role::Junk)
        );
        assert_eq!(Role::from_key("bogus"), Role::Other);
    }
}
