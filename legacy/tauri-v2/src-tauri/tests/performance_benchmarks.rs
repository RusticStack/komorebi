use std::{
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use komorebi_lib::schema::{apply_migrations, check_messages_fts_integrity, search_messages_fts};
use rusqlite::{params, Connection};
use serde_json::json;

const MESSAGE_COUNT: usize = 10_000;
const ATTACHMENT_COUNT: usize = 500;

#[test]
#[ignore = "benchmark-style measurement; run with pnpm perf:backend"]
fn sqlite_fts_latency_report() {
    let workspace = BenchmarkWorkspace::new("sqlite_fts");
    let mut connection = open_cache(&workspace.db_path());
    seed_account_and_folder(&connection);
    seed_messages(&mut connection, MESSAGE_COUNT);
    check_messages_fts_integrity(&connection).expect("FTS integrity check should pass");

    let first_query = measure_ms(|| {
        let results =
            search_messages_fts(&connection, "needle9999", 25).expect("FTS query should succeed");
        assert!(results.iter().any(|row| row.subject.contains("needle9999")));
    });
    let body_query = measure_ms(|| {
        let results = search_messages_fts(&connection, "portuguese japanese attachment", 25)
            .expect("FTS body query should succeed");
        assert!(!results.is_empty());
    });

    write_report(
        "sqlite-fts-benchmark.json",
        json!({
            "fixture": {
                "messages": MESSAGE_COUNT,
                "databaseBytes": fs::metadata(workspace.db_path()).expect("db metadata").len()
            },
            "measurements": {
                "needleQueryMs": first_query,
                "bodyQueryMs": body_query
            },
            "thresholds": {
                "singleQueryWarningMs": 250
            }
        }),
    );
}

#[test]
#[ignore = "benchmark-style measurement; run with pnpm perf:backend"]
fn sync_and_attachment_storage_report() {
    let workspace = BenchmarkWorkspace::new("sync_attachment");
    let mut connection = open_cache(&workspace.db_path());
    seed_account_and_folder(&connection);

    let initial_sync_ms = measure_ms(|| seed_messages(&mut connection, MESSAGE_COUNT));
    let incremental_sync_ms = measure_ms(|| {
        let transaction = connection.transaction().expect("incremental transaction");
        for uid in (MESSAGE_COUNT + 1)..=(MESSAGE_COUNT + 250) {
            insert_message(&transaction, uid);
        }
        transaction.commit().expect("incremental commit");
    });
    let flag_update_ms = measure_ms(|| {
        let transaction = connection.transaction().expect("flag transaction");
        for uid in 1..=1_000 {
            transaction
                .execute(
                    "UPDATE messages
                     SET is_read = 1,
                         flags_json = '[\"\\\\Seen\"]',
                         updated_at = CURRENT_TIMESTAMP
                     WHERE account_id = 'acct_perf'
                       AND folder_id = 'folder_perf_inbox'
                       AND uid_validity = 77
                       AND imap_uid = ?1",
                    params![uid as i64],
                )
                .expect("flag update");
        }
        transaction.commit().expect("flag commit");
    });
    let attachment_write_ms = measure_ms(|| seed_attachment_files(&workspace));

    write_report(
        "sync-attachment-benchmark.json",
        json!({
            "fixture": {
                "initialMessages": MESSAGE_COUNT,
                "incrementalMessages": 250,
                "flagUpdates": 1000,
                "attachmentFiles": ATTACHMENT_COUNT,
                "attachmentBytes": ATTACHMENT_COUNT * 4096
            },
            "measurements": {
                "initialSyncInsertMs": initial_sync_ms,
                "incrementalSyncInsertMs": incremental_sync_ms,
                "flagUpdateMs": flag_update_ms,
                "attachmentWriteMs": attachment_write_ms
            },
            "thresholds": {
                "initialSyncInsertWarningMs": 5000,
                "incrementalSyncInsertWarningMs": 1000,
                "flagUpdateWarningMs": 1000,
                "attachmentWriteWarningMs": 2000
            }
        }),
    );
}

fn open_cache(path: &Path) -> Connection {
    let mut connection = Connection::open(path).expect("open benchmark database");
    apply_migrations(&mut connection).expect("apply migrations");
    connection
}

fn seed_account_and_folder(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO accounts (
               id,
               provider_kind,
               display_email,
               display_name,
               username,
               auth_kind,
               credential_ref,
               imap_host,
               imap_port,
               imap_security,
               smtp_host,
               smtp_port,
               smtp_security,
               auth_status
             )
             VALUES (
               'acct_perf',
               'generic_imap_smtp',
               'performance@example.com',
               'Performance',
               'performance@example.com',
               'app_password',
               'komorebi://perf-secret',
               'imap.example.com',
               993,
               'tls',
               'smtp.example.com',
               587,
               'starttls',
               'connected'
             )",
            [],
        )
        .expect("insert benchmark account");
    connection
        .execute(
            "INSERT INTO folders (
               id,
               account_id,
               remote_id,
               name,
               display_name,
               kind,
               special_use,
               uid_validity,
               uid_next,
               sync_state
             )
             VALUES (
               'folder_perf_inbox',
               'acct_perf',
               'INBOX',
               'INBOX',
               'Inbox',
               'inbox',
               'inbox',
               77,
               1,
               'idle'
             )",
            [],
        )
        .expect("insert benchmark folder");
}

