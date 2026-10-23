//! The Calendars pane of the Calendar page and the CalDAV passes.
//!
//! Per account: the DAV server address and sign-in shared with Contacts
//! (`dav_accounts`; the mail password or `account:{id}:dav` from Windows
//! Credential Manager). "Find calendars" is the first request with
//! credentials, made only on that press; with no address it first reads
//! SRV and `/.well-known/caldav` without credentials.
//!
//! A pass is a short-lived thread per account (at most one at a time) on
//! its own connection to the cache (`Db::open_worker`): it first sends
//! the calendar's queued changes (`cal::write`), then pulls. It starts
//! when the Calendar page opens, on Sync now, after a calendar is turned
//! on, after a change to an event, and from the mail check timer at most every 15 minutes, and
//! only while a CalDAV calendar is on: never before the first frame, and
//! never under `KOMOREBI_OFFLINE`.

use super::*;
use crate::{BookItem, CalSync};
use komorebi::dav::{self, Client, DavError, caldav};
use std::collections::HashSet;
use std::time::Instant;

/// Passes start from the timer at most this often.
const EVERY: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
struct State {
    /// The account the pane shows.
    account: Option<i64>,
    /// Accounts with a pass or a request running.
    busy: HashSet<i64>,
    /// When each account's last pass started.
    last: HashMap<i64, Instant>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// The sentence for a failed calendar request.
fn say(e: &DavError) -> String {
    match e {
        DavError::Offline => t!(invoke_cal_dav_offline),
        DavError::Auth => t!(invoke_cal_dav_auth),
        DavError::AuthRedirect => t!(invoke_dav_redirect),
        DavError::NotFound => t!(invoke_cal_dav_not_found),
        DavError::Tls(_) => t!(invoke_cal_dav_tls),
        other => t!(invoke_cal_dav_failed, other.to_string().into()),
    }
}

impl App {
    fn cal_pane_open(&mut self) {
        let ui = self.ui();
        let g = ui.global::<CalSync>();
        let names: Vec<SharedString> = self
            .accounts
            .iter()
            .map(|a| a.email.as_str().into())
            .collect();
        g.set_accounts(ModelRc::new(VecModel::from(names)));
        let account = state(|s| s.account)
            .filter(|id| self.accounts.iter().any(|a| a.id == *id))
            .or_else(|| self.accounts.first().map(|a| a.id));
        state(|s| s.account = account);
        g.set_account(
            account
                .and_then(|id| self.accounts.iter().position(|a| a.id == id))
                .map_or(0, |i| i as i32),
        );
        if let Some(id) = account {
            self.cal_show_account(id);
        }
        g.set_open(true);
    }

    fn cal_show_account(&mut self, account: i64) {
        let url = self.cal_url(account);
        let ui = self.ui();
        let g = ui.global::<CalSync>();
        g.set_url(url.as_str().into());
        g.set_error(SharedString::new());
        g.set_busy(state(|s| s.busy.contains(&account)));
        self.cal_show_list(account);
    }

    fn cal_show_list(&mut self, account: i64) {
        let cals = self.db.dav_calendars(account).unwrap_or_default();
        let items: Vec<BookItem> = cals
            .iter()
            .map(|c| BookItem {
                name: c.name.as_str().into(),
                enabled: c.enabled,
            })
            .collect();
        let ui = self.ui();
        let g = ui.global::<CalSync>();
        let status = if cals.is_empty() {
            String::new()
        } else {
            let n = self.db.dav_event_count(account).unwrap_or(0);
            t!(invoke_cal_dav_count, n.clamp(0, i64::from(i32::MAX)) as i32)
        };
        g.set_status(status.into());
        g.set_calendars(ModelRc::new(VecModel::from(items)));
    }

    /// The calendar address: the one that last worked, else the shared
    /// DAV address.
    fn cal_url(&self, account: i64) -> String {
        self.db
            .cal_home(account)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.dav_settings(account).url)
    }

