//! The address-book sync pane and the CardDAV passes without a toolkit
//! (the Contacts view of both apps).
//!
//! Per account: sync off, read only, or read and write; the server address
//! ("Find" reads SRV and `/.well-known/carddav` without credentials); the
//! user name; and the mail password or a separate one kept in Windows
//! Credential Manager (`account:{id}:dav`). "Find address books" is the
//! first authenticated request, made only on that press.
//!
//! A pass is a short-lived thread per account (at most one at a time, the
//! caller's rule) on its own connection to the cache
//! ([`Db::open_worker`]). It starts when the Contacts view opens, on Sync
//! now, after a local edit, and from the mail check timer at most every
//! `interval_min` minutes: never before the first frame, and never under
//! `KOMOREBI_OFFLINE`. Threads and the way back to the UI are the app's;
//! this module is what runs on them and what the pane shows.

use crate::dav::{self, Client, DavError, carddav};
use crate::db::contacts::{BOOK_LOCAL, Book, DavAccount};
use crate::db::{Account, Db};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// The DAV settings of `account`, or the defaults for an account with
/// none: sync off, its address as the user name, every 30 minutes.
pub fn settings(db: &Db, accounts: &[Account], account: i64) -> DavAccount {
    db.dav_account(account)
        .ok()
        .flatten()
        .unwrap_or_else(|| DavAccount {
            account_id: account,
            user: accounts
                .iter()
                .find(|a| a.id == account)
                .map(|a| a.email.clone())
                .unwrap_or_default(),
            interval_min: 30,
            ..DavAccount::default()
        })
}

/// The password a pass signs in with.
pub fn password(d: &DavAccount) -> Result<String> {
    if d.separate_secret {
        crate::secrets::get_dav_password(d.account_id)
    } else {
        crate::secrets::get_password(d.account_id)
    }
}

/// The pane's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub url: String,
    pub user: String,
    /// 0 off, 1 read only, 2 read and write.
    pub mode: i64,
    /// The mail password signs in (else a separate one).
    pub same: bool,
    /// A separate password typed; kept in Windows Credential Manager on
    /// save, never in the cache.
    pub password: String,
}

impl Form {
    pub fn of(d: &DavAccount) -> Self {
        Self {
            url: d.url.clone(),
            user: d.user.clone(),
            mode: d.mode,
            same: !d.separate_secret,
            password: String::new(),
        }
    }
}

/// Save the form's fields for `account`. `allow_private` stays as stored:
/// only an explicit choice (shared with CalDAV) allows a server on the
/// local network. Returns the settings saved and whether a typed password
/// was kept (the field then empties). The error is the credential store's
/// or the cache's.
pub fn save(
    db: &Db,
    accounts: &[Account],
    account: i64,
    form: &Form,
) -> Result<(DavAccount, bool)> {
    let mut d = settings(db, accounts, account);
    d.url = form.url.trim().to_string();
    d.user = form.user.trim().to_string();
    d.mode = form.mode.clamp(0, 2);
    d.separate_secret = !form.same;
    let mut kept = false;
    if d.separate_secret
        && !form.password.is_empty()
        && crate::dev_var("KOMOREBI_OFFLINE").is_none()
    {
        crate::secrets::set_dav_password(account, &form.password)?;
        kept = true;
    }
    db.set_dav_account(&d)?;
    Ok((d, kept))
}

/// Whether choosing `mode` asks "Remove the synced contacts?" first: sync
/// goes off while it was on and there are contacts it synced.
pub fn asks_to_remove(db: &Db, account: i64, previous_mode: i64, mode: i64) -> bool {
    mode == 0 && previous_mode > 0 && db.synced_contact_count(account).unwrap_or(0) > 0
}

/// Sync goes off and the synced contacts stay on this computer, no longer
/// synced.
pub fn turn_off_keeping(db: &Db, account: i64) {
    db.set_dav_mode(account, 0).ok();
}

/// An account's address books and what the pane says under them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Books {
    /// Name and whether it syncs.
    pub lines: Vec<(String, bool)>,
    /// Contacts of the account on this computer.
    pub count: i64,
    /// The latest sync of any of its books (Unix seconds).
    pub last: Option<i64>,
}

impl Books {
    /// The count and the time of the last sync, once there is something to
    /// say ("12 contacts on this computer. Last synced 14:05.").
    pub fn status(&self) -> Option<(i64, i64)> {
        match self.last {
            Some(ts) if !self.lines.is_empty() => Some((self.count, ts)),
            _ => None,
        }
    }
}

/// The books of `account`.
pub fn books(db: &Db, account: i64) -> Books {
    let all: Vec<Book> = db
        .address_books()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.account_id == Some(account))
        .collect();
    Books {
        count: db.synced_contact_count(account).unwrap_or(0),
        last: all.iter().filter_map(|b| b.last_sync).max(),
        lines: all.into_iter().map(|b| (b.name, b.enabled)).collect(),
    }
}

/// Turn the book at position `index` of the account's books on or off.
/// True when it was found.
pub fn set_book_enabled(db: &Db, account: i64, index: usize, on: bool) -> bool {
    let book = db
        .address_books()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.account_id == Some(account))
        .nth(index);
    match book {
        Some(b) => {
            db.set_book_enabled(b.id, on).ok();
            true
        }
        None => false,
    }
}

/// Find: the server address from the account's domain, without
/// credentials (a network request: on a worker thread).
pub fn find_server(domain: &str) -> Result<String, DavError> {
    dav::discover::well_known(dav::discover::Service::CardDav, domain)
}

/// The domain of an account's address.
pub fn domain_of(email: &str) -> Option<String> {
    email.rsplit_once('@').map(|(_, d)| d.to_string())
}

