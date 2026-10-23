//! The calendar's tables and queries (an `impl Db` kept out of `db.rs`).
//!
//! `calendars` holds the local "On this PC" calendar (`account_id` NULL,
//! href `local:`) and, later, each account's CalDAV calendars; `events`
//! the raw iCalendar resource of each UID with indexed columns and the
//! sync fields CalDAV needs (href, etag, the calendar's sync token and
//! ctag, `cal_queue` for writes not yet sent); `occurrences` is derived
//! from `events` for a window around today and rebuilt at will.

use super::{Expanded, RecurState};
use crate::db::Db;
use crate::mail::ical::{self, Resource};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

/// The schema the calendar adds (v17): created on a fresh install and by
/// the migration step, with the local calendar's row. The per-account DAV
/// settings (`dav_accounts`, with the principal and the calendar home) are
/// the contacts' v15 table, shared with CalDAV (`db::contacts::CONTACTS`).
pub const SCHEMA: &str = r#"
CREATE TABLE calendars (
    id INTEGER PRIMARY KEY,
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    href TEXT NOT NULL,
    name TEXT,
    color INTEGER,
    sync_token TEXT,
    ctag TEXT,
    can_write INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1,
    ord INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, href)
);
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    href TEXT,
    etag TEXT,
    uid TEXT NOT NULL,
    ics BLOB NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    location TEXT NOT NULL DEFAULT '',
    start_key INTEGER,
    end_key INTEGER,
    all_day INTEGER NOT NULL DEFAULT 0,
    floating INTEGER NOT NULL DEFAULT 0,
    series_end INTEGER,
    recur INTEGER NOT NULL DEFAULT 0,
    mine INTEGER NOT NULL DEFAULT 1,
    partstat TEXT,
    cancelled INTEGER NOT NULL DEFAULT 0,
    alarm_min INTEGER,
    conflict INTEGER NOT NULL DEFAULT 0,
    dirty INTEGER NOT NULL DEFAULT 0,
    UNIQUE(calendar_id, href)
);
CREATE INDEX events_uid ON events(uid, calendar_id);
CREATE TABLE occurrences (
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL,
    recurrence_id TEXT NOT NULL DEFAULT '',
    flags INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX occurrences_start ON occurrences(start, event_id);
CREATE INDEX occurrences_event ON occurrences(event_id);
CREATE TABLE cal_queue (
    id INTEGER PRIMARY KEY,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    event_id INTEGER,
    op INTEGER NOT NULL,
    href TEXT,
    if_match TEXT,
    ics BLOB,
    attempts INTEGER NOT NULL DEFAULT 0,
    error INTEGER,
    created INTEGER NOT NULL
);
CREATE TABLE alarms_done (
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    occ_start INTEGER NOT NULL,
    PRIMARY KEY(event_id, occ_start)
);
INSERT INTO calendars (account_id, href, name, ord)
    SELECT NULL, 'local:', '', 0
    WHERE NOT EXISTS (SELECT 1 FROM calendars WHERE account_id IS NULL AND href = 'local:');
"#;

/// Occurrences are kept from this long before now...
pub const WINDOW_BEFORE: i64 = 366 * 86_400;
/// ...to this long after.
pub const WINDOW_AFTER: i64 = 2 * 366 * 86_400;
/// Agenda rows read at most for one range.
pub const MAX_AGENDA: usize = 2_000;

/// A calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub id: i64,
    pub account_id: Option<i64>,
    /// The server's name; empty for the local calendar (the UI names it).
    pub name: String,
    pub enabled: bool,
    pub can_write: bool,
}

impl Calendar {
    pub fn local(&self) -> bool {
        self.account_id.is_none()
    }
}

/// One agenda line: an occurrence with its event's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaRow {
    pub event_id: i64,
    pub calendar_id: i64,
    pub start: i64,
    pub end: i64,
    pub flags: i64,
    pub summary: String,
    pub location: String,
    pub recurring: bool,
    pub alarm: bool,
    /// The reminder, minutes before the start.
    pub alarm_min: Option<i64>,
    pub cancelled: bool,
}

/// A due reminder with what its notification shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub alarm: super::remind::Alarm,
    pub summary: String,
    pub location: String,
}

/// A CalDAV calendar and its sync state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavCalendar {
    pub id: i64,
    pub href: String,
    pub name: String,
    /// NULL: the server has no sync-collection; '': it has, no token yet.
    pub sync_token: Option<String>,
    pub ctag: Option<String>,
    pub enabled: bool,
}

