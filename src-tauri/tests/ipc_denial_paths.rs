use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process,
};

use komorebi_lib::{
    account_connection::{
        AccountConnectionTestError, GenericAccountConnectionConfig, GenericAccountConnectionTester,
    },
    account_management::{
        create_account_from_connection, get_account_auth_status_from_connection,
        reauth_account_from_connection, remove_account_from_connection,
        test_account_connection_request_with_tester, update_account_settings_from_connection,
        AccountAuthStatusRequest, AccountCredentialStore, AccountLocalPreferences,
        AccountReauthRequest, AccountRemovalRequest, AccountSettingsUpdateRequest,
        AccountSetupRequest,
    },
    attachment_commands::{
        download_attachment_from_connection, preview_attachment_from_connection,
        save_attachment_from_connection, AttachmentCommandRequest,
    },
    attachment_safety::MAX_ATTACHMENT_BYTES,
    composer::{
        autosave_composer_draft_from_connection, list_recent_recipients_from_connection,
        stage_composer_attachment_from_connection, ComposerAddress, ComposerAttachmentDisposition,
        ComposerAttachmentStageRequest, ComposerDraftSaveRequest, RecentRecipientSearchRequest,
    },
    credentials::{CredentialError, CredentialSlot},
    external_links::{
        open_external_link_with_opener, ExternalLinkCommandError, ExternalLinkOpenRequest,
        ExternalLinkOpener,
    },
    ipc_validation::{reject_client_supplied_path, MAX_TEXT_FIELD_BYTES},
    mail_actions::{
        execute_mail_action, perform_mail_action_with_store, FolderActionKind, MailAction,
        MailActionCommand, MailActionError, MailActionFailure, MailActionStore,
        MailActionStoreError, MailActionTargetFolderKind, MessageActionKind, PerformMailActionKind,
        PerformMailActionRequest, RetryDecision, UserConfirmation,
    },
    mail_shell::{
        allow_remote_content_from_connection, search_mail_from_connection, MailSearchRequest,
        RemoteContentPreferenceRequest, RemoteContentPreferenceScope,
    },
    schema::apply_migrations,
    token_lifecycle::SecretString,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

const ACCOUNT_ID: &str = "acct_ipc";
const OTHER_ACCOUNT_ID: &str = "acct_other";
const DRAFT_ID: &str = "draft_ipc";
const INBOX_ID: &str = "folder_inbox";
const OTHER_FOLDER_ID: &str = "folder_other";
const MESSAGE_ID: &str = "msg_ipc";
const ATTACHMENT_ID: &str = "att_ipc";

#[test]
fn mail_search_command_rejects_sql_html_invalid_ids_unowned_scope_and_oversized_queries() {
    let connection = migrated_connection();
    seed_accounts_and_folders(&connection);

    for hostile_query in [
        "SELECT * FROM messages",
        "subject; DELETE FROM messages",
        "<img src=x onerror=alert(1)>",
    ] {
        let error = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: hostile_query.into(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .expect_err("hostile search query is rejected");

        assert!(!error.reason.contains(hostile_query));
    }

    let oversized_query = "q".repeat(513);
    let oversized_error = search_mail_from_connection(
        &connection,
        MailSearchRequest {
            query: oversized_query,
            account_id: None,
            folder_id: None,
            limit: Some(10),
        },
    )
    .expect_err("oversized search query is rejected");
    assert!(oversized_error.reason.contains("query exceeds 512 bytes"));

    let invalid_account = search_mail_from_connection(
        &connection,
        MailSearchRequest {
            query: "invoice".into(),
            account_id: Some("../secret".into()),
            folder_id: None,
            limit: Some(10),
        },
    )
    .expect_err("invalid account id is rejected");
    assert!(invalid_account.reason.contains("account_id is invalid"));

    let unknown_account = search_mail_from_connection(
        &connection,
        MailSearchRequest {
            query: "invoice".into(),
            account_id: Some("acct_missing".into()),
            folder_id: None,
            limit: Some(10),
        },
    )
    .expect_err("unknown account is rejected");
    assert_eq!(unknown_account.reason, "account was not found");

    let unowned_folder = search_mail_from_connection(
        &connection,
        MailSearchRequest {
            query: "invoice".into(),
            account_id: Some(ACCOUNT_ID.into()),
            folder_id: Some(OTHER_FOLDER_ID.into()),
            limit: Some(10),
        },
    )
    .expect_err("folder outside account scope is rejected");
    assert_eq!(unowned_folder.reason, "folder was not found for account");

    for path in [
        "C:\\Users\\Alice\\mail-cache.sqlite3",
        "..\\AppData\\Local\\Komorebi\\mail-cache.sqlite3",
        "\\\\server\\share\\mail-cache.sqlite3",
    ] {
        let error = reject_client_supplied_path("cache_path", path)
            .expect_err("frontend-supplied paths are rejected");
        assert_eq!(
            error.to_string(),
            "cache_path must be generated by the backend"
        );
    }
}

#[test]
fn remote_content_preference_command_rejects_invalid_scope_and_missing_intent() {
    let connection = migrated_connection();
    seed_accounts_and_folders(&connection);
    let app_data_dir = temp_app_data_dir("ipc-denial-remote");
    seed_message_and_attachment(&connection, &app_data_dir, b"notes");

    let no_intent = allow_remote_content_from_connection(
        &connection,
        remote_content_request(false, RemoteContentPreferenceScope::Message),
    )
    .expect_err("remote content preference requires user intent");
    assert_eq!(
        no_intent.reason,
        "remote content preference requires user intent"
    );

    let mut invalid_account = remote_content_request(true, RemoteContentPreferenceScope::Message);
    invalid_account.account_id = "../secret".into();
    let invalid_account_error = allow_remote_content_from_connection(&connection, invalid_account)
        .expect_err("invalid account id is rejected");
    assert!(invalid_account_error
        .reason
        .contains("account_id is invalid"));

    let mut wrong_folder = remote_content_request(true, RemoteContentPreferenceScope::Message);
    wrong_folder.folder_id = OTHER_FOLDER_ID.into();
    let wrong_folder_error = allow_remote_content_from_connection(&connection, wrong_folder)
        .expect_err("unowned folder is rejected");
    assert_eq!(
        wrong_folder_error.reason,
        "folder was not found for account"
    );

    let mut missing_message = remote_content_request(true, RemoteContentPreferenceScope::Message);
    missing_message.message_id = "msg_missing".into();
    let missing_message_error = allow_remote_content_from_connection(&connection, missing_message)
        .expect_err("unknown message is rejected");
    assert_eq!(
        missing_message_error.reason,
        "message was not found for folder"
    );

    let response = allow_remote_content_from_connection(
        &connection,
        remote_content_request(true, RemoteContentPreferenceScope::Sender),
    )
    .expect("owned sender preference is persisted");
    assert_eq!(response.sender_email.as_deref(), Some("sender@example.com"));

    fs::remove_dir_all(app_data_dir).expect("remove temp app data dir");
}

#[test]
fn external_link_command_rejects_missing_intent_and_unsafe_urls_before_opening() {
    let opener = RecordingExternalLinkOpener::default();

    let missing_intent = open_external_link_with_opener(
        ExternalLinkOpenRequest {
            url: "https://example.com/message".into(),
            user_requested: false,
        },
        &opener,
    )
    .expect_err("external link opening requires user intent");
    assert_eq!(
        missing_intent.reason,
        "external link opening requires explicit user intent"
    );

    for url in [
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///C:/Windows/win.ini",
        "C:\\Windows\\win.ini",
        "https://user:pass@example.com",
        "https://localhost/message",
        "mailto:person@example.com?body=hidden",
        "mailto:person@example.com?attach=file:///C:/secret.txt",
        "mailto:person@example.com%0D%0ABcc:other@example.com",
    ] {
        let error = open_external_link_with_opener(
            ExternalLinkOpenRequest {
                url: url.into(),
                user_requested: true,
            },
            &opener,
        )
        .expect_err("unsafe external link is rejected");
        assert!(!error.reason.contains(url));
    }

    let response = open_external_link_with_opener(
        ExternalLinkOpenRequest {
            url: "mailto:person@example.com?subject=Hello".into(),
            user_requested: true,
        },
        &opener,
    )
    .expect("safe mailto link opens after intent");
    assert_eq!(
        response.opened_url,
        "mailto:person@example.com?subject=Hello"
    );
    assert_eq!(
        opener.opened_urls.borrow().as_slice(),
        ["mailto:person@example.com?subject=Hello"]
    );
}

#[test]
fn composer_commands_reject_invalid_ids_unowned_attachments_unsafe_html_and_oversized_payloads() {
    let connection = migrated_connection();
    seed_accounts_and_folders(&connection);
    let app_data_dir = temp_app_data_dir("ipc-denial-composer");

    autosave_composer_draft_from_connection(&connection, draft_request())
        .expect("seed active draft");

    let mut invalid_account_request = draft_request();
    invalid_account_request.account_id = "../secret".into();
    let invalid_account =
        autosave_composer_draft_from_connection(&connection, invalid_account_request)
            .expect_err("invalid account id is rejected");
    assert!(invalid_account.reason.contains("account_id is invalid"));

    let mut oversized_subject_request = draft_request();
    oversized_subject_request.subject = "s".repeat(MAX_TEXT_FIELD_BYTES + 1);
    let oversized_subject =
        autosave_composer_draft_from_connection(&connection, oversized_subject_request)
            .expect_err("oversized subject is rejected");
    assert!(oversized_subject.reason.contains("subject exceeds"));

    let mut unsafe_html_request = draft_request();
    unsafe_html_request.body_html = Some("<img src=x onerror=alert(1)>".into());
    let unsafe_html = autosave_composer_draft_from_connection(&connection, unsafe_html_request)
        .expect_err("unsanitized composer HTML is rejected");
    assert!(unsafe_html.reason.contains("body_html is not sanitized"));

    let mut missing_attachment_request = draft_request();
    missing_attachment_request.attachment_ids = vec!["att_missing".into()];
    let missing_attachment =
        autosave_composer_draft_from_connection(&connection, missing_attachment_request)
            .expect_err("unowned attachment id is rejected");
    assert_eq!(
        missing_attachment.reason,
        "composer attachment was not found"
    );

    let oversized_attachment = stage_composer_attachment_from_connection(
        &connection,
        &app_data_dir,
        ComposerAttachmentStageRequest {
            account_id: ACCOUNT_ID.into(),
            draft_id: DRAFT_ID.into(),
            display_filename: "large.txt".into(),
            content_type: Some("text/plain".into()),
            disposition: ComposerAttachmentDisposition::Attachment,
            bytes: vec![b'x'; MAX_ATTACHMENT_BYTES as usize + 1],
        },
    )
    .expect_err("oversized attachment bytes are rejected");
    assert!(oversized_attachment.reason.contains("attachment exceeds"));

    let mut invalid_recent = RecentRecipientSearchRequest {
        account_id: "../secret".into(),
        query: "recipient".into(),
        limit: Some(5),
    };
    let invalid_recent_error =
        list_recent_recipients_from_connection(&connection, invalid_recent.clone())
            .expect_err("invalid recent-recipient account id is rejected");
    assert!(invalid_recent_error
        .reason
        .contains("account_id is invalid"));

    invalid_recent.account_id = "acct_missing".into();
    let unknown_recent =
        list_recent_recipients_from_connection(&connection, invalid_recent.clone())
            .expect_err("unknown recent-recipient account is rejected");
    assert_eq!(unknown_recent.reason, "account was not found");

    invalid_recent.account_id = ACCOUNT_ID.into();
    invalid_recent.query = "r".repeat(257);
    let oversized_recent = list_recent_recipients_from_connection(&connection, invalid_recent)
        .expect_err("oversized recent-recipient query is rejected");
    assert!(oversized_recent
        .reason
        .contains("recipient_query exceeds 256 bytes"));

    fs::remove_dir_all(app_data_dir).expect("remove temp app data dir");
}

#[test]
fn attachment_commands_reject_invalid_scope_corrupt_content_and_missing_intent() {
    let connection = migrated_connection();
    seed_accounts_and_folders(&connection);
    let app_data_dir = temp_app_data_dir("ipc-denial-attachments");
    let save_root = temp_app_data_dir("ipc-denial-attachment-save");
    seed_message_and_attachment(&connection, &app_data_dir, b"safe attachment notes");

    let no_intent =
        preview_attachment_from_connection(&connection, &app_data_dir, attachment_request(false))
            .expect_err("preview requires explicit user intent");
    assert_eq!(no_intent.reason, "attachment preview requires user intent");

    let mut invalid_id = attachment_request(true);
    invalid_id.attachment_id = "../secret".into();
    let invalid_id_error =
        download_attachment_from_connection(&connection, &app_data_dir, invalid_id)
            .expect_err("invalid attachment id is rejected");
    assert!(invalid_id_error.reason.contains("attachment_id is invalid"));

    let mut wrong_scope = attachment_request(true);
    wrong_scope.folder_id = OTHER_FOLDER_ID.into();
    let wrong_scope_error =
        download_attachment_from_connection(&connection, &app_data_dir, wrong_scope)
            .expect_err("unowned attachment scope is rejected");
    assert_eq!(wrong_scope_error.reason, "attachment was not found");

    let preview =
        preview_attachment_from_connection(&connection, &app_data_dir, attachment_request(true))
            .expect("owned text attachment previews");
    assert_eq!(preview.text.as_deref(), Some("safe attachment notes"));
    assert!(!preview.auto_open);

    let saved = save_attachment_from_connection(
        &connection,
        &app_data_dir,
        &save_root,
        attachment_request(true),
    )
    .expect("owned attachment saves under selected test scope");
    assert_eq!(saved.saved_filename, "notes.txt");
    assert_eq!(
        fs::read(save_root.join("notes.txt")).unwrap(),
        b"safe attachment notes"
    );

    connection
        .execute(
            "UPDATE attachments
             SET sha256 = ?1
             WHERE id = ?2",
            params![
                "0000000000000000000000000000000000000000000000000000000000000000",
                ATTACHMENT_ID
            ],
        )
        .expect("corrupt stored hash");
    let corrupt =
        download_attachment_from_connection(&connection, &app_data_dir, attachment_request(true))
            .expect_err("hash mismatch is rejected before export");
    assert_eq!(corrupt.reason, "attachment hash mismatch");

    connection
        .execute(
            "UPDATE attachments
             SET sha256 = ?1,
                 storage_relative_path = '../outside.txt'
             WHERE id = ?2",
            params![sha256_hex(b"safe attachment notes"), ATTACHMENT_ID],
        )
        .expect("corrupt stored path");
    let traversal =
        download_attachment_from_connection(&connection, &app_data_dir, attachment_request(true))
            .expect_err("stored traversal path is rejected");
    assert_eq!(traversal.reason, "attachment path is outside app data");

    connection
        .execute(
            "UPDATE attachments
             SET storage_relative_path = NULL
             WHERE id = ?1",
            [ATTACHMENT_ID],
        )
        .expect("mark metadata-only");
    let unavailable =
        download_attachment_from_connection(&connection, &app_data_dir, attachment_request(true))
            .expect_err("metadata-only attachment cannot be exported");
    assert_eq!(unavailable.reason, "attachment content is unavailable");

    fs::remove_dir_all(app_data_dir).expect("remove temp app data dir");
    fs::remove_dir_all(save_root).expect("remove temp save dir");
}

#[test]
fn server_affecting_mail_actions_reject_unauthorized_operations_before_mutation() {
    let mut store = RecordingMailActionStore::seeded();

    assert_action_failure(
        execute_mail_action(
            &mut store,
            message_command(
                "acct_missing",
                INBOX_ID,
                MESSAGE_ID,
                MessageActionKind::MarkRead { read: true },
            ),
        ),
        MailActionError::UnknownAccount,
    );
    assert_eq!(store.mutation_count, 0);

    assert_action_failure(
        execute_mail_action(
            &mut store,
            message_command(
                ACCOUNT_ID,
                OTHER_FOLDER_ID,
                MESSAGE_ID,
                MessageActionKind::MarkRead { read: true },
            ),
        ),
        MailActionError::UnknownFolder {
            folder_id: OTHER_FOLDER_ID.into(),
        },
    );
    assert_eq!(store.mutation_count, 0);

    assert_action_failure(
        execute_mail_action(
            &mut store,
            message_command(
                ACCOUNT_ID,
                INBOX_ID,
                "msg_missing",
                MessageActionKind::MarkRead { read: true },
            ),
        ),
        MailActionError::UnknownMessage {
            folder_id: INBOX_ID.into(),
            message_id: "msg_missing".into(),
        },
    );
    assert_eq!(store.mutation_count, 0);

    let invalid_target = execute_mail_action(
        &mut store,
        message_command(
            ACCOUNT_ID,
            INBOX_ID,
            MESSAGE_ID,
            MessageActionKind::Move {
                target_folder_id: "../trash",
            },
        ),
    )
    .expect_err("invalid target folder id is rejected");
    assert!(matches!(
        invalid_target.error,
        MailActionError::IpcValidation(_)
    ));
    assert_eq!(store.mutation_count, 0);

    assert_action_failure(
        execute_mail_action(
            &mut store,
            MailActionCommand {
                account_id: ACCOUNT_ID,
                action: MailAction::Message {
                    folder_id: INBOX_ID,
                    message_id: MESSAGE_ID,
                    kind: MessageActionKind::DeleteToTrash {
                        trash_folder_id: "trash",
                    },
                },
                confirmation: UserConfirmation::NotConfirmed,
                attempt: 1,
            },
        ),
        MailActionError::ConfirmationRequired,
    );
    assert_eq!(store.mutation_count, 0);

    assert_action_failure(
        execute_mail_action(
            &mut store,
            MailActionCommand {
                account_id: ACCOUNT_ID,
                action: MailAction::Folder {
                    folder_id: INBOX_ID,
                    kind: FolderActionKind::ExpungeDeleted,
                },
                confirmation: UserConfirmation::NotConfirmed,
                attempt: 1,
            },
        ),
        MailActionError::ConfirmationRequired,
    );
    assert_eq!(store.mutation_count, 0);
}

#[test]
fn mail_action_ipc_command_rejects_missing_intent_bad_ids_unowned_scope_and_confirmation_gaps() {
    let mut store = RecordingMailActionStore::seeded();

    let no_intent = perform_mail_action_with_store(
        &mut store,
        mail_action_request(
            ACCOUNT_ID,
            INBOX_ID,
            Some(MESSAGE_ID),
            PerformMailActionKind::MarkRead { read: true },
            false,
            false,
        ),
    )
    .expect_err("mail actions require explicit user intent");
    assert_eq!(
        no_intent.reason,
        "mail action requires explicit user intent"
    );
    assert_eq!(store.mutation_count, 0);

    let invalid_id = perform_mail_action_with_store(
        &mut store,
        mail_action_request(
            "../secret",
            INBOX_ID,
            Some(MESSAGE_ID),
            PerformMailActionKind::MarkRead { read: true },
            false,
            true,
        ),
    )
    .expect_err("invalid ids are rejected before mutation");
    assert!(invalid_id.reason.contains("account_id is invalid"));
    assert_eq!(store.mutation_count, 0);

    let unowned_folder = perform_mail_action_with_store(
        &mut store,
        mail_action_request(
            ACCOUNT_ID,
            OTHER_FOLDER_ID,
            Some(MESSAGE_ID),
            PerformMailActionKind::MarkRead { read: true },
            false,
            true,
        ),
    )
    .expect_err("unowned folder is rejected before mutation");
    assert_eq!(unowned_folder.reason, "folder was not found");
    assert_eq!(store.mutation_count, 0);

    let missing_confirmation = perform_mail_action_with_store(
        &mut store,
        mail_action_request(
            ACCOUNT_ID,
            INBOX_ID,
            Some(MESSAGE_ID),
            PerformMailActionKind::DeleteToTrash,
            false,
            true,
        ),
    )
    .expect_err("delete-to-trash requires confirmation");
    assert_eq!(
        missing_confirmation.reason,
        "mail action requires confirmation"
    );
    assert_eq!(store.mutation_count, 0);

    let confirmed_delete = perform_mail_action_with_store(
        &mut store,
        mail_action_request(
            ACCOUNT_ID,
            INBOX_ID,
            Some(MESSAGE_ID),
            PerformMailActionKind::DeleteToTrash,
            true,
            true,
        ),
    )
    .expect("confirmed delete-to-trash is allowed");
    assert!(confirmed_delete.undo.is_some());
    assert_eq!(store.mutation_count, 1);
}

#[test]
fn account_management_commands_reject_bad_auth_ids_payloads_and_credential_failures() {
    let connection = migrated_connection();
    let mut credentials = RecordingAccountCredentialStore::default();

    let mut gmail_password = account_setup_request();
    gmail_password.provider_kind = "gmail".into();
    gmail_password.auth_kind = "manual_password".into();
    gmail_password.imap_host = None;
    gmail_password.imap_port = None;
    gmail_password.imap_security = None;
    gmail_password.smtp_host = None;
    gmail_password.smtp_port = None;
    gmail_password.smtp_security = None;

    let unsupported_auth =
        create_account_from_connection(&connection, &mut credentials, gmail_password)
            .expect_err("major provider password auth is rejected");
    assert!(unsupported_auth.reason.contains("Gmail requires OAuth2"));
    assert!(credentials.secrets.is_empty());
    assert_eq!(account_count(&connection), 0);

    let mut invalid_id = account_setup_request();
    invalid_id.account_id = Some("../secret".into());
    let invalid_id_error =
        create_account_from_connection(&connection, &mut credentials, invalid_id)
            .expect_err("invalid account id is rejected");
    assert!(invalid_id_error.reason.contains("account_id is invalid"));
    assert!(credentials.secrets.is_empty());

    let mut oversized = account_setup_request();
    oversized.display_name = Some("x".repeat(257));
    let oversized_error = create_account_from_connection(&connection, &mut credentials, oversized)
        .expect_err("oversized display name is rejected");
    assert!(oversized_error.reason.contains("display_name exceeds"));
    assert!(credentials.secrets.is_empty());

    let mut plaintext_transport = account_setup_request();
    plaintext_transport.smtp_security = Some("none".into());
    let plaintext_error =
        create_account_from_connection(&connection, &mut credentials, plaintext_transport)
            .expect_err("plaintext mail transport is rejected");
    assert!(plaintext_error
        .reason
        .contains("smtp_security must use TLS or STARTTLS"));
    assert!(credentials.secrets.is_empty());

    credentials.fail_writes = true;
    let credential_error =
        create_account_from_connection(&connection, &mut credentials, account_setup_request())
            .expect_err("credential-store failure prevents metadata insert");
    assert!(credential_error.reason.contains("credential write failed"));
    assert_eq!(account_count(&connection), 0);
    credentials.fail_writes = false;

    create_account_from_connection(&connection, &mut credentials, account_setup_request())
        .expect("create account after denial checks");

    let settings_response = update_account_settings_from_connection(
        &connection,
        AccountSettingsUpdateRequest {
            account_id: ACCOUNT_ID.into(),
            display_name: Some("Inbox Owner".into()),
            signature_html: "<p>Regards</p>".into(),
            local_preferences: AccountLocalPreferences {
                include_in_unified_inbox: false,
            },
        },
    )
    .expect("account settings update succeeds");
    assert_eq!(
        settings_response.local_preferences,
        AccountLocalPreferences {
            include_in_unified_inbox: false
        }
    );

    let mut invalid_settings_id = AccountSettingsUpdateRequest {
        account_id: "../secret".into(),
        display_name: Some("Inbox Owner".into()),
        signature_html: "<p>Regards</p>".into(),
        local_preferences: AccountLocalPreferences::default(),
    };
    let invalid_settings_error =
        update_account_settings_from_connection(&connection, invalid_settings_id.clone())
            .expect_err("settings invalid account id is rejected");
    assert!(invalid_settings_error
        .reason
        .contains("account_id is invalid"));

    invalid_settings_id.account_id = "acct_missing".into();
    let unknown_settings_error =
        update_account_settings_from_connection(&connection, invalid_settings_id.clone())
            .expect_err("settings unknown account is rejected");
    assert_eq!(unknown_settings_error.reason, "account was not found");

    invalid_settings_id.account_id = ACCOUNT_ID.into();
    invalid_settings_id.display_name = Some("x".repeat(257));
    let oversized_settings_error =
        update_account_settings_from_connection(&connection, invalid_settings_id)
            .expect_err("settings oversized display name is rejected");
    assert!(oversized_settings_error
        .reason
        .contains("display_name exceeds"));

    let bad_reauth = reauth_account_from_connection(
        &connection,
        &mut credentials,
        AccountReauthRequest {
            account_id: "..\\secret".into(),
            secret: "replacement".into(),
        },
    )
    .expect_err("reauth invalid id is rejected");
    assert!(bad_reauth.reason.contains("account_id is invalid"));

    let status = get_account_auth_status_from_connection(
        &connection,
        &mut credentials,
        AccountAuthStatusRequest {
            account_id: ACCOUNT_ID.into(),
        },
    )
    .expect("status returns redacted credential presence");
    assert!(status.credential_present);

    let invalid_removal = remove_account_from_connection(
        &connection,
        &mut credentials,
        AccountRemovalRequest {
            account_id: "acct_missing".into(),
            delete_local_cache: true,
        },
    )
    .expect_err("unknown account removal is rejected");
    assert_eq!(invalid_removal.reason, "account was not found");

    let mut connection_tester = RecordingGenericConnectionTester::default();
    let config_check = test_account_connection_request_with_tester(
        account_setup_request(),
        &mut connection_tester,
    )
    .expect("live generic setup validates");
    assert_eq!(config_check.status, "connection-ready");
    assert_eq!(connection_tester.calls.len(), 1);

    let mut failed_connection_tester = RecordingGenericConnectionTester {
        error: Some(AccountConnectionTestError::AuthenticationRejected { protocol: "IMAP" }),
        ..Default::default()
    };
    let live_login_error = test_account_connection_request_with_tester(
        account_setup_request(),
        &mut failed_connection_tester,
    )
    .expect_err("live login failure is rejected");
    assert_eq!(live_login_error.reason, "IMAP authentication failed");
}

fn migrated_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open disposable sqlite database");
    apply_migrations(&mut connection).expect("apply migrations");
    connection
}

fn seed_accounts_and_folders(connection: &Connection) {
    for (account_id, email) in [
        (ACCOUNT_ID, "ipc@example.com"),
        (OTHER_ACCOUNT_ID, "other-ipc@example.com"),
    ] {
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
                 VALUES (?1, 'generic_imap_smtp', ?2, ?2, ?2, 'app_password', ?3, 'imap.example.com', 993, 'tls', 'smtp.example.com', 587, 'starttls', 'connected')",
                params![account_id, email, format!("cred_{account_id}")],
            )
            .expect("seed account");
    }

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
             VALUES (?1, ?2, ?1, ?1, ?1, 'inbox', 1, 2, 0, 0, 'idle')",
            params![INBOX_ID, ACCOUNT_ID],
        )
        .expect("seed primary folder");
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
             VALUES (?1, ?2, ?1, ?1, ?1, 'inbox', 1, 2, 0, 0, 'idle')",
            params![OTHER_FOLDER_ID, OTHER_ACCOUNT_ID],
        )
        .expect("seed other folder");
}

