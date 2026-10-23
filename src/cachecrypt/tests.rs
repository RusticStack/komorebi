//! Throwaway temp-file caches only; never the real cache.

use super::*;
use rusqlite::OpenFlags;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("komorebi-cc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("cache.sqlite3")
}

const SECRET: &str = "Very secret subject line zebra";

fn fill(conn: &rusqlite::Connection, n: i64) {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS m (id INTEGER PRIMARY KEY, subject TEXT)")
        .unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    for i in 0..n {
        tx.execute(
            "INSERT INTO m (subject) VALUES (?1)",
            [format!("{SECRET} {i}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

fn count(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM m WHERE subject LIKE '%zebra%'",
        [],
        |r| r.get(0),
    )
}

fn contains(path: &Path, needle: &str) -> bool {
    let bytes = std::fs::read(path).unwrap_or_default();
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn round_trip_leaves_no_plaintext_in_db_or_wal() {
    let db = tmp("round");
    let key = [7u8; 32];
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        c.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
        fill(&c, 500);
        assert!(!contains(&with(&db, "-wal"), "zebra"));
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        assert_eq!(count(&c).unwrap(), 500);
    }
    assert!(is_encrypted(&db));
    assert!(!contains(&db, "zebra"));
    assert!(!contains(&db, "SQLite format 3"));
    let c = open(&db, &key, OpenFlags::default()).unwrap();
    assert_eq!(count(&c).unwrap(), 500);
    let ok: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap();
    assert_eq!(ok, "ok");
}

#[test]
fn wrong_key_fails_closed_and_leaves_the_file_alone() {
    let db = tmp("wrongkey");
    {
        let c = open(&db, &[1u8; 32], OpenFlags::default()).unwrap();
        fill(&c, 50);
    }
    let before = std::fs::read(&db).unwrap();
    let r = open(&db, &[2u8; 32], OpenFlags::default()).and_then(|c| Ok(count(&c)?));
    let e = r.unwrap_err();
    assert!(!format!("{e:#}").is_empty());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let c = open(&db, &[1u8; 32], OpenFlags::default()).unwrap();
    assert_eq!(count(&c).unwrap(), 50);
}

#[test]
fn a_changed_byte_is_an_error_not_wrong_data() {
    let db = tmp("tamper");
    let key = [3u8; 32];
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        fill(&c, 300);
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let mut bytes = std::fs::read(&db).unwrap();
    assert!(bytes.len() > 3 * PAGE);
    bytes[2 * PAGE + 100] ^= 1;
    std::fs::write(&db, &bytes).unwrap();
    let c = open(&db, &key, OpenFlags::default()).unwrap();
    let e = c
        .prepare("SELECT group_concat(subject) FROM m")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, String>(0)))
        .unwrap_err();
    let rusqlite::Error::SqliteFailure(f, _) = e else {
        panic!("{e}")
    };
    assert_eq!(f.extended_code, ffi::SQLITE_IOERR_DATA);
}

#[test]
fn a_zeroed_page_inside_the_file_is_tampering() {
    let db = tmp("zeroed");
    let key = [6u8; 32];
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        fill(&c, 300);
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let mut bytes = std::fs::read(&db).unwrap();
    assert!(bytes.len() > 3 * PAGE);
    bytes[2 * PAGE..3 * PAGE].fill(0);
    std::fs::write(&db, &bytes).unwrap();
    let c = open(&db, &key, OpenFlags::default()).unwrap();
    let e = c
        .prepare("SELECT group_concat(subject) FROM m")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, String>(0)))
        .unwrap_err();
    let rusqlite::Error::SqliteFailure(f, _) = e else {
        panic!("{e}")
    };
    assert_eq!(f.extended_code, ffi::SQLITE_IOERR_DATA);
}

