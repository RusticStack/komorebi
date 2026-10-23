//! The pull step CardDAV and CalDAV share: what changed in one collection
//! since the last pass, and the changed resources fetched in batches.

use super::{Client, DavError, xml};
use std::collections::{HashMap, HashSet};

/// Hrefs per multiget.
pub const BATCH: usize = 50;
/// sync-collection rounds read at most in one pass (507 truncation).
const MAX_ROUNDS: usize = 100;
/// Resources per collection kept at most.
pub const MAX_RESOURCES: usize = 20_000;

/// What changed in a collection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Changed or new resources with their ETags.
    pub changed: Vec<(String, String)>,
    /// Removed resources.
    pub removed: Vec<String>,
    /// The token or ctag to store once every batch is stored.
    pub token: Option<String>,
    pub ctag: Option<String>,
}

/// Whether `item` is the collection itself or another collection.
fn is_member(item: &xml::Item, collection: &str) -> bool {
    !item.collection && item.href.trim_end_matches('/') != collection.trim_end_matches('/')
}

/// Changes in `collection` since `token` (sync-collection, when the
/// server offers it) or else since `ctag`, against the `local` ETags by
/// href. `None`: nothing changed (the ctag is the same).
pub fn changes(
    client: &Client,
    collection: &str,
    sync_collection: bool,
    token: Option<&str>,
    ctag: Option<&str>,
    local: &HashMap<String, String>,
) -> Result<Option<Changes>, DavError> {
    let path = client.resolve(collection)?;
    let collection = &path[url::Position::BeforePath..url::Position::AfterQuery];
    if sync_collection {
        match by_token(client, collection, token.unwrap_or(""), local) {
            // The token expired: start again from nothing.
            Err(DavError::Status(403 | 409)) if token.is_some_and(|t| !t.is_empty()) => {
                return by_token(client, collection, "", local).map(Some);
            }
            other => return other.map(Some),
        }
    }
    let top = client.propfind(collection, "0", xml::PROPFIND_CTAG)?;
    let new_ctag = top.items.first().and_then(|i| i.ctag.clone());
    if new_ctag.is_some() && new_ctag.as_deref() == ctag {
        return Ok(None);
    }
    let list = client.propfind(collection, "1", xml::PROPFIND_ETAGS)?;
    let mut seen = HashSet::new();
    let mut out = Changes {
        ctag: new_ctag,
        ..Changes::default()
    };
    for item in list.items.iter().filter(|i| is_member(i, collection)) {
        if seen.len() >= MAX_RESOURCES {
            return Err(DavError::TooLarge);
        }
        seen.insert(item.href.clone());
        let etag = item.etag.clone().unwrap_or_default();
        if local.get(&item.href) != Some(&etag) || etag.is_empty() {
            out.changed.push((item.href.clone(), etag));
        }
    }
    out.removed = local
        .keys()
        .filter(|h| !seen.contains(*h))
        .cloned()
        .collect();
    Ok(Some(out))
}

fn by_token(
    client: &Client,
    collection: &str,
    token: &str,
    local: &HashMap<String, String>,
) -> Result<Changes, DavError> {
    let full = token.is_empty();
    let mut token = token.to_string();
    let mut out = Changes::default();
    let mut seen = HashSet::new();
    for _ in 0..MAX_ROUNDS {
        let m = client.report(collection, "0", &xml::sync_collection(&token))?;
        for item in m.items.iter().filter(|i| is_member(i, collection)) {
            if item.status == 404 {
                out.removed.push(item.href.clone());
            } else {
                if seen.len() >= MAX_RESOURCES {
                    return Err(DavError::TooLarge);
                }
                seen.insert(item.href.clone());
                let etag = item.etag.clone().unwrap_or_default();
                if local.get(&item.href) != Some(&etag) || etag.is_empty() {
                    out.changed.push((item.href.clone(), etag));
                }
            }
        }
        let next = m.sync_token.clone().unwrap_or_default();
        let more = m.truncated && next != token;
        token = next;
        if !more {
            break;
        }
    }
    if full {
        // A full listing: what we hold and the server didn't list is gone.
        out.removed
            .extend(local.keys().filter(|h| !seen.contains(*h)).cloned());
    }
    out.token = Some(token).filter(|t| !t.is_empty());
    Ok(out)
}

/// The resources at `hrefs` with their ETags and data, [`BATCH`] per
/// request (`card`: addressbook-multiget, else calendar-multiget). A
/// resource the server didn't return is left out.
pub fn multiget(
    client: &Client,
    collection: &str,
    card: bool,
    hrefs: &[String],
) -> Result<Vec<(String, String, String)>, DavError> {
    let mut out = Vec::new();
    for batch in hrefs.chunks(BATCH) {
        let m = client.report(collection, "1", &xml::multiget(card, batch))?;
        for item in m.items {
            if item.status / 100 == 2
                && let Some(data) = item.data
            {
                out.push((item.href, item.etag.unwrap_or_default(), data));
            }
        }
    }
    Ok(out)
}
