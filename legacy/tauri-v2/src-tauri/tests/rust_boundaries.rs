use std::{collections::BTreeSet, env, path::PathBuf, process};

use komorebi_lib::{
    attachment_safety::{
        attachment_storage_path, decide_attachment_preview, sanitize_display_filename,
        validate_attachment_for_storage, AttachmentContentType, AttachmentPreviewRequest,
        AttachmentStorageKey, AttachmentTypeEvidence, PreviewKind,
    },
    credentials::{credential_target_name, CredentialSlot, CREDENTIAL_NAMESPACE},
    imap_sync::{
        FolderSyncState, RemoteFlag, RemoteFolder, RemoteFolderKind, RemoteFolderStatus,
        RemoteMessage, RemoteSpecialUse, SqliteSyncCache, SyncCache,
    },
    ipc_validation::{
        reject_client_supplied_path, reject_raw_sql, reject_unsanitized_html,
        validate_bounded_text, validate_checked_url, validate_entity_id,
    },
    mail_shell::{
        load_mail_shell_from_connection, search_mail_from_connection, MailSearchIndexStatus,
        MailSearchRequest,
    },
    mime_normalization::{
        parse_inbound_mime, persist_parsed_inbound_mime, ParseInboundMimeRequest,
    },
    schema::{apply_migrations, check_messages_fts_integrity, search_messages_fts},
};
use rusqlite::{params, Connection};

const ACCOUNT_ID: &str = "acct_integration";

fn migrated_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open disposable sqlite database");
    apply_migrations(&mut connection).expect("apply production migrations");
    connection
}

fn seed_account(connection: &Connection) {
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
               auth_status
             )
             VALUES (?1, 'gmail', 'integration@example.com', 'Integration', 'integration@example.com', 'oauth2', ?2, 'connected')",
            params![
                ACCOUNT_ID,
                credential_target_name(ACCOUNT_ID, CredentialSlot::OAuthRefresh)
                    .expect("credential target name")
            ],
        )
        .expect("seed account");
}

#[test]
fn schema_mime_fts_and_mail_shell_search_share_the_same_cache_contract() {
    let mut connection = migrated_connection();
    seed_account(&connection);

    connection
        .execute(
            "INSERT INTO folders (
               id,
               account_id,
               remote_id,
               name,
               display_name,
               kind,
               uid_validity,
               uid_next,
               unread_count,
               total_count,
               sync_state
             )
             VALUES ('folder_inbox', ?1, 'INBOX', 'INBOX', 'Inbox', 'inbox', 77, 2, 1, 1, 'idle')",
            [ACCOUNT_ID],
        )
        .expect("seed folder");

    let raw_message = integration_message();
    let parsed = parse_inbound_mime(ParseInboundMimeRequest {
        account_id: ACCOUNT_ID,
        folder_id: "folder_inbox",
        uid_validity: 77,
        uid: 1,
        raw_message: &raw_message,
        internal_date: Some("2026-05-23T12:00:00Z"),
    })
    .expect("parse RFC822 fixture");

    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].display_filename, "report.txt");
    assert_eq!(
        parsed.attachments[0].preview_kind,
        Some(komorebi_lib::mime_normalization::AttachmentPreviewKind::PlainText)
    );

    persist_parsed_inbound_mime(&mut connection, &parsed).expect("persist parsed MIME");
    check_messages_fts_integrity(&connection).expect("FTS integrity check");

    let fts_results =
        search_messages_fts(&connection, "komorebi", 10).expect("search messages FTS");
    assert_eq!(fts_results.len(), 1);
    assert_eq!(fts_results[0].account_id, ACCOUNT_ID);
    assert_eq!(fts_results[0].folder_id, "folder_inbox");

    let snapshot = load_mail_shell_from_connection(&connection).expect("load shell snapshot");
    assert_eq!(snapshot.accounts.len(), 1);
    assert_eq!(snapshot.accounts[0].id, ACCOUNT_ID);
    assert_eq!(snapshot.folders.len(), 1);
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].subject, "Komorebi integration search");
    assert_eq!(snapshot.messages[0].attachments.len(), 1);
    assert_eq!(
        snapshot.messages[0].attachments[0].display_filename,
        "report.txt"
    );

    let response = search_mail_from_connection(
        &connection,
        MailSearchRequest {
            query: "komorebi".into(),
            account_id: Some(ACCOUNT_ID.into()),
            folder_id: Some("folder_inbox".into()),
            limit: Some(10),
        },
    )
    .expect("mail-shell search");

    assert_eq!(response.index_status, MailSearchIndexStatus::Ready);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].account_id, ACCOUNT_ID);
    assert_eq!(response.results[0].folder_id, "folder_inbox");
}

