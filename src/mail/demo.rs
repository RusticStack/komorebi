//! Opt-in demo mailbox seeding (local-only account with fixture mail).

use super::*;

/// Create a local demo account so the shell works with zero configuration
/// (opt-in from onboarding).
pub fn seed_local_demo(db: &Database) -> Result<()> {
    let account = match db
        .list_accounts()?
        .into_iter()
        .find(|a| a.provider_kind == "local")
    {
        Some(account) => account,
        None => db.insert_account(&crate::db::AccountDraft {
            email: "haruto.sato@komorebi.com".into(),
            display_name: Some("Haruto Sato".into()),
            provider_kind: "local".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })?,
    };

    // Standard folders are upserted idempotently so existing demo databases
    // gain any newly introduced folders on the next launch.
    let inbox = db.upsert_folder(&account.id, "INBOX", Some("Inbox"), Some("inbox"), None)?;
    let sent = db.upsert_folder(&account.id, "Sent", Some("Sent"), Some("sent"), None)?;
    let drafts = db.upsert_folder(&account.id, "Drafts", Some("Drafts"), Some("drafts"), None)?;
    let archive = db.upsert_folder(
        &account.id,
        "Archive",
        Some("Archive"),
        Some("archive"),
        None,
    )?;
    let trash = db.upsert_folder(&account.id, "Trash", Some("Trash"), Some("trash"), None)?;
    let _junk = db.upsert_folder(&account.id, "Junk", Some("Junk"), Some("junk"), None)?;

    // Upgrade path: the original minimal seed is replaced with the rich demo
    // set (matches the intended product experience). Once present, it is stable.
    let existing = db.list_messages(&inbox.id, 1000, 0)?;
    let has_rich = existing
        .iter()
        .any(|m| m.subject.as_deref() == Some("Re: Partnership Proposal"));
    // Re-seed older demo databases that predate threaded conversations.
    let sent_existing = db.list_messages(&sent.id, 1000, 0)?;
    let has_thread_roots = sent_existing
        .iter()
        .any(|m| m.subject.as_deref() == Some("Partnership Proposal"));
    if has_rich && has_thread_roots {
        return Ok(());
    }
    for folder in [&inbox, &sent, &drafts, &archive, &trash] {
        db.delete_all_messages(&folder.id)?;
    }

    struct DemoMsg<'a> {
        sender_name: &'a str,
        sender_email: &'a str,
        subject: &'a str,
        body: &'a str,
        label: &'a str,
        flagged: bool,
        unread: bool,
        seconds_ago: i64,
        attachments: &'a [(&'a str, &'a str)],
        message_id: &'a str,
        in_reply_to: Option<&'a str>,
    }

    let now = Utc::now().timestamp();

    // Sent roots so the demo has real multi-message conversations.
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("partnership-root@komorebi.mail".into()),
        subject: Some("Partnership Proposal".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some(
            "Hi Isabella, please find the partnership proposal attached. I'd welcome your feedback on scope and timeline."
                .into(),
        ),
        date_received: Some(now - 2 * 3600),
        is_read: true,
        ..Default::default()
    })?;
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("landing-root@komorebi.mail".into()),
        subject: Some("Feedback on Landing Page".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some(
            "Hi Ava, here's the latest landing page. Keen to hear your thoughts on the palette and hero."
                .into(),
        ),
        date_received: Some(now - (2 * 86400 + 3600)),
        is_read: true,
        ..Default::default()
    })?;

    let samples: [DemoMsg; 9] = [
        DemoMsg {
            sender_name: "Isabella Martinez",
            sender_email: "isabella.martinez@company.com",
            subject: "Re: Partnership Proposal",
            message_id: "isabella-partnership@company.com",
            in_reply_to: Some("partnership-root@komorebi.mail"),
            body: "Hi Haruto,\n\nThanks for the detailed proposal. I'm excited about the potential partnership with Komorebi.\n\nA few questions before we move forward:\n\n\u{2022} Can you share more about the integration timeline?\n\u{2022} What kind of support will be provided during onboarding?\n\u{2022} Are there any case studies from similar implementations?\n\nLooking forward to your thoughts!\n\nBest regards,\nIsabella",
            label: "Clients",
            flagged: true,
            unread: true,
            seconds_ago: 12 * 60,
            attachments: &[
                ("Partnership Proposal.pdf", "application/pdf"),
                (
                    "Pricing Overview.xlsx",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                ),
            ],
        },
        DemoMsg {
            sender_name: "Sophia Bennett",
            sender_email: "sophia.bennett@company.com",
            subject: "Q2 Roadmap Alignment",
            message_id: "sophia-q2@company.com",
            in_reply_to: None,
            body: "Hey team, please find the latest Q2 roadmap attached. I'd love your input before we lock the timeline on Friday.",
            label: "Work",
            flagged: false,
            unread: true,
            seconds_ago: 38 * 60,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Liam O'Connor",
            sender_email: "liam.oconnor@company.com",
            subject: "Design System Update",
            message_id: "liam-design@company.com",
            in_reply_to: None,
            body: "We've updated the component library with the new Japandi tokens - softer borders, warmer neutrals, and a calmer focus ring.",
            label: "Work",
            flagged: false,
            unread: false,
            seconds_ago: 66 * 60,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Noah Williams",
            sender_email: "noah.williams@company.com",
            subject: "User Research Insights",
            message_id: "noah-research@company.com",
            in_reply_to: None,
            body: "Here are the key takeaways from this week's interviews. Most people want email to feel quieter, not gamified.",
            label: "Product",
            flagged: false,
            unread: false,
            seconds_ago: 86400,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Emma Johnson",
            sender_email: "emma.johnson@company.com",
            subject: "Meeting Notes - May 20",
            message_id: "emma-notes@company.com",
            in_reply_to: None,
            body: "Thanks everyone for joining. Notes and action items are below - please review and flag anything I missed.",
            label: "Work",
            flagged: false,
            unread: false,
            seconds_ago: 86400 + 7200,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Olivia Davis",
            sender_email: "olivia.davis@company.com",
            subject: "Invoice INV-00234",
            message_id: "olivia-invoice@company.com",
            in_reply_to: None,
            body: "Please find attached the invoice for the March engagement. Let me know if you need any changes before payment.",
            label: "Finance",
            flagged: false,
            unread: false,
            seconds_ago: 86400 + 14400,
            attachments: &[("Invoice INV-00234.pdf", "application/pdf")],
        },
        DemoMsg {
            sender_name: "Ava Thompson",
            sender_email: "ava.thompson@company.com",
            subject: "Re: Feedback on Landing Page",
            message_id: "ava-landing@company.com",
            in_reply_to: Some("landing-root@komorebi.mail"),
            body: "The latest version looks great! The softer palette reads much calmer. One small nit on the hero spacing.",
            label: "Design",
            flagged: false,
            unread: false,
            seconds_ago: 2 * 86400,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "William Anderson",
            sender_email: "william.anderson@company.com",
            subject: "Offsite Agenda",
            message_id: "william-offsite@company.com",
            in_reply_to: None,
            body: "Sharing the agenda for our spring offsite. Tuesday is all-hands, Wednesday is breakout workshops.",
            label: "Work",
            flagged: true,
            unread: false,
            seconds_ago: 2 * 86400 + 3600,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Mia Garcia",
            sender_email: "mia.garcia@company.com",
            subject: "Project Aurora Brief",
            message_id: "mia-aurora@company.com",
            in_reply_to: None,
            body: "Attached is the project brief for Aurora. The scope and success criteria are on pages 4-6.",
            label: "Product",
            flagged: false,
            unread: false,
            seconds_ago: 3 * 86400,
            attachments: &[("Project Aurora Brief.pdf", "application/pdf")],
        },
    ];

    let mut unread = 0i64;
    for m in samples {
        let snippet = mime::make_snippet(m.body);
        let message_id = db.insert_message(&MessageDraft {
            account_id: account.id.clone(),
            folder_id: inbox.id.clone(),
            subject: Some(m.subject.into()),
            sender_name: Some(m.sender_name.into()),
            sender_email: Some(m.sender_email.into()),
            snippet: Some(snippet),
            body_text: Some(m.body.into()),
            date_received: Some(now - m.seconds_ago),
            is_read: !m.unread,
            is_flagged: m.flagged,
            has_attachments: !m.attachments.is_empty(),
            rfc_message_id: Some(m.message_id.into()),
            in_reply_to: m.in_reply_to.map(str::to_string),
            references_header: m.in_reply_to.map(str::to_string),
            ..Default::default()
        })?;
        if m.unread {
            unread += 1;
        }
        db.set_ai_fields(&message_id, Some(m.label), None)?;
        for (name, mime_type) in m.attachments {
            let data = demo_attachment_bytes(mime_type);
            db.insert_attachment(&message_id, name, Some(mime_type), &data)?;
        }
    }

    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("haruto-q2-reply@komorebi.mail".into()),
        in_reply_to: Some("sophia-q2@company.com".into()),
        references_header: Some("sophia-q2@company.com".into()),
        subject: Some("Re: Q2 Roadmap Alignment".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some("Thanks Sophia - reviewed the roadmap and left a few comments. The timeline works for me.".into()),
        date_received: Some(now - 20 * 60),
        is_read: true,
        ..Default::default()
    })?;
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: drafts.id.clone(),
        subject: Some("Draft: Weekly sync notes".into()),
        body_text: Some("Quick notes for this week's sync...".into()),
        date_received: Some(now - 1800),
        is_read: true,
        ..Default::default()
    })?;

    db.set_folder_counts(&inbox.id, unread, 9)?;
    db.set_folder_counts(&sent.id, 0, 3)?;
    db.set_folder_counts(&drafts.id, 0, 1)?;
    Ok(())
}

/// A tiny placeholder payload so the demo's attachment cards render with real
/// (but non-sensitive) bytes. Not a real document.
fn demo_attachment_bytes(mime_type: &str) -> Vec<u8> {
    if mime_type.contains("spreadsheet") || mime_type.contains("excel") {
        b"PK\x03\x04 demo xlsx placeholder".to_vec()
    } else {
        b"%PDF-1.4 demo pdf placeholder".to_vec()
    }
}
