//! Deterministic data for a throwaway cache: a fake account on `.invalid`
//! servers and generated messages, nothing that touches the network. Used
//! by `examples/seed.rs` (timing at scale, CI's boot snapshot) and by the
//! GPUI app's tests (docs/GPUI-TESTING.md), so both see the same mail.
//!
//! The newest [`BODIES`] messages get bodies, like a first sync. One sender
//! has accents ("João Gonçalves": the 12th newest inbox message, two
//! messages in Archive and one sent to him), for the folded search.

use crate::db::{Db, NewAccount, NewHeader};
use crate::mail::mime;
use crate::providers::{Security, Server};
use crate::role::Role;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// How many of the newest inbox messages get a body.
pub const BODIES: usize = 300;
const BATCH: usize = 5_000;
/// 2026-01-01T00:00:00Z; message `uid` is `uid` hours older than this.
pub const NEWEST: i64 = 1_767_225_600;
/// The seed account's address.
pub const EMAIL: &str = "seed@komorebi.invalid";

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

/// What [`seed`] made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seeded {
    pub account: i64,
    pub inbox: i64,
    pub sent: i64,
    pub archive: i64,
}

/// The seed account with Inbox, Sent and Archive, and `count` generated
/// inbox messages (the same on every run).
pub fn seed(db: &mut Db, count: u32) -> Result<Seeded> {
    let server = |port| Server {
        host: "mail.komorebi.invalid".into(),
        port,
        security: Security::Tls,
    };
    let account = db.add_account(&NewAccount {
        imap_user: String::new(),
        smtp_user: String::new(),
        email: EMAIL.into(),
        name: "Seed Account".into(),
        imap: server(993),
        smtp: server(465),
        save_sent: true,
        smtp_no_auth: false,
    })?;
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
                // Every 13th from the third newest: the sender set high
                // importance; every 13th from the eighth newest, low
                // (outside the random stream, so the rest of the cache
                // stays as it was).
                importance: match (count - uid) % 13 {
                    2 => crate::db::Importance::High,
                    7 => crate::db::Importance::Low,
                    _ => crate::db::Importance::Normal,
                },
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
            from_addr: EMAIL.into(),
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
        db.set_body(
            id,
            raw.as_bytes(),
            &mime::preview(&text),
            false,
            Default::default(),
        )?;
    }
    Ok(Seeded {
        account,
        inbox,
        sent,
        archive,
    })
}

/// Whether `path` is in the app's own cache directory
/// (`%LOCALAPPDATA%\komorebi`), where the real cache lives.
pub fn in_real_cache_dir(path: &Path) -> bool {
    let lower = |p: &Path| {
        std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    dirs::data_local_dir().is_some_and(|dir| lower(path).starts_with(&lower(&dir.join("komorebi"))))
}

/// Seed only throwaway files: never the app's cache directory, never a file
/// that already holds data.
pub fn refuse_real_cache(path: &Path) -> Result<()> {
    if in_real_cache_dir(path) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_is_the_same_every_time() {
        let mut a = Db::in_memory().unwrap();
        let mut b = Db::in_memory().unwrap();
        let sa = seed(&mut a, 40).unwrap();
        let sb = seed(&mut b, 40).unwrap();
        assert_eq!(sa, sb);
        let subjects = |db: &Db| -> Vec<(String, bool)> {
            db.message_ids(&[sa.inbox], "")
                .unwrap()
                .into_iter()
                .map(|id| {
                    let m = db.message(id).unwrap().unwrap();
                    (m.subject, m.seen)
                })
                .collect()
        };
        assert_eq!(subjects(&a).len(), 40);
        assert_eq!(subjects(&a), subjects(&b));
        // The accented sender is the 12th newest.
        let twelfth = a.message_ids(&[sa.inbox], "").unwrap()[11];
        assert_eq!(
            a.message(twelfth).unwrap().unwrap().from_name,
            "João Gonçalves"
        );
    }

    #[test]
    fn only_throwaway_paths_are_seeded() {
        if let Some(dir) = dirs::data_local_dir() {
            let real = dir.join("komorebi").join("cache.sqlite3");
            assert!(in_real_cache_dir(&real));
            assert!(refuse_real_cache(&real).is_err());
        }
        let elsewhere = std::env::temp_dir().join("komorebi-seed-test-none.sqlite3");
        assert!(!in_real_cache_dir(&elsewhere));
    }
}
