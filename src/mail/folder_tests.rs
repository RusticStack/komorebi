//! Folder actions against [`FakeImapd`]: create (at the top, nested, under
//! a namespace), rename (the cached mail and the rules follow), delete (the
//! subfolders first), a refusal in the server's words, a name the server
//! can't take, special folders, and an action cut off from the server that
//! waits for the next connection.

use super::fake_imapd::{FakeImapd, MailBox, Profile, sample};
use super::tests::worker_with;
use super::*;
use crate::db::NewAccount;
use crate::providers::Security;

struct Rig {
    server: FakeImapd,
    db: Db,
    w: Worker,
    events: Arc<Mutex<Vec<Event>>>,
    account: i64,
}

fn rig(profile: Profile, boxes: Vec<MailBox>) -> Rig {
    let server = FakeImapd::start(profile, boxes);
    let mut db = Db::in_memory().unwrap();
    let host = Server {
        host: "imap.example.com".into(),
        port: 993,
        security: Security::Tls,
    };
    let account = db
        .add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "me@example.com".into(),
            name: "Me".into(),
            imap: host.clone(),
            smtp: host,
            save_sent: false,
            smtp_no_auth: false,
        })
        .unwrap();
    let (mut w, events) = worker_with(&db, None);
    let addr = server.addr;
    w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
    w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
    events.lock().unwrap().clear();
    Rig {
        server,
        db,
        w,
        events,
        account,
    }
}

fn boxes() -> Vec<MailBox> {
    let mut work = MailBox::new("Work", "");
    work.add(sample(1), &[]);
    let mut year = MailBox::new("Work/2024", "");
    year.uidvalidity = 9;
    year.add(sample(2), &[]);
    vec![
        MailBox::new("INBOX", ""),
        MailBox::new("Sent", "\\Sent"),
        work,
        year,
    ]
}

impl Rig {
    fn run(&mut self, job: Job) {
        self.w.run_job(&mut self.db, job, &mut || None);
    }

    fn folder(&self, name: &str) -> Option<Folder> {
        self.db
            .folders()
            .unwrap()
            .into_iter()
            .find(|f| f.account_id == self.account && f.name == name)
    }

    fn id(&self, name: &str) -> i64 {
        self.folder(name)
            .unwrap_or_else(|| panic!("no cached folder {name:?}"))
            .id
    }

    /// The folder actions' ends told so far, as (op, name, outcome).
    fn done(&self) -> Vec<(FolderOp, String, FolderOutcome)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::FolderDone {
                    op, name, outcome, ..
                } => Some((*op, name.clone(), outcome.clone())),
                _ => None,
            })
            .collect()
    }

    fn sent(&self, verb: &str) -> Vec<String> {
        self.server
            .state()
            .sent(verb)
            .into_iter()
            .map(|l| l.split_once(' ').map_or(l, |(_, rest)| rest).to_string())
            .collect()
    }
}

