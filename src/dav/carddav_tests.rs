//! CardDAV sync against the fake server (`dav::fake`): never a real one.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::dav::fake::{self, PASS, USER, card};
use crate::db::contacts::{BOOK_READ_ONLY, DavAccount};
use crate::mail::fake_tls::CERT;

fn setup(mode: i64) -> (fake::FakeDav, Db, i64, Client) {
    let dav = fake::start();
    let db = Db::in_memory().unwrap();
    let s = crate::providers::Server {
        host: "imap.example.com".into(),
        port: 993,
        security: crate::providers::Security::Tls,
    };
    let account = db
        .add_account(&crate::db::NewAccount {
            email: "ana@example.com".into(),
            name: "Ana".into(),
            imap: s.clone(),
            smtp: s,
            save_sent: true,
            smtp_no_auth: false,
            imap_user: String::new(),
            smtp_user: String::new(),
        })
        .unwrap();
    db.set_dav_account(&DavAccount {
        account_id: account,
        url: dav.base(),
        user: USER.into(),
        mode,
        interval_min: 30,
        ..DavAccount::default()
    })
    .unwrap();
    let client = Client::for_test(&dav.base(), Some((USER, PASS)), CERT);
    (dav, db, account, client)
}

fn books(db: &Db, account: i64) -> Vec<Book> {
    db.address_books()
        .unwrap()
        .into_iter()
        .filter(|b| b.account_id == Some(account))
        .collect()
}

fn sync_all(db: &Db, account: i64, client: &Client) -> Outcome {
    let mut total = Outcome::default();
    for b in books(db, account) {
        let o = sync_book(client, db, &b).unwrap();
        total.changed |= o.changed;
        total.skipped += o.skipped;
        total.conflicts += o.conflicts;
        total.pushed += o.pushed;
    }
    total
}

#[test]
fn finds_books_and_syncs_read_only_with_tokens_and_ctags() {
    let (dav, db, account, client) = setup(BOOK_READ_ONLY);
    {
        let mut st = dav.state.lock().unwrap();
        st.put(
            "/dav/books/a/",
            "1.vcf",
            &card("u1", "Rui Alves", "rui@example.com"),
        );
        st.put(
            "/dav/books/a/",
            "2.vcf",
            &card("u2", "Sara Lima", "sara@example.com"),
        );
        st.put(
            "/dav/books/b/",
            "3.vcf",
            &card("u3", "Tia Rosa", "rosa@example.com"),
        );
        st.put("/dav/books/b/", "bad.vcf", "not a card");
    }
    // The well-known redirect is read without credentials.
    let anon = Client::for_test(&dav.base(), None, CERT);
    let start = discover::first_hop(&anon, "/.well-known/carddav").unwrap();
    assert!(start.ends_with("/dav/"), "{start}");
    let found = find_books(&client, &start).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].1, "Work");
    db.set_found_books(account, BOOK_READ_ONLY, &found).unwrap();
    // The fake lists sync-collection for book a only.
    let a = books(&db, account)
        .into_iter()
        .find(|b| b.name == "Work")
        .unwrap();
    db.set_book_state(a.id, None, None, true, None).unwrap();

    let o = sync_all(&db, account, &client);
    assert!(o.changed);
    assert_eq!(o.skipped, 1);
    assert_eq!(db.synced_contact_count(account).unwrap(), 3);

    // A delta: one added, one changed, one removed in each book.
    {
        let mut st = dav.state.lock().unwrap();
        st.put(
            "/dav/books/a/",
            "4.vcf",
            &card("u4", "Vera", "vera@example.com"),
        );
        st.put(
            "/dav/books/a/",
            "1.vcf",
            &card("u1", "Rui Alves Costa", "rui@example.com"),
        );
        st.remove("/dav/books/a/", "/dav/books/a/2.vcf");
        st.remove("/dav/books/b/", "/dav/books/b/3.vcf");
    }
    sync_all(&db, account, &client);
    assert_eq!(db.synced_contact_count(account).unwrap(), 2);
    assert_eq!(db.contact_ids(None, None, "costa").unwrap().len(), 1);

    // An expired token: a full resync, no duplicates.
    dav.state.lock().unwrap().expire_token = true;
    sync_all(&db, account, &client);
    assert_eq!(db.synced_contact_count(account).unwrap(), 2);

    // Truncated (507) answers continue with the new token.
    {
        let mut st = dav.state.lock().unwrap();
        st.truncate = true;
        st.put(
            "/dav/books/a/",
            "5.vcf",
            &card("u5", "Xana", "xana@example.com"),
        );
        st.put(
            "/dav/books/a/",
            "6.vcf",
            &card("u6", "Zeca", "zeca@example.com"),
        );
    }
    sync_all(&db, account, &client);
    assert_eq!(db.synced_contact_count(account).unwrap(), 4);

    // Read only: never a write; credentials on every request but the
    // well-known one.
    assert_eq!(dav.writes(), 0);
    let log = dav.state.lock().unwrap().log.clone();
    for (_, p, auth) in &log {
        assert_eq!(*auth, !p.starts_with("/.well-known"), "{p}");
    }
}

