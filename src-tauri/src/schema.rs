use std::{error::Error, fmt, time::Duration};

use rusqlite::{params, Connection};

pub const KOMOREBI_APPLICATION_ID: i64 = 1_263_354_434;
pub const LATEST_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        sql: include_str!("../migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "messages_fts",
        sql: include_str!("../migrations/002_messages_fts.sql"),
    },
    Migration {
        version: 3,
        name: "folder_semantics",
        sql: include_str!("../migrations/003_folder_semantics.sql"),
    },
];

#[derive(Debug)]
pub enum SchemaMigrationError {
    Sqlite(rusqlite::Error),
    DatabaseNewer { current: u32, latest: u32 },
}

impl fmt::Display for SchemaMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::DatabaseNewer { current, latest } => write!(
                formatter,
                "database schema version {current} is newer than supported version {latest}"
            ),
        }
    }
}

impl Error for SchemaMigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::DatabaseNewer { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for SchemaMigrationError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub fn configure_connection(connection: &Connection) -> Result<(), SchemaMigrationError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;

    Ok(())
}

pub fn current_schema_version(connection: &Connection) -> Result<u32, SchemaMigrationError> {
    let version =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?;

    Ok(version.max(0) as u32)
}

pub fn apply_migrations(
    connection: &mut Connection,
) -> Result<MigrationReport, SchemaMigrationError> {
    configure_connection(connection)?;

    let current_version = current_schema_version(connection)?;
    if current_version > LATEST_SCHEMA_VERSION {
        return Err(SchemaMigrationError::DatabaseNewer {
            current: current_version,
            latest: LATEST_SCHEMA_VERSION,
        });
    }

    let mut applied = Vec::new();
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current_version)
    {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            (migration.version, migration.name),
        )?;
        transaction.execute_batch(&format!("PRAGMA user_version = {};", migration.version))?;
        transaction.commit()?;
        applied.push(*migration);
    }

    Ok(MigrationReport {
        current_version,
        latest_version: current_schema_version(connection)?,
        applied,
    })
}

pub fn rebuild_messages_fts(connection: &Connection) -> Result<(), SchemaMigrationError> {
    connection.execute(
        "INSERT INTO messages_fts (messages_fts) VALUES ('rebuild')",
        [],
    )?;

    Ok(())
}

