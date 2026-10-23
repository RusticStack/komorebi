//! Servers that do less: the workers against [`FakeImapd`] with
//! capabilities taken away (no UIDPLUS, no MOVE, no SPECIAL-USE, a `.`
//! delimiter and an `INBOX.` prefix, modified UTF-7 names good and bad), and
//! against a server that fails: a changed certificate or a rejected sign-in
//! makes one attempt and then waits.
//!
//! Every profile syncs a small corpus, sees a flag change made elsewhere,
//! sets one, moves, deletes and saves a sent copy, and in none of them may a
//! bare `EXPUNGE` run (it would remove what another client flagged), a
//! failed COPY touch its source, or the data of a message be lost.

use super::fake_imapd::{FakeImapd, MailBox, Profile, sample};
use super::tests::{queued, worker_with};
use super::*;
use crate::db::NewAccount;
use crate::providers::Security;

/// How a server names its folders.
struct Style {
    boxes: fn() -> Vec<MailBox>,
    delimiter: &'static str,
    inbox: &'static str,
    sent: &'static str,
    trash: &'static str,
    archive: &'static str,
    junk: &'static str,
    /// A folder named in modified UTF-7, and what it shows as.
    utf7: (&'static str, &'static str),
    /// A name that is not valid modified UTF-7 (shown as it is).
    broken: &'static str,
}

fn filled(name: &str, attr: &'static str, first: u32, n: u32) -> MailBox {
    let mut b = MailBox::new(name, attr);
    for i in first..first + n {
        b.add(sample(i), &[]);
    }
    b
}

/// Folders as Courier and Cyrus name them: under `INBOX.`, no attributes.
fn dotted() -> Vec<MailBox> {
    let mut trash = filled("INBOX.Trash", "\\Trash", 20, 1);
    // Flagged \Deleted by another client; not ours to remove.
    trash.add(sample(21), &["\\Deleted"]);
    vec![
        filled("INBOX", "", 1, 4),
        filled("INBOX.Sent", "\\Sent", 0, 0),
        filled("INBOX.Drafts", "\\Drafts", 0, 0),
        trash,
        filled("INBOX.Archive", "\\Archive", 0, 0),
        filled("INBOX.Junk", "\\Junk", 0, 0),
        MailBox::header("INBOX.Projects"),
        filled("INBOX.Projects.Archive", "", 0, 0),
        filled("INBOX.Entw&APw-rfe", "", 30, 1),
        filled("INBOX.Broken&AB", "", 31, 1),
    ]
}

/// Plain top-level names with a `/` delimiter.
fn slash() -> Vec<MailBox> {
    let mut trash = filled("Trash", "\\Trash", 20, 1);
    trash.add(sample(21), &["\\Deleted"]);
    vec![
        filled("INBOX", "", 1, 4),
        filled("Sent Items", "\\Sent", 0, 0),
        filled("Drafts", "\\Drafts", 0, 0),
        trash,
        filled("Archive", "\\Archive", 0, 0),
        filled("Junk E-mail", "\\Junk", 0, 0),
        filled("Entw&APw-rfe", "", 30, 1),
        filled("Broken&AB", "", 31, 1),
    ]
}

const DOTTED: Style = Style {
    boxes: dotted,
    delimiter: ".",
    inbox: "INBOX",
    sent: "INBOX.Sent",
    trash: "INBOX.Trash",
    archive: "INBOX.Archive",
    junk: "INBOX.Junk",
    utf7: ("INBOX.Entw&APw-rfe", "Entwürfe"),
    broken: "INBOX.Broken&AB",
};

const SLASH: Style = Style {
    boxes: slash,
    delimiter: "/",
    inbox: "INBOX",
    sent: "Sent Items",
    trash: "Trash",
    archive: "Archive",
    junk: "Junk E-mail",
    utf7: ("Entw&APw-rfe", "Entwürfe"),
    broken: "Broken&AB",
};