fn seed_messages(connection: &mut Connection, count: usize) {
    let transaction = connection.transaction().expect("message transaction");
    for uid in 1..=count {
        insert_message(&transaction, uid);
    }
    transaction.commit().expect("message commit");
}

fn insert_message(connection: &Connection, uid: usize) {
    let needle = if uid == 9_999 {
        "needle9999"
    } else {
        "ordinary"
    };
    connection
        .execute(
            "INSERT INTO messages (
               id,
               account_id,
               folder_id,
               imap_uid,
               uid_validity,
               rfc_message_id,
               subject,
               sender_name,
               sender_email,
               to_addresses_json,
               cc_addresses_json,
               bcc_addresses_json,
               reply_to_addresses_json,
               references_json,
               date_received,
               internal_date,
               is_read,
               body_text,
               body_html,
               snippet,
               size_bytes
             )
             VALUES (
               ?1,
               'acct_perf',
               'folder_perf_inbox',
               ?2,
               77,
               ?3,
               ?4,
               'Performance Sender',
               'sender@example.com',
               '[]',
               '[]',
               '[]',
               '[]',
               '[]',
               ?5,
               ?5,
               0,
               ?6,
               NULL,
               ?7,
               ?8
             )",
            params![
                format!("msg_perf_{uid:05}"),
                uid as i64,
                format!("<perf-{uid}@example.com>"),
                format!("Performance subject {uid:05} {needle}"),
                format!("2026-05-25T{:02}:00:00Z", uid % 24),
                format!(
                    "Body {uid} with portuguese japanese attachment search terms and {needle}."
                ),
                format!("Body {uid} with {needle}."),
                2048 + uid as i64
            ],
        )
        .expect("insert benchmark message");
}

fn seed_attachment_files(workspace: &BenchmarkWorkspace) {
    let root = workspace.path.join("attachments").join("acct_perf");
    fs::create_dir_all(&root).expect("create attachment root");
    let bytes = vec![b'a'; 4096];
    for index in 0..ATTACHMENT_COUNT {
        let path = root.join(format!("attachment-{index:04}.bin"));
        fs::write(path, &bytes).expect("write attachment fixture");
    }
}

fn measure_ms(action: impl FnOnce()) -> f64 {
    let started = Instant::now();
    action();
    round_ms(started.elapsed().as_secs_f64() * 1000.0)
}

fn round_ms(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn write_report(filename: &str, value: serde_json::Value) {
    let report_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("test-results")
        .join("perf");
    fs::create_dir_all(&report_dir).expect("create perf report dir");
    let path = report_dir.join(filename);
    fs::write(
        &path,
        serde_json::to_string_pretty(&value).expect("serialize report"),
    )
    .expect("write report");
    println!("KOMOREBI_PERF_REPORT {} {}", path.display(), value);
}

struct BenchmarkWorkspace {
    path: PathBuf,
}

impl BenchmarkWorkspace {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("komorebi-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create benchmark workspace");

        Self { path }
    }

    fn db_path(&self) -> PathBuf {
        self.path.join("mail-cache.sqlite")
    }
}

impl Drop for BenchmarkWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
