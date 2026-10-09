//! Fill a new cache with a fake account and deterministic messages, for
//! timing the app at scale (nothing touches the network).
//!
//! ```text
//! cargo run --release --example seed -- <new.sqlite3> <count> [eml-dir] [--drafts] [--outbox] [--signature] [--html-reply]
//! $env:KOMOREBI_OFFLINE=1; $env:KOMOREBI_DB="<new.sqlite3>"; cargo run --release
//! ```
//!
//! The newest 300 messages get bodies, like a first sync. One sender has
//! accents ("João Gonçalves": the 12th newest inbox message, two messages
//! in Archive and one sent to him), for the folded search. With `eml-dir`
//! (for example the synthetic corpus in `tests/fixtures/html/`), each
//! `.eml` file there is added as well, newest first in file-name order, so
//! the reader can be checked on real MIME and HTML (`tests/fixtures/invite/`
//! holds meeting invitations to the seed account, for the invite card). `--drafts` adds two
//! saved drafts: a new message, and a reply to the newest message. `--outbox`
//! adds a message whose send failed (in the Outbox) and, newest, a draft with
//! no subject that mentions an attachment (for the send guards).
//! `--signature` gives the account a two-line signature. `--html-reply`
//! (with `eml-dir`) adds, newest, a reply to the newest HTML message as
//! Komorebi sends it (Markdown-lite text, signature and the sanitized HTML
//! quote, built by `compose_html` and `smtp::build_raw`), received from
//! its sender, to see the HTML part as a recipient would. It refuses
//! the app's own cache directory and any existing non-empty file.

use anyhow::{Context, Result, bail};
use komorebi::db::{Db, DraftRow, NewAccount, NewHeader, Outbox};
use komorebi::mail::{compose_html, mime, smtp};
use komorebi::providers::{Security, Server};
use komorebi::role::Role;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const BODIES: usize = 300;
const BATCH: usize = 5_000;
/// 2026-01-01T00:00:00Z; message `uid` is `uid` hours older than this.
const NEWEST: i64 = 1_767_225_600;

const NAMES: &[&str] = &[
    "Ada Lovelace",
    "Alan Turing",
    "Grace Hopper",
    "Edsger Dijkstra",
    "Barbara Liskov",
    "Donald Knuth",
    "Frances Allen",
    "Ken Thompson",
];
const TOPICS: &[&str] = &[
    "Quarterly report",
    "Lunch on Friday?",
    "Invoice 2041",
    "Build is green again",
    "Re: travel plans",
    "Meeting notes",
    "Your order has shipped",
    "Draft proposal for review",
    "Weekend hike",
    "Server maintenance window",
];
const WORDS: &[&str] = &[
    "please", "find", "attached", "the", "latest", "numbers", "for", "next", "week", "thanks",
    "again", "let", "me", "know", "if", "anything", "changes", "before", "monday", "cheers",
];

