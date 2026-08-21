//! Send and AI job execution, run on the background executor.

use super::*;

/// Send one message through the account's SMTP server, or record a local
/// sent copy for accounts without SMTP settings (the local demo account).
/// Runs off the UI thread.
#[allow(clippy::too_many_arguments)]
pub(crate) fn perform_send(
    db: &Database,
    account: &Account,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    attachments: &[mail::smtp::Attachment],
) -> anyhow::Result<()> {
    if account.smtp_host.is_some() {
        return mail::send_message(db, account, to, cc, subject, body, attachments);
    }
    // Local-only sent copy (no SMTP server configured).
    let sent = db
        .list_folders(&account.id)?
        .into_iter()
        .find(|f| f.special_use.as_deref() == Some("sent"))
        .or_else(|| db.list_folders(&account.id).ok()?.into_iter().next());
    if let Some(sent) = sent {
        let now = chrono::Utc::now().timestamp();
        let message_id = db.insert_message(&crate::db::MessageDraft {
            account_id: account.id.clone(),
            folder_id: sent.id.clone(),
            subject: Some(subject.to_string()),
            sender_email: Some(account.email.clone()),
            to_addresses: serde_json::to_string(to).ok(),
            cc_addresses: serde_json::to_string(cc).ok(),
            bcc_addresses: serde_json::to_string(bcc).ok(),
            date_received: Some(now),
            is_read: true,
            body_text: Some(body.to_string()),
            snippet: Some(mail::mime::make_snippet(body)),
            has_attachments: !attachments.is_empty(),
            ..Default::default()
        })?;
        for attachment in attachments {
            db.insert_attachment(
                &message_id,
                &attachment.filename,
                Some(&attachment.content_type),
                &attachment.data,
            )?;
        }
    }
    Ok(())
}

/// Run one AI action and persist the result to the cache.
#[allow(clippy::too_many_arguments)]
pub(crate) fn perform_ai(
    ai: &Ai,
    db: &Database,
    kind: AiKind,
    account_id: &str,
    message_id: &str,
    sender: Option<&str>,
    subject: Option<&str>,
    body: &str,
    tone: &str,
) -> anyhow::Result<String> {
    let (output, category) = match kind {
        AiKind::Summarize => {
            // `body` is the pre-formatted conversation text for summarize.
            let text = ai.summarize_thread(body)?;
            (text, None)
        }
        AiKind::DraftReply => {
            let text = ai.draft_reply(sender, subject, body, tone)?;
            (text, None)
        }
        AiKind::Polish => {
            let text = ai.polish(body, tone)?;
            (text, None)
        }
        AiKind::Triage => {
            let word = ai.triage(sender, subject, body)?;
            let category = Category::parse(&word).unwrap_or(Category::Updates);
            (format!("Category: {}", category.label()), Some(category))
        }
        AiKind::FollowUp => anyhow::bail!("follow-up is scheduled locally, not via an AI call"),
    };

    ai::record_and_apply(
        db,
        Some(account_id),
        Some(message_id),
        kind.as_str(),
        ai.config(),
        &output,
    )?;
    if let Some(category) = category {
        ai::apply_triage(db, message_id, category)?;
    } else if matches!(kind, AiKind::Summarize) {
        ai::apply_summary(db, message_id, &output)?;
    }
    Ok(output)
}
