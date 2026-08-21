//! Shared UI view-state types, sentinel ids, and option tables.

use super::*;

pub(crate) const CONTEXT: &str = "Komorebi";
/// Sentinel ids for the virtual views in the sidebar (not real DB folders).
pub(crate) const PRIORITY: &str = "priority";
pub(crate) const SNOOZED: &str = "snoozed";
pub(crate) const LABEL_PREFIX: &str = "label:";
pub(crate) const VIEW_UNREAD: &str = "view:unread";
pub(crate) const VIEW_ATTACHMENTS: &str = "view:attachments";
pub(crate) const VIEW_MENTIONS: &str = "view:mentions";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    Mail,
    Compose,
    Accounts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyMode {
    Reply,
    Note,
}

/// Connection security selection for manual account setup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupSecurity {
    Tls,
    StartTls,
}

impl SetupSecurity {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            SetupSecurity::Tls => "tls",
            SetupSecurity::StartTls => "starttls",
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            SetupSecurity::Tls => "TLS",
            SetupSecurity::StartTls => "STARTTLS",
        }
    }

    pub(crate) fn security(&self) -> Security {
        match self {
            SetupSecurity::Tls => Security::Tls,
            SetupSecurity::StartTls => Security::StartTls,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AiKind {
    Summarize,
    DraftReply,
    Polish,
    Triage,
    FollowUp,
}

impl AiKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AiKind::Summarize => "summarize",
            AiKind::DraftReply => "draft_reply",
            AiKind::Polish => "polish",
            AiKind::Triage => "triage",
            AiKind::FollowUp => "follow_up",
        }
    }
}

// ---------------------------------------------------------------------------
// Drag & drop payloads
// ---------------------------------------------------------------------------

/// Internal drag payload for moving a message onto a folder.
#[derive(Clone)]
pub(crate) struct DragMessage {
    pub(crate) message_id: String,
}

pub(crate) struct DragMessagePreview {
    pub(crate) message_id: String,
}

impl Render for DragMessagePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.message_id;
        h_flex()
            .px_3()
            .py_1p5()
            .rounded(RADIUS_SM)
            .shadow_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .gap_2()
            .items_center()
            .child(
                Icon::new(IconName::Folder)
                    .size_3p5()
                    .text_color(cx.theme().muted_foreground),
            )
            .child("Move message")
    }
}

#[derive(Clone)]
pub(crate) struct PendingAttachment {
    pub(crate) name: String,
    pub(crate) size_bytes: i64,
    pub(crate) data: Vec<u8>,
}

/// Enough to undo the last snooze/follow-up action: restore the previous
/// reminder (if any) or clear the one we just scheduled.
pub(crate) struct UndoReminder {
    pub(crate) message_id: String,
    pub(crate) previous: Option<Reminder>,
}

/// Quick snooze choices: label + seconds from now.
pub(crate) const SNOOZE_OPTIONS: &[(&str, i64)] = &[
    ("Later today", 4 * 3600),
    ("Tomorrow", 24 * 3600),
    ("In 3 days", 3 * 24 * 3600),
    ("Next week", 7 * 24 * 3600),
];

/// An attachment written to disk for in-app image preview.
pub(crate) struct AttachmentPreview {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}