/// A calendar "Find calendars" found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundCalendar {
    pub href: String,
    pub name: String,
    pub sync_collection: bool,
}

/// What [`Db::store_invite`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// Added or replaced; the event's id.
    Saved(i64),
    /// An older version (lower SEQUENCE) than the stored copy: ignored.
    Older,
    /// Not readable.
    Unreadable,
}

/// The indexed columns of a resource.
struct Columns {
    uid: String,
    summary: String,
    location: String,
    all_day: bool,
    floating: bool,
    partstat: Option<String>,
    cancelled: bool,
    alarm_min: Option<i64>,
    mine: bool,
}

fn columns(res: &Resource, me: &str) -> Columns {
    let item = res.master.as_ref().or(res.overrides.first());
    let ev = item.map(|i| &i.event);
    let mine_addr = |a: &str| !me.is_empty() && a.eq_ignore_ascii_case(me);
    Columns {
        uid: res.uid().to_string(),
        summary: ev.map_or(String::new(), |e| e.summary.clone()),
        location: ev.map_or(String::new(), |e| e.location.clone()),
        all_day: ev.is_some_and(|e| e.start.all_day),
        floating: ev.is_some_and(|e| !e.start.all_day && e.start.utc.is_none()),
        partstat: ev.and_then(|e| {
            e.attendees
                .iter()
                .find(|p| mine_addr(&p.addr))
                .map(|p| p.partstat.clone())
        }),
        cancelled: ev.is_some_and(|e| e.status == "CANCELLED"),
        alarm_min: item.and_then(|i| i.alarms.first()).map(|a| a.before_min),
        mine: ev.is_none_or(|e| e.organizer.as_ref().is_none_or(|o| mine_addr(&o.addr))),
    }
}

/// The SEQUENCE of a resource (its master's, else the highest).
fn sequence(res: &Resource) -> i64 {
    res.master
        .iter()
        .chain(res.overrides.iter())
        .map(|i| i.event.sequence)
        .max()
        .unwrap_or(0)
}

/// The top-level VEVENT blocks of a raw resource (each as written).
pub(super) fn vevents(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut depth = 0usize;
    for line in raw.split_inclusive('\n') {
        let up = line.trim_end().to_ascii_uppercase();
        if up.starts_with("BEGIN:") {
            depth = depth.saturating_add(1);
            if depth == 2 && up == "BEGIN:VEVENT" {
                cur = Some(String::new());
            }
        }
        if let Some(c) = cur.as_mut() {
            c.push_str(line);
        }
        if up.starts_with("END:") {
            if depth == 2 && up == "END:VEVENT" {
                out.extend(cur.take());
            }
            depth = depth.saturating_sub(1);
        }
    }
    out
}

pub(super) fn rid_of(block: &str) -> Option<String> {
    let line = block
        .split('\n')
        .find(|l| l.to_ascii_uppercase().starts_with("RECURRENCE-ID"))?;
    Some(line.rsplit(':').next()?.trim().to_string())
}

/// `stored` with the single-occurrence events of `incoming` merged in
/// (each replacing the stored one with the same RECURRENCE-ID).
pub(super) fn merge_overrides(stored: &str, incoming: &str) -> String {
    let new = vevents(incoming);
    let rids: Vec<String> = new.iter().filter_map(|b| rid_of(b)).collect();
    let mut out = stored.to_string();
    for old in vevents(stored) {
        if rid_of(&old).is_some_and(|r| rids.contains(&r)) {
            out = out.replacen(&old, "", 1);
        }
    }
    let end = out
        .to_ascii_uppercase()
        .rfind("END:VCALENDAR")
        .unwrap_or(out.len());
    let (head, tail) = out.split_at(end);
    let mut merged = head.to_string();
    for b in new {
        merged.push_str(&b);
    }
    merged.push_str(tail);
    merged
}

