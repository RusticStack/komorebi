//! Changing events: the user's own creates, edits and deletes, written to
//! the cache at once (the change shows straight away) and, for a CalDAV
//! calendar, to `cal_queue` for the next pass (the outbox pattern). The
//! pass sends each queued write with its precondition: a new event with
//! `If-None-Match: *`, an edit or delete with `If-Match` on the ETag the
//! change was made over. A 412 keeps the change in the queue as a
//! conflict (the server's copy comes back on the pull) until the user
//! applies it again or discards it; a 403 marks the calendar read only and
//! drops the change. Nothing is ever overwritten without a precondition.

use super::store::{merge_overrides, rid_of, vevents};
use crate::db::Db;
use crate::mail::ical::{self, Edit, When};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};

/// `cal_queue.op`: PUT the queued resource.
pub const OP_PUT: i64 = 1;
/// DELETE the resource.
pub const OP_DELETE: i64 = 2;
/// DELETE an event someone else organized, with `Schedule-Reply: F` so
/// the server sends the organizer nothing.
pub const OP_REMOVE: i64 = 3;
/// `cal_queue.error` of a conflict (the server's copy changed).
pub const CONFLICT: i64 = 412;
/// Failed sends kept before a change is given up.
pub const MAX_ATTEMPTS: i64 = 8;

/// A stored event with what the editor needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventInfo {
    pub id: i64,
    pub calendar_id: i64,
    pub uid: String,
    pub ics: String,
    /// No organizer, or the organizer is this account: editable.
    pub mine: bool,
    pub can_write: bool,
    /// A change of ours was refused because the event changed elsewhere.
    pub conflict: bool,
}

/// A conflicted queue row: id, calendar, event, op, href and body.
type ConflictRow = (i64, i64, Option<i64>, i64, String, Vec<u8>);

/// One queued write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub id: i64,
    pub calendar_id: i64,
    pub event_id: Option<i64>,
    pub op: i64,
    pub href: String,
    /// `None`: a new resource (`If-None-Match: *`) for a PUT, no
    /// precondition for a DELETE of a resource never synced.
    pub if_match: Option<String>,
    pub ics: Vec<u8>,
}

/// A RECURRENCE-ID (or EXDATE) value for the occurrence `key` (its wall
/// time as written, `YYYYMMDD[THHMMSS]`) in the form of the series'
/// start: a date, UTC, a zone, or floating. The part after the name.
pub fn rid_for(series_start: &When, key: &str) -> String {
    if series_start.all_day {
        format!(";VALUE=DATE:{}", key.get(..8).unwrap_or(key))
    } else if !series_start.tzid.is_empty() {
        let z = series_start.tzid.replace('"', "");
        if z.contains([':', ';', ',']) {
            format!(";TZID=\"{z}\":{key}")
        } else {
            format!(";TZID={z}:{key}")
        }
    } else if series_start.utc.is_some() {
        format!(":{key}Z")
    } else {
        format!(":{key}")
    }
}

/// A resource name for `uid` in a collection: letters, digits, `-`, `_`
/// and `.` kept, the rest made `-`.
fn file_name(uid: &str) -> String {
    let mut s: String = uid
        .chars()
        .take(120)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.push_str(".ics");
    s
}