#[test]
fn swapped_pages_are_refused() {
    let db = tmp("swap");
    let key = [4u8; 32];
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        fill(&c, 300);
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let mut bytes = std::fs::read(&db).unwrap();
    let (a, b) = bytes.split_at_mut(3 * PAGE);
    a[2 * PAGE..].swap_with_slice(&mut b[..PAGE]);
    std::fs::write(&db, &bytes).unwrap();
    let c = open(&db, &key, OpenFlags::default()).unwrap();
    assert!(
        c.query_row("SELECT group_concat(subject) FROM m", [], |r| r
            .get::<_, String>(0))
            .is_err()
    );
}

#[test]
fn committed_frames_survive_wal_recovery() {
    let db = tmp("walrec");
    let key = [5u8; 32];
    let copy = db.with_file_name("copy.sqlite3");
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        c.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
        fill(&c, 10);
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        fill(&c, 200);
        c.execute("UPDATE m SET subject = subject || ' x' WHERE id < 50", [])
            .unwrap();
        // A crash: copy the cache and its log while the writer is open.
        std::fs::copy(&db, &copy).unwrap();
        std::fs::copy(with(&db, "-wal"), with(&copy, "-wal")).unwrap();
        assert!(std::fs::metadata(with(&copy, "-wal")).unwrap().len() > FRAME);
    }
    let c = open(&copy, &key, OpenFlags::default()).unwrap();
    assert_eq!(count(&c).unwrap(), 210);
    let x: i64 = c
        .query_row("SELECT count(*) FROM m WHERE subject LIKE '% x'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(x, 49);
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
}

#[test]
fn a_torn_last_frame_is_dropped_not_an_error() {
    let db = tmp("torn");
    let key = [6u8; 32];
    let copy = db.with_file_name("copy.sqlite3");
    {
        let c = open(&db, &key, OpenFlags::default()).unwrap();
        c.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
        fill(&c, 20);
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        fill(&c, 1);
        std::fs::copy(&db, &copy).unwrap();
        let mut wal = std::fs::read(with(&db, "-wal")).unwrap();
        let last = wal.len() - 10;
        wal[last] ^= 0xff;
        std::fs::write(with(&copy, "-wal"), wal).unwrap();
    }
    let c = open(&copy, &key, OpenFlags::default()).unwrap();
    assert_eq!(count(&c).unwrap(), 20);
}

#[test]
fn journal_and_temp_files_are_refused() {
    let db = tmp("journal");
    let c = open(&db, &[8u8; 32], OpenFlags::default()).unwrap();
    fill(&c, 5);
    // Leaving WAL would need a rollback journal holding plaintext pages.
    let r = c
        .query_row("PRAGMA journal_mode = DELETE", [], |r| {
            r.get::<_, String>(0)
        })
        .and_then(|_| c.execute("INSERT INTO m (subject) VALUES ('j')", []));
    assert!(r.is_err() || !with(&db, "-journal").exists());
    // Sorting spills stay in memory.
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM (SELECT subject FROM m ORDER BY subject)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n >= 5);
}

#[test]
fn dpapi_key_round_trip_and_missing_key() {
    let db = tmp("dpapi");
    assert_eq!(load_key(&db), Err(KeyError::Missing));
    let k = new_key().unwrap();
    store_key(&db, &k).unwrap();
    assert_eq!(load_key(&db).unwrap(), k);
    assert!(
        !std::fs::read(key_path(&db))
            .unwrap()
            .windows(32)
            .any(|w| w == k)
    );
    std::fs::write(key_path(&db), b"not a dpapi blob").unwrap();
    assert_eq!(load_key(&db), Err(KeyError::Locked));
}

fn plain_cache(name: &str) -> PathBuf {
    let db = tmp(name);
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("PRAGMA journal_mode = WAL").unwrap();
    fill(&c, 400);
    db
}

fn plain_count(db: &Path) -> i64 {
    count(&rusqlite::Connection::open(db).unwrap()).unwrap()
}

fn enc_count(db: &Path) -> i64 {
    count(&open(db, &load_key(db).unwrap(), OpenFlags::default()).unwrap()).unwrap()
}

