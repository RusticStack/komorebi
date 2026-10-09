//! Print UID, MODSEQ and FLAGS for the given inbox UIDs (read-only, EXAMINE;
//! no message content). Shows in which order flag changes happened.
//!
//! ```text
//! cargo run --release --example modseq_check -- <env-file> <uid> [<uid> ...]
//! ```

use anyhow::{Context, Result};
use komorebi::mail::imap::Imap;
use komorebi::providers::PRESETS;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .context("usage: modseq_check <env-file> <uid>...")?;
    let uids: Vec<u32> = args[2..]
        .iter()
        .map(|u| u.parse())
        .collect::<Result<_, _>>()?;
    let text = std::fs::read_to_string(path)?;
    let var = |k: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| key.trim() == k)
            .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_string())
            .with_context(|| format!("{k} missing"))
    };
    let mut imap = Imap::connect(
        &PRESETS[1].imap_server(),
        &var("EMAIL")?,
        &var("PASSWORD_EMAIL")?,
    )?;
    imap.set_read_only(true);
    let sel = imap.select("INBOX")?;
    println!("inbox uidnext {}", sel.uid_next);
    let set: Vec<String> = uids.iter().map(u32::to_string).collect();
    let raw = imap.raw(&format!("UID FETCH {} (UID MODSEQ FLAGS)", set.join(",")))?;
    for line in String::from_utf8_lossy(&raw).lines() {
        println!("  {line}");
    }
    imap.logout();
    Ok(())
}
