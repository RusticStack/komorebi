//! Contacts: the people in the local address book and in the CardDAV
//! address books an account syncs (`crate::dav::carddav`). Plain data and
//! the pure helpers around it: validation, the sort key and the search
//! terms. Storage lives in `db/contacts.rs`, vCard in `mail/vcard.rs`.
//!
//! Contacts are user data, not cache: they live in their own tables and
//! are never written into the derived `addresses` index, which stays the
//! fallback for recipient suggestions.

use crate::fold::fold;

/// Emails and phones kept per contact (more are dropped on reading).
pub const MAX_EMAILS: usize = 100;
pub const MAX_PHONES: usize = 100;
/// Characters kept in one field.
pub const MAX_FIELD: usize = 4_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Email {
    /// Lower-cased.
    pub addr: String,
    /// `work`, `home`, `other` or the server's own word (may be empty).
    pub label: String,
    /// The address to write to when the contact is chosen.
    pub pref: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Phone {
    pub number: String,
    pub label: String,
}

/// One person (a vCard with `KIND:individual`, or none).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    /// 0 for one not stored yet.
    pub id: i64,
    pub book_id: i64,
    /// The vCard UID (a new contact gets one from [`new_uid`]).
    pub uid: String,
    /// The formatted name (vCard `FN`).
    pub name: String,
    pub given: String,
    pub family: String,
    pub org: String,
    pub title: String,
    pub note: String,
    pub emails: Vec<Email>,
    pub phones: Vec<Phone>,
    /// Group names (vCard `CATEGORIES`).
    pub groups: Vec<String>,
}

/// Why a contact can't be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// No name, organization, email or phone.
    Empty,
    /// The email at this position is not an address.
    Email(usize),
}

impl Contact {
    /// The name shown: `FN`, else given and family, else the organization,
    /// else the first email.
    pub fn display(&self) -> String {
        let n = self.name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        let gf = format!("{} {}", self.given.trim(), self.family.trim());
        let gf = gf.trim();
        if !gf.is_empty() {
            return gf.to_string();
        }
        if !self.org.trim().is_empty() {
            return self.org.trim().to_string();
        }
        self.emails
            .first()
            .map(|e| e.addr.clone())
            .unwrap_or_default()
    }

    /// The preferred email: the one marked, else the first.
    pub fn preferred_email(&self) -> Option<&Email> {
        self.emails
            .iter()
            .find(|e| e.pref)
            .or_else(|| self.emails.first())
    }

    /// Trim the fields, lower-case the addresses, drop blank rows, and
    /// check what is left.
    pub fn normalize(&mut self) -> Result<(), Invalid> {
        for s in [
            &mut self.name,
            &mut self.given,
            &mut self.family,
            &mut self.org,
            &mut self.title,
        ] {
            *s = clip(s.trim());
        }
        self.note = clip(self.note.trim_end());
        self.emails.retain(|e| !e.addr.trim().is_empty());
        self.emails.truncate(MAX_EMAILS);
        for (i, e) in self.emails.iter_mut().enumerate() {
            e.addr = e.addr.trim().to_lowercase();
            e.label = clip(e.label.trim());
            if !valid_address(&e.addr) {
                return Err(Invalid::Email(i));
            }
        }
        self.phones.retain(|p| !p.number.trim().is_empty());
        self.phones.truncate(MAX_PHONES);
        for p in &mut self.phones {
            p.number = clip(p.number.trim());
            p.label = clip(p.label.trim());
        }
        let mut seen = std::collections::HashSet::new();
        self.groups = std::mem::take(&mut self.groups)
            .into_iter()
            .map(|g| clip(g.trim()))
            .filter(|g| !g.is_empty() && seen.insert(fold(g).into_owned()))
            .collect();
        if self.name.is_empty()
            && self.given.is_empty()
            && self.family.is_empty()
            && self.org.is_empty()
            && self.emails.is_empty()
            && self.phones.is_empty()
        {
            return Err(Invalid::Empty);
        }
        if self.uid.is_empty() {
            self.uid = new_uid();
        }
        Ok(())
    }
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_FIELD).collect()
}

/// Whether `addr` looks like `local@domain` (the composer's own check).
pub fn valid_address(addr: &str) -> bool {
    let (_, a) = crate::mail::smtp::mailbox_parts(addr);
    a == addr
        && a.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.ends_with('.'))
        && !a
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '<' || c == '>')
}

/// The list order: family name, then given name, then the shown name,
/// folded so case and accents don't split the alphabet.
pub fn sort_key(c: &Contact) -> String {
    let mut key = String::new();
    for part in [c.family.as_str(), c.given.as_str(), &c.display()] {
        key.push_str(&fold(part.trim()));
        key.push('\u{1}');
    }
    key
}

/// The words a contact is found by: each folded word of the names and
/// organization, and each address with its local part and domain.
pub fn terms(c: &Contact) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        let t = t.trim();
        if !t.is_empty() && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    for field in [&c.name, &c.given, &c.family, &c.org] {
        let f = fold(field);
        for w in f.split(|ch: char| !ch.is_alphanumeric()) {
            push(w);
        }
        // The whole name too, so "ana sil" finds "Ana Silva".
        push(&f);
    }
    for e in &c.emails {
        push(&e.addr);
        if let Some((local, domain)) = e.addr.split_once('@') {
            push(domain);
            for w in local.split(['.', '_', '-', '+']) {
                push(w);
            }
        }
    }
    out
}

/// A vCard UID nobody else has (a random UUID-shaped string, no machine
/// details).
pub fn new_uid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.finish()
    };
    let (a, b) = (part(1), part(2));
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        b >> 52,
        b & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ana() -> Contact {
        Contact {
            name: "Ana Conceição".into(),
            given: "Ana".into(),
            family: "Conceição".into(),
            emails: vec![Email {
                addr: " Ana.Silva@Example.PT ".into(),
                ..Email::default()
            }],
            ..Contact::default()
        }
    }

    #[test]
    fn normalize_validates_and_lowercases() {
        let mut c = ana();
        c.normalize().unwrap();
        assert_eq!(c.emails[0].addr, "ana.silva@example.pt");
        assert!(!c.uid.is_empty());
        let mut bad = ana();
        bad.emails.push(Email {
            addr: "not an address".into(),
            ..Email::default()
        });
        assert_eq!(bad.normalize(), Err(Invalid::Email(1)));
        assert_eq!(Contact::default().normalize(), Err(Invalid::Empty));
    }

    #[test]
    fn terms_and_sort_key_fold() {
        let mut c = ana();
        c.normalize().unwrap();
        let t = terms(&c);
        for want in ["ana", "conceicao", "ana conceicao", "example.pt", "silva"] {
            assert!(t.iter().any(|x| x == want), "{want} in {t:?}");
        }
        assert!(sort_key(&c).starts_with("conceicao\u{1}ana"));
        assert_ne!(new_uid(), new_uid());
    }
}