#[test]
fn migration_round_trip_keeps_every_row() {
    let db = plain_cache("migrate");
    migrate(&db, true).unwrap();
    assert!(is_encrypted(&db));
    assert!(!contains(&db, "zebra"));
    assert!(!with(&db, ".previous").exists() && !with(&db, ".migrate").exists());
    // The copy got the reserve bytes and WAL.
    let c = open(&db, &load_key(&db).unwrap(), OpenFlags::default()).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    fill(&c, 1);
    drop(c);
    assert_eq!(enc_count(&db), 401);
    migrate(&db, false).unwrap();
    assert!(!is_encrypted(&db));
    assert!(!key_path(&db).exists());
    assert_eq!(plain_count(&db), 401);
}

#[test]
fn migration_crash_points_recover_to_a_whole_cache() {
    for (i, stop) in [Stop::AfterKey, Stop::AfterCopy, Stop::AfterAside]
        .into_iter()
        .enumerate()
    {
        let db = plain_cache(&format!("crash{i}"));
        migrate_until(&db, true, stop).unwrap();
        recover(&db).unwrap();
        let n = if is_encrypted(&db) {
            enc_count(&db)
        } else {
            plain_count(&db)
        };
        assert_eq!(n, 400, "{stop:?}");
        // After the swap point the encrypted copy is the cache.
        assert_eq!(is_encrypted(&db), stop == Stop::AfterAside, "{stop:?}");
        assert!(!with(&db, ".migrate").exists() && !with(&db, ".previous").exists());
        // A retry finishes the job.
        migrate(&db, true).unwrap();
        assert_eq!(enc_count(&db), 400);
    }
}

#[test]
fn uri_escapes_windows_paths() {
    assert_eq!(
        uri(Path::new(r"C:\a b\x%?#.db"), 3),
        "file:///C:/a b/x%25%3f%23.db?vfs=komorebi-aes&kid=3"
    );
}

#[test]
fn the_app_cache_opens_encrypted_and_fails_closed_without_its_key() {
    use crate::db::Db;
    let db = tmp("appdb");
    drop(Db::open(&db).unwrap());
    migrate(&db, true).unwrap();
    {
        let (d, aside) = Db::open_ui(&db).unwrap();
        assert!(aside.is_none());
        d.settings().unwrap();
        d.checkpoint();
        drop(Db::open_worker(&db).unwrap());
    }
    assert!(is_encrypted(&db));
    let key = std::fs::read(key_path(&db)).unwrap();
    std::fs::remove_file(key_path(&db)).unwrap();
    let before = std::fs::read(&db).unwrap();
    let e = Db::open_ui(&db).err().unwrap();
    assert_eq!(e.downcast_ref::<KeyError>(), Some(&KeyError::Missing));
    assert!(Db::open_worker(&db).is_err());
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "never set aside or changed"
    );
    // Another key (another user's) is refused, not read as damage.
    store_key(&db, &[9u8; 32]).unwrap();
    assert!(Db::open_ui(&db).is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    std::fs::write(key_path(&db), key).unwrap();
    Db::open_ui(&db).unwrap();
}

#[test]
fn the_settings_choice_converts_at_the_next_start_and_is_then_gone() {
    let db = plain_cache("intent");
    let rows = plain_count(&db);
    assert_eq!(intent(&db), None);
    // Asking for what it already is records nothing.
    set_intent(&db, false).unwrap();
    assert_eq!(intent(&db), None);
    set_intent(&db, true).unwrap();
    assert_eq!(intent(&db), Some(true));
    apply_intent(&db).unwrap();
    assert!(is_encrypted(&db));
    assert_eq!(intent(&db), None);
    assert_eq!(enc_count(&db), rows);
    set_intent(&db, false).unwrap();
    apply_intent(&db).unwrap();
    assert!(!is_encrypted(&db));
    assert!(!intent_path(&db).exists());
    // Nothing waiting: nothing happens.
    apply_intent(&db).unwrap();
    assert!(!is_encrypted(&db));
}
