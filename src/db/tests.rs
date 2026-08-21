//! Database test suite.

use super::*;

#[test]
fn migrations_apply_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.db");
    let db = Database::open(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), 6);

    // Reopen applies nothing new.
    let db2 = Database::open(&path).unwrap();
    assert_eq!(db2.schema_version().unwrap(), 6);
}

#[test]
fn account_crud() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "me@example.org".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            imap_host: Some("imap.example.org".into()),
            imap_port: Some(993),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(db.list_accounts().unwrap().len(), 1);
    assert_eq!(
        db.get_account(&acc.id).unwrap().unwrap().email,
        "me@example.org"
    );
    db.delete_account(&acc.id).unwrap();
    assert!(db.list_accounts().unwrap().is_empty());
}

#[test]
fn folder_and_message_flow() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", Some("Inbox"), Some("inbox"), None)
        .unwrap();
    let id = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            imap_uid: Some(1),
            subject: Some("Hello".into()),
            sender_email: Some("x@y.test".into()),
            snippet: Some("hi there".into()),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();
    let rows = db.list_messages(&inbox.id, 50, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject.as_deref(), Some("Hello"));

    // Dedupe: same UID resolves to same message.
    let found = db.find_message_by_uid(&inbox.id, 1).unwrap().unwrap();
    assert_eq!(found, id);

    let detail = db.get_message_detail(&id).unwrap().unwrap();
    assert_eq!(detail.sender_email.as_deref(), Some("x@y.test"));

    db.mark_read(&id, true).unwrap();
    assert!(db.get_message_detail(&id).unwrap().unwrap().is_read);

    db.set_ai_fields(&id, Some("work"), Some("short summary"))
        .unwrap();
    let d = db.get_message_detail(&id).unwrap().unwrap();
    assert_eq!(d.ai_category.as_deref(), Some("work"));
}

#[test]
fn fts_search() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", None, None, None)
        .unwrap();
    db.insert_message(&MessageDraft {
        account_id: acc.id.clone(),
        folder_id: inbox.id.clone(),
        subject: Some("Quarterly report".into()),
        body_text: Some("the zebra crossed the river".into()),
        date_received: Some(1_700_000_000),
        ..Default::default()
    })
    .unwrap();
    let hits = db.search_messages("zebra", 10).unwrap();
    assert_eq!(hits.len(), 1);
    let none = db.search_messages("giraffe", 10).unwrap();
    assert!(none.is_empty());
}

#[test]
fn drafts_attachments_settings_ai() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", None, None, None)
        .unwrap();
    let msg = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            ..Default::default()
        })
        .unwrap();

    let draft_id = db
        .save_draft(&acc.id, Some("Hi"), Some("a@b.test"), Some("body"))
        .unwrap();
    assert_eq!(db.list_drafts().unwrap().len(), 1);
    db.delete_draft(&draft_id).unwrap();

    let att = db
        .insert_attachment(&msg, "file.txt", Some("text/plain"), b"hello bytes")
        .unwrap();
    assert_eq!(
        db.get_attachment_data(&att).unwrap().unwrap(),
        b"hello bytes"
    );
    assert_eq!(db.list_attachments(&msg).unwrap().len(), 1);

    db.set_setting("ui.density", "compact").unwrap();
    assert_eq!(db.get_setting("ui.density").unwrap().unwrap(), "compact");

    db.insert_ai_action(
        Some(&acc.id),
        Some(&msg),
        "summarize",
        Some("ollama"),
        Some("llama3.2"),
        "sum",
    )
    .unwrap();
    assert_eq!(db.list_ai_actions(&msg, 10).unwrap().len(), 1);
}

