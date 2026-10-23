//! CalDAV sync against the fake server (`dav::fake`): never a real one.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::dav::fake::{self, PASS, USER};
use crate::mail::fake_tls::CERT;

const NOW: i64 = 1_790_000_000; // 2026-09-21
const FROM: i64 = NOW - 86_400;
const TO: i64 = NOW + 60 * 86_400;

fn event(uid: &str, day: u32, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\n\
         DTSTART:202610{day:02}T090000Z\r\nDTEND:202610{day:02}T100000Z\r\nSUMMARY:{summary}\r\n\
         X-KEEP:yes\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn setup() -> (fake::FakeDav, Db, i64, Client) {
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
    let client = Client::for_test(&dav.base(), Some((USER, PASS)), CERT);
    (dav, db, account, client)
}

fn sync_all(db: &Db, account: i64, client: &Client) -> Outcome {
    let mut total = Outcome::default();
    for c in db.dav_calendars(account).unwrap() {
        if !c.enabled {
            continue;
        }
        let o = sync_calendar(client, db, &c, "ana@example.com", NOW).unwrap();
        total.changed |= o.changed;
        total.skipped += o.skipped;
    }
    total
}

fn titles(db: &Db) -> Vec<String> {
    let mut t: Vec<String> = db
        .agenda(FROM, TO)
        .unwrap()
        .into_iter()
        .map(|r| r.summary)
        .collect();
    t.sort();
    t
}

fn reports(dav: &fake::FakeDav, path: &str) -> usize {
    dav.state
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|(m, p, _)| m == "REPORT" && p == path)
        .count()
}

#[test]
fn finds_event_calendars_only() {
    let (_dav, db, account, client) = setup();
    let found = find_calendars(&client, "/dav/").unwrap();
    let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Work", "Home"], "the task list is hidden");
    assert!(found[0].sync_collection);
    assert!(!found[1].sync_collection);
    // A calendar's own address is used as it is.
    let one = find_calendars(&client, "/dav/cals/c/").unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].href, "/dav/cals/c/");
    db.set_found_calendars(account, &found).unwrap();
    let cals = db.dav_calendars(account).unwrap();
    assert_eq!(cals.len(), 2);
    assert_eq!(cals[0].sync_token.as_deref(), Some(""));
    assert_eq!(cals[1].sync_token, None);
    // Finding again keeps them; one no longer found goes.
    db.set_found_calendars(account, &found[..1]).unwrap();
    assert_eq!(db.dav_calendars(account).unwrap().len(), 1);
    assert_eq!(db.dav_calendars(account).unwrap()[0].id, cals[0].id);
}

#[test]
fn syncs_with_tokens_and_ctags() {
    let (dav, db, account, client) = setup();
    {
        let mut st = dav.state.lock().unwrap();
        st.put("/dav/cals/w/", "1.ics", &event("u1", 1, "Review"));
        st.put("/dav/cals/w/", "a%20b.ics", &event("u2", 2, "Lunch"));
        st.put("/dav/cals/c/", "3.ics", &event("u3", 3, "Dentist"));
    }
    let found = find_calendars(&client, "/dav/").unwrap();
    db.set_found_calendars(account, &found).unwrap();
    let o = sync_all(&db, account, &client);
    assert!(o.changed);
    assert_eq!(titles(&db), ["Dentist", "Lunch", "Review"]);
    let cals = db.dav_calendars(account).unwrap();
    assert!(cals[0].sync_token.as_deref().is_some_and(|t| !t.is_empty()));
    assert!(cals[1].ctag.is_some());
    // The raw resource is kept as the server sent it.
    let (id, _) = db.event_by_uid("u1").unwrap().unwrap();
    let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
    assert!(ics.contains("X-KEEP:yes"));

    // Nothing changed: no data fetched again, the ctag calendar skipped.
    let before = reports(&dav, "/dav/cals/c/");
    let o = sync_all(&db, account, &client);
    assert!(!o.changed);
    assert_eq!(reports(&dav, "/dav/cals/c/"), before);

    // An edit and a removal on the server.
    {
        let mut st = dav.state.lock().unwrap();
        st.put("/dav/cals/w/", "1.ics", &event("u1", 1, "Review moved"));
        st.remove("/dav/cals/w/", "/dav/cals/w/a%20b.ics");
        st.put("/dav/cals/c/", "3.ics", &event("u3", 3, "Dentist 2"));
        st.put("/dav/cals/c/", "4.ics", &event("u4", 4, "Gym"));
    }
    assert!(sync_all(&db, account, &client).changed);
    assert_eq!(titles(&db), ["Dentist 2", "Gym", "Review moved"]);

    // A disabled calendar's events are hidden.
    db.set_calendar_enabled(cals[1].id, false).unwrap();
    assert_eq!(titles(&db), ["Review moved"]);
    // Credentials went only to the server's origin.
    assert!(
        dav.state
            .lock()
            .unwrap()
            .log
            .iter()
            .all(|(_, _, auth)| *auth)
    );
}