impl Db {
    /// An event with its calendar's write state.
    pub fn event_info(&self, id: i64) -> Result<Option<EventInfo>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT e.id, e.calendar_id, e.uid, e.ics, e.mine, c.can_write, e.conflict
                 FROM events e JOIN calendars c ON c.id = e.calendar_id WHERE e.id = ?1",
                [id],
                |r| {
                    Ok(EventInfo {
                        id: r.get(0)?,
                        calendar_id: r.get(1)?,
                        uid: r.get(2)?,
                        ics: String::from_utf8_lossy(&r.get::<_, Vec<u8>>(3)?).into_owned(),
                        mine: r.get::<_, i64>(4)? != 0,
                        can_write: r.get::<_, i64>(5)? != 0,
                        conflict: r.get::<_, i64>(6)? != 0,
                    })
                },
            )
            .optional()?)
    }

    /// The RECURRENCE-ID key of an event's occurrence starting at `start`
    /// (empty for a single event).
    pub fn occurrence_key(&self, event_id: i64, start: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT recurrence_id FROM occurrences WHERE event_id = ?1 AND start = ?2 LIMIT 1",
                params![event_id, start],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The calendar's account and collection href (`None` for the local
    /// calendar, whose changes go nowhere).
    fn dav_target(&self, calendar_id: i64) -> Result<Option<String>> {
        let row: Option<(Option<i64>, String)> = self
            .conn()
            .query_row(
                "SELECT account_id, href FROM calendars WHERE id = ?1",
                [calendar_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(a, h)| a.map(|_| h)))
    }

    /// Save the user's version of an event (new when `id` is `None`) in
    /// `calendar_id` and queue its PUT. The event's id.
    pub fn write_event(
        &self,
        calendar_id: i64,
        id: Option<i64>,
        ics: &str,
        me: &str,
        now: i64,
    ) -> Result<i64> {
        let id = self.save_event(calendar_id, id, ics, me, now)?;
        self.queue_put(calendar_id, id, ics, now)?;
        Ok(id)
    }

    fn queue_put(&self, calendar_id: i64, id: i64, ics: &str, now: i64) -> Result<()> {
        let Some(home) = self.dav_target(calendar_id)? else {
            return Ok(());
        };
        let (href, etag, uid): (Option<String>, Option<String>, String) = self.conn().query_row(
            "SELECT href, etag, uid FROM events WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let href = match href {
            Some(h) => h,
            None => {
                let base = if home.ends_with('/') {
                    home
                } else {
                    format!("{home}/")
                };
                let h = format!("{base}{}", file_name(&uid));
                self.conn()
                    .execute("UPDATE events SET href = ?2 WHERE id = ?1", params![id, h])?;
                h
            }
        };
        // One pending PUT per event: a later edit replaces the queued
        // body and keeps the ETag the first change was made over.
        let updated = self.conn().execute(
            "UPDATE cal_queue SET ics = ?2 WHERE event_id = ?1 AND op = 1 AND error IS NULL",
            params![id, ics.as_bytes()],
        )?;
        if updated == 0 {
            self.conn().execute(
                "INSERT INTO cal_queue (calendar_id, event_id, op, href, if_match, ics, created)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)",
                params![calendar_id, id, href, etag, ics.as_bytes(), now],
            )?;
        }
        Ok(())
    }

    /// Delete an event (all its occurrences) here and queue the DELETE;
    /// `remove` for an event someone else organized (`OP_REMOVE`).
    pub fn delete_event(&self, id: i64, remove: bool, now: i64) -> Result<()> {
        let row: Option<(i64, Option<String>, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT calendar_id, href, etag FROM events WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((calendar_id, href, etag)) = row else {
            return Ok(());
        };
        let tx = self.write()?;
        tx.execute(
            "DELETE FROM cal_queue WHERE event_id = ?1 AND error IS NULL",
            [id],
        )?;
        tx.execute("DELETE FROM events WHERE id = ?1", [id])?;
        // Never on the server (no ETag yet): nothing to send.
        if let (Some(href), Some(etag)) = (href, etag)
            && self.dav_target(calendar_id)?.is_some()
        {
            let op = if remove { OP_REMOVE } else { OP_DELETE };
            tx.execute(
                "INSERT INTO cal_queue (calendar_id, op, href, if_match, created)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![calendar_id, op, href, etag, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Change one occurrence (`key`, its RECURRENCE-ID key) of a series:
    /// an override in the same resource, written and queued.
    pub fn edit_occurrence(&self, id: i64, key: &str, e: &Edit, me: &str, now: i64) -> Result<()> {
        let info = self.event_info(id)?.ok_or_else(|| anyhow!("no event"))?;
        let res = ical::parse_resource(info.ics.as_bytes()).ok_or_else(|| anyhow!("unreadable"))?;
        let master = res.master.ok_or_else(|| anyhow!("not a series"))?;
        let rid = rid_for(&master.event.start, key);
        let block = ical::override_event(e, &rid);
        let merged = merge_overrides(&info.ics, &block);
        if ical::parse_resource(merged.as_bytes()).is_none() {
            return Err(anyhow!("unreadable result"));
        }
        self.write_event(info.calendar_id, Some(id), &merged, me, now)?;
        Ok(())
    }

    /// Leave one occurrence (`key`) out of a series: an EXDATE, and its
    /// override dropped; written and queued.
    pub fn delete_occurrence(&self, id: i64, key: &str, now: i64) -> Result<()> {
        let info = self.event_info(id)?.ok_or_else(|| anyhow!("no event"))?;
        let res = ical::parse_resource(info.ics.as_bytes()).ok_or_else(|| anyhow!("unreadable"))?;
        let master = res.master.ok_or_else(|| anyhow!("not a series"))?;
        let rid = rid_for(&master.event.start, key);
        let value = rid.rsplit(':').next().unwrap_or_default().to_string();
        let mut out = info.ics.clone();
        for block in vevents(&info.ics) {
            if rid_of(&block).as_deref() == Some(value.as_str()) {
                out = out.replacen(&block, "", 1);
            }
        }
        let series = vevents(&out)
            .into_iter()
            .find(|b| rid_of(b).is_none())
            .ok_or_else(|| anyhow!("not a series"))?;
        let end = series
            .to_ascii_uppercase()
            .rfind("END:VEVENT")
            .unwrap_or(series.len());
        let (head, tail) = series.split_at(end);
        let with = out.replacen(&series, &format!("{head}EXDATE{rid}\r\n{tail}"), 1);
        if ical::parse_resource(with.as_bytes()).is_none() {
            return Err(anyhow!("unreadable result"));
        }
        self.write_event(info.calendar_id, Some(id), &with, "", now)?;
        Ok(())
    }

    // ---- The queue, read and settled by the pass thread ----------------

    /// The writes waiting for `calendar_id`, oldest first (conflicts and
    /// given-up writes left out).
    pub fn queued(&self, calendar_id: i64) -> Result<Vec<Queued>> {
        let mut st = self.conn().prepare_cached(
            "SELECT id, calendar_id, event_id, op, COALESCE(href, ''), if_match, COALESCE(ics, x'')
             FROM cal_queue WHERE calendar_id = ?1 AND error IS NULL ORDER BY id",
        )?;
        let rows = st.query_map([calendar_id], |r| {
            Ok(Queued {
                id: r.get(0)?,
                calendar_id: r.get(1)?,
                event_id: r.get(2)?,
                op: r.get(3)?,
                href: r.get(4)?,
                if_match: r.get(5)?,
                ics: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A write went through: the event takes the server's new ETag (none:
    /// the next pull reads it again).
    pub fn queue_sent(&self, q: &Queued, etag: Option<&str>) -> Result<()> {
        let tx = self.write()?;
        tx.execute("DELETE FROM cal_queue WHERE id = ?1", [q.id])?;
        if let Some(ev) = q.event_id.filter(|_| q.op == OP_PUT) {
            tx.execute(
                "UPDATE events SET etag = ?2, dirty = 0, conflict = 0 WHERE id = ?1",
                params![ev, etag],
            )?;
            // Later edits queued over the old ETag now follow the new one.
            tx.execute(
                "UPDATE cal_queue SET if_match = ?2 WHERE event_id = ?1 AND error IS NULL",
                params![ev, etag],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The server's copy changed (412): the change stays as a conflict,
    /// and the calendar is pulled in full so the server's version shows.
    pub fn queue_conflict(&self, q: &Queued) -> Result<()> {
        let tx = self.write()?;
        tx.execute(
            "UPDATE cal_queue SET error = ?2 WHERE id = ?1",
            params![q.id, CONFLICT],
        )?;
        if let Some(ev) = q.event_id {
            tx.execute(
                "UPDATE events SET conflict = 1, etag = NULL WHERE id = ?1",
                [ev],
            )?;
        }
        reset_sync(&tx, q.calendar_id)?;
        tx.commit()?;
        Ok(())
    }

    /// The server refused the write (403): the calendar is read only and
    /// the change is dropped; a new event goes, an edited or deleted one
    /// comes back from the server on the next pull.
    pub fn queue_refused(&self, q: &Queued) -> Result<()> {
        let tx = self.write()?;
        tx.execute("DELETE FROM cal_queue WHERE id = ?1", [q.id])?;
        tx.execute(
            "UPDATE calendars SET can_write = 0 WHERE id = ?1",
            [q.calendar_id],
        )?;
        if let Some(ev) = q.event_id.filter(|_| q.op == OP_PUT) {
            if q.if_match.is_none() {
                tx.execute("DELETE FROM events WHERE id = ?1", [ev])?;
            } else {
                tx.execute("UPDATE events SET etag = NULL WHERE id = ?1", [ev])?;
            }
        }
        reset_sync(&tx, q.calendar_id)?;
        tx.commit()?;
        Ok(())
    }

    /// A send failed with HTTP `status`: tried again on a later pass, and
    /// given up (kept with the status) after [`MAX_ATTEMPTS`].
    pub fn queue_failed(&self, q: &Queued, status: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE cal_queue SET attempts = attempts + 1,
               error = CASE WHEN attempts + 1 >= ?2 THEN ?3 ELSE NULL END WHERE id = ?1",
            params![q.id, MAX_ATTEMPTS, status],
        )?;
        Ok(())
    }

    /// Changes refused because the event changed on another device.
    pub fn conflicts(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM cal_queue WHERE error = ?1",
            [CONFLICT],
            |r| r.get(0),
        )?)
    }

    /// Settle every conflict: `again` sends the user's change once more
    /// over the server's current version (shown here again at once);
    /// otherwise the change is discarded and the server's version stays.
    /// The calendars touched.
    pub fn resolve_conflicts(&self, again: bool, me: &str, now: i64) -> Result<Vec<i64>> {
        let rows: Vec<ConflictRow> = self
            .conn()
            .prepare_cached(
                "SELECT id, calendar_id, event_id, op, COALESCE(href, ''), COALESCE(ics, x'')
                 FROM cal_queue WHERE error = ?1",
            )?
            .query_map([CONFLICT], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut cals = Vec::new();
        for (qid, cal, event, op, href, ics) in rows {
            if !cals.contains(&cal) {
                cals.push(cal);
            }
            // The server's version as the pull stored it, by href.
            let server: Option<(i64, Option<String>)> = self
                .conn()
                .query_row(
                    "SELECT id, etag FROM events WHERE calendar_id = ?1 AND href = ?2",
                    params![cal, href],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if !again {
                self.conn()
                    .execute("DELETE FROM cal_queue WHERE id = ?1", [qid])?;
                if let Some(ev) = event {
                    self.conn()
                        .execute("UPDATE events SET conflict = 0 WHERE id = ?1", [ev])?;
                }
                continue;
            }
            let etag = server.as_ref().and_then(|(_, e)| e.clone());
            self.conn().execute(
                "UPDATE cal_queue SET error = NULL, attempts = 0, if_match = ?2 WHERE id = ?1",
                params![qid, etag],
            )?;
            match (op, server) {
                (OP_PUT, Some((ev, _))) => {
                    let text = String::from_utf8_lossy(&ics).into_owned();
                    self.save_event(cal, Some(ev), &text, me, now)?;
                    self.conn()
                        .execute("UPDATE events SET conflict = 0 WHERE id = ?1", [ev])?;
                }
                (_, Some((ev, _))) => {
                    self.conn()
                        .execute("DELETE FROM events WHERE id = ?1", [ev])?;
                }
                // Gone from the server meanwhile: a PUT creates it again,
                // a DELETE has nothing left to do.
                (OP_PUT, None) => {}
                (_, None) => {
                    self.conn()
                        .execute("DELETE FROM cal_queue WHERE id = ?1", [qid])?;
                }
            }
        }
        Ok(cals)
    }

    /// A calendar setting (`cal_view`, `cal_default`), if set.
    pub fn cal_setting(&self, key: &str) -> Option<String> {
        self.conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .ok()
    }

    /// Changes not yet sent (all accounts, or one), for the warnings of
    /// "Remove all data" and account removal.
    pub fn unsent_changes(&self, account: Option<i64>) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM cal_queue q JOIN calendars c ON c.id = q.calendar_id
             WHERE ?1 IS NULL OR c.account_id = ?1",
            [account],
            |r| r.get(0),
        )?)
    }
}

/// Pull `calendar_id` in full on the next pass (no token, no ctag).
fn reset_sync(tx: &rusqlite::Transaction, calendar_id: i64) -> Result<()> {
    tx.execute(
        "UPDATE calendars SET ctag = NULL,
           sync_token = CASE WHEN sync_token IS NULL THEN NULL ELSE '' END WHERE id = ?1",
        [calendar_id],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
