//! The address-book sync pane of the Contacts view and the CardDAV passes.
//!
//! Per account: sync off, read only, or read and write; the server
//! address ("Find" reads SRV and `/.well-known/carddav` without
//! credentials); the user name; and the mail password or a separate one
//! kept in Windows Credential Manager (`account:{id}:dav`). "Find address
//! books" is the first authenticated request, made only on that press.
//!
//! A pass is a short-lived thread per account (at most one at a time) on
//! its own connection to the cache (`Db::open_worker`). It starts when the
//! Contacts view opens, on Sync now, after a local edit, and from the mail
//! check timer at most every `interval_min` minutes: never before the first
//! frame, and never under `KOMOREBI_OFFLINE`.

use super::*;
use crate::{BookItem, Contacts};
use komorebi::dav::{self, Client, DavError, carddav};
use komorebi::db::contacts::{BOOK_LOCAL, DavAccount};
use std::collections::HashSet;
use std::time::Instant;

/// The sync pane's state on the UI thread.
#[derive(Default)]
pub(in crate::app) struct SyncState {
    /// The account the pane shows.
    account: Option<i64>,
    /// The mode before a change to Off waits for its answer.
    previous_mode: i64,
    /// Accounts with a pass or a request running.
    busy: HashSet<i64>,
    /// When each account's last pass started.
    last: HashMap<i64, Instant>,
}

impl SyncState {
    /// Hide the pane.
    pub(super) fn close(&mut self, ui: &AppWindow) {
        ui.global::<Contacts>().set_sync_open(false);
    }
}

/// The sentence for a failed DAV request.
fn say(e: &DavError) -> String {
    match e {
        DavError::Offline => t!(invoke_dav_offline),
        DavError::Auth => t!(invoke_dav_auth),
        DavError::AuthRedirect => t!(invoke_dav_redirect),
        DavError::NotFound => t!(invoke_dav_not_found),
        DavError::Tls(_) => t!(invoke_dav_tls),
        other => t!(invoke_dav_failed, other.to_string().into()),
    }
}

/// The password a pass signs in with.
pub(super) fn password(d: &DavAccount) -> anyhow::Result<String> {
    if d.separate_secret {
        komorebi::secrets::get_dav_password(d.account_id)
    } else {
        komorebi::secrets::get_password(d.account_id)
    }
}

impl App {
    pub(super) fn dav_settings(&self, account: i64) -> DavAccount {
        self.db
            .dav_account(account)
            .ok()
            .flatten()
            .unwrap_or_else(|| DavAccount {
                account_id: account,
                user: self
                    .accounts
                    .iter()
                    .find(|a| a.id == account)
                    .map(|a| a.email.clone())
                    .unwrap_or_default(),
                interval_min: 30,
                ..DavAccount::default()
            })
    }