#[test]
fn expired_token_and_truncation_recover() {
    let (dav, db, account, client) = setup();
    {
        let mut st = dav.state.lock().unwrap();
        for i in 1..=4 {
            st.put(
                "/dav/cals/w/",
                &format!("{i}.ics"),
                &event(&format!("u{i}"), i, &format!("E{i}")),
            );
        }
        st.truncate = true;
    }
    let found = find_calendars(&client, "/dav/").unwrap();
    db.set_found_calendars(account, &found[..1]).unwrap();
    sync_all(&db, account, &client);
    assert_eq!(titles(&db), ["E1", "E2", "E3", "E4"]);
    {
        let mut st = dav.state.lock().unwrap();
        st.truncate = false;
        st.expire_token = true;
        st.remove("/dav/cals/w/", "/dav/cals/w/2.ics");
    }
    assert!(sync_all(&db, account, &client).changed);
    assert_eq!(titles(&db), ["E1", "E3", "E4"]);
}

#[test]
fn hostile_and_oversized_resources_are_skipped() {
    let (dav, db, account, client) = setup();
    {
        let mut st = dav.state.lock().unwrap();
        st.put("/dav/cals/w/", "ok.ics", &event("ok", 1, "Fine"));
        st.put(
            "/dav/cals/w/",
            "bad.ics",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:9",
        );
        let big = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:big\r\nDTSTART:20261005T090000Z\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            "x".repeat(300 * 1024)
        );
        st.put("/dav/cals/w/", "big.ics", &big);
        let deep = "BEGIN:VCALENDAR\r\n".repeat(5_000);
        st.put("/dav/cals/w/", "deep.ics", &deep);
    }
    let found = find_calendars(&client, "/dav/").unwrap();
    db.set_found_calendars(account, &found[..1]).unwrap();
    let o = sync_all(&db, account, &client);
    assert_eq!(o.skipped, 3);
    assert_eq!(titles(&db), ["Fine"]);
    // Kept as "too large" rows: not fetched again while unchanged.
    let before = reports(&dav, "/dav/cals/w/");
    sync_all(&db, account, &client);
    assert_eq!(
        reports(&dav, "/dav/cals/w/"),
        before + 1,
        "only the sync report"
    );
}

#[test]
fn the_calendar_address_leaves_the_contacts_one_alone() {
    let (_dav, db, account, _client) = setup();
    assert_eq!(db.cal_home(account).unwrap(), None);
    db.set_dav_account(&crate::db::contacts::DavAccount {
        account_id: account,
        url: "https://carddav.example.com/".into(),
        user: USER.into(),
        mode: 1,
        interval_min: 30,
        ..Default::default()
    })
    .unwrap();
    db.set_cal_home(account, "https://caldav.example.com/")
        .unwrap();
    assert_eq!(
        db.cal_home(account).unwrap().as_deref(),
        Some("https://caldav.example.com/")
    );
    let d = db.dav_account(account).unwrap().unwrap();
    assert_eq!(d.url, "https://carddav.example.com/");
    assert_eq!(d.user, USER);
    assert_eq!(d.mode, 1);
}

#[test]
fn wrong_password_and_foreign_redirect() {
    let (dav, _db, _account, _client) = setup();
    let bad = Client::for_test(&dav.base(), Some((USER, "nope")), CERT);
    assert_eq!(find_calendars(&bad, "/dav/").unwrap_err(), DavError::Auth);
    let client = Client::for_test(&dav.base(), Some((USER, PASS)), CERT);
    assert_eq!(
        find_calendars(&client, "/foreign").unwrap_err(),
        DavError::AuthRedirect
    );
    // Nothing reached the other origin: two requests, both here.
    assert_eq!(dav.state.lock().unwrap().log.len(), 2);
}

// ---- Writes: the queue against the fake server --------------------------

fn pass(db: &Db, account: i64, client: &Client) -> Outcome {
    let mut total = Outcome::default();
    for c in db.dav_calendars(account).unwrap() {
        let o = flush_and_pull(client, db, account, &c, "ana@example.com", NOW).unwrap();
        total.conflicts += o.conflicts;
        total.refused += o.refused;
    }
    total
}

