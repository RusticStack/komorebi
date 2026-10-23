//! Fill a new cache with a fake account and deterministic messages, for
//! timing the app at scale (nothing touches the network).
//!
//! ```text
//! cargo run --release --example seed -- <new.sqlite3> <count> [eml-dir] [--drafts] [--outbox] [--signature] [--html-reply]
//!     [--accounts] [--folders N] [--fold a2,f117]
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
//! `--signature` adds two named signatures, the account's defaults (a
//! two-line one for new mail, a short one for replies and forwards). `--html-reply`
//! (with `eml-dir`) adds, newest, a reply to the newest HTML message as
//! Komorebi sends it (Markdown-lite text, signature and the sanitized HTML
//! quote, built by `compose_html` and `smtp::build_raw`), received from
//! its sender, to see the HTML part as a recipient would. It refuses
//! the app's own cache directory and any existing non-empty file.
//!
//! For the folder pane: `--accounts` adds nested folders to the seed
//! account (`INBOX/Receipts`, a `Projects` tree with a `Projects-old`
//! sibling and a path deeper than the drawn level cap, Newsletters with
//! unread mail, Drafts, Junk and Trash) and two more `.invalid` accounts
//! ("Maria Gonçalves" with a 64-character address and a long folder name
//! with 1234 unread, and "Work" with `Team/Standups`). `--folders N` adds
//! N more folders (depths 0 to 3, about a third with unread mail) to the
//! seed account, for scrolling and folding at scale. `--fold a2,f117`
//! saves those folded lines (the `nav_folded` setting); the ids of the
//! accounts and of the folders with subfolders are printed.

