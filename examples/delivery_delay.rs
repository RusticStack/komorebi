//! How long the provider took to deliver Komorebi's own `[komorebi-test]`
//! messages: their Date header (stamped when built) vs the server's
//! INTERNALDATE. Read-only (EXAMINE); touches no other message.
//!
//! ```text
//! cargo run --release --example delivery_delay -- <env-file> [folder]
//! ```

use anyhow::{Context, Result};
use komorebi::mail::imap::Imap;
use komorebi::providers::PRESETS;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: delivery_delay <env-file>")?;
    let text = std::fs::read_to_string(&path)?;
    let var = |k: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| key.trim() == k)
            .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_string())
            .with_context(|| format!("{k} missing"))
    };
    let (email, password) = (var("EMAIL")?, var("PASSWORD_EMAIL")?);
    let mut imap = Imap::connect(&PRESETS[1].imap_server(), None, &email, &password)?;
    imap.set_read_only(true);
    let folder = std::env::args().nth(2).unwrap_or_else(|| "INBOX".into());
    imap.select(&folder)?;
    let mut uids: Vec<u32> = imap
        .search(r#"SUBJECT "[komorebi-test]""#)?
        .into_iter()
        .collect();
    uids.sort_unstable();
    println!("{} test messages in {folder}", uids.len());
    let built: std::collections::HashMap<u32, i64> = imap
        .fetch_headers(&uids)?
        .into_iter()
        .map(|h| (h.uid, h.date))
        .collect();
    for (uid, delivered) in imap.internal_dates(&uids)? {
        if let Some(date) = built.get(&uid) {
            println!(
                "  uid {uid}: delivered {} s after it was built",
                delivered - date
            );
        }
    }
    imap.logout();
    Ok(())
}