/// Find address books: the first request with credentials (a network
/// request: on a worker thread). `(href, name, vCard 4 supported)`.
pub fn find_books(d: &DavAccount, password: &str) -> Result<Vec<(String, String, bool)>, DavError> {
    Client::new(&d.url, &d.user, password, d.pin.as_deref(), d.allow_private)
        .and_then(|c| carddav::find_books(&c, &d.url))
}

/// What one pass for an account does.
#[derive(Debug, Clone)]
pub struct Plan {
    pub settings: DavAccount,
    pub books: Vec<Book>,
    password: String,
}

/// The pass for `account`, unless sync is off, no book is on, there is no
/// password, or `KOMOREBI_OFFLINE` is set. (Whether one is already running
/// is the caller's to know.)
pub fn plan(db: &Db, account: i64) -> Option<Plan> {
    if crate::dev_var("KOMOREBI_OFFLINE").is_some() {
        return None;
    }
    let settings = db
        .dav_account(account)
        .ok()
        .flatten()
        .filter(|d| d.mode > 0)?;
    let books: Vec<Book> = db
        .address_books()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.account_id == Some(account) && b.enabled && b.mode != BOOK_LOCAL)
        .collect();
    if books.is_empty() {
        return None;
    }
    let password = password(&settings).ok()?;
    Some(Plan {
        settings,
        books,
        password,
    })
}

/// Run a pass on its own connection to the cache at `db_path` (on a worker
/// thread).
pub fn run(db_path: &Path, plan: &Plan) -> Result<carddav::Outcome, DavError> {
    let d = &plan.settings;
    let db = Db::open_worker(db_path).map_err(|e| DavError::Network(e.to_string()))?;
    let client = Client::new(
        &d.url,
        &d.user,
        &plan.password,
        d.pin.as_deref(),
        d.allow_private,
    )?;
    let mut total = carddav::Outcome::default();
    for b in &plan.books {
        let o = carddav::sync_book(&client, &db, b)?;
        total.changed |= o.changed;
        total.skipped += o.skipped;
        total.conflicts += o.conflicts;
    }
    Ok(total)
}

/// The accounts whose interval has passed since their last pass (`last`),
/// for the mail check timer.
pub fn due(db: &Db, accounts: &[i64], last: &HashMap<i64, Instant>) -> Vec<i64> {
    accounts
        .iter()
        .copied()
        .filter(|id| {
            let Some(d) = db.dav_account(*id).ok().flatten().filter(|d| d.mode > 0) else {
                return false;
            };
            let every = Duration::from_secs(u64::try_from(d.interval_min.max(5)).unwrap_or(5) * 60);
            last.get(id).is_none_or(|t| t.elapsed() >= every)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> (Db, i64) {
        let mut db = Db::in_memory().expect("cache");
        let s = crate::seed::seed(&mut db, 1).expect("seed");
        (db, s.account)
    }

    #[test]
    fn an_account_without_settings_starts_off_with_its_address_as_user() {
        let (db, account) = seeded();
        let accounts = db.accounts().expect("accounts");
        let d = settings(&db, &accounts, account);
        assert_eq!(d.mode, 0);
        assert_eq!(d.interval_min, 30);
        assert_eq!(d.user, accounts[0].email);
    }

    #[test]
    fn the_form_is_saved_trimmed_and_clamped() {
        let (db, account) = seeded();
        let accounts = db.accounts().expect("accounts");
        let mut form = Form::of(&settings(&db, &accounts, account));
        assert!(form.same);
        form.url = "  https://dav.example.com/  ".into();
        form.mode = 9;
        let (d, kept) = save(&db, &accounts, account, &form).expect("saved");
        assert!(!kept);
        assert_eq!((d.url.as_str(), d.mode), ("https://dav.example.com/", 2));
        assert_eq!(settings(&db, &accounts, account).mode, 2);
    }

    #[test]
    fn nothing_is_asked_when_there_is_nothing_to_remove() {
        let (db, account) = seeded();
        assert!(!asks_to_remove(&db, account, 2, 0));
        assert!(!asks_to_remove(&db, account, 0, 0));
        assert!(!asks_to_remove(&db, account, 2, 1));
    }

    #[test]
    fn books_and_the_status_line() {
        let (db, account) = seeded();
        let b = books(&db, account);
        assert_eq!(b, Books::default());
        assert_eq!(b.status(), None);
        let found = [("/dav/main/".to_string(), "Main".to_string(), true)];
        db.set_found_books(account, 1, &found).expect("found");
        let b = books(&db, account);
        assert_eq!(b.lines, [("Main".to_string(), true)]);
        // Never synced: no status line yet.
        assert_eq!(b.status(), None);
        assert!(set_book_enabled(&db, account, 0, false));
        assert!(!books(&db, account).lines[0].1);
        assert!(!set_book_enabled(&db, account, 5, true));
    }

    #[test]
    fn a_pass_needs_sync_on_and_a_book() {
        let (db, account) = seeded();
        assert!(plan(&db, account).is_none());
        let accounts = db.accounts().expect("accounts");
        let mut form = Form::of(&settings(&db, &accounts, account));
        form.mode = 1;
        save(&db, &accounts, account, &form).expect("saved");
        // On, but no book found yet.
        assert!(plan(&db, account).is_none());
        assert_eq!(due(&db, &[account], &HashMap::new()), [account]);
        let mut last = HashMap::new();
        last.insert(account, Instant::now());
        assert!(due(&db, &[account], &last).is_empty());
    }

    #[test]
    fn the_domain_is_what_follows_the_last_at() {
        assert_eq!(domain_of("ana@example.pt").as_deref(), Some("example.pt"));
        assert_eq!(domain_of("nobody"), None);
    }
}