fn draft_request() -> ComposerDraftSaveRequest {
    ComposerDraftSaveRequest {
        account_id: ACCOUNT_ID.into(),
        draft_id: DRAFT_ID.into(),
        subject: "IPC draft".into(),
        to: vec![ComposerAddress {
            name: Some("Recipient".into()),
            address: "recipient@example.com".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        body_text: Some("Plain body".into()),
        body_html: Some("<p>Plain body</p>".into()),
        attachment_ids: Vec::new(),
    }
}

fn attachment_request(user_requested: bool) -> AttachmentCommandRequest {
    AttachmentCommandRequest {
        account_id: ACCOUNT_ID.into(),
        folder_id: INBOX_ID.into(),
        message_id: MESSAGE_ID.into(),
        attachment_id: ATTACHMENT_ID.into(),
        user_requested,
    }
}

fn remote_content_request(
    user_requested: bool,
    scope: RemoteContentPreferenceScope,
) -> RemoteContentPreferenceRequest {
    RemoteContentPreferenceRequest {
        account_id: ACCOUNT_ID.into(),
        folder_id: INBOX_ID.into(),
        message_id: MESSAGE_ID.into(),
        scope,
        user_requested,
    }
}

fn seed_message_and_attachment(connection: &Connection, app_data_dir: &Path, bytes: &[u8]) {
    connection
        .execute(
            "INSERT INTO messages (
               id,
               account_id,
               folder_id,
               imap_uid,
               uid_validity,
               subject,
               sender_email,
               body_text
             )
             VALUES (?1, ?2, ?3, 1, 1, 'Attachment IPC', 'sender@example.com', 'Body')",
            params![MESSAGE_ID, ACCOUNT_ID, INBOX_ID],
        )
        .expect("seed message");

    let relative_path =
        format!("attachments/{ACCOUNT_ID}/{INBOX_ID}/{MESSAGE_ID}/{ATTACHMENT_ID}.txt");
    let full_path = app_data_dir.join(&relative_path);
    fs::create_dir_all(full_path.parent().expect("attachment parent"))
        .expect("create attachment storage");
    fs::write(&full_path, bytes).expect("write attachment bytes");

    connection
        .execute(
            "INSERT INTO attachments (
               id,
               account_id,
               folder_id,
               message_id,
               content_id,
               display_filename,
               canonical_mime,
               content_type,
               size_bytes,
               storage_relative_path,
               sha256,
               disposition
             )
             VALUES (?1, ?2, ?3, ?4, NULL, 'notes.exe', 'text/plain', 'text/plain', ?5, ?6, ?7, 'attachment')",
            params![
                ATTACHMENT_ID,
                ACCOUNT_ID,
                INBOX_ID,
                MESSAGE_ID,
                bytes.len() as i64,
                relative_path,
                sha256_hex(bytes)
            ],
        )
        .expect("seed attachment");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::new();
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn account_setup_request() -> AccountSetupRequest {
    AccountSetupRequest {
        account_id: Some(ACCOUNT_ID.into()),
        provider_kind: "generic_imap_smtp".into(),
        display_email: "ipc@example.com".into(),
        display_name: Some("IPC".into()),
        username: "ipc@example.com".into(),
        auth_kind: "app_password".into(),
        secret: "app-password".into(),
        imap_host: Some("imap.example.com".into()),
        imap_port: Some(993),
        imap_security: Some("tls".into()),
        smtp_host: Some("smtp.example.com".into()),
        smtp_port: Some(587),
        smtp_security: Some("starttls".into()),
    }
}

fn account_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .expect("count accounts")
}