#[test]
fn folder_bulk_actions_and_delete() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
        .unwrap();
    let archive = db
        .upsert_folder(&acc.id, "Archive", None, Some("archive"), None)
        .unwrap();
    let trash = db
        .upsert_folder(&acc.id, "Trash", None, Some("trash"), None)
        .unwrap();
    let custom = db
        .upsert_folder(&acc.id, "Projects", None, None, None)
        .unwrap();

    for uid in 1..=3 {
        db.insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            imap_uid: Some(uid),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();
    }

    // Mark all read.
    db.mark_folder_read(&inbox.id).unwrap();
    assert!(
        db.list_messages(&inbox.id, 10, 0)
            .unwrap()
            .iter()
            .all(|m| m.is_read)
    );

    // Move all to archive.
    db.move_all_messages(&inbox.id, &archive.id).unwrap();
    assert_eq!(db.list_messages(&inbox.id, 10, 0).unwrap().len(), 0);
    assert_eq!(db.list_messages(&archive.id, 10, 0).unwrap().len(), 3);

    // Empty trash permanently deletes messages.
    db.move_all_messages(&archive.id, &trash.id).unwrap();
    db.delete_all_messages(&trash.id).unwrap();
    assert_eq!(db.list_messages(&trash.id, 10, 0).unwrap().len(), 0);

    // Deleting a folder cascades its messages.
    db.insert_message(&MessageDraft {
        account_id: acc.id.clone(),
        folder_id: custom.id.clone(),
        date_received: Some(1_700_000_000),
        ..Default::default()
    })
    .unwrap();
    db.delete_folder(&custom.id).unwrap();
    assert!(db.get_folder(&custom.id).unwrap().is_none());
}

#[test]
fn reminders_snooze_and_followup() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
        .unwrap();
    let msg = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            subject: Some("Hi".into()),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();

    let now = 1_700_000_000;
    db.upsert_reminder(&acc.id, &msg, "snooze", Some("tomorrow"), now + 86_400)
        .unwrap();
    assert_eq!(db.get_reminder(&msg).unwrap().unwrap().kind, "snooze");
    assert_eq!(db.list_snoozed_messages(now).unwrap().len(), 1);

    // Scheduling a follow-up overwrites the snooze (one reminder per message).
    db.upsert_reminder(&acc.id, &msg, "follow_up", Some("tomorrow"), now + 86_400)
        .unwrap();
    assert!(db.list_snoozed_messages(now).unwrap().is_empty());
    assert_eq!(db.get_reminder(&msg).unwrap().unwrap().kind, "follow_up");

    // An expired snooze is released lazily.
    db.upsert_reminder(&acc.id, &msg, "snooze", Some("later"), now - 1)
        .unwrap();
    assert_eq!(db.delete_expired_snoozes(now).unwrap(), 1);
    assert!(db.get_reminder(&msg).unwrap().is_none());

    // Deleting a message cascades to its reminder.
    db.upsert_reminder(&acc.id, &msg, "snooze", Some("later"), now + 10)
        .unwrap();
    db.delete_all_messages(&inbox.id).unwrap();
    assert!(db.get_reminder(&msg).unwrap().is_none());
}

#[test]
fn threading_groups_replies_by_in_reply_to() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let inbox = db
        .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
        .unwrap();
    let sent = db
        .upsert_folder(&acc.id, "Sent", None, Some("sent"), None)
        .unwrap();

    // Root message (inserted first so the reply can resolve to it).
    let root = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: sent.id.clone(),
            rfc_message_id: Some("<root@example.com>".into()),
            subject: Some("Hello".into()),
            sender_email: Some("me@example.com".into()),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();

    // Reply in another folder references the root.
    let reply = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            rfc_message_id: Some("<reply@example.com>".into()),
            in_reply_to: Some("<root@example.com>".into()),
            references_header: Some("<root@example.com>".into()),
            subject: Some("Re: Hello".into()),
            sender_email: Some("jane@example.com".into()),
            date_received: Some(1_700_000_100),
            ..Default::default()
        })
        .unwrap();

    let root_detail = db.get_message_detail(&root).unwrap().unwrap();
    let reply_detail = db.get_message_detail(&reply).unwrap().unwrap();
    assert_eq!(root_detail.thread_id.as_deref(), Some("root@example.com"));
    assert_eq!(root_detail.thread_id, reply_detail.thread_id);

    let tid = root_detail.thread_id.clone().unwrap();
    let thread = db.list_thread_messages(&tid).unwrap();
    assert_eq!(thread.len(), 2);
    assert_eq!(thread[0].id, root);
    assert_eq!(thread[1].id, reply);

    // A message with no identifiers stands alone.
    let solo = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            subject: Some("No ids".into()),
            date_received: Some(1_700_000_200),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        db.get_message_detail(&solo).unwrap().unwrap().thread_id,
        None
    );
}