    pub(super) fn open_sync_pane(&mut self) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let names: Vec<SharedString> = self
            .accounts
            .iter()
            .map(|a| a.email.as_str().into())
            .collect();
        g.set_s_accounts(ModelRc::new(VecModel::from(names)));
        let account = self
            .contacts
            .sync
            .account
            .filter(|id| self.accounts.iter().any(|a| a.id == *id))
            .or_else(|| self.accounts.first().map(|a| a.id));
        self.contacts.sync.account = account;
        g.set_s_account(
            account
                .and_then(|id| self.accounts.iter().position(|a| a.id == id))
                .map_or(0, |i| i as i32),
        );
        if let Some(id) = account {
            self.show_dav_form(id);
        }
        self.contacts.editing = None;
        g.set_editing(false);
        g.set_sync_open(true);
    }

    fn show_dav_form(&mut self, account: i64) {
        let d = self.dav_settings(account);
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        g.set_s_url(d.url.as_str().into());
        g.set_s_user(d.user.as_str().into());
        g.set_s_mode(d.mode as i32);
        g.set_s_same(!d.separate_secret);
        g.set_s_password(SharedString::new());
        g.set_s_error(SharedString::new());
        g.set_s_busy(self.contacts.sync.busy.contains(&account));
        self.contacts.sync.previous_mode = d.mode;
        self.show_dav_books(account);
    }

    fn show_dav_books(&mut self, account: i64) {
        let books: Vec<BookItem> = self
            .db
            .address_books()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.account_id == Some(account))
            .map(|b| BookItem {
                name: b.name.as_str().into(),
                enabled: b.enabled,
            })
            .collect();
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let count = self.db.synced_contact_count(account).unwrap_or(0);
        let last = self
            .db
            .address_books()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.account_id == Some(account))
            .filter_map(|b| b.last_sync)
            .max();
        let status = match last {
            Some(ts) if !books.is_empty() => format!(
                "{} {}",
                t!(invoke_dav_count, count as i32),
                t!(invoke_dav_synced_at, format::time(ts).into())
            ),
            _ => String::new(),
        };
        g.set_s_status(status.into());
        g.set_s_books(ModelRc::new(VecModel::from(books)));
    }

    /// The form's account, with its fields saved (the password too, when
    /// it is a separate one).
    fn save_dav_form(&mut self) -> Option<DavAccount> {
        let account = self.contacts.sync.account?;
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let mut d = self.dav_settings(account);
        d.url = g.get_s_url().trim().to_string();
        d.user = g.get_s_user().trim().to_string();
        d.mode = i64::from(g.get_s_mode().clamp(0, 2));
        d.separate_secret = !g.get_s_same();
        // `allow_private` stays as stored: only an explicit choice (shared
        // with CalDAV) allows a server on the local network.
        let pass = g.get_s_password();
        if d.separate_secret && !pass.is_empty() && komorebi::dev_var("KOMOREBI_OFFLINE").is_none()
        {
            if let Err(e) = komorebi::secrets::set_dav_password(account, &pass) {
                g.set_s_error(text::detail(&e).into());
                return None;
            }
            g.set_s_password(SharedString::new());
        }
        if let Err(e) = self.db.set_dav_account(&d) {
            g.set_s_error(text::detail(&e).into());
            return None;
        }
        Some(d)
    }

    fn s_account_chosen(&mut self, index: i32) {
        let Some(id) = usize::try_from(index)
            .ok()
            .and_then(|i| self.accounts.get(i))
            .map(|a| a.id)
        else {
            return;
        };
        self.contacts.sync.account = Some(id);
        self.show_dav_form(id);
    }

    fn s_mode_chosen(&mut self, mode: i32) {
        let Some(account) = self.contacts.sync.account else {
            return;
        };
        let has_books = self.db.synced_contact_count(account).unwrap_or(0) > 0;
        if mode == 0 && self.contacts.sync.previous_mode > 0 && has_books {
            self.ui().global::<Contacts>().set_s_confirm_off(true);
            return;
        }
        if self.save_dav_form().is_some() {
            self.contacts.sync.previous_mode = i64::from(mode);
            self.show_dav_books(account);
            self.reload_contacts(true);
        }
    }

    /// The answer to "Remove the synced contacts?": sync turns off either
    /// way; yes removes the synced contacts, no keeps them on this
    /// computer, no longer synced.
    fn s_off_confirmed(&mut self, remove: bool) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        g.set_s_confirm_off(false);
        if !remove {
            if let Some(a) = self.contacts.sync.account {
                self.db.set_dav_mode(a, 0).ok();
            }
            self.contacts.sync.previous_mode = 0;
            return;
        }
        if self.save_dav_form().is_some() {
            self.contacts.sync.previous_mode = 0;
            if let Some(a) = self.contacts.sync.account {
                self.show_dav_books(a);
            }
            self.reload_contacts(true);
        }
    }

    /// Find: the server address from the account's domain, without
    /// credentials.
    fn s_find(&mut self) {
        let Some(account) = self.contacts.sync.account else {
            return;
        };
        let Some(domain) = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .and_then(|a| a.email.rsplit_once('@').map(|(_, d)| d.to_string()))
        else {
            return;
        };
        self.dav_busy(account, true);
        std::thread::spawn(move || {
            let found = dav::discover::well_known(dav::discover::Service::CardDav, &domain);
            slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.dav_busy(account, false);
                    let ui = a.ui();
                    let g = ui.global::<Contacts>();
                    match found {
                        Ok(url) if a.contacts.sync.account == Some(account) => {
                            g.set_s_url(url.into());
                        }
                        Ok(_) => {}
                        Err(e) => g.set_s_error(say(&e).into()),
                    }
                });
            })
            .ok();
        });
    }

    fn dav_busy(&mut self, account: i64, on: bool) {
        if on {
            self.contacts.sync.busy.insert(account);
        } else {
            self.contacts.sync.busy.remove(&account);
        }
        if self.contacts.sync.account == Some(account) {
            let ui = self.ui();
            let g = ui.global::<Contacts>();
            g.set_s_busy(on);
            if on {
                g.set_s_error(SharedString::new());
                g.set_s_status(t!(invoke_dav_working).into());
            }
        }
    }

    /// Find address books: the first request with credentials.
    fn s_find_books(&mut self) {
        let Some(d) = self.save_dav_form() else {
            return;
        };
        if d.mode == 0 || d.url.is_empty() {
            return;
        }
        let account = d.account_id;
        let pass = match password(&d) {
            Ok(p) => p,
            Err(e) => {
                return self
                    .ui()
                    .global::<Contacts>()
                    .set_s_error(text::detail(&e).into());
            }
        };
        self.dav_busy(account, true);
        std::thread::spawn(move || {
            let found = Client::new(&d.url, &d.user, &pass, d.pin.as_deref(), d.allow_private)
                .and_then(|c| carddav::find_books(&c, &d.url));
            slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.dav_busy(account, false);
                    match found {
                        Ok(books) if books.is_empty() => {
                            a.ui()
                                .global::<Contacts>()
                                .set_s_error(t!(invoke_dav_none_found).into());
                        }
                        Ok(books) => {
                            a.db.set_found_books(account, d.mode, &books).ok();
                            a.show_dav_books(account);
                            a.start_contacts_sync(account);
                        }
                        Err(e) => a.ui().global::<Contacts>().set_s_error(say(&e).into()),
                    }
                });
            })
            .ok();
        });
    }

    fn s_book_toggled(&mut self, index: i32, on: bool) {
        let Some(account) = self.contacts.sync.account else {
            return;
        };
        let book = self
            .db
            .address_books()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.account_id == Some(account))
            .nth(index.max(0) as usize);
        if let Some(b) = book {
            self.db.set_book_enabled(b.id, on).ok();
            self.reload_contacts(true);
            if on {
                self.start_contacts_sync(account);
            }
        }
    }

    /// One pass for `account`, unless one is running or sync is off.
    pub(super) fn start_contacts_sync(&mut self, account: i64) {
        if self.contacts.sync.busy.contains(&account)
            || komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
        {
            return;
        }
        let Some(d) = self
            .db
            .dav_account(account)
            .ok()
            .flatten()
            .filter(|d| d.mode > 0)
        else {
            return;
        };
        let books: Vec<_> = self
            .db
            .address_books()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.account_id == Some(account) && b.enabled && b.mode != BOOK_LOCAL)
            .collect();
        if books.is_empty() {
            return;
        }
        let Ok(pass) = password(&d) else {
            return;
        };
        self.contacts.sync.last.insert(account, Instant::now());
        self.dav_busy(account, true);
        let path = self.db_path.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            let result = (|| -> Result<carddav::Outcome, DavError> {
                let db = Db::open_worker(&path).map_err(|e| DavError::Network(e.to_string()))?;
                let client =
                    Client::new(&d.url, &d.user, &pass, d.pin.as_deref(), d.allow_private)?;
                let mut total = carddav::Outcome::default();
                for b in &books {
                    let o = carddav::sync_book(&client, &db, b)?;
                    total.changed |= o.changed;
                    total.skipped += o.skipped;
                    total.conflicts += o.conflicts;
                }
                Ok(total)
            })();
            slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.dav_busy(account, false);
                    match &result {
                        Ok(o) => {
                            komorebi::diag::record(account, "CARDDAV", "ok", started.elapsed());
                            if o.changed && a.ui().get_show_contacts() {
                                a.reload_contacts(true);
                            }
                        }
                        Err(e) => {
                            komorebi::diag::record(account, "CARDDAV", "failed", started.elapsed());
                            if a.contacts.sync.account == Some(account) {
                                a.ui().global::<Contacts>().set_s_error(say(e).into());
                            }
                        }
                    }
                    if a.contacts.sync.account == Some(account) {
                        a.show_dav_books(account);
                    }
                });
            })
            .ok();
        });
    }

    /// A local edit or the view opening: sync the accounts that write.
    pub(super) fn contacts_kick(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in ids {
            self.start_contacts_sync(id);
        }
    }

    /// From the mail check timer: each account whose interval has passed.
    pub(super) fn contacts_timer_tick(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in ids {
            let Some(d) = self
                .db
                .dav_account(id)
                .ok()
                .flatten()
                .filter(|d| d.mode > 0)
            else {
                continue;
            };
            let every = std::time::Duration::from_secs(d.interval_min.max(5) as u64 * 60);
            if self
                .contacts
                .sync
                .last
                .get(&id)
                .is_none_or(|t| t.elapsed() >= every)
            {
                self.start_contacts_sync(id);
            }
        }
    }
}

pub(super) fn wire(ui: &AppWindow) {
    let g = ui.global::<Contacts>();
    g.on_open_sync(|| {
        with_app(|a| a.open_sync_pane());
    });
    g.on_close_sync(|| {
        with_app(|a| a.ui().global::<Contacts>().set_sync_open(false));
    });
    g.on_s_account_chosen(|i| {
        with_app(|a| a.s_account_chosen(i));
    });
    g.on_s_mode_chosen(|m| {
        with_app(|a| a.s_mode_chosen(m));
    });
    g.on_s_off_confirmed(|yes| {
        with_app(|a| a.s_off_confirmed(yes));
    });
    g.on_s_find(|| {
        with_app(|a| a.s_find());
    });
    g.on_s_find_books(|| {
        with_app(|a| a.s_find_books());
    });
    g.on_s_book_toggled(|i, on| {
        with_app(|a| a.s_book_toggled(i, on));
    });
    g.on_s_sync_now(|| {
        with_app(|a| {
            if let Some(id) = a.contacts.sync.account {
                a.start_contacts_sync(id);
            }
        });
    });
}