fn temp_app_data_dir(label: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("komorebi-{label}-{}", process::id()));
    fs::create_dir_all(&root).expect("create temp app data dir");
    root
}

fn message_command<'a>(
    account_id: &'a str,
    folder_id: &'a str,
    message_id: &'a str,
    kind: MessageActionKind<'a>,
) -> MailActionCommand<'a> {
    MailActionCommand {
        account_id,
        action: MailAction::Message {
            folder_id,
            message_id,
            kind,
        },
        confirmation: UserConfirmation::Confirmed,
        attempt: 1,
    }
}

fn mail_action_request(
    account_id: &str,
    folder_id: &str,
    message_id: Option<&str>,
    action: PerformMailActionKind,
    confirmed: bool,
    user_requested: bool,
) -> PerformMailActionRequest {
    PerformMailActionRequest {
        account_id: account_id.into(),
        folder_id: folder_id.into(),
        message_id: message_id.map(str::to_string),
        action,
        confirmed,
        user_requested,
        attempt: None,
    }
}

fn assert_action_failure(
    result: Result<komorebi_lib::mail_actions::MailActionReceipt, MailActionFailure>,
    expected_error: MailActionError,
) {
    let failure = result.expect_err("mail action must be denied");
    assert_eq!(failure.error, expected_error);
    assert_eq!(failure.retry, RetryDecision::NotRetryable);
}