#[test]
fn a_new_folder_is_created_subscribed_and_listed() {
    let mut r = rig(Profile::FULL, boxes());
    r.run(Job::CreateFolder {
        parent: None,
        name: " Receipts ".into(),
    });
    assert_eq!(r.sent("CREATE"), vec![r#"CREATE "Receipts""#]);
    assert_eq!(r.sent("SUBSCRIBE"), vec![r#"SUBSCRIBE "Receipts""#]);
    assert!(r.server.state().subscribed.contains("Receipts"));
    // Listed again at once: the pane shows it without waiting for a sync.
    assert!(r.folder("Receipts").is_some());
    assert_eq!(
        r.done(),
        vec![(FolderOp::Create, "Receipts".into(), FolderOutcome::Done)]
    );
    assert!(
        r.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::FolderChanged { folder_id: None }))
    );
}

#[test]
fn a_subfolder_takes_the_servers_delimiter_and_unicode_names_are_encoded() {
    let mut r = rig(Profile::FULL, boxes());
    let work = r.id("Work");
    r.run(Job::CreateFolder {
        parent: Some(work),
        name: "Café".into(),
    });
    assert_eq!(r.sent("CREATE"), vec![r#"CREATE "Work/Caf&AOk-""#]);
    let made = r.folder("Work/Caf&AOk-").expect("the subfolder is cached");
    assert_eq!(made.display, "Café");
    assert_eq!(made.depth, 1);

    // A dotted server whose folders are all under INBOX. (Courier): new
    // top-level folders go there too, and a dot in the name is refused
    // before anything is sent.
    let dotted = Profile {
        delimiter: ".",
        ..Profile::FULL
    };
    let mut r = rig(
        dotted,
        vec![
            MailBox::new("INBOX", ""),
            MailBox::new("INBOX.Sent", "\\Sent"),
            MailBox::new("INBOX.Work", ""),
        ],
    );
    r.run(Job::CreateFolder {
        parent: None,
        name: "Projects".into(),
    });
    assert_eq!(r.sent("CREATE"), vec![r#"CREATE "INBOX.Projects""#]);
    assert!(r.folder("INBOX.Projects").is_some());
    r.run(Job::CreateFolder {
        parent: None,
        name: "v1.2".into(),
    });
    assert_eq!(r.sent("CREATE").len(), 1, "nothing sent for a bad name");
    assert_eq!(
        r.done().last().map(|d| d.2.clone()),
        Some(FolderOutcome::BadName(folders::BadName::Delimiter(
            ".".into()
        )))
    );
}

#[test]
fn a_rename_keeps_the_cached_mail_the_subfolders_and_the_rules() {
    let mut r = rig(Profile::FULL, boxes());
    let (work, year) = (r.id("Work"), r.id("Work/2024"));
    let mail = r.db.message_ids(&[work], "").unwrap();
    assert_eq!(mail.len(), 1);
    r.db.conn()
        .execute(
            "INSERT INTO rules (account_id, position, name, move_to) VALUES (?1, 0, 'r', 'Work/2024')",
            [r.account],
        )
        .unwrap();
    r.run(Job::RenameFolder {
        folder_id: work,
        name: "Jobs".into(),
    });
    assert_eq!(r.sent("RENAME"), vec![r#"RENAME "Work" "Jobs""#]);
    assert_eq!(r.sent("UNSUBSCRIBE"), vec![r#"UNSUBSCRIBE "Work""#]);
    // The same rows under the new names: nothing to download again.
    assert_eq!(r.id("Jobs"), work);
    assert_eq!(r.id("Jobs/2024"), year);
    assert!(r.folder("Work").is_none());
    assert_eq!(r.db.message_ids(&[work], "").unwrap(), mail);
    let dest: String =
        r.db.conn()
            .query_row("SELECT move_to FROM rules", [], |row| row.get(0))
            .unwrap();
    assert_eq!(dest, "Jobs/2024");
    assert_eq!(
        r.done(),
        vec![(FolderOp::Rename, "Jobs".into(), FolderOutcome::Done)]
    );
}

#[test]
fn a_delete_takes_the_subfolders_first() {
    let mut r = rig(Profile::FULL, boxes());
    let work = r.id("Work");
    r.run(Job::DeleteFolder { folder_id: work });
    assert_eq!(
        r.sent("DELETE"),
        vec![r#"DELETE "Work/2024""#, r#"DELETE "Work""#]
    );
    assert!(r.folder("Work").is_none() && r.folder("Work/2024").is_none());
    assert!(r.db.message_ids(&[work], "").unwrap().is_empty());
    assert_eq!(
        r.done(),
        vec![(FolderOp::Delete, "Work".into(), FolderOutcome::Done)]
    );
}

#[test]
fn a_refusal_is_said_in_the_servers_words_and_special_folders_are_kept() {
    let mut r = rig(Profile::FULL, boxes());
    r.server.state().refuse_folders = Some("[CANNOT] Folder limit reached".into());
    r.run(Job::CreateFolder {
        parent: None,
        name: "More".into(),
    });
    assert_eq!(
        r.done(),
        vec![(
            FolderOp::Create,
            "More".into(),
            FolderOutcome::Refused("Folder limit reached".into())
        )]
    );
    assert!(r.folder("More").is_none());

    // The Sent folder is never renamed or deleted, whatever is asked.
    r.server.state().refuse_folders = None;
    let sent = r.id("Sent");
    r.run(Job::DeleteFolder { folder_id: sent });
    r.run(Job::RenameFolder {
        folder_id: sent,
        name: "Old".into(),
    });
    assert!(r.sent("DELETE").is_empty() && r.sent("RENAME").is_empty());
    assert!(r.folder("Sent").is_some());
    assert!(
        r.done()[1..]
            .iter()
            .all(|d| matches!(d.2, FolderOutcome::Failed(_)))
    );
}

#[test]
fn a_folder_action_out_of_reach_waits_for_the_next_connection() {
    let mut r = rig(Profile::FULL, boxes());
    let addr = r.server.addr;
    let online = Arc::new(Mutex::new(false));
    let on = online.clone();
    r.w.imap = None;
    r.w.connect = Box::new(move |_| {
        if *on.lock().unwrap() {
            Imap::connect_plain(addr, "user", "pass")
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into())
        }
    });
    r.run(Job::CreateFolder {
        parent: None,
        name: "Later".into(),
    });
    assert_eq!(
        r.done(),
        vec![(FolderOp::Create, "Later".into(), FolderOutcome::Waiting)]
    );
    assert_eq!(r.w.parked.len(), 1);
    assert!(r.sent("CREATE").is_empty());

    *online.lock().unwrap() = true;
    r.w.retry_parked(&mut r.db);
    assert!(r.w.parked.is_empty());
    assert_eq!(r.sent("CREATE"), vec![r#"CREATE "Later""#]);
    assert!(r.folder("Later").is_some());
    assert_eq!(
        r.done().last().cloned(),
        Some((FolderOp::Create, "Later".into(), FolderOutcome::Done))
    );
}