fn server_body(dav: &fake::FakeDav, href: &str) -> Option<String> {
    let st = dav.state.lock().unwrap();
    st.books["/dav/cals/w/"]
        .cards
        .get(href)
        .map(|c| c.1.clone())
}

fn headers_of(dav: &fake::FakeDav, method: &str) -> usize {
    dav.state
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|(m, _, _)| m == method)
        .count()
}

fn one_calendar(dav: &fake::FakeDav, db: &Db, account: i64, client: &Client) -> i64 {
    dav.state
        .lock()
        .unwrap()
        .put("/dav/cals/w/", "1.ics", &event("u1", 1, "Review"));
    let found = find_calendars(client, "/dav/").unwrap();
    db.set_found_calendars(account, &found[..1]).unwrap();
    sync_all(db, account, client);
    db.dav_calendars(account).unwrap()[0].id
}

#[test]
fn create_edit_and_delete_are_sent_with_preconditions() {
    let (dav, db, account, client) = setup();
    let cal = one_calendar(&dav, &db, account, &client);
    // A new event: shown at once, queued, then PUT with If-None-Match.
    let raw = event("new-1", 5, "Dinner");
    let id = db
        .write_event(cal, None, &raw, "ana@example.com", NOW)
        .unwrap();
    assert_eq!(titles(&db), ["Dinner", "Review"]);
    assert_eq!(db.queued(cal).unwrap().len(), 1);
    assert_eq!(db.queued(cal).unwrap()[0].if_match, None);
    // A second edit before the pass replaces the queued body.
    let raw2 = event("new-1", 5, "Dinner at 8");
    db.write_event(cal, Some(id), &raw2, "ana@example.com", NOW)
        .unwrap();
    assert_eq!(db.queued(cal).unwrap().len(), 1);
    pass(&db, account, &client);
    assert!(db.queued(cal).unwrap().is_empty());
    let href = "/dav/cals/w/new-1.ics";
    assert!(server_body(&dav, href).unwrap().contains("Dinner at 8"));
    assert_eq!(headers_of(&dav, "PUT"), 1);
    // Edit the server's own event: If-Match on the ETag it was synced at;
    // unknown properties stay.
    let (rid, _) = db.event_by_uid("u1").unwrap().unwrap();
    let info = db.event_info(rid).unwrap().unwrap();
    assert!(info.mine && info.can_write);
    let e = crate::cal::edit::edit(
        &crate::cal::edit::Form {
            summary: "Review, moved".into(),
            location: "Room 4".into(),
            description: String::new(),
            all_day: false,
            start: chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
                .unwrap()
                .and_hms_opt(11, 0, 0)
                .unwrap(),
            end: chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
            rrule: String::new(),
            alarm_min: None,
        },
        "u1",
        chrono::DateTime::from_timestamp(NOW, 0).unwrap(),
        &|w| Some(w.and_utc().timestamp()),
    )
    .unwrap();
    let patched = crate::mail::ical::patch(&info.ics, &e).unwrap();
    db.write_event(cal, Some(rid), &patched, "ana@example.com", NOW)
        .unwrap();
    let q = db.queued(cal).unwrap();
    assert!(
        q[0].if_match
            .as_deref()
            .is_some_and(|m| m.starts_with("\"e"))
    );
    pass(&db, account, &client);
    let body = server_body(&dav, "/dav/cals/w/1.ics").unwrap();
    assert!(body.contains("SUMMARY:Review\\, moved"), "{body}");
    assert!(body.contains("X-KEEP:yes"));
    assert!(body.contains("SEQUENCE:1"));
    assert_eq!(titles(&db), ["Dinner at 8", "Review, moved"]);
    // Delete: DELETE with If-Match; gone here at once and there after.
    db.delete_event(id, false, NOW).unwrap();
    assert_eq!(titles(&db), ["Review, moved"]);
    pass(&db, account, &client);
    assert!(server_body(&dav, href).is_none());
    assert_eq!(headers_of(&dav, "DELETE"), 1);
    // A new event deleted before it was sent sends nothing at all.
    let id = db
        .write_event(cal, None, &event("gone", 6, "Never"), "", NOW)
        .unwrap();
    db.delete_event(id, false, NOW).unwrap();
    assert!(db.queued(cal).unwrap().is_empty());
    assert_eq!(db.unsent_changes(None).unwrap(), 0);
}