#[derive(Default)]
struct RecordingMailActionStore {
    accounts: HashSet<String>,
    folders: HashMap<String, HashSet<String>>,
    messages: HashSet<(String, String, String)>,
    mutation_count: usize,
}

impl RecordingMailActionStore {
    fn seeded() -> Self {
        Self {
            accounts: HashSet::from([ACCOUNT_ID.into()]),
            folders: HashMap::from([(
                ACCOUNT_ID.into(),
                HashSet::from([INBOX_ID.into(), "trash".into()]),
            )]),
            messages: HashSet::from([(ACCOUNT_ID.into(), INBOX_ID.into(), MESSAGE_ID.into())]),
            mutation_count: 0,
        }
    }
}

impl MailActionStore for RecordingMailActionStore {
    fn account_exists(&self, account_id: &str) -> bool {
        self.accounts.contains(account_id)
    }

    fn folder_exists(&self, account_id: &str, folder_id: &str) -> bool {
        self.folders
            .get(account_id)
            .is_some_and(|folders| folders.contains(folder_id))
    }

    fn message_exists(&self, account_id: &str, folder_id: &str, message_id: &str) -> bool {
        self.messages.contains(&(
            account_id.to_string(),
            folder_id.to_string(),
            message_id.to_string(),
        ))
    }