fn run(profile: Profile, style: &Style) {
    let label = format!("{profile:?} {}", style.delimiter);
    let server = FakeImapd::start(profile, (style.boxes)());
    let mut db = Db::in_memory().unwrap();
    let host = Server {
        host: "imap.example.com".into(),
        port: 993,
        security: Security::Tls,
    };
    let account_id = db
        .add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "me@example.com".into(),
            name: "Me".into(),
            imap: host.clone(),
            smtp: host,
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap();
    let (mut w, events) = worker_with(&db, None);
    let addr = server.addr;
    w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
    w.smtp = Box::new(|_, _, _| Ok(()));
    let sync = |w: &mut Worker, db: &mut Db| {
        w.run_job(db, Job::SyncAll { full: true }, &mut || None);
    };
    let folder = |db: &Db, name: &str| {
        db.folders()
            .unwrap()
            .into_iter()
            .find(|f| f.account_id == account_id && f.name == name)
            .unwrap_or_else(|| panic!("{label}: no cached folder {name:?}"))
    };
    let cached = |db: &Db, name: &str| {
        let mut uids: Vec<u32> = db
            .known_uids(folder(db, name).id)
            .unwrap()
            .into_iter()
            .collect();
        uids.sort_unstable();
        uids
    };

    // ---- a full sync of the corpus: roles, names and messages
    sync(&mut w, &mut db);
    for (role, name) in [
        (Role::Inbox, style.inbox),
        (Role::Sent, style.sent),
        (Role::Trash, style.trash),
        (Role::Archive, style.archive),
        (Role::Junk, style.junk),
    ] {
        let f = db.folder_by_role(account_id, role).unwrap();
        assert_eq!(
            f.map(|f| f.name),
            Some(name.to_string()),
            "{label}: {role:?}"
        );
    }
    // The Archive nested in another folder is not the account's Archive.
    if style.delimiter == "." {
        assert_eq!(folder(&db, "INBOX.Projects.Archive").role, Role::Other);
    }
    assert_eq!(folder(&db, style.utf7.0).display, style.utf7.1, "{label}");
    assert_eq!(cached(&db, style.inbox), [1, 2, 3, 4], "{label}");
    // The message another client flagged \Deleted is hidden, not cached.
    assert_eq!(cached(&db, style.trash), [1], "{label}");
    // Odd names are selected as the server wrote them.
    assert_eq!(cached(&db, style.utf7.0), [1], "{label}");
    assert_eq!(cached(&db, style.broken), [1], "{label}");
    assert!(
        folder(&db, style.broken).display.contains("Broken&AB"),
        "{label}"
    );

    // ---- a flag changed elsewhere is found without CONDSTORE
    let inbox = folder(&db, style.inbox);
    server
        .state()
        .mailbox_mut(style.inbox)
        .msg_mut(2)
        .unwrap()
        .flags
        .insert("\\Flagged".into());
    sync(&mut w, &mut db);
    let ids = db.message_ids(&[inbox.id], "").unwrap();
    let flagged: Vec<u32> = db
        .briefs(&ids)
        .unwrap()
        .iter()
        .filter(|b| b.flagged)
        .map(|b| b.uid)
        .collect();
    assert_eq!(flagged, [2], "{label}");

    // ---- a flag set here reaches the server
    w.run_job(
        &mut db,
        Job::SetFlags {
            folder_id: inbox.id,
            uids: vec![3],
            flag: "\\Seen",
            on: true,
        },
        &mut || None,
    );
    assert!(
        server
            .state()
            .mailbox(style.inbox)
            .msg(3)
            .unwrap()
            .has("\\Seen"),
        "{label}"
    );

    // ---- a move
    let archive = folder(&db, style.archive);
    events.lock().unwrap().clear();
    w.run_job(
        &mut db,
        Job::Move {
            folder_id: inbox.id,
            uids: vec![1],
            dest: archive.id,
        },
        &mut || None,
    );
    {
        let st = server.state();
        let moved = st.mailbox(style.archive);
        assert_eq!(moved.uids().len(), 1, "{label}");
        assert!(
            String::from_utf8_lossy(&moved.msgs[0].raw).contains("<m1@example.com>"),
            "{label}"
        );
        let left = st.mailbox(style.inbox).msg(1);
        if profile.mv || profile.uidplus {
            assert!(left.is_none(), "{label}: the original is gone");
        } else {
            // Copied first; flagged only afterwards; never expunged.
            assert!(left.is_some_and(|m| m.has("\\Deleted")), "{label}");
        }
        assert_eq!(
            st.sent("UID MOVE").len(),
            usize::from(profile.mv),
            "{label}"
        );
        assert_eq!(
            st.sent("UID COPY").len(),
            usize::from(!profile.mv),
            "{label}"
        );
        assert_eq!(
            st.sent("UID EXPUNGE").len(),
            usize::from(!profile.mv && profile.uidplus),
            "{label}"
        );
    }
    let notices = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, Event::Notice { .. }))
        .count();
    assert_eq!(
        notices,
        usize::from(!profile.mv && !profile.uidplus),
        "{label}: only a COPY move that left the original flagged says so"
    );
    sync(&mut w, &mut db);
    assert_eq!(cached(&db, style.inbox), [2, 3, 4], "{label}");
    assert_eq!(cached(&db, style.archive).len(), 1, "{label}");

    // ---- a failed COPY (or MOVE) leaves the source alone
    let junk = folder(&db, style.junk);
    server.state().refuse_copy_to = Some(style.junk.to_string());
    events.lock().unwrap().clear();
    w.run_job(
        &mut db,
        Job::Move {
            folder_id: inbox.id,
            uids: vec![4],
            dest: junk.id,
        },
        &mut || None,
    );
    {
        let st = server.state();
        let kept = st.mailbox(style.inbox).msg(4).expect("the source stays");
        assert!(
            !kept.has("\\Deleted"),
            "{label}: not flagged after a failed copy"
        );
        assert!(st.mailbox(style.junk).msgs.is_empty(), "{label}");
    }
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::ActionFailed { .. })),
        "{label}"
    );
    server.state().refuse_copy_to = None;

    // ---- a delete in Trash; what another client flagged there stays
    let trash = folder(&db, style.trash);
    w.run_job(
        &mut db,
        Job::Delete {
            folder_id: trash.id,
            uids: vec![1],
        },
        &mut || None,
    );
    {
        let st = server.state();
        let left = &st.mailbox(style.trash);
        let uids = left.uids();
        if profile.uidplus {
            assert_eq!(uids, [2], "{label}: only the chosen message was expunged");
        } else {
            assert_eq!(uids, [1, 2], "{label}: flagged, not expunged");
            assert!(left.msg(1).unwrap().has("\\Deleted"), "{label}");
        }
        assert!(left.msg(2).unwrap().has("\\Deleted"), "{label}");
    }
    sync(&mut w, &mut db);
    assert!(cached(&db, style.trash).is_empty(), "{label}");

    // ---- a sent copy is appended and found again by Message-ID, not by
    // guessing its UID
    let row = queued(&db, account_id, "hello");
    events.lock().unwrap().clear();
    w.run_job(&mut db, Job::Send { row }, &mut || None);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Sent { outcome, .. } if *outcome == Outcome::Delivered)),
        "{label}: {:?}",
        events.lock().unwrap()
    );
    {
        let st = server.state();
        let sent = st.mailbox(style.sent);
        assert_eq!(sent.msgs.len(), 1, "{label}");
        assert!(sent.msgs[0].has("\\Seen"), "{label}");
        assert!(
            String::from_utf8_lossy(&sent.msgs[0].raw).contains("<m1@example.com>"),
            "{label}"
        );
    }
    let sent = folder(&db, style.sent);
    w.run_job(
        &mut db,
        Job::SyncFolder {
            id: sent.id,
            full: true,
        },
        &mut || None,
    );
    assert_eq!(cached(&db, style.sent), [1], "{label}");

    // ---- through all of it
    assert_eq!(server.state().bare_expunges, 0, "{label}: no bare EXPUNGE");
    let st = server.state();
    assert_eq!(st.sent("LOGIN").len(), 1, "{label}: one sign-in");
    drop(st);
    if let Some(imap) = w.imap.take() {
        imap.logout();
    }
}

#[test]
fn servers_without_extensions_sync_move_delete_and_save_sent_copies() {
    for style in [&SLASH, &DOTTED] {
        for mv in [false, true] {
            for uidplus in [false, true] {
                run(
                    Profile {
                        mv,
                        uidplus,
                        delimiter: style.delimiter,
                        ..Profile::BARE
                    },
                    style,
                );
            }
        }
    }
}

#[test]
fn a_server_with_everything_behaves_the_same() {
    run(
        Profile::FULL,
        &Style {
            delimiter: "/",
            ..SLASH
        },
    );
}