#[test]
fn sqlite_sync_cache_preserves_uid_flag_and_expunged_message_state() {
    let connection = migrated_connection();
    seed_account(&connection);

    let mut cache = SqliteSyncCache::new(&connection);
    assert!(cache.account_exists(ACCOUNT_ID).expect("account lookup"));

    let folder = RemoteFolder::new(
        "INBOX",
        "INBOX",
        "Inbox",
        RemoteFolderKind::Inbox,
        Some(RemoteSpecialUse::Inbox),
        Some("/".into()),
    );
    let status = RemoteFolderStatus {
        uid_validity: 42,
        uid_next: 8,
        existing_uids: BTreeSet::from([7]),
        unread_count: 1,
        total_count: 1,
    };
    let folder_id = cache
        .upsert_folder(ACCOUNT_ID, &folder, &status, FolderSyncState::Syncing)
        .expect("upsert sync folder");

    let mut message = RemoteMessage::fixture(42, 7, "Sync integration boundary");
    message.body_text = Some("Komorebi sync cache body".into());
    message.flags = RemoteFlag::from_imap_tokens(["\\Seen", "\\Flagged", "$Custom"]);
    cache
        .upsert_message(ACCOUNT_ID, &folder_id, &message)
        .expect("upsert sync message");

    assert_eq!(
        cache
            .message_uids(ACCOUNT_ID, &folder_id, 42)
            .expect("message UID set"),
        BTreeSet::from([7])
    );
    assert!(search_messages_fts(&connection, "sync", 10)
        .expect("FTS sees sync cache writes")
        .iter()
        .any(|result| result.subject == "Sync integration boundary"));

    let changed = cache
        .update_message_flags(
            ACCOUNT_ID,
            &folder_id,
            42,
            7,
            &RemoteFlag::from_imap_tokens(["\\Seen", "\\Answered"]),
        )
        .expect("update flags");
    assert!(changed);

    let (is_flagged, is_answered): (i64, i64) = connection
        .query_row(
            "SELECT is_flagged, is_answered FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND imap_uid = 7",
            params![ACCOUNT_ID, &folder_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read updated flags");
    assert_eq!((is_flagged, is_answered), (0, 1));

    let deleted = cache
        .delete_message_by_uid(ACCOUNT_ID, &folder_id, 42, 7)
        .expect("delete expunged message");
    assert!(deleted);
    assert!(cache
        .message_uids(ACCOUNT_ID, &folder_id, 42)
        .expect("UID set after expunge")
        .is_empty());
}

#[test]
fn ipc_attachment_and_credential_boundaries_reject_dangerous_inputs() {
    validate_bounded_text("query", "komorebi", 128).expect("bounded text");
    validate_entity_id("account_id", ACCOUNT_ID).expect("valid account id");
    assert!(validate_entity_id("account_id", "../secret").is_err());
    assert!(reject_raw_sql("query", "DROP TABLE messages").is_err());
    assert!(reject_raw_sql("query", "quarterly review").is_ok());
    assert!(reject_client_supplied_path("attachment_path", "C:\\Users\\user\\secret.txt").is_err());
    assert!(reject_unsanitized_html("body", "<img src=x onerror=alert(1)>").is_err());
    assert!(validate_checked_url("href", "https://example.com/message").is_ok());
    assert!(validate_checked_url("href", "https://user:pass@example.com/message").is_err());

    let target =
        credential_target_name(ACCOUNT_ID, CredentialSlot::OAuthRefresh).expect("target name");
    assert_eq!(
        target,
        format!("{CREDENTIAL_NAMESPACE}/{ACCOUNT_ID}/oauth-refresh")
    );
    assert!(credential_target_name("acct/../secret", CredentialSlot::OAuthRefresh).is_err());

    let png_bytes = b"\x89PNG\r\n\x1A\nkomorebi";
    let content_type = validate_attachment_for_storage(AttachmentTypeEvidence {
        declared_mime: Some("image/png"),
        sniffed_bytes: png_bytes,
        byte_count: png_bytes.len() as u64,
    })
    .expect("valid png storage evidence");
    assert_eq!(content_type, AttachmentContentType::Png);
    assert_eq!(
        sanitize_display_filename(Some("..\\quarterly:image?.png"), content_type),
        "quarterly_image_.png"
    );

    let storage_root = unique_temp_root();
    let storage_path = attachment_storage_path(
        &storage_root,
        AttachmentStorageKey {
            account_id: ACCOUNT_ID,
            folder_id: "folder_inbox",
            message_id: "msg_1",
            attachment_id: "att_1",
            content_type,
        },
    )
    .expect("safe app-generated attachment path");
    assert!(storage_path.starts_with(&storage_root));
    assert_eq!(
        storage_path,
        storage_root
            .join(ACCOUNT_ID)
            .join("folder_inbox")
            .join("msg_1")
            .join("att_1.png")
    );

    assert!(attachment_storage_path(
        &storage_root,
        AttachmentStorageKey {
            account_id: ACCOUNT_ID,
            folder_id: "folder_inbox",
            message_id: "msg_1",
            attachment_id: "../att",
            content_type,
        },
    )
    .is_err());

    assert!(decide_attachment_preview(AttachmentPreviewRequest {
        content_type,
        byte_count: png_bytes.len() as u64,
        user_requested: false,
    })
    .is_err());

    let preview = decide_attachment_preview(AttachmentPreviewRequest {
        content_type,
        byte_count: png_bytes.len() as u64,
        user_requested: true,
    })
    .expect("explicit png preview");
    assert_eq!(preview.kind, PreviewKind::RasterImage);
    assert!(!preview.auto_open);
}

fn integration_message() -> Vec<u8> {
    concat!(
        "From: \"Ana\" <ana@example.com>\r\n",
        "To: \"Komorebi\" <integration@example.com>\r\n",
        "Subject: Komorebi integration search\r\n",
        "Message-ID: <integration-message@example.com>\r\n",
        "Date: Sat, 23 May 2026 12:00:00 +0000\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
        "\r\n",
        "--outer\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "Komorebi integration body for local cache search.\r\n",
        "\r\n",
        "--outer\r\n",
        "Content-Type: text/plain; name=\"../report.txt\"\r\n",
        "Content-Disposition: attachment; filename=\"../report.txt\"\r\n",
        "\r\n",
        "Attachment body\r\n",
        "--outer--\r\n",
    )
    .as_bytes()
    .to_vec()
}

fn unique_temp_root() -> PathBuf {
    env::temp_dir().join(format!("komorebi-rust-boundary-test-{}", process::id()))
}