    fn folder_id_for_kind(
        &self,
        account_id: &str,
        kind: MailActionTargetFolderKind,
    ) -> Option<String> {
        let folder_id = match kind {
            MailActionTargetFolderKind::Archive => "archive",
            MailActionTargetFolderKind::Trash => "trash",
        };
        self.folder_exists(account_id, folder_id)
            .then(|| folder_id.to_string())
    }

    fn set_seen(
        &mut self,
        _account_id: &str,
        _folder_id: &str,
        _message_id: &str,
        _read: bool,
    ) -> Result<(), MailActionStoreError> {
        self.mutation_count += 1;
        Ok(())
    }

    fn move_message(
        &mut self,
        _account_id: &str,
        _from_folder_id: &str,
        _message_id: &str,
        _to_folder_id: &str,
    ) -> Result<(), MailActionStoreError> {
        self.mutation_count += 1;
        Ok(())
    }

    fn expunge_deleted(
        &mut self,
        _account_id: &str,
        _folder_id: &str,
    ) -> Result<usize, MailActionStoreError> {
        self.mutation_count += 1;
        Ok(1)
    }
}

#[derive(Default)]
struct RecordingExternalLinkOpener {
    opened_urls: RefCell<Vec<String>>,
}

impl ExternalLinkOpener for RecordingExternalLinkOpener {
    fn open_external_link(&self, url: &str) -> Result<(), ExternalLinkCommandError> {
        self.opened_urls.borrow_mut().push(url.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingAccountCredentialStore {
    secrets: HashMap<(String, CredentialSlot), String>,
    fail_writes: bool,
}

impl AccountCredentialStore for RecordingAccountCredentialStore {
    fn write_account_secret(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        if self.fail_writes {
            return Err(CredentialError::WindowsUnavailable {
                operation: "write",
                code: 5,
            });
        }

        self.secrets.insert(
            (account_id.to_string(), slot),
            secret.expose_secret().to_string(),
        );
        Ok(())
    }

    fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), CredentialError> {
        self.secrets
            .retain(|(stored_account_id, _), _| stored_account_id != account_id);
        Ok(())
    }

    fn account_secret_exists(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<bool, CredentialError> {
        Ok(self.secrets.contains_key(&(account_id.to_string(), slot)))
    }
}

#[derive(Default)]
struct RecordingGenericConnectionTester {
    calls: Vec<GenericAccountConnectionConfig>,
    error: Option<AccountConnectionTestError>,
}

impl GenericAccountConnectionTester for RecordingGenericConnectionTester {
    fn test_generic_account(
        &mut self,
        config: &GenericAccountConnectionConfig,
    ) -> Result<(), AccountConnectionTestError> {
        self.calls.push(config.clone());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        Ok(())
    }
}