#[test]
fn wrong_password_and_foreign_redirects() {
    let (dav, _db, _account, _client) = setup(BOOK_READ_ONLY);
    let bad = Client::for_test(&dav.base(), Some((USER, "nope")), CERT);
    assert_eq!(find_books(&bad, "/dav/").unwrap_err(), DavError::Auth);
    let client = Client::for_test(&dav.base(), Some((USER, PASS)), CERT);
    assert_eq!(
        client
            .propfind("/foreign", "0", crate::dav::xml::PROPFIND_PRINCIPAL)
            .unwrap_err(),
        DavError::AuthRedirect
    );
    assert_eq!(
        client.resolve("https://localhost:1/x").unwrap_err(),
        DavError::AuthRedirect
    );
    // Nothing followed the redirect to the other origin.
    let log = dav.state.lock().unwrap().log.clone();
    // Only the refused sign-in reached /dav/; the redirect was not followed.
    assert_eq!(log.iter().filter(|(_, p, _)| p == "/dav/").count(), 1);
}

#[test]
fn read_write_round_trip_and_conflicts() {
    let (dav, db, account, client) = setup(BOOK_READ_WRITE);
    dav.state.lock().unwrap().put(
        "/dav/books/a/",
        "1.vcf",
        &card("u1", "Rui", "rui@example.com"),
    );
    let found = find_books(&client, "/dav/").unwrap();
    db.set_found_books(account, BOOK_READ_WRITE, &found[..1])
        .unwrap();
    sync_all(&db, account, &client);
    let book = books(&db, account)[0].clone();

    // Create.
    let mut c = Contact {
        book_id: book.id,
        name: "Nova".into(),
        emails: vec![crate::contacts::Email {
            addr: "nova@example.com".into(),
            ..Default::default()
        }],
        ..Contact::default()
    };
    db.save_contact(&mut c).unwrap();
    assert_eq!(sync_all(&db, account, &client).pushed, 1);
    assert_eq!(
        dav.state.lock().unwrap().books["/dav/books/a/"].cards.len(),
        2
    );

    // Edit keeps unknown lines byte for byte.
    let rui = db.contact_ids(None, None, "rui").unwrap()[0];
    let mut r = db.contact(rui).unwrap().unwrap();
    r.title = "Chefe".into();
    db.save_contact(&mut r).unwrap();
    sync_all(&db, account, &client);
    let stored = dav.state.lock().unwrap().books["/dav/books/a/"].cards["/dav/books/a/1.vcf"]
        .1
        .clone();
    assert!(
        stored.contains("X-KEEP:yes") && stored.contains("TITLE:Chefe"),
        "{stored}"
    );

    // A disjoint edit against a server change merges and pushes.
    let mut r = db.contact(rui).unwrap().unwrap();
    r.org = "Mutual".into();
    db.save_contact(&mut r).unwrap();
    let theirs = stored.replace("FN:Rui", "FN:Rui Server");
    dav.state
        .lock()
        .unwrap()
        .put("/dav/books/a/", "1.vcf", &theirs);
    let o = sync_all(&db, account, &client);
    assert_eq!(o.conflicts, 0);
    let merged = db.contact(rui).unwrap().unwrap();
    assert_eq!(
        (merged.name.as_str(), merged.org.as_str()),
        ("Rui Server", "Mutual")
    );
    let stored = dav.state.lock().unwrap().books["/dav/books/a/"].cards["/dav/books/a/1.vcf"]
        .1
        .clone();
    assert!(
        stored.contains("ORG:Mutual") && stored.contains("FN:Rui Server"),
        "{stored}"
    );

    // An overlapping edit: the server wins and ours is kept.
    let mut r = db.contact(rui).unwrap().unwrap();
    r.title = "Mine".into();
    db.save_contact(&mut r).unwrap();
    let theirs = stored.replace("TITLE:Chefe", "TITLE:Theirs");
    dav.state
        .lock()
        .unwrap()
        .put("/dav/books/a/", "1.vcf", &theirs);
    let o = sync_all(&db, account, &client);
    assert_eq!(o.conflicts, 1);
    assert_eq!(db.contact(rui).unwrap().unwrap().title, "Theirs");
    let mine = db.contact_conflict(rui).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&mine).contains("TITLE:Mine"));

    // Delete.
    db.delete_contact(rui).unwrap();
    sync_all(&db, account, &client);
    assert!(
        !dav.state.lock().unwrap().books["/dav/books/a/"]
            .cards
            .contains_key("/dav/books/a/1.vcf")
    );
    // The fake refuses writes without the right If-Match / If-None-Match.
    assert!(dav.writes() >= 4);
}
