//! Does IMAP IDLE wake, and how fast? One read-only connection idles on the
//! inbox while a second toggles `\Flagged` on and off on one of Komorebi's
//! own `[komorebi-test]` messages (found by subject; no other message is
//! touched, nothing is sent or deleted).
//!
//! With `--mail`, it instead sends one `[komorebi-test]` message to the
//! account's own address while idling (up to 3 minutes) and times the wake.
//!
//! ```text
//! cargo run --release --example idle_check -- <env-file> [--mail]
//! ```

use anyhow::{Context, Result, bail};
use komorebi::mail::imap::Imap;
use komorebi::mail::smtp;
use komorebi::providers::PRESETS;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: idle_check <env-file>")?;
    let text = std::fs::read_to_string(&path)?;
    let var = |k: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| key.trim() == k)
            .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_string())
            .with_context(|| format!("{k} missing"))
    };
    let (email, password) = (var("EMAIL")?, var("PASSWORD_EMAIL")?);
    let server = PRESETS[1].imap_server();

    let mut writer = Imap::connect(&server, &email, &password)?;
    let uidvalidity = writer.select("INBOX")?.uidvalidity;
    let Some(&uid) = writer.search(r#"SUBJECT "[komorebi-test]""#)?.iter().max() else {
        bail!("no [komorebi-test] message to flag");
    };

    let mut idler = Imap::connect(&server, &email, &password)?;
    idler.set_read_only(true);
    idler.select("INBOX")?;
    if std::env::args().any(|a| a == "--mail") {
        // An observer polls STATUS (nothing selected, so it is safe) to see
        // when the server has the message; the idler waits in 50 s chunks
        // because the crate's timeout restarts on every keepalive line.
        let mut observer = Imap::connect(&server, &email, &password)?;
        observer.set_read_only(true);
        let count =
            |o: &mut Imap| -> Result<u32> { Ok(o.status("INBOX")?.context("no STATUS")?.messages) };
        let base = count(&mut observer)?;
        let t0 = Instant::now();
        let waiter = std::thread::spawn(move || {
            let mut log = Vec::new();
            for _ in 0..4 {
                let before = idler.bytes();
                let outcome = idler.idle(Duration::from_secs(50));
                let changed = matches!(
                    outcome,
                    Ok(komorebi::mail::imap::WaitOutcome::MailboxChanged)
                );
                log.push((
                    t0.elapsed().as_secs_f64(),
                    format!("{:?}", outcome.map_err(|e| format!("{e:#}"))),
                    idler.bytes() - before,
                ));
                if changed {
                    break;
                }
            }
            let noop = idler
                .raw("NOOP")
                .map(|r| String::from_utf8_lossy(&r).contains("EXISTS"));
            (log, noop, idler)
        });
        std::thread::sleep(Duration::from_secs(3));
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let msg = smtp::build(smtp::Outgoing {
            from_email: email.clone(),
            to: email.clone(),
            subject: format!("[komorebi-test] idle {stamp}"),
            body: "Sent by Komorebi's IDLE check. Safe to delete.".into(),
            ..Default::default()
        })?;
        let t_send = t0.elapsed().as_secs_f64();
        smtp::send(&PRESETS[1].smtp_server(), &email, &password, &msg)?;
        println!(
            "sent at {t_send:.1} s, smtp done at {:.1} s",
            t0.elapsed().as_secs_f64()
        );
        let mut visible = None;
        while t0.elapsed() < Duration::from_secs(120) {
            if count(&mut observer)? > base {
                visible = Some(t0.elapsed().as_secs_f64());
                break;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        println!("observer STATUS sees it at: {visible:?} s");
        let (log, noop, idler) = waiter
            .join()
            .map_err(|_| anyhow::anyhow!("idle panicked"))?;
        for (at, outcome, bytes) in log {
            println!("idle chunk ended at {at:.1} s: {outcome}, {bytes} B received");
        }
        println!("NOOP on the idler afterwards reports EXISTS: {noop:?}");
        idler.logout();
        observer.logout();
        writer.logout();
        return Ok(());
    }
    for round in 1..=2 {
        let before = idler.bytes();
        let waiter = std::thread::spawn(move || {
            let outcome = idler.idle(Duration::from_secs(45));
            (outcome, Instant::now(), idler)
        });
        std::thread::sleep(Duration::from_secs(3));
        let t = Instant::now();
        writer.set_flags(("INBOX", uidvalidity), &[uid], "\\Flagged", round == 1)?;
        let (outcome, woke, back) = waiter
            .join()
            .map_err(|_| anyhow::anyhow!("idle panicked"))?;
        idler = back;
        println!(
            "round {round}: outcome {:?}, wake {:.0} ms after the flag change, {} B received while idling",
            outcome.map_err(|e| format!("{e:#}")),
            woke.saturating_duration_since(t).as_secs_f64() * 1000.0,
            idler.bytes() - before
        );
    }
    idler.logout();
    writer.logout();
    Ok(())
}