#[test]
fn labels_crud_and_apply() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let folder = db
        .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
        .unwrap();
    let msg = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: folder.id.clone(),
            subject: Some("hello".into()),
            ..Default::default()
        })
        .unwrap();

    // Create + list
    let work = db.insert_label(&acc.id, "Work", Some("#6b9bd6")).unwrap();
    let personal = db.insert_label(&acc.id, "Personal", None).unwrap();
    let labels = db.list_labels(&acc.id).unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].name, "Personal"); // alphabetical
    assert_eq!(labels[1].name, "Work");
    assert_eq!(work.color.as_deref(), Some("#6b9bd6"));

    // Rename
    db.rename_label(&personal.id, "Home").unwrap();
    assert_eq!(db.list_labels(&acc.id).unwrap()[0].name, "Home");

    // Apply + list-message-labels + list-messages-by-label
    db.apply_label(&msg, &work.id).unwrap();
    db.apply_label(&msg, &work.id).unwrap(); // idempotent
    let msg_labels = db.list_message_labels(&msg).unwrap();
    assert_eq!(msg_labels.len(), 1);
    assert_eq!(msg_labels[0].name, "Work");
    let labeled = db.list_messages_by_label(&work.id, 100).unwrap();
    assert_eq!(labeled.len(), 1);
    assert_eq!(labeled[0].id, msg);

    // Remove
    db.remove_label(&msg, &work.id).unwrap();
    db.remove_label(&msg, &work.id).unwrap(); // idempotent
    assert!(db.list_message_labels(&msg).unwrap().is_empty());

    // Delete label cascades the join rows
    db.delete_label(&work.id).unwrap();
    assert_eq!(db.list_labels(&acc.id).unwrap().len(), 1);
}

#[test]
fn outbox_enqueue_list_and_reconcile() {
    let db = Database::in_memory().unwrap();
    let acc = db
        .insert_account(&AccountDraft {
            email: "a@b.test".into(),
            provider_kind: "custom".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })
        .unwrap();
    let folder = db
        .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
        .unwrap();
    let msg = db
        .insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: folder.id.clone(),
            imap_uid: Some(42),
            ..Default::default()
        })
        .unwrap();

    // Enqueue two mutations
    db.enqueue_mutation(&acc.id, &msg, 42, "move", Some("Archive"))
        .unwrap();
    db.enqueue_mutation(&acc.id, &msg, 42, "mark_read", None)
        .unwrap();
    assert_eq!(db.pending_mutation_count().unwrap(), 2);

    let pending = db.list_pending_mutations().unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].kind, "move");
    assert_eq!(pending[0].target_folder.as_deref(), Some("Archive"));
    assert_eq!(pending[1].kind, "mark_read");

    // Reconcile one
    db.mark_mutation_applied(&pending[0].id).unwrap();
    assert_eq!(db.pending_mutation_count().unwrap(), 1);

    // Purge applied (use a future timestamp so the applied_at < before)
    db.purge_applied_mutations(chrono::Utc::now().timestamp() + 3600)
        .unwrap();
    assert_eq!(db.pending_mutation_count().unwrap(), 1);

    // Reconcile the rest and purge all
    db.mark_mutation_applied(&pending[1].id).unwrap();
    assert_eq!(db.pending_mutation_count().unwrap(), 0);
    db.purge_applied_mutations(chrono::Utc::now().timestamp() + 3600)
        .unwrap();
    assert!(db.list_pending_mutations().unwrap().is_empty());
}
