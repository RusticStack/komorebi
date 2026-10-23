//! CalDAV (RFC 4791): sending the queued writes ([lush]), finding an account's calendars (only on
//! the user's press of "Find calendars"), and one pull per calendar:
//! sync-collection or the ctag and ETag listing (`sync::changes`), the
//! changed events fetched with calendar-multiget 50 at a time, each batch
//! stored in one transaction. The sync token or ctag is stored only after
//! the last batch, so an interrupted pass starts again safely.

use super::discover::{self, Service};
use super::sync;
use super::{Client, DavError};
use crate::cal::store::{DavCalendar, FoundCalendar};
use crate::db::Db;

/// Calendars kept at most per account.
pub const MAX_CALENDARS: usize = 50;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The store changed (the view should reload).
    pub changed: bool,
    /// Resources over the size cap or unreadable.
    pub skipped: usize,
    /// Queued changes refused because the event changed on the server.
    pub conflicts: usize,
    /// Queued changes the server forbids (the calendar is read only).
    pub refused: usize,
}

/// One calendar's turn in a pass: its queued writes sent ([`flush`]),
/// then a pull ([`sync_calendar`]) with the sync state as the flush left
/// it (a conflict asks for a full pull, which brings the server's copy).
pub fn flush_and_pull(
    client: &Client,
    db: &Db,
    account: i64,
    cal: &DavCalendar,
    me: &str,
    now: i64,
) -> Result<Outcome, DavError> {
    let sent = flush(client, db, cal)?;
    let cal = db
        .dav_calendars(account)
        .map_err(db_err)?
        .into_iter()
        .find(|c| c.id == cal.id)
        .unwrap_or_else(|| cal.clone());
    let pulled = sync_calendar(client, db, &cal, me, now)?;
    Ok(Outcome {
        changed: sent.changed || pulled.changed,
        skipped: pulled.skipped,
        conflicts: sent.conflicts,
        refused: sent.refused,
    })
}

/// Send `cal`'s queued writes, oldest first (`cal::write`): a PUT with
/// `If-None-Match: *` (new) or `If-Match`, a DELETE with `If-Match` (and
/// `Schedule-Reply: F` for an event someone else organized). A 412 keeps
/// the change as a conflict, a 403 drops it, another status counts an
/// attempt; a network or sign-in failure stops the pass, so the pull
/// never runs over changes not yet sent.
pub fn flush(client: &Client, db: &Db, cal: &DavCalendar) -> Result<Outcome, DavError> {
    use super::Precondition;
    use crate::cal::write::{OP_PUT, OP_REMOVE};
    let mut out = Outcome::default();
    for q in db.queued(cal.id).map_err(db_err)? {
        let sent = if q.op == OP_PUT {
            let pre = match q.if_match.as_deref() {
                Some(e) => Precondition::Match(e),
                None => Precondition::New,
            };
            client.put(&q.href, &q.ics, "text/calendar; charset=utf-8", pre)
        } else {
            let extra: &[(&str, &str)] = if q.op == OP_REMOVE {
                &[("Schedule-Reply", "F")]
            } else {
                &[]
            };
            client
                .delete_with(&q.href, q.if_match.as_deref(), extra)
                .map(|()| None)
        };
        let stored = match sent {
            Ok(etag) => db.queue_sent(&q, etag.as_deref()),
            Err(DavError::Precondition) => {
                out.conflicts = out.conflicts.saturating_add(1);
                db.queue_conflict(&q)
            }
            Err(DavError::Status(403)) => {
                out.refused = out.refused.saturating_add(1);
                db.queue_refused(&q)
            }
            Err(DavError::Status(s)) => db.queue_failed(&q, i64::from(s)),
            Err(e) => return Err(e),
        };
        stored.map_err(db_err)?;
        out.changed = true;
    }
    Ok(out)
}

/// Whether a collection holds events: a missing component set counts as
/// events (Purelymail sends none); a task-only list does not.
fn has_events(components: &[String]) -> bool {
    components.is_empty() || components.iter().any(|c| c == "VEVENT")
}

/// The event calendars under `url` (a server, principal, home or calendar
/// URL), at most [`MAX_CALENDARS`].
pub fn find_calendars(client: &Client, url: &str) -> Result<Vec<FoundCalendar>, DavError> {
    // A calendar pasted directly is used as it is.
    let top = client.propfind(url, "0", super::xml::PROPFIND_COLLECTIONS)?;
    let mut items = match top.items.into_iter().next() {
        Some(i) if i.calendar => vec![i],
        _ => {
            let home = discover::home(client, url, Service::CalDav)?;
            discover::collections(client, &home, Service::CalDav)?
        }
    };
    items.retain(|i| has_events(&i.components));
    items.truncate(MAX_CALENDARS);
    Ok(items
        .into_iter()
        .map(|i| {
            let name = i
                .displayname
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| {
                    i.href
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .to_string()
                });
            FoundCalendar {
                href: i.href,
                name,
                sync_collection: i.sync_collection,
            }
        })
        .collect())
}

fn db_err(e: anyhow::Error) -> DavError {
    DavError::Network(e.to_string())
}

/// One pull of `cal` into the store, as attendee `me` (the account's
/// address), with occurrences placed around `now`.
pub fn sync_calendar(
    client: &Client,
    db: &Db,
    cal: &DavCalendar,
    me: &str,
    now: i64,
) -> Result<Outcome, DavError> {
    let mut out = Outcome::default();
    let by_token = cal.sync_token.is_some();
    let local = db.calendar_etags(cal.id).map_err(db_err)?;
    let token = cal.sync_token.as_deref().filter(|t| !t.is_empty());
    let Some(changes) = sync::changes(
        client,
        &cal.href,
        by_token,
        token,
        cal.ctag.as_deref(),
        &local,
    )?
    else {
        return Ok(out);
    };
    let wanted: Vec<String> = changes.changed.iter().map(|(h, _)| h.clone()).collect();
    for batch in wanted.chunks(sync::BATCH) {
        let got: Vec<(String, String, Vec<u8>)> = sync::multiget(client, &cal.href, false, batch)?
            .into_iter()
            .map(|(h, e, d)| (h, e, d.into_bytes()))
            .collect();
        if got.is_empty() {
            continue;
        }
        let skipped = db
            .store_synced(cal.id, &got, &[], me, now)
            .map_err(db_err)?;
        out.skipped = out.skipped.saturating_add(skipped);
        out.changed = true;
    }
    if !changes.removed.is_empty() {
        db.store_synced(cal.id, &[], &changes.removed, me, now)
            .map_err(db_err)?;
        out.changed = true;
    }
    db.set_calendar_sync(
        cal.id,
        by_token,
        changes.token.as_deref(),
        changes.ctag.as_deref(),
    )
    .map_err(db_err)?;
    Ok(out)
}

#[cfg(test)]
#[path = "caldav_tests.rs"]
mod tests;