fn main() -> Result<()> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let drafts = all.iter().any(|a| a == "--drafts");
    let outbox = all.iter().any(|a| a == "--outbox");
    let signature = all.iter().any(|a| a == "--signature");
    let html_reply = all.iter().any(|a| a == "--html-reply");
    let mut args = all.into_iter().filter(|a| {
        !matches!(
            a.as_str(),
            "--drafts" | "--outbox" | "--signature" | "--html-reply"
        )
    });
    let usage = "usage: seed <new.sqlite3> <count> [eml-dir] [--drafts] [--outbox] [--signature] [--html-reply]";
    let path = PathBuf::from(args.next().context(usage)?);
    let count: u32 = args.next().context(usage)?.parse().context(usage)?;
    let eml_dir = args.next().map(PathBuf::from);
    refuse_real_cache(&path)?;

    let mut db = Db::open(&path)?;
    let server = |port| Server {
        host: "mail.komorebi.invalid".into(),
        port,
        security: Security::Tls,
    };
    let account = db.add_account(&NewAccount {
        email: "seed@komorebi.invalid".into(),
        name: "Seed Account".into(),
        imap: server(993),
        smtp: server(465),
        save_sent: true,
    })?;
    if signature && let Some(mut a) = db.accounts()?.pop() {
        a.signature = "Seed Account\nKomorebi test data".into();
        db.update_account(&a)?;
    }
    let inbox = db.upsert_folder(account, "INBOX", "Inbox", 0, Role::Inbox)?;
    let sent = db.upsert_folder(account, "Sent", "Sent", 0, Role::Sent)?;
    let archive = db.upsert_folder(account, "Archive", "Archive", 0, Role::Archive)?;

    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut headers: Vec<NewHeader> = (1..=count)
        .map(|uid| {
            let name = rng.pick(NAMES);
            NewHeader {
                uid,
                msgid: format!("<{uid}@seed.komorebi.invalid>"),
                subject: format!("{} #{uid}", rng.pick(TOPICS)),
                from_name: name.to_string(),
                from_addr: format!("{}@example.invalid", name.to_lowercase().replace(' ', ".")),
                to: "Seed Account <seed@komorebi.invalid>".into(),
                date: NEWEST - i64::from(count - uid) * 3_600,
                size: 2_000 + rng.below(30_000) as i64,
                // Older mail is read; about one in ten recent messages is not.
                seen: uid + 200 < count || rng.below(10) != 0,
                flagged: rng.below(40) == 0,
                answered: rng.below(8) == 0,
                ..NewHeader::default()
            }
        })
        .collect();
    const JOAO: (&str, &str) = ("João Gonçalves", "joao.goncalves@example.invalid");
    if let Some(h) = headers.iter_mut().rev().nth(11) {
        h.from_name = JOAO.0.into();
        h.from_addr = JOAO.1.into();
    }
    for batch in headers.chunks(BATCH) {
        db.insert_headers(account, inbox, batch)?;
    }
    let joao = |uid: u32, days: i64, subject: &str| NewHeader {
        uid,
        msgid: format!("<joao-{uid}@seed.komorebi.invalid>"),
        subject: subject.into(),
        from_name: JOAO.0.into(),
        from_addr: JOAO.1.into(),
        to: "Seed Account <seed@komorebi.invalid>".into(),
        date: NEWEST - days * 86_400,
        size: 4_000,
        seen: true,
        ..NewHeader::default()
    };
    db.insert_headers(
        account,
        archive,
        &[
            joao(1, 40, "Ação de formação em março"),
            joao(2, 12, "Orçamento aprovado"),
        ],
    )?;
    db.insert_headers(
        account,
        sent,
        &[NewHeader {
            from_name: "Seed Account".into(),
            from_addr: "seed@komorebi.invalid".into(),
            to: format!("{} <{}>", JOAO.0, JOAO.1),
            ..joao(1, 11, "Re: Orçamento aprovado")
        }],
    )?;

    for id in db.message_ids(&[inbox], "")?.into_iter().take(BODIES) {
        let m = db.message(id)?.context("seeded message vanished")?;
        let text = (0..60)
            .map(|i| {
                let word = rng.pick(WORDS);
                if i % 12 == 11 {
                    format!("{word}.\n")
                } else {
                    format!("{word} ")
                }
            })
            .collect::<String>();
        let raw = format!(
            "Message-ID: {}\r\nFrom: {} <{}>\r\nTo: {}\r\nSubject: {}\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\r\n{}",
            m.msgid,
            m.from_name,
            m.from_addr,
            m.to,
            m.subject,
            text.replace('\n', "\r\n"),
        );
        db.set_body(id, raw.as_bytes(), &mime::preview(&text), false)?;
    }
    let mut added = 0;
    if let Some(dir) = eml_dir {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("eml")))
            .collect();
        files.sort();
        for (i, file) in files.iter().enumerate() {
            let raw = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            let uid = count + 1 + i as u32;
            let header = NewHeader {
                uid,
                // The first file sorts to the top of the list.
                date: NEWEST + 3_600 * (files.len() - i) as i64,
                size: raw.len() as i64,
                seen: true,
                ..mime::parse_header(&raw)
            };
            db.insert_headers(account, inbox, std::slice::from_ref(&header))?;
            let id = db
                .message_ids(&[inbox], "")?
                .into_iter()
                .find(|&id| {
                    db.message(id)
                        .is_ok_and(|m| m.is_some_and(|m| m.uid == uid))
                })
                .context("added message vanished")?;
            let body = mime::parse_body(&raw);
            db.set_body(id, &raw, &body.preview(), header.has_attach)?;
            added += 1;
        }
    }
    if html_reply {
        let raw = html_reply_to(&db, inbox)?;
        let uid = count + 1 + added;
        let header = NewHeader {
            uid,
            date: NEWEST + 3_600 * (i64::from(added) + 1),
            size: raw.len() as i64,
            ..mime::parse_header(&raw)
        };
        db.insert_headers(account, inbox, std::slice::from_ref(&header))?;
        let id = *db
            .message_ids(&[inbox], "")?
            .first()
            .context("added reply vanished")?;
        let body = mime::parse_body(&raw);
        db.set_body(id, &raw, &body.preview(), false)?;
    }
    if drafts {
        let newest = db.message_ids(&[inbox], "")?.first().copied();
        let reply_to = match newest {
            Some(id) => db.message(id)?,
            None => None,
        };
        db.insert_draft(&DraftRow {
            account_id: account,
            to: "Grace Hopper <grace.hopper@example.invalid>".into(),
            subject: "Notes for Thursday".into(),
            body: "Hi Grace,\n\nHere are the notes I promised.\n".into(),
            message_id: "<draft-1@seed.komorebi.invalid>".into(),
            ..Default::default()
        })?;
        if let Some(m) = reply_to {
            db.insert_draft(&DraftRow {
                account_id: account,
                kind: 1,
                reply_to_msg: Some(m.id),
                in_reply_to: m.msgid.clone(),
                to: m.from_addr.clone(),
                subject: format!("Re: {}", m.subject),
                body: "Thanks, I'll look at it tomorrow.\n\n> (quoted text)".into(),
                message_id: "<draft-2@seed.komorebi.invalid>".into(),
                ..Default::default()
            })?;
        }
    }
    if outbox {
        // Through the real steps: Send, build, queue, a send takes it and
        // the server refuses it.
        let failed = DraftRow {
            account_id: account,
            to: "Alan Turing <alan.turing@example.invalid>".into(),
            subject: "Slides for Monday".into(),
            body: "Here are the slides.\n".into(),
            message_id: "<outbox-1@seed.komorebi.invalid>".into(),
            ..Default::default()
        };
        let raw = smtp::build_raw(smtp::Outgoing {
            from_email: "seed@komorebi.invalid".into(),
            to: failed.to.clone(),
            subject: failed.subject.clone(),
            body: failed.body.clone(),
            message_id: failed.message_id.clone(),
            ..Default::default()
        })?;
        let id = db.insert_draft(&failed)?;
        db.outbox_send(id)?;
        db.store_raw(id, &raw, &AtomicBool::new(false))?;
        db.outbox_queue(id)?;
        db.claim_outbox(id)?;
        db.outbox_result(
            id,
            Outbox::Failed,
            None,
            "sending failed: permanent error (550): 5.1.1 <alan.turing@example.invalid>: \
             Recipient address rejected: User unknown",
        )?;
        db.insert_draft(&DraftRow {
            account_id: account,
            to: "Barbara Liskov <barbara.liskov@example.invalid>".into(),
            body: "Hi Barbara,\n\nThe signed contract is attached.\n".into(),
            message_id: "<draft-3@seed.komorebi.invalid>".into(),
            ..Default::default()
        })?;
    }
    println!(
        "seeded {count} messages and {added} .eml files into {}{}{}{}",
        path.display(),
        if drafts { ", with drafts" } else { "" },
        if outbox { ", with an outbox" } else { "" },
        if html_reply {
            ", with an HTML reply"
        } else {
            ""
        }
    );
    Ok(())
}