#[test]
fn a_conflict_is_never_overwritten_and_can_be_settled_either_way() {
    for again in [false, true] {
        let (dav, db, account, client) = setup();
        let cal = one_calendar(&dav, &db, account, &client);
        let (id, _) = db.event_by_uid("u1").unwrap().unwrap();
        // Our edit, then another device's edit on the server.
        db.write_event(cal, Some(id), &event("u1", 1, "Mine"), "", NOW)
            .unwrap();
        dav.state
            .lock()
            .unwrap()
            .put("/dav/cals/w/", "1.ics", &event("u1", 1, "Theirs"));
        // A pull while ours is waiting keeps ours shown.
        sync_all(&db, account, &client);
        assert_eq!(titles(&db), ["Mine"]);
        let o = pass(&db, account, &client);
        assert_eq!(o.conflicts, 1);
        // Not overwritten on the server; the server's version shows here.
        assert!(
            server_body(&dav, "/dav/cals/w/1.ics")
                .unwrap()
                .contains("Theirs")
        );
        assert_eq!(titles(&db), ["Theirs"]);
        assert_eq!(db.conflicts().unwrap(), 1);
        assert!(db.event_info(id).unwrap().unwrap().conflict);
        assert!(db.queued(cal).unwrap().is_empty(), "held, not retried");
        db.resolve_conflicts(again, "", NOW).unwrap();
        assert_eq!(db.conflicts().unwrap(), 0);
        pass(&db, account, &client);
        let body = server_body(&dav, "/dav/cals/w/1.ics").unwrap();
        if again {
            assert!(body.contains("Mine"));
            assert_eq!(titles(&db), ["Mine"]);
        } else {
            assert!(body.contains("Theirs"));
            assert_eq!(titles(&db), ["Theirs"]);
        }
        assert!(!db.event_info(id).unwrap().unwrap().conflict);
    }
}

#[test]
fn a_refused_write_marks_the_calendar_read_only() {
    let (dav, db, account, client) = setup();
    let cal = one_calendar(&dav, &db, account, &client);
    dav.state.lock().unwrap().read_only = true;
    let id = db
        .write_event(cal, None, &event("n", 7, "New"), "", NOW)
        .unwrap();
    let (old, _) = db.event_by_uid("u1").unwrap().unwrap();
    db.write_event(cal, Some(old), &event("u1", 1, "Changed"), "", NOW)
        .unwrap();
    let o = pass(&db, account, &client);
    assert_eq!(o.refused, 2);
    assert!(db.event_info(id).unwrap().is_none(), "the new one goes");
    assert_eq!(titles(&db), ["Review"], "the server's version is back");
    assert!(!db.event_info(old).unwrap().unwrap().can_write);
    assert!(db.queued(cal).unwrap().is_empty());
}

#[test]
fn a_network_failure_keeps_the_queue_for_the_next_pass() {
    let (dav, db, account, client) = setup();
    let cal = one_calendar(&dav, &db, account, &client);
    db.write_event(cal, None, &event("n", 7, "Offline edit"), "", NOW)
        .unwrap();
    // Nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let down = Client::for_test(
        &format!("https://127.0.0.1:{port}/"),
        Some((USER, PASS)),
        CERT,
    );
    let c = &db.dav_calendars(account).unwrap()[0];
    assert!(flush(&down, &db, c).is_err());
    assert_eq!(db.queued(cal).unwrap().len(), 1);
    assert_eq!(db.unsent_changes(Some(account)).unwrap(), 1);
    // The next pass sends it.
    pass(&db, account, &client);
    assert!(db.queued(cal).unwrap().is_empty());
    assert!(server_body(&dav, "/dav/cals/w/n.ics").is_some());
}

#[test]
fn removing_someone_elses_event_sends_no_reply() {
    let (dav, db, account, client) = setup();
    let invite = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:inv\r\nDTSTART:20261009T090000Z\r\n\
                  SUMMARY:Board\r\nORGANIZER:mailto:boss@example.com\r\n\
                  ATTENDEE;PARTSTAT=ACCEPTED:mailto:ana@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    dav.state
        .lock()
        .unwrap()
        .put("/dav/cals/w/", "inv.ics", invite);
    let cal = one_calendar(&dav, &db, account, &client);
    let (id, _) = db.event_by_uid("inv").unwrap().unwrap();
    assert!(!db.event_info(id).unwrap().unwrap().mine, "read only");
    db.delete_event(id, true, NOW).unwrap();
    assert_eq!(db.queued(cal).unwrap()[0].op, crate::cal::write::OP_REMOVE);
    pass(&db, account, &client);
    assert!(server_body(&dav, "/dav/cals/w/inv.ics").is_none());
}
