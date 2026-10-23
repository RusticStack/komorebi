//! CardDAV (RFC 6352): finding an account's address books, and one sync
//! pass per book: local changes pushed first (read-and-write books only),
//! then the server's changes pulled and stored a batch per transaction.
//! The sync token or ctag is stored only after the last batch, so an
//! interrupted pass starts again safely.
//!
//! Writes patch the card as last synced (`contacts.raw`), so properties
//! Komorebi doesn't own survive. A PUT refused with 412 merges per field:
//! a field changed on one side only is merged and pushed; a field changed
//! on both sides takes the server's value, and the local version is kept
//! for the user to choose (`contact_conflicts`).

use super::discover::{self, Service};
use super::sync;
use super::{Client, DavError, Precondition};
use crate::contacts::Contact;
use crate::db::Db;
use crate::db::contacts::{BOOK_READ_WRITE, Book, CHANGED, CLEAN, DELETED, NEW};
use crate::mail::vcard;

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The store changed (the view should reload).
    pub changed: bool,
    /// Cards over a cap or unreadable, skipped.
    pub skipped: usize,
    /// Local edits the server overrode (kept for the user).
    pub conflicts: usize,
    pub pushed: usize,
}

/// The address books under `url` (a server, principal, home or book URL):
/// `(href, name, vCard 4 supported)`.
pub fn find_books(client: &Client, url: &str) -> Result<Vec<(String, String, bool)>, DavError> {
    let home = discover::home(client, url, Service::CardDav)?;
    let mut books: Vec<(String, String, bool)> =
        discover::collections(client, &home, Service::CardDav)?
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
                (i.href, name, i.vcard4)
            })
            .collect();
    books.truncate(50);
    Ok(books)
}

fn db_err(e: anyhow::Error) -> DavError {
    DavError::Network(e.to_string())
}

fn now_rev() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// One pass over `book`: push, then pull.
pub fn sync_book(client: &Client, db: &Db, book: &Book) -> Result<Outcome, DavError> {
    let mut out = Outcome::default();
    let Some(href) = book.href.clone() else {
        return Ok(out);
    };
    if book.mode == BOOK_READ_WRITE {
        push(client, db, book, &href, &mut out)?;
    }
    let local = db.book_etags(book.id).map_err(db_err)?;
    let changes = match sync::changes(
        client,
        &href,
        book.sync_collection,
        book.sync_token.as_deref(),
        book.ctag.as_deref(),
        &local,
    ) {
        Ok(c) => c,
        Err(e) => {
            db.set_book_state(
                book.id,
                book.sync_token.as_deref(),
                book.ctag.as_deref(),
                book.sync_collection,
                Some(1),
            )
            .ok();
            return Err(e);
        }
    };
    let Some(changes) = changes else {
        db.set_book_state(
            book.id,
            book.sync_token.as_deref(),
            book.ctag.as_deref(),
            book.sync_collection,
            None,
        )
        .map_err(db_err)?;
        return Ok(out);
    };
    let wanted: Vec<String> = changes.changed.iter().map(|(h, _)| h.clone()).collect();
    for batch in wanted.chunks(sync::BATCH) {
        let got = sync::multiget(client, &href, true, batch)?;
        let mut cards = Vec::new();
        for (h, etag, data) in got {
            match vcard::parse(data.as_bytes()) {
                Ok(card) => cards.push((h, etag, data.into_bytes(), card)),
                Err(_) => out.skipped += 1,
            }
        }
        let held = db.store_cards(book.id, &cards, &[]).map_err(db_err)?;
        out.changed |= !cards.is_empty();
        for h in held {
            if let Some((_, etag, raw, card)) = cards.iter().find(|c| c.0 == h) {
                merge_into(db, book, &h, etag, raw, &card.contact, &mut out)?;
            }
        }
    }
    if !changes.removed.is_empty() {
        let held = db
            .store_cards(book.id, &[], &changes.removed)
            .map_err(db_err)?;
        out.changed = true;
        // A local edit against a remote delete: the contact is created
        // again on the server at the next push.
        for h in held {
            if let Some(id) = contact_by_href(db, book.id, &h) {
                db.forget_href(id).map_err(db_err)?;
            }
        }
    }
    db.set_book_state(
        book.id,
        changes.token.as_deref(),
        changes.ctag.as_deref(),
        book.sync_collection,
        None,
    )
    .map_err(db_err)?;
    // Merged edits go out in the same pass.
    if book.mode == BOOK_READ_WRITE && out.changed {
        push(client, db, book, &href, &mut out)?;
    }
    Ok(out)
}

fn contact_by_href(db: &Db, book: i64, href: &str) -> Option<i64> {
    db.contact_by_href(book, href).ok().flatten()
}