/// A reply to the newest HTML message in `inbox`, built as the composer
/// sends one, as received from that message's sender.
fn html_reply_to(db: &Db, inbox: i64) -> Result<Vec<u8>> {
    let (m, html) = db
        .message_ids(&[inbox], "")?
        .into_iter()
        .filter_map(|id| db.message(id).ok().flatten())
        .find_map(|m| {
            let html = compose_html::sanitize(&mime::html_body(m.raw.as_deref()?)?)?;
            Some((m, html))
        })
        .context("--html-reply needs an HTML message: pass an eml-dir")?;
    let attribution = format!("On Thu, 1 Jan 2026 at 09:00, {} wrote:", m.from_name);
    let quote: String = mime::parse_body(m.raw.as_deref().unwrap_or_default())
        .text
        .lines()
        .map(|l| format!("> {l}\n"))
        .collect();
    let body = format!(
        "Thanks, this is **really** useful. Two things:\n\n\
         - the *dates* work for me\n\
         - I left notes in [the agenda](https://example.invalid/agenda)\n\n\
         -- \nSeed Account\nKomorebi test data\n\n{attribution}\n{quote}"
    );
    let block = compose_html::reply_quote(&attribution, &html);
    let stored = compose_html::attach(&body, &attribution, &block);
    let (body, body_html) = compose_html::alternative(&body, stored.as_deref());
    smtp::build_raw(smtp::Outgoing {
        from_name: m.from_name.clone(),
        from_email: if m.from_addr.is_empty() {
            "sender@example.invalid".into()
        } else {
            m.from_addr.clone()
        },
        to: "Seed Account <seed@komorebi.invalid>".into(),
        subject: format!("Re: {}", m.subject),
        body,
        body_html,
        in_reply_to: m.msgid.clone(),
        ..Default::default()
    })
}

/// Seed only throwaway files: never the app's cache directory, never a file
/// that already holds data.
fn refuse_real_cache(path: &Path) -> Result<()> {
    let lower = |p: &Path| -> Result<String> {
        Ok(std::path::absolute(p)?.to_string_lossy().to_lowercase())
    };
    if let Some(dir) = dirs::data_local_dir()
        && lower(path)?.starts_with(&lower(&dir.join("komorebi"))?)
    {
        bail!("refusing to seed the app's cache directory");
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 0) {
        bail!("refusing to seed {}: it already exists", path.display());
    }
    Ok(())
}

/// xorshift64: the same data on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}