    fn cal_busy(&mut self, account: i64, on: bool) {
        state(|s| {
            if on {
                s.busy.insert(account);
            } else {
                s.busy.remove(&account);
            }
        });
        if state(|s| s.account) == Some(account) {
            let ui = self.ui();
            let g = ui.global::<CalSync>();
            g.set_busy(on);
            if on {
                g.set_error(SharedString::new());
                g.set_status(t!(invoke_cal_dav_working).into());
            }
        }
    }

    fn cal_account_chosen(&mut self, index: i32) {
        let Some(id) = usize::try_from(index)
            .ok()
            .and_then(|i| self.accounts.get(i))
            .map(|a| a.id)
        else {
            return;
        };
        state(|s| s.account = Some(id));
        self.cal_show_account(id);
    }

    /// Find calendars: the address (looked up without credentials when it
    /// is empty), then the calendars there, with credentials.
    fn cal_find(&mut self) {
        let Some(account) = state(|s| s.account) else {
            return;
        };
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            self.ui()
                .global::<CalSync>()
                .set_error(t!(invoke_cal_dav_offline).into());
            return;
        }
        let mut d = self.dav_settings(account);
        d.url = self.ui().global::<CalSync>().get_url().trim().to_string();
        let domain = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .and_then(|a| a.email.rsplit_once('@').map(|(_, d)| d.to_string()))
            .unwrap_or_default();
        let pass = match contacts_sync::password(&d) {
            Ok(p) => p,
            Err(e) => {
                return self
                    .ui()
                    .global::<CalSync>()
                    .set_error(text::detail(&e).into());
            }
        };
        self.cal_busy(account, true);
        std::thread::spawn(move || {
            // Err(Ok(url)): a looked-up server outside the mail domain,
            // shown to the user before any credentials go there.
            let found = (|| {
                if d.url.is_empty() {
                    let url = dav::discover::well_known(dav::discover::Service::CalDav, &domain)
                        .map_err(Err)?;
                    if !dav::discover::same_domain(&url, &domain) {
                        return Err(Ok(url));
                    }
                    d.url = url;
                }
                let client = Client::new(&d.url, &d.user, &pass, d.pin.as_deref(), d.allow_private)
                    .map_err(Err)?;
                caldav::find_calendars(&client, &d.url)
                    .map(|c| (c, d))
                    .map_err(Err)
            })();
            slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.cal_busy(account, false);
                    let ui = a.ui();
                    let g = ui.global::<CalSync>();
                    match found {
                        Ok((cals, _)) if cals.is_empty() => {
                            g.set_error(t!(invoke_cal_dav_none_found).into());
                        }
                        Ok((cals, d)) => {
                            // The address that worked is the calendar's;
                            // the shared address (Contacts') is set only
                            // when there was none.
                            match a.db.dav_account(account).ok().flatten() {
                                Some(s) if !s.url.is_empty() => {}
                                Some(mut s) => {
                                    s.url = d.url.clone();
                                    a.db.set_dav_account(&s).ok();
                                }
                                None => {
                                    a.db.set_dav_account(&d).ok();
                                }
                            }
                            a.db.set_cal_home(account, &d.url).ok();
                            a.db.set_found_calendars(account, &cals).ok();
                            if state(|s| s.account) == Some(account) {
                                g.set_url(d.url.as_str().into());
                                a.cal_show_list(account);
                            }
                            a.start_calendar_sync(account);
                        }
                        Err(Ok(url)) => {
                            if state(|s| s.account) == Some(account) {
                                g.set_url(url.as_str().into());
                            }
                            g.set_error(t!(invoke_cal_dav_check_host, url.into()).into());
                        }
                        Err(Err(e)) => g.set_error(say(&e).into()),
                    }
                });
            })
            .ok();
        });
    }

    fn cal_toggled(&mut self, index: i32, on: bool) {
        let Some(account) = state(|s| s.account) else {
            return;
        };
        let cal = self
            .db
            .dav_calendars(account)
            .unwrap_or_default()
            .into_iter()
            .nth(index.max(0) as usize);
        if let Some(c) = cal {
            self.db.set_calendar_enabled(c.id, on).ok();
            self.calendar_reload();
            if on {
                self.start_calendar_sync(account);
            }
        }
    }

    /// One pass for `account`, unless one is running or no calendar is on.
    pub(super) fn start_calendar_sync(&mut self, account: i64) {
        if state(|s| s.busy.contains(&account)) || komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            return;
        }
        let cals: Vec<_> = self
            .db
            .dav_calendars(account)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.enabled)
            .collect();
        if cals.is_empty() {
            return;
        }
        let mut d = self.dav_settings(account);
        d.url = self.cal_url(account);
        if d.url.is_empty() {
            return;
        }
        let Ok(pass) = contacts_sync::password(&d) else {
            return;
        };
        let me = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        state(|s| s.last.insert(account, Instant::now()));
        self.cal_busy(account, true);
        let path = self.db_path.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            let result = (|| -> Result<caldav::Outcome, DavError> {
                let db = Db::open_worker(&path).map_err(|e| DavError::Network(e.to_string()))?;
                let client =
                    Client::new(&d.url, &d.user, &pass, d.pin.as_deref(), d.allow_private)?;
                let now = chrono::Utc::now().timestamp();
                let mut total = caldav::Outcome::default();
                for c in &cals {
                    let o = caldav::flush_and_pull(&client, &db, account, c, &me, now)?;
                    total.changed |= o.changed;
                    total.conflicts = total.conflicts.saturating_add(o.conflicts);
                    total.refused = total.refused.saturating_add(o.refused);
                    total.skipped = total.skipped.saturating_add(o.skipped);
                }
                Ok(total)
            })();
            slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.cal_busy(account, false);
                    match &result {
                        Ok(o) => {
                            komorebi::diag::record(account, "CALDAV", "ok", started.elapsed());
                            if o.changed {
                                a.calendar_reload();
                            }
                        }
                        Err(e) => {
                            komorebi::diag::record(account, "CALDAV", "failed", started.elapsed());
                            if state(|s| s.account) == Some(account) {
                                a.ui().global::<CalSync>().set_error(say(e).into());
                            }
                        }
                    }
                    if state(|s| s.account) == Some(account) {
                        a.cal_show_list(account);
                    }
                });
            })
            .ok();
        });
    }

    /// The Calendar page opened, or Sync now: every account with a
    /// calendar on.
    pub(super) fn calendar_kick(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in ids {
            self.start_calendar_sync(id);
        }
    }

    /// From the mail check timer: each account not synced for
    /// [`EVERY`].
    pub(super) fn calendar_timer_tick(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in ids {
            if state(|s| s.last.get(&id).is_none_or(|t| t.elapsed() >= EVERY)) {
                self.start_calendar_sync(id);
            }
        }
    }
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=calendar:calendars`, offline on a
/// throwaway cache only): two made-up calendars for the first account,
/// then the Calendars pane.
pub fn sample_calendar_sync() {
    with_app(|a| {
        if let Some(id) = a.accounts.first().map(|a| a.id) {
            let found = [("Work", true), ("Family", false)].map(|(n, s)| {
                komorebi::cal::store::FoundCalendar {
                    href: format!("/dav/cals/{}/", n.to_ascii_lowercase()),
                    name: n.into(),
                    sync_collection: s,
                }
            });
            a.db.set_found_calendars(id, &found).ok();
            if let Some(c) =
                a.db.dav_calendars(id)
                    .ok()
                    .and_then(|c| c.into_iter().nth(1))
            {
                a.db.set_calendar_enabled(c.id, false).ok();
            }
        }
        a.cal_pane_open();
    });
}

pub(super) fn wire(ui: &AppWindow) {
    let g = ui.global::<CalSync>();
    g.on_open_pane(|| {
        with_app(|a| a.cal_pane_open());
    });
    g.on_account_chosen(|i| {
        with_app(|a| a.cal_account_chosen(i));
    });
    g.on_find(|| {
        with_app(|a| a.cal_find());
    });
    g.on_toggled(|i, on| {
        with_app(|a| a.cal_toggled(i, on));
    });
    g.on_sync_now(|| {
        with_app(|a| {
            if let Some(id) = state(|s| s.account) {
                a.start_calendar_sync(id);
            }
        });
    });
}