fn push(
    client: &Client,
    db: &Db,
    book: &Book,
    collection: &str,
    out: &mut Outcome,
) -> Result<(), DavError> {
    for d in db.dirty_contacts(book.id).map_err(db_err)? {
        let id = d.contact.id;
        match d.dirty {
            DELETED => {
                if let Some(h) = &d.href {
                    match client.delete(h, d.etag.as_deref()) {
                        Ok(()) => {}
                        // Changed on the server: keep the server's version.
                        Err(DavError::Precondition) => {
                            db.set_contact_dirty(id, CLEAN).map_err(db_err)?;
                            db.clear_etag(id).map_err(db_err)?;
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                db.contact_pushed(id, d.href.as_deref().unwrap_or(""), None, None)
                    .map_err(db_err)?;
                out.pushed += 1;
                out.changed = true;
            }
            NEW => {
                let href = format!(
                    "{}/{}.vcf",
                    collection.trim_end_matches('/'),
                    safe_name(&d.contact.uid)
                );
                let version = if book.vcard_version >= 4 { 4 } else { 3 };
                let body = vcard::write(&d.contact, version, &now_rev());
                let etag = client.put(
                    &href,
                    body.as_bytes(),
                    "text/vcard; charset=utf-8",
                    Precondition::New,
                )?;
                db.contact_pushed(id, &href, etag.as_deref(), Some(body.as_bytes()))
                    .map_err(db_err)?;
                out.pushed += 1;
            }
            CHANGED => {
                let Some(href) = d.href.clone() else {
                    db.set_contact_dirty(id, NEW).map_err(db_err)?;
                    continue;
                };
                let body = write_over(d.raw.as_deref(), &d.contact, book);
                let pre = d
                    .etag
                    .as_deref()
                    .map_or(Precondition::New, Precondition::Match);
                match client.put(&href, body.as_bytes(), "text/vcard; charset=utf-8", pre) {
                    Ok(etag) => {
                        db.contact_pushed(id, &href, etag.as_deref(), Some(body.as_bytes()))
                            .map_err(db_err)?;
                        out.pushed += 1;
                    }
                    Err(DavError::Precondition) => {
                        let got =
                            sync::multiget(client, collection, true, std::slice::from_ref(&href))?;
                        match got.into_iter().next() {
                            Some((_, etag, data)) => {
                                if let Ok(card) = vcard::parse(data.as_bytes()) {
                                    merge_into(
                                        db,
                                        book,
                                        &href,
                                        &etag,
                                        data.as_bytes(),
                                        &card.contact,
                                        out,
                                    )?;
                                }
                            }
                            // Removed on the server: create it again.
                            None => db.forget_href(id).map_err(db_err)?,
                        }
                    }
                    Err(DavError::NotFound) => db.forget_href(id).map_err(db_err)?,
                    Err(e) => return Err(e),
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn safe_name(uid: &str) -> String {
    uid.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The card to PUT for `c`: its stored card patched, or a new one.
fn write_over(raw: Option<&[u8]>, c: &Contact, book: &Book) -> String {
    let rev = now_rev();
    raw.and_then(|r| vcard::patch(r, c, &rev))
        .unwrap_or_else(|| {
            let version = if book.vcard_version >= 4 { 4 } else { 3 };
            vcard::write(c, version, &rev)
        })
}

/// The fields a merge compares, as comparable values.
fn fields(c: &Contact) -> [String; 9] {
    let emails: Vec<String> = c
        .emails
        .iter()
        .map(|e| format!("{}|{}|{}", e.addr, e.label, e.pref))
        .collect();
    let phones: Vec<String> = c
        .phones
        .iter()
        .map(|p| format!("{}|{}", p.number, p.label))
        .collect();
    [
        c.name.clone(),
        c.given.clone(),
        c.family.clone(),
        c.org.clone(),
        c.title.clone(),
        c.note.clone(),
        emails.join("\n"),
        phones.join("\n"),
        c.groups.join("\n"),
    ]
}

/// Three-way merge of a contact with an unpushed local edit and the
/// server's newer card (`theirs`), with the card last synced as the base.
fn merge_into(
    db: &Db,
    book: &Book,
    href: &str,
    etag: &str,
    raw: &[u8],
    theirs: &Contact,
    out: &mut Outcome,
) -> Result<(), DavError> {
    let Some(id) = contact_by_href(db, book.id, href) else {
        return Ok(());
    };
    let Some(ours) = db.contact(id).map_err(db_err)? else {
        return Ok(());
    };
    let base = db
        .contact_raw(id)
        .map_err(db_err)?
        .and_then(|r| vcard::parse(&r).ok())
        .map(|c| c.contact)
        .unwrap_or_default();
    let (b, o, t) = (fields(&base), fields(&ours), fields(theirs));
    let overlap = (0..9).any(|i| o[i] != b[i] && t[i] != b[i] && o[i] != t[i]);
    let mut merged = theirs.clone();
    merged.id = id;
    merged.book_id = book.id;
    merged.uid = ours.uid.clone();
    if !overlap {
        // Our side's changes over theirs.
        if o[0] != b[0] {
            merged.name = ours.name.clone();
        }
        if o[1] != b[1] {
            merged.given = ours.given.clone();
        }
        if o[2] != b[2] {
            merged.family = ours.family.clone();
        }
        if o[3] != b[3] {
            merged.org = ours.org.clone();
        }
        if o[4] != b[4] {
            merged.title = ours.title.clone();
        }
        if o[5] != b[5] {
            merged.note = ours.note.clone();
        }
        if o[6] != b[6] {
            merged.emails = ours.emails.clone();
        }
        if o[7] != b[7] {
            merged.phones = ours.phones.clone();
        }
        if o[8] != b[8] {
            merged.groups = ours.groups.clone();
        }
    }
    // The server's card becomes the base; the merge (or, on overlap, the
    // server's version) is stored.
    db.set_contact_synced(id, href, etag, raw).map_err(db_err)?;
    db.save_contact(&mut merged).map_err(db_err)?;
    if overlap {
        let mine = vcard::write(&ours, 4, "");
        db.set_contact_conflict(id, Some(mine.as_bytes()))
            .map_err(db_err)?;
        db.set_contact_dirty(id, CLEAN).map_err(db_err)?;
        out.conflicts += 1;
    } else {
        // Pushed over the server's new version at the next push.
        db.set_contact_dirty(id, CHANGED).map_err(db_err)?;
    }
    out.changed = true;
    Ok(())
}

#[cfg(test)]
#[path = "carddav_tests.rs"]
mod tests;