impl Db {
    /// The local calendar's id (created with the schema).
    pub fn local_calendar(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT id FROM calendars WHERE account_id IS NULL AND href = 'local:'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Every calendar, the local one first.
    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let mut st = self.conn().prepare_cached(
            "SELECT id, account_id, COALESCE(name, ''), enabled, can_write FROM calendars
             ORDER BY account_id IS NOT NULL, account_id, ord, id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Calendar {
                id: r.get(0)?,
                account_id: r.get(1)?,
                name: r.get(2)?,
                enabled: r.get::<_, i64>(3)? != 0,
                can_write: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The stored event of `uid` in any calendar: (event id, calendar id).
    pub fn event_by_uid(&self, uid: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, calendar_id FROM events WHERE uid = ?1 ORDER BY id LIMIT 1",
                [uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// The first start (UTC) of an event's occurrences from `from` on.
    pub fn next_occurrence(&self, event_id: i64, from: i64) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT MIN(start) FROM occurrences WHERE event_id = ?1 AND end > ?2",
                params![event_id, from],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Store an invitation the user accepted (or tentatively accepted) in
    /// `calendar_id`, as attendee `me`: METHOD stripped, `me`'s PARTSTAT
    /// set. The same UID with a higher or equal SEQUENCE replaces the
    /// stored copy; a lower one is ignored. A single occurrence (with
    /// RECURRENCE-ID) is merged into the stored series when there is one.
    pub fn store_invite(
        &self,
        calendar_id: i64,
        raw: &[u8],
        me: &str,
        partstat: &str,
        now: i64,
    ) -> Result<Stored> {
        let text = String::from_utf8_lossy(raw);
        let mut ics = ical::strip_method(&text);
        if let Some(set) = ical::set_partstat(&ics, me, partstat) {
            ics = set;
        }
        let Some(res) = ical::parse_resource(ics.as_bytes()) else {
            return Ok(Stored::Unreadable);
        };
        let uid = res.uid().to_string();
        if uid.is_empty() {
            return Ok(Stored::Unreadable);
        }
        let stored: Option<(i64, Vec<u8>)> = self
            .conn()
            .query_row(
                "SELECT id, ics FROM events WHERE calendar_id = ?1 AND uid = ?2",
                params![calendar_id, uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((_, old)) = &stored {
            let old_text = String::from_utf8_lossy(old);
            if let Some(old_res) = ical::parse_resource(old_text.as_bytes()) {
                if res.master.is_none() && old_res.master.is_some() {
                    // One occurrence of a stored series: merged into it.
                    let merged = merge_overrides(&old_text, &ics);
                    if ical::parse_resource(merged.as_bytes()).is_some() {
                        ics = merged;
                    }
                } else if sequence(&res) < sequence(&old_res) {
                    return Ok(Stored::Older);
                }
            }
        }
        let id = self.save_event(calendar_id, stored.map(|(id, _)| id), &ics, me, now)?;
        Ok(Stored::Saved(id))
    }

    /// Write one resource (new, or over event `id`) and rebuild its
    /// occurrences around `now`.
    pub fn save_event(
        &self,
        calendar_id: i64,
        id: Option<i64>,
        ics: &str,
        me: &str,
        now: i64,
    ) -> Result<i64> {
        let res = ical::parse_resource(ics.as_bytes())
            .ok_or_else(|| anyhow::anyhow!("unreadable calendar resource"))?;
        let tx = self.conn().unchecked_transaction()?;
        let id = put_event(&tx, calendar_id, id, ics, &res, me, now)?;
        tx.execute("UPDATE events SET dirty = 1 WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(id)
    }

    // ---- CalDAV ---------------------------------------------------------

    /// The CalDAV calendars of `account` (enabled or not).
    pub fn dav_calendars(&self, account: i64) -> Result<Vec<DavCalendar>> {
        let mut st = self.conn().prepare_cached(
            "SELECT id, href, COALESCE(name, ''), sync_token, ctag, enabled FROM calendars
             WHERE account_id = ?1 ORDER BY ord, id",
        )?;
        let rows = st.query_map([account], |r| {
            Ok(DavCalendar {
                id: r.get(0)?,
                href: r.get(1)?,
                name: r.get(2)?,
                sync_token: r.get(3)?,
                ctag: r.get(4)?,
                enabled: r.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Store what "Find calendars" found for `account`: new calendars are
    /// added enabled and writable (a refused write marks one read only),
    /// known ones renamed, and those no longer found removed with their
    /// events. `sync_token` records sync-collection support: NULL means
    /// none, '' supported with no token yet.
    pub fn set_found_calendars(&self, account: i64, found: &[FoundCalendar]) -> Result<()> {
        let tx = self.write()?;
        let known: Vec<(i64, String)> = tx
            .prepare_cached("SELECT id, href FROM calendars WHERE account_id = ?1")?
            .query_map([account], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, href) in &known {
            if !found.iter().any(|f| &f.href == href) {
                tx.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
            }
        }
        for (ord, f) in found.iter().enumerate() {
            let token = f.sync_collection.then_some("");
            tx.execute(
                "INSERT INTO calendars (account_id, href, name, sync_token, can_write, ord)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT (account_id, href) DO UPDATE SET name = ?3, ord = ?5,
                   sync_token = CASE WHEN ?4 IS NULL THEN NULL ELSE COALESCE(sync_token, '') END",
                params![account, f.href, f.name, token, ord as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_calendar_enabled(&self, id: i64, on: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE calendars SET enabled = ?2 WHERE id = ?1",
            params![id, on],
        )?;
        Ok(())
    }

    /// The ETags of a calendar's synced events, by href.
    pub fn calendar_etags(&self, calendar_id: i64) -> Result<HashMap<String, String>> {
        let mut st = self.conn().prepare_cached(
            "SELECT href, COALESCE(etag, '') FROM events WHERE calendar_id = ?1 AND href IS NOT NULL",
        )?;
        let rows = st.query_map([calendar_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One batch of a pull, in one transaction: `items` (href, ETag, raw
    /// resource) stored over the events with the same href, `removed`
    /// deleted. A resource over the size cap or unreadable is kept as a
    /// "too large" row (no occurrences) so it is not fetched again until
    /// it changes. The number of such resources.
    pub fn store_synced(
        &self,
        calendar_id: i64,
        items: &[(String, String, Vec<u8>)],
        removed: &[String],
        me: &str,
        now: i64,
    ) -> Result<usize> {
        let parsed: Vec<_> = items
            .iter()
            .map(|(h, e, raw)| (h, e, raw, ical::parse_resource(raw)))
            .collect();
        let tx = self.write()?;
        let mut skipped: usize = 0;
        for (href, etag, raw, res) in parsed {
            let id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM events WHERE calendar_id = ?1 AND href = ?2",
                    params![calendar_id, href],
                    |r| r.get(0),
                )
                .optional()?;
            // A change of ours not yet sent stays shown; its write (with
            // If-Match) decides, and a conflict brings this copy back.
            if let Some(id) = id
                && tx
                    .query_row(
                        "SELECT 1 FROM cal_queue WHERE event_id = ?1 AND error IS NULL",
                        [id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some()
            {
                continue;
            }
            let id = match res.filter(|r| !r.uid().is_empty()) {
                Some(res) => {
                    let text = String::from_utf8_lossy(raw);
                    put_event(&tx, calendar_id, id, &text, &res, me, now)?
                }
                None => {
                    skipped = skipped.saturating_add(1_usize);
                    if let Some(id) = id {
                        tx.execute("DELETE FROM events WHERE id = ?1", [id])?;
                    }
                    tx.execute(
                        "INSERT INTO events (calendar_id, uid, ics, recur) VALUES (?1, ?2, x'', 3)",
                        params![calendar_id, href],
                    )?;
                    tx.last_insert_rowid()
                }
            };
            tx.execute(
                "UPDATE events SET href = ?2, etag = ?3, dirty = 0 WHERE id = ?1",
                params![id, href, etag],
            )?;
        }
        for href in removed {
            tx.execute(
                "DELETE FROM events WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
            )?;
        }
        tx.commit()?;
        Ok(skipped)
    }

    /// Record a finished pull: the new sync token (kept as '' when the
    /// server offers sync-collection but sent none) and ctag.
    pub fn set_calendar_sync(
        &self,
        calendar_id: i64,
        sync_collection: bool,
        token: Option<&str>,
        ctag: Option<&str>,
    ) -> Result<()> {
        let token = sync_collection.then(|| token.unwrap_or(""));
        self.conn().execute(
            "UPDATE calendars SET sync_token = ?2, ctag = ?3 WHERE id = ?1",
            params![calendar_id, token, ctag],
        )?;
        Ok(())
    }

    /// The calendar server address that last worked for `account`
    /// (`dav_accounts.cal_home`), kept apart from the contacts' `url` so
    /// a provider with separate hosts keeps both working.
    pub fn cal_home(&self, account: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT cal_home FROM dav_accounts WHERE account_id = ?1",
                [account],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .filter(|u| !u.is_empty()))
    }

    /// Record the calendar address that worked (the row must exist).
    pub fn set_cal_home(&self, account: i64, url: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE dav_accounts SET cal_home = ?2, found_at = strftime('%s','now') WHERE account_id = ?1",
            params![account, url],
        )?;
        Ok(())
    }

    /// Events stored for `account`'s CalDAV calendars.
    pub fn dav_event_count(&self, account: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM events e JOIN calendars c ON c.id = e.calendar_id
             WHERE c.account_id = ?1",
            [account],
            |r| r.get(0),
        )?)
    }

    /// Remove the event of `uid` from `calendar_id` (a declined or
    /// removed invitation). Whether there was one.
    pub fn remove_event_uid(&self, calendar_id: i64, uid: &str) -> Result<bool> {
        Ok(self.conn().execute(
            "DELETE FROM events WHERE calendar_id = ?1 AND uid = ?2",
            params![calendar_id, uid],
        )? > 0)
    }

    /// Decline one occurrence (`rid`, the RECURRENCE-ID line after its
    /// name) of the stored series `uid`: an EXDATE for it in the series,
    /// and any stored copy of that occurrence dropped. Whether it changed.
    pub fn decline_occurrence(
        &self,
        calendar_id: i64,
        uid: &str,
        rid: &str,
        now: i64,
    ) -> Result<bool> {
        let stored: Option<(i64, Vec<u8>)> = self
            .conn()
            .query_row(
                "SELECT id, ics FROM events WHERE calendar_id = ?1 AND uid = ?2",
                params![calendar_id, uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, ics)) = stored else {
            return Ok(false);
        };
        let text = String::from_utf8_lossy(&ics).into_owned();
        let value = rid.rsplit(':').next().unwrap_or_default().trim();
        if value.is_empty() || rid.contains(['\r', '\n']) {
            return Ok(false);
        }
        // The stored copy of that one occurrence goes.
        let mut out = text.clone();
        for block in vevents(&text) {
            if rid_of(&block).as_deref() == Some(value) {
                out = out.replacen(&block, "", 1);
            }
        }
        // The EXDATE goes into the series (the event without RECURRENCE-ID).
        let Some(series) = vevents(&out).into_iter().find(|b| rid_of(b).is_none()) else {
            return Ok(false);
        };
        let end = series
            .to_ascii_uppercase()
            .rfind("END:VEVENT")
            .unwrap_or(series.len());
        let (head, tail) = series.split_at(end);
        let with = out.replacen(&series, &format!("{head}EXDATE{}\r\n{tail}", rid.trim()), 1);
        if ical::parse_resource(with.as_bytes()).is_none() {
            return Ok(false);
        }
        self.save_event(calendar_id, Some(id), &with, "", now)?;
        Ok(true)
    }

    /// Mark the stored copy of `uid` cancelled (its organizer cancelled
    /// it); it stays until the user removes it.
    /// A cancellation older (lower SEQUENCE `seq`) than the stored copy
    /// changes nothing.
    pub fn cancel_event_uid(&self, uid: &str, seq: i64) -> Result<bool> {
        let rows: Vec<(i64, Vec<u8>)> = self
            .conn()
            .prepare_cached("SELECT id, ics FROM events WHERE uid = ?1 AND cancelled = 0")?
            .query_map([uid], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut changed = false;
        for (id, ics) in rows {
            if ical::parse_resource(&ics).is_some_and(|r| sequence(&r) > seq) {
                continue;
            }
            self.conn()
                .execute("UPDATE events SET cancelled = 1 WHERE id = ?1", [id])?;
            changed = true;
        }
        Ok(changed)
    }

    /// Rebuild every event's occurrences around `now` (after a zone or
    /// clock change, or when the window's month rolls over).
    pub fn rebuild_occurrences(&self, now: i64) -> Result<()> {
        let rows: Vec<(i64, Vec<u8>)> = self
            .conn()
            .prepare_cached("SELECT id, ics FROM events")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let tx = self.conn().unchecked_transaction()?;
        for (id, ics) in rows {
            let Some(res) = ical::parse_resource(&ics) else {
                continue;
            };
            let e = super::expand(
                &res,
                now.saturating_sub(WINDOW_BEFORE),
                now.saturating_add(WINDOW_AFTER),
                &super::local_utc,
            );
            write_occurrences(&tx, id, &e)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Rebuild the occurrences when `key` (the month and the current
    /// zone's offsets, [`super::window_key`]) differs from the one they
    /// were built for: the window moved on, or the time zone changed.
    /// Whether it rebuilt.
    pub fn refresh_occurrences(&self, key: &str, now: i64) -> Result<bool> {
        let built: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'cal_window'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if built.as_deref() == Some(key) {
            return Ok(false);
        }
        self.rebuild_occurrences(now)?;
        self.set_setting("cal_window", key)?;
        Ok(true)
    }

    /// The occurrences in `[from, to)` of enabled calendars, by start.
    pub fn agenda(&self, from: i64, to: i64) -> Result<Vec<AgendaRow>> {
        let mut st = self.conn().prepare_cached(
            "SELECT o.event_id, e.calendar_id, o.start, o.end, o.flags, e.summary, e.location,
                    e.recur, e.alarm_min, e.cancelled
             FROM occurrences o JOIN events e ON e.id = o.event_id
             JOIN calendars c ON c.id = e.calendar_id
             WHERE o.start < ?2 AND o.end > ?1 AND c.enabled = 1
             ORDER BY o.start, o.end, e.summary LIMIT ?3",
        )?;
        let rows = st.query_map(params![from, to, MAX_AGENDA as i64], |r| {
            Ok(AgendaRow {
                event_id: r.get(0)?,
                calendar_id: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                flags: r.get(4)?,
                summary: r.get(5)?,
                location: r.get(6)?,
                recurring: r.get::<_, i64>(7)? != 0,
                alarm: r.get::<_, Option<i64>>(8)?.is_some(),
                alarm_min: r.get(8)?,
                cancelled: r.get::<_, i64>(9)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The occurrences with a reminder starting in `[from, to)`, of
    /// enabled calendars and not cancelled.
    pub fn reminders(&self, from: i64, to: i64) -> Result<Vec<Reminder>> {
        let mut st = self.conn().prepare_cached(
            "SELECT o.event_id, o.start, e.alarm_min, e.summary, e.location
             FROM occurrences o JOIN events e ON e.id = o.event_id
             JOIN calendars c ON c.id = e.calendar_id
             WHERE o.start >= ?1 AND o.start < ?2 AND c.enabled = 1
               AND e.alarm_min IS NOT NULL AND e.cancelled = 0
             ORDER BY o.start LIMIT ?3",
        )?;
        let rows = st.query_map(params![from, to, MAX_AGENDA as i64], |r| {
            Ok(Reminder {
                alarm: super::remind::Alarm {
                    event_id: r.get(0)?,
                    start: r.get(1)?,
                    before_min: r.get(2)?,
                },
                summary: r.get(3)?,
                location: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The reminders already shown, (event, occurrence start), for
    /// occurrences from `since` on; older entries are dropped.
    pub fn alarms_done(&self, since: i64) -> Result<std::collections::HashSet<(i64, i64)>> {
        self.conn()
            .execute("DELETE FROM alarms_done WHERE occ_start < ?1", [since])?;
        let mut st = self
            .conn()
            .prepare_cached("SELECT event_id, occ_start FROM alarms_done")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Remember that the reminder of this occurrence was shown.
    pub fn mark_alarm_done(&self, event_id: i64, occ_start: i64) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO alarms_done (event_id, occ_start) VALUES (?1, ?2)",
            params![event_id, occ_start],
        )?;
        Ok(())
    }

    /// Whether reminder notifications are on (setting `cal_reminders`,
    /// off unless the user turned it on).
    pub fn reminders_on(&self) -> bool {
        self.conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'cal_reminders'",
                [],
                |r| r.get::<_, String>(0),
            )
            .is_ok_and(|v| v == "1")
    }

    /// An event's raw resource.
    pub fn event_ics(&self, event_id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT ics FROM events WHERE id = ?1", [event_id], |r| {
                r.get(0)
            })
            .optional()?)
    }
}

/// Write one parsed resource (new, or over event `id`) and its
/// occurrences around `now`, inside the caller's transaction.
fn put_event(
    tx: &rusqlite::Transaction,
    calendar_id: i64,
    id: Option<i64>,
    ics: &str,
    res: &Resource,
    me: &str,
    now: i64,
) -> Result<i64> {
    let c = columns(res, me);
    let id = match id {
        Some(id) => {
            tx.execute(
                "UPDATE events SET uid = ?2, ics = ?3, summary = ?4, location = ?5, all_day = ?6,
                 floating = ?7, mine = ?8, partstat = ?9, cancelled = ?10, alarm_min = ?11
                 WHERE id = ?1",
                params![
                    id,
                    c.uid,
                    ics.as_bytes(),
                    c.summary,
                    c.location,
                    c.all_day,
                    c.floating,
                    c.mine,
                    c.partstat,
                    c.cancelled,
                    c.alarm_min
                ],
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO events (calendar_id, uid, ics, summary, location, all_day, floating,
                 mine, partstat, cancelled, alarm_min)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    calendar_id,
                    c.uid,
                    ics.as_bytes(),
                    c.summary,
                    c.location,
                    c.all_day,
                    c.floating,
                    c.mine,
                    c.partstat,
                    c.cancelled,
                    c.alarm_min
                ],
            )?;
            tx.last_insert_rowid()
        }
    };
    let e = super::expand(
        res,
        now.saturating_sub(WINDOW_BEFORE),
        now.saturating_add(WINDOW_AFTER),
        &super::local_utc,
    );
    write_occurrences(tx, id, &e)?;
    Ok(id)
}

fn write_occurrences(tx: &rusqlite::Transaction, id: i64, e: &Expanded) -> Result<()> {
    tx.execute("DELETE FROM occurrences WHERE event_id = ?1", [id])?;
    let mut ins = tx.prepare_cached(
        "INSERT INTO occurrences (event_id, start, end, recurrence_id, flags) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for o in &e.occurrences {
        ins.execute(params![id, o.start, o.end, o.recurrence_id, o.flags])?;
    }
    let first = e.occurrences.first();
    tx.execute(
        "UPDATE events SET recur = ?2, series_end = ?3, start_key = ?4, end_key = ?5 WHERE id = ?1",
        params![
            id,
            e.state as i64,
            e.series_end,
            first.map(|o| o.start),
            first.map(|o| o.end)
        ],
    )?;
    Ok(())
}

impl RecurState {
    pub fn from_code(code: i64) -> Self {
        match code {
            1 => RecurState::Expanded,
            2 => RecurState::Refused,
            3 => RecurState::TooLarge,
            _ => RecurState::None,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000; // 2026-09-21

    fn invite(seq: i64, summary: &str, extra: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-1@x\r\nSEQUENCE:{seq}\r\n\
             DTSTART:20261008T130000Z\r\nDTEND:20261008T140000Z\r\nSUMMARY:{summary}\r\n{extra}\
             ORGANIZER:mailto:jane@x.org\r\nATTENDEE;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:me@x.org\r\n\
             END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn fresh_schema_has_one_local_calendar() {
        let db = Db::in_memory().unwrap();
        let cals = db.calendars().unwrap();
        assert_eq!(cals.len(), 1);
        assert!(cals[0].local());
        assert_eq!(db.local_calendar().unwrap(), cals[0].id);
        // Running the schema's insert again adds no second row.
        db.conn()
            .execute_batch(
                SCHEMA
                    .rsplit("CREATE TABLE alarms_done")
                    .next()
                    .unwrap()
                    .split_once(");")
                    .unwrap()
                    .1,
            )
            .unwrap();
        assert_eq!(db.calendars().unwrap().len(), 1);
    }

    #[test]
    fn accepting_stores_and_sequence_rules() {
        let db = Db::in_memory().unwrap();
        let cal = db.local_calendar().unwrap();
        let Stored::Saved(id) = db
            .store_invite(
                cal,
                invite(1, "Plan", "").as_bytes(),
                "me@x.org",
                "ACCEPTED",
                NOW,
            )
            .unwrap()
        else {
            panic!("not stored")
        };
        let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
        assert!(!ics.contains("METHOD"));
        assert!(ics.contains("PARTSTAT=ACCEPTED:mailto:me@x.org"));
        let rows = db.agenda(NOW, NOW + 30 * 86_400).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary, "Plan");
        assert_eq!(rows[0].end - rows[0].start, 3600);
        assert_eq!(db.event_by_uid("inv-1@x").unwrap(), Some((id, cal)));
        // A lower SEQUENCE is ignored, a higher one replaces the copy.
        assert_eq!(
            db.store_invite(
                cal,
                invite(0, "Old", "").as_bytes(),
                "me@x.org",
                "ACCEPTED",
                NOW
            )
            .unwrap(),
            Stored::Older
        );
        assert_eq!(
            db.store_invite(
                cal,
                invite(2, "New", "").as_bytes(),
                "me@x.org",
                "TENTATIVE",
                NOW
            )
            .unwrap(),
            Stored::Saved(id)
        );
        assert_eq!(db.agenda(NOW, NOW + 30 * 86_400).unwrap()[0].summary, "New");
        // A cancellation marks the copy; decline removes it.
        assert!(!db.cancel_event_uid("inv-1@x", 1).unwrap());
        assert!(db.cancel_event_uid("inv-1@x", 2).unwrap());
        assert!(!db.cancel_event_uid("inv-1@x", 3).unwrap());
        assert!(db.agenda(NOW, NOW + 30 * 86_400).unwrap()[0].cancelled);
        assert!(db.remove_event_uid(cal, "inv-1@x").unwrap());
        assert!(db.agenda(NOW, NOW + 30 * 86_400).unwrap().is_empty());
        assert_eq!(
            db.store_invite(cal, b"garbage", "me@x.org", "ACCEPTED", NOW)
                .unwrap(),
            Stored::Unreadable
        );
    }

    #[test]
    fn single_occurrence_merges_into_the_series() {
        let db = Db::in_memory().unwrap();
        let cal = db.local_calendar().unwrap();
        let series = invite(0, "Weekly", "RRULE:FREQ=WEEKLY;COUNT=4\r\n");
        let Stored::Saved(id) = db
            .store_invite(cal, series.as_bytes(), "me@x.org", "ACCEPTED", NOW)
            .unwrap()
        else {
            panic!("not stored")
        };
        assert_eq!(db.agenda(NOW, NOW + 60 * 86_400).unwrap().len(), 4);
        let one = invite(1, "Moved one", "RECURRENCE-ID:20261015T130000Z\r\n")
            .replace("DTSTART:20261008T130000Z", "DTSTART:20261015T170000Z")
            .replace("DTEND:20261008T140000Z", "DTEND:20261015T180000Z");
        assert_eq!(
            db.store_invite(cal, one.as_bytes(), "me@x.org", "ACCEPTED", NOW)
                .unwrap(),
            Stored::Saved(id)
        );
        let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(
            rows.iter()
                .any(|r| r.flags & super::super::FLAG_OVERRIDE != 0)
        );
        // The same occurrence again replaces, not duplicates, the override.
        db.store_invite(cal, one.as_bytes(), "me@x.org", "ACCEPTED", NOW)
            .unwrap();
        let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
        assert_eq!(ics.matches("RECURRENCE-ID").count(), 1);
        // Declining one occurrence keeps the rest of the series.
        assert!(
            db.decline_occurrence(cal, "inv-1@x", ":20261015T130000Z", NOW)
                .unwrap()
        );
        assert!(
            db.decline_occurrence(cal, "inv-1@x", ":20261022T130000Z", NOW)
                .unwrap()
        );
        let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.flags & super::super::FLAG_OVERRIDE == 0)
        );
        assert!(
            !db.decline_occurrence(cal, "nobody", ":20261022T130000Z", NOW)
                .unwrap()
        );
    }

    #[test]
    fn reminders_are_read_and_remembered() {
        let db = Db::in_memory().unwrap();
        let cal = db.local_calendar().unwrap();
        assert!(!db.reminders_on(), "off unless turned on");
        db.set_setting("cal_reminders", "1").unwrap();
        assert!(db.reminders_on());
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:r1\r\nDTSTART:20261008T130000Z\r\n\
                   SUMMARY:Call\r\nDESCRIPTION:Secret notes\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\n\
                   TRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let id = db.save_event(cal, None, raw, "", NOW).unwrap();
        let start = 1_791_464_400; // 2026-10-08 13:00 UTC
        let rows = db.reminders(start - 3600, start + 3600).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].alarm.before_min, 15);
        assert_eq!(rows[0].summary, "Call");
        // A restart five minutes after the reminder: it fires once.
        let now = start - 10 * 60;
        let done = db.alarms_done(now - 86_400).unwrap();
        let alarms: Vec<_> = rows.iter().map(|r| r.alarm.clone()).collect();
        assert_eq!(
            super::super::remind::next_due(now, &alarms, &done).0.len(),
            1
        );
        db.mark_alarm_done(id, start).unwrap();
        db.mark_alarm_done(id, start).unwrap();
        let done = db.alarms_done(now - 86_400).unwrap();
        assert!(
            super::super::remind::next_due(now, &alarms, &done)
                .0
                .is_empty()
        );
        // Old entries are dropped.
        assert!(db.alarms_done(start + 1).unwrap().is_empty());
    }

    #[test]
    fn many_events_agenda_is_bounded() {
        let db = Db::in_memory().unwrap();
        let cal = db.local_calendar().unwrap();
        assert!(db.refresh_occurrences("2026-09|0|3600", NOW).unwrap());
        assert!(!db.refresh_occurrences("2026-09|0|3600", NOW).unwrap());
        assert!(db.refresh_occurrences("2026-10|0|3600", NOW).unwrap());
        for i in 0..300 {
            let raw = format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{i}\r\nDTSTART:20261001T{:02}0000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                i % 24
            );
            db.save_event(cal, None, &raw, "", NOW).unwrap();
        }
        let rows = db.agenda(NOW, NOW + 30 * 86_400).unwrap();
        assert_eq!(rows.len(), MAX_AGENDA);
        assert!(rows.windows(2).all(|w| w[0].start <= w[1].start));
        db.rebuild_occurrences(NOW).unwrap();
        assert_eq!(db.agenda(NOW, NOW + 30 * 86_400).unwrap().len(), MAX_AGENDA);
    }
}