pub fn check_messages_fts_integrity(connection: &Connection) -> Result<(), SchemaMigrationError> {
    connection.execute(
        "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        [],
    )?;

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageSearchResult {
    pub message_id: String,
    pub account_id: String,
    pub folder_id: String,
    pub subject: String,
    pub sender_email: Option<String>,
    pub rank: f64,
}

pub fn search_messages_fts(
    connection: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<MessageSearchResult>, SchemaMigrationError> {
    let bounded_limit = limit.clamp(1, 100);
    let mut statement = connection.prepare(
        "SELECT
           messages.id,
           messages.account_id,
           messages.folder_id,
           messages.subject,
           messages.sender_email,
           bm25(messages_fts) AS rank
         FROM messages_fts
         JOIN messages ON messages.rowid = messages_fts.rowid
         WHERE messages_fts MATCH ?1
         ORDER BY rank, messages.date_received DESC, messages.id
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![query, bounded_limit], |row| {
        Ok(MessageSearchResult {
            message_id: row.get(0)?,
            account_id: row.get(1)?,
            folder_id: row.get(2)?,
            subject: row.get(3)?,
            sender_email: row.get(4)?,
            rank: row.get(5)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }

    Ok(results)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub current_version: u32,
    pub latest_version: u32,
    pub applied: Vec<Migration>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open temp sqlite database");
        apply_migrations(&mut connection).expect("apply migrations");
        connection
    }

    fn schema_object_names(connection: &Connection, object_type: &str) -> HashSet<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = ?1")
            .unwrap();
        let rows = statement
            .query_map([object_type], |row| row.get::<_, String>(0))
            .unwrap();

        rows.map(|row| row.unwrap()).collect()
    }

    fn table_columns(connection: &Connection, table_name: &str) -> HashSet<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table_name})"))
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap();

        rows.map(|row| row.unwrap()).collect()
    }

    fn seed_account_folder_message(connection: &Connection) {
        connection
            .execute_batch(
                r#"
                INSERT INTO accounts (
                  id,
                  provider_kind,
                  display_email,
                  username,
                  auth_kind,
                  credential_ref
                )
                VALUES (
                  'acct_1',
                  'gmail',
                  'person@example.com',
                  'person@example.com',
                  'oauth2',
                  'Komorebi/mail-account/acct_1/oauth-refresh'
                );

                INSERT INTO folders (
                  id,
                  account_id,
                  remote_id,
                  name,
                  display_name,
                  kind,
                  uid_validity
                )
                VALUES (
                  'folder_1',
                  'acct_1',
                  'INBOX',
                  'INBOX',
                  'Inbox',
                  'inbox',
                  777
                );

                INSERT INTO messages (
                  id,
                  account_id,
                  folder_id,
                  imap_uid,
                  uid_validity,
                  rfc_message_id,
                  subject,
                  sender_email,
                  to_addresses_json,
                  date_received
                )
                VALUES (
                  'msg_1',
                  'acct_1',
                  'folder_1',
                  42,
                  777,
                  '<message-1@example.com>',
                  'Schema test',
                  'sender@example.com',
                  '[{"email":"person@example.com"}]',
                  '2026-05-22T00:00:00Z'
                );
                "#,
            )
            .unwrap();
    }

    #[test]
    fn applies_initial_schema_records_version_and_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();

        let first_report = apply_migrations(&mut connection).unwrap();
        let second_report = apply_migrations(&mut connection).unwrap();
        let application_id: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .unwrap();
        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let foreign_keys_enabled: i64 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();

        assert_eq!(first_report.current_version, 0);
        assert_eq!(first_report.latest_version, LATEST_SCHEMA_VERSION);
        assert_eq!(first_report.applied.len(), MIGRATIONS.len());
        assert_eq!(second_report.current_version, LATEST_SCHEMA_VERSION);
        assert!(second_report.applied.is_empty());
        assert_eq!(migration_count, MIGRATIONS.len() as i64);
        assert_eq!(application_id, KOMOREBI_APPLICATION_ID);
        assert_eq!(foreign_keys_enabled, 1);
    }

    #[test]
    fn rejects_database_newer_than_supported_schema() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("PRAGMA user_version = 99;")
            .unwrap();

        let error = apply_migrations(&mut connection).unwrap_err();

        assert!(matches!(
            error,
            SchemaMigrationError::DatabaseNewer {
                current: 99,
                latest: LATEST_SCHEMA_VERSION
            }
        ));
    }

    #[test]
    fn creates_required_tables_indexes_and_timestamp_triggers() {
        let connection = migrated_connection();
        let tables = schema_object_names(&connection, "table");
        let indexes = schema_object_names(&connection, "index");
        let triggers = schema_object_names(&connection, "trigger");

        for table in [
            "schema_migrations",
            "settings",
            "accounts",
            "folders",
            "messages",
            "messages_fts",
            "attachments",
            "templates",
            "template_versions",
        ] {
            assert!(tables.contains(table), "missing table {table}");
        }

        for index in [
            "ux_messages_imap_identity",
            "ux_attachments_content_id",
            "ux_templates_global_name",
            "ux_templates_account_name",
            "idx_accounts_provider_email",
            "idx_folders_account_kind",
            "idx_messages_folder_received",
            "idx_messages_account_rfc_message_id",
            "idx_messages_unread",
            "idx_attachments_message",
            "idx_templates_account",
        ] {
            assert!(indexes.contains(index), "missing index {index}");
        }

        for trigger in [
            "settings_touch_updated_at",
            "accounts_touch_updated_at",
            "folders_touch_updated_at",
            "messages_touch_updated_at",
            "messages_fts_after_insert",
            "messages_fts_after_delete",
            "messages_fts_after_update",
            "attachments_touch_updated_at",
            "templates_touch_updated_at",
        ] {
            assert!(triggers.contains(trigger), "missing trigger {trigger}");
        }
    }

    #[test]
    fn defines_account_folder_message_attachment_and_template_columns() {
        let connection = migrated_connection();

        for (table, required_columns) in [
            (
                "accounts",
                [
                    "id",
                    "provider_kind",
                    "display_email",
                    "username",
                    "auth_kind",
                    "credential_ref",
                    "signature_html",
                    "created_at",
                    "updated_at",
                ]
                .as_slice(),
            ),
            (
                "folders",
                [
                    "id",
                    "account_id",
                    "remote_id",
                    "name",
                    "kind",
                    "attributes_json",
                    "uid_validity",
                    "unread_count",
                    "sync_state",
                    "created_at",
                    "updated_at",
                ]
                .as_slice(),
            ),
            (
                "messages",
                [
                    "id",
                    "account_id",
                    "folder_id",
                    "imap_uid",
                    "uid_validity",
                    "rfc_message_id",
                    "subject",
                    "sender_email",
                    "to_addresses_json",
                    "body_text",
                    "body_html",
                    "created_at",
                    "updated_at",
                ]
                .as_slice(),
            ),
            (
                "attachments",
                [
                    "id",
                    "account_id",
                    "folder_id",
                    "message_id",
                    "display_filename",
                    "canonical_mime",
                    "size_bytes",
                    "storage_relative_path",
                    "created_at",
                    "updated_at",
                ]
                .as_slice(),
            ),
            (
                "templates",
                [
                    "id",
                    "account_id",
                    "name",
                    "block_data_json",
                    "html_output",
                    "schema_version",
                    "created_at",
                    "updated_at",
                ]
                .as_slice(),
            ),
        ] {
            let columns = table_columns(&connection, table);

            for column in required_columns {
                assert!(columns.contains(*column), "{table} missing {column}");
            }
        }
    }

    #[test]
    fn account_metadata_does_not_define_secret_columns() {
        let connection = migrated_connection();
        let account_columns = table_columns(&connection, "accounts");

        for forbidden in [
            "password",
            "app_password",
            "access_token",
            "refresh_token",
            "client_secret",
            "authorization_code",
            "authorization_header",
        ] {
            assert!(!account_columns.contains(forbidden));
        }
    }

    #[test]
    fn foreign_keys_and_cascades_protect_account_folder_message_relations() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        connection
            .execute(
                "INSERT INTO attachments (
                   id,
                   account_id,
                   folder_id,
                   message_id,
                   display_filename,
                   canonical_mime,
                   size_bytes
                 )
                 VALUES ('att_1', 'acct_1', 'folder_1', 'msg_1', 'report.txt', 'text/plain', 12)",
                [],
            )
            .unwrap();

        assert!(connection
            .execute(
                "INSERT INTO attachments (
                   id,
                   account_id,
                   folder_id,
                   message_id,
                   display_filename,
                   canonical_mime,
                   size_bytes
                 )
                 VALUES (
                   'att_missing',
                   'acct_1',
                   'folder_1',
                   'missing_msg',
                   'missing.txt',
                   'text/plain',
                   1
                 )",
                [],
            )
            .is_err());

        connection
            .execute("DELETE FROM accounts WHERE id = 'acct_1'", [])
            .unwrap();

        for table in ["folders", "messages", "attachments"] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} should cascade after account delete");
        }
    }

    #[test]
    fn uniqueness_rules_separate_imap_uid_from_rfc_message_id() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        connection
            .execute(
                "INSERT INTO folders (
                   id,
                   account_id,
                   remote_id,
                   name,
                   display_name,
                   kind,
                   uid_validity
                 )
                 VALUES (
                   'folder_2',
                   'acct_1',
                   'Archive',
                   'Archive',
                   'Archive',
                   'archive',
                   777
                 )",
                [],
            )
            .unwrap();

        assert!(connection
            .execute(
                "INSERT INTO messages (
                   id,
                   account_id,
                   folder_id,
                   imap_uid,
                   uid_validity,
                   rfc_message_id,
                   subject
                 )
                 VALUES (
                   'msg_duplicate_uid',
                   'acct_1',
                   'folder_1',
                   42,
                   777,
                   '<different@example.com>',
                   'Duplicate UID'
                 )",
                [],
            )
            .is_err());

        connection
            .execute(
                "INSERT INTO messages (
                   id,
                   account_id,
                   folder_id,
                   imap_uid,
                   uid_validity,
                   rfc_message_id,
                   subject
                 )
                 VALUES (
                   'msg_same_uid_other_folder',
                   'acct_1',
                   'folder_2',
                   42,
                   777,
                   '<message-1@example.com>',
                   'Same UID in different folder'
                 )",
                [],
            )
            .unwrap();
    }

    #[test]
    fn settings_and_template_json_are_checked_and_deduplicated() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        connection
            .execute(
                "INSERT INTO settings (key, value_json, value_type)
                 VALUES ('system_language', '\"en\"', 'string')",
                [],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO settings (key, value_json, value_type)
                 VALUES ('bad_json', '{', 'object')",
                [],
            )
            .is_err());

        connection
            .execute(
                "INSERT INTO templates (id, name, block_data_json, html_output)
                 VALUES ('tpl_global', 'Follow up', '{\"blocks\":[]}', '<p>Follow up</p>')",
                [],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO templates (id, name, block_data_json)
                 VALUES ('tpl_global_duplicate', 'Follow up', '{\"blocks\":[]}' )",
                [],
            )
            .is_err());

        connection
            .execute(
                "INSERT INTO templates (id, account_id, name, block_data_json)
                 VALUES ('tpl_account', 'acct_1', 'Follow up', '{\"blocks\":[]}' )",
                [],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO template_versions (
                   id,
                   template_id,
                   version_number,
                   block_data_json
                 )
                 VALUES ('tpl_v1', 'tpl_account', 1, '{' )",
                [],
            )
            .is_err());
    }

    #[test]
    fn no_foreign_key_violations_after_valid_seed_data() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        let violation_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(violation_count, 0);
    }

    #[test]
    fn fts_search_returns_expected_fixture_rows() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);
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
                   body_text,
                   date_received
                 )
                 VALUES (
                   'msg_2',
                   'acct_1',
                   'folder_1',
                   43,
                   777,
                   '<message-2@example.com>',
                   'Cache status',
                   'Komorebi Ops',
                   'ops@example.com',
                   'Phoenix search fixture for the local cache.',
                   '2026-05-23T00:00:00Z'
                 )",
                [],
            )
            .unwrap();

        let schema_results = search_messages_fts(&connection, "schema", 10).unwrap();
        let phoenix_results = search_messages_fts(&connection, "phoenix", 10).unwrap();

        assert_eq!(schema_results.len(), 1);
        assert_eq!(schema_results[0].message_id, "msg_1");
        assert_eq!(phoenix_results.len(), 1);
        assert_eq!(phoenix_results[0].message_id, "msg_2");
        check_messages_fts_integrity(&connection).unwrap();
    }

    #[test]
    fn fts_triggers_track_message_updates_and_deletes() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        assert_eq!(
            search_messages_fts(&connection, "schema", 10)
                .unwrap()
                .len(),
            1
        );

        connection
            .execute(
                "UPDATE messages
                 SET subject = 'Updated agenda', body_text = 'Phoenix follow-up body'
                 WHERE id = 'msg_1'",
                [],
            )
            .unwrap();

        assert!(search_messages_fts(&connection, "schema", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            search_messages_fts(&connection, "phoenix", 10)
                .unwrap()
                .len(),
            1
        );

        connection
            .execute("DELETE FROM messages WHERE id = 'msg_1'", [])
            .unwrap();

        assert!(search_messages_fts(&connection, "phoenix", 10)
            .unwrap()
            .is_empty());
        check_messages_fts_integrity(&connection).unwrap();
    }

    #[test]
    fn fts_rebuild_restores_missing_index_entries() {
        let connection = migrated_connection();
        seed_account_folder_message(&connection);

        connection
            .execute(
                "INSERT INTO messages_fts (messages_fts) VALUES ('delete-all')",
                [],
            )
            .unwrap();
        assert!(search_messages_fts(&connection, "schema", 10)
            .unwrap()
            .is_empty());

        rebuild_messages_fts(&connection).unwrap();

        assert_eq!(
            search_messages_fts(&connection, "schema", 10)
                .unwrap()
                .len(),
            1
        );
        check_messages_fts_integrity(&connection).unwrap();
    }

    #[test]
    fn fts_migration_rebuilds_rows_from_existing_v1_database() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        connection.execute_batch(MIGRATIONS[0].sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                (MIGRATIONS[0].version, MIGRATIONS[0].name),
            )
            .unwrap();
        connection
            .execute_batch("PRAGMA user_version = 1;")
            .unwrap();
        seed_account_folder_message(&connection);

        let report = apply_migrations(&mut connection).unwrap();

        assert_eq!(report.current_version, 1);
        assert_eq!(
            report
                .applied
                .iter()
                .map(|migration| migration.version)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            search_messages_fts(&connection, "schema", 10)
                .unwrap()
                .len(),
            1
        );
        let attributes_json = connection
            .query_row(
                "SELECT attributes_json FROM folders WHERE remote_id = 'INBOX'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(attributes_json, "[]");
        check_messages_fts_integrity(&connection).unwrap();
    }

    #[test]
    fn folder_semantics_migration_adds_attribute_preservation_to_v2_database() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        connection.execute_batch(MIGRATIONS[0].sql).unwrap();
        connection.execute_batch(MIGRATIONS[1].sql).unwrap();
        for migration in &MIGRATIONS[..2] {
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    (migration.version, migration.name),
                )
                .unwrap();
        }
        connection
            .execute_batch("PRAGMA user_version = 2;")
            .unwrap();
        seed_account_folder_message(&connection);

        let report = apply_migrations(&mut connection).unwrap();

        assert_eq!(report.current_version, 2);
        assert_eq!(
            report
                .applied
                .iter()
                .map(|migration| migration.version)
                .collect::<Vec<_>>(),
            vec![3]
        );
        let attributes_json = connection
            .query_row(
                "SELECT attributes_json FROM folders WHERE remote_id = 'INBOX'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(attributes_json, "[]");
    }
}