use anyhow::{Context, Result, bail};
use komorebi::db::{Db, DraftRow, Importance, NewAccount, NewHeader, Outbox};
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
    let usage = "usage: seed <new.sqlite3> <count> [eml-dir] [--drafts] [--outbox] [--signature] [--html-reply] [--accounts] [--folders N] [--fold a2,f117]";
    let mut all: Vec<String> = std::env::args().skip(1).collect();
    // The flags with a value, taken out first.
    let mut value = |flag: &str| -> Result<Option<String>> {
        match all.iter().position(|a| a == flag) {
            Some(i) if i + 1 < all.len() => {
                let v = all.remove(i + 1);
                all.remove(i);
                Ok(Some(v))
            }
            Some(_) => bail!("{flag} needs a value; {usage}"),
            None => Ok(None),
        }
    };
    let extra_folders: usize = match value("--folders")? {
        Some(n) => n.parse().context(usage)?,
        None => 0,
    };
    let fold = value("--fold")?;
    let drafts = all.iter().any(|a| a == "--drafts");
    let outbox = all.iter().any(|a| a == "--outbox");
    let signature = all.iter().any(|a| a == "--signature");
    let html_reply = all.iter().any(|a| a == "--html-reply");
    let accounts = all.iter().any(|a| a == "--accounts");
    let mut args = all.into_iter().filter(|a| {
        !matches!(
            a.as_str(),
            "--drafts" | "--outbox" | "--signature" | "--html-reply" | "--accounts"
        )
    });
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
        imap_user: String::new(),
        smtp_user: String::new(),
        email: "seed@komorebi.invalid".into(),
        name: "Seed Account".into(),
        imap: server(993),
        smtp: server(465),
        save_sent: true,
        smtp_no_auth: false,
    })?;
    if signature {
        // Two named signatures: one for new mail, a short one for replies
        // and forwards.
        use komorebi::mail::rich::Doc;
        let long = db.add_signature(
            "Seed",
            &Doc::from_plain("Seed Account\nKomorebi test data"),
            None,
        )?;
        let short = db.add_signature("Seed (short)", &Doc::from_plain("Seed"), None)?;
        db.set_account_signatures(account, Some(long), Some(short))?;
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
                // Every 13th from the third newest: the sender set high
                // importance; every 13th from the eighth newest, low
                // (outside the random stream, so the rest of the cache
                // stays as it was).
                importance: match (count - uid) % 13 {
                    2 => Importance::High,
                    7 => Importance::Low,
                    _ => Importance::Normal,
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
        db.set_body(
            id,
            raw.as_bytes(),
            &mime::preview(&text),
            false,
            Default::default(),
        )?;
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
            db.set_body(
                id,
                &raw,
                &body.preview(),
                header.has_attach,
                Default::default(),
            )?;
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
        db.set_body(id, &raw, &body.preview(), false, Default::default())?;
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
    if accounts {
        add_accounts(&mut db, account)?;
    }
    if extra_folders > 0 {
        add_folders(&mut db, account, extra_folders)?;
    }
    if let Some(fold) = &fold {
        db.set_setting("nav_folded", fold)?;
    }
    if accounts || extra_folders > 0 || fold.is_some() {
        print_tree(&db)?;
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

/// Unread-count test mail: `n` messages in `folder`, the newest `unread`
/// of them unread.
fn folder_mail(
    db: &mut Db,
    (account, folder): (i64, i64),
    n: u32,
    unread: u32,
    to: &str,
    uid: &mut u32,
) -> Result<()> {
    let headers: Vec<NewHeader> = (0..n)
        .map(|i| {
            *uid += 1;
            NewHeader {
                uid: *uid,
                msgid: format!("<m{}@sidebar.komorebi.invalid>", *uid),
                subject: format!("Sidebar test message {}", *uid),
                from_name: "Test Sender".into(),
                from_addr: "sender@example.invalid".into(),
                to: to.into(),
                date: NEWEST - i64::from(i) * 3_600 - folder * 60,
                size: 3_000,
                seen: i >= unread,
                ..NewHeader::default()
            }
        })
        .collect();
    db.insert_headers(account, folder, &headers)
}

/// The last part of a folder name, as a sync shows it.
fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn depth(name: &str) -> i64 {
    name.matches('/').count() as i64
}

/// `--accounts`: nested folders for the seed account and two more
/// `.invalid` accounts.
fn add_accounts(db: &mut Db, seed: i64) -> Result<()> {
    const DESIGN: &str = "Projects/Alpha/Design reviews and long folder names 2026";
    let mut uid = 100_000;
    let seed_to = "seed@komorebi.invalid";
    let deep = |rest: &str| format!("{DESIGN}{rest}");
    let nested: [(String, u32, u32); 11] = [
        ("INBOX/Receipts".into(), 6, 2),
        ("Projects".into(), 4, 0),
        ("Projects/Alpha".into(), 8, 3),
        (DESIGN.into(), 5, 5),
        (deep("/Q1"), 2, 1),
        (deep("/Q1/Notes"), 2, 0),
        (deep("/Q1/Notes/Deep"), 1, 1),
        // Deeper than the level drawn.
        (deep("/Q1/Notes/Deep/Deeper"), 1, 0),
        ("Projects/Beta".into(), 3, 0),
        ("Projects-old".into(), 2, 0),
        ("Newsletters".into(), 40, 27),
    ];
    for (name, n, unread) in &nested {
        let f = db.upsert_folder(seed, name, leaf(name), depth(name), Role::Other)?;
        folder_mail(db, (seed, f), *n, *unread, seed_to, &mut uid)?;
    }
    for (name, role) in [
        ("Drafts", Role::Drafts),
        ("Junk", Role::Junk),
        ("Trash", Role::Trash),
    ] {
        let f = db.upsert_folder(seed, name, name, 0, role)?;
        folder_mail(db, (seed, f), 3, 1, seed_to, &mut uid)?;
    }
    let server = |host: &str, port| Server {
        host: host.into(),
        port,
        security: Security::Tls,
    };
    let new = |email: &str, name: &str| NewAccount {
        imap_user: String::new(),
        smtp_user: String::new(),
        email: email.into(),
        name: name.into(),
        imap: server("imap.mail.invalid", 993),
        smtp: server("smtp.mail.invalid", 465),
        save_sent: true,
        smtp_no_auth: false,
    };
    let maria = db.add_account(&new(
        "maria.goncalves.long-address-for-testing@purelymail-example.invalid",
        "Maria Gonçalves",
    ))?;
    let work = db.add_account(&new("work@postale.invalid", "Work"))?;
    let role = |name: &str| match name {
        "INBOX" => Role::Inbox,
        "Sent" => Role::Sent,
        "Drafts" => Role::Drafts,
        "Archive" => Role::Archive,
        "Junk" => Role::Junk,
        "Trash" => Role::Trash,
        _ => Role::Other,
    };
    let maria_folders: &[(&str, u32, u32)] = &[
        ("INBOX", 60, 37),
        ("Sent", 5, 0),
        ("Drafts", 2, 0),
        ("Archive", 10, 0),
        ("Junk", 9, 5),
        ("Trash", 4, 2),
        ("Clientes", 3, 0),
        ("Clientes/Associação Exemplo - contratos", 1300, 1234),
        ("Clientes/Porto", 4, 1),
        ("Faturas", 6, 0),
    ];
    let work_folders: &[(&str, u32, u32)] = &[
        ("INBOX", 12, 4),
        ("Sent", 3, 0),
        ("Archive", 3, 0),
        ("Trash", 1, 0),
        ("Team", 5, 2),
        ("Team/Standups", 4, 0),
    ];
    for (id, to, folders) in [
        (maria, "maria@example.invalid", maria_folders),
        (work, "work@postale.invalid", work_folders),
    ] {
        for &(name, n, unread) in folders {
            let role = role(name);
            let display = if role == Role::Inbox {
                "Inbox"
            } else {
                leaf(name)
            };
            let f = db.upsert_folder(id, name, display, depth(name), role)?;
            folder_mail(db, (id, f), n, unread, to, &mut uid)?;
        }
    }
    Ok(())
}

/// `--folders N`: N more folders for the seed account, depths 0 to 3
/// (each one under the folder before it a level up), every third with
/// unread mail.
fn add_folders(db: &mut Db, seed: i64, n: usize) -> Result<()> {
    let mut uid = 500_000;
    let mut path: Vec<String> = Vec::new();
    for k in 0..n {
        path.truncate(k % 4);
        let name = match path.last() {
            Some(parent) => format!("{parent}/Item {k:03}"),
            None => format!("Bulk {k:03}"),
        };
        let f = db.upsert_folder(seed, &name, leaf(&name), depth(&name), Role::Other)?;
        let unread = if k % 3 == 0 { 1 + (k % 5) as u32 } else { 0 };
        let to = "seed@komorebi.invalid";
        folder_mail(db, (seed, f), 2 + unread, unread, to, &mut uid)?;
        path.push(name);
    }
    Ok(())
}

/// The ids `--fold` takes: every account, and every folder with subfolders.
fn print_tree(db: &Db) -> Result<()> {
    let folders = db.folders()?;
    let tree = komorebi::tree::tree(&folders);
    for a in db.accounts()? {
        println!("a{}  {}", a.id, a.email);
        for node in tree.iter().filter(|n| n.has_children) {
            if let Some(f) = folders.get(node.index).filter(|f| f.account_id == a.id) {
                println!("  f{}  {}", f.id, f.name);
            }
        }
    }
    Ok(())
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
