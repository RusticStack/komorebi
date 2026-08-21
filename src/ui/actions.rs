//! Message-level actions: selection ops, flag/read/archive/delete/move, reply/forward composition state, attachments intake, reply send, AI reply.

use super::send::{perform_ai, perform_send};
use super::*;

impl App {
    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    pub(crate) fn toggle_ai_panel(&mut self, cx: &mut Context<Self>) {
        self.ai_panel_visible = !self.ai_panel_visible;
        cx.notify();
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search_input.read(cx).focus_handle(cx);
        handle.focus(window);
    }

    pub(crate) fn toggle_selected_flag(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.toggle_flag(&id, cx);
    }

    pub(crate) fn archive_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.archive_message(&id, cx);
    }

    pub(crate) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.delete_message(&id, cx);
    }

    /// Snooze the selected message until `due_at`, hiding it from the inbox.
    pub(crate) fn snooze_message(
        &mut self,
        id: &str,
        label: &str,
        due_at: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let previous = self.db.get_reminder(id).ok().flatten();
        let _ = self.db.mark_read(id, true);
        let _ = self
            .db
            .upsert_reminder(&detail.account_id, id, "snooze", Some(label), due_at);
        self.undo = Some(UndoReminder {
            message_id: id.to_string(),
            previous,
        });
        self.reload_messages();
        self.set_toast(&format!("Snoozed until {label}"), cx);
        cx.notify();
    }

    /// Schedule a follow-up reminder (defaults to tomorrow) on the selected message.
    pub(crate) fn schedule_followup(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            self.set_toast("Select a message to schedule a follow-up.", cx);
            return;
        };
        let Some(detail) = self.db.get_message_detail(&id).ok().flatten() else {
            return;
        };
        let previous = self.db.get_reminder(&id).ok().flatten();
        let due_at = chrono::Utc::now().timestamp() + 24 * 3600;
        let _ = self.db.upsert_reminder(
            &detail.account_id,
            &id,
            "follow_up",
            Some("tomorrow"),
            due_at,
        );
        self.undo = Some(UndoReminder {
            message_id: id.clone(),
            previous,
        });
        self.reload_messages();
        self.set_toast("Follow-up scheduled for tomorrow", cx);
        cx.notify();
    }

    /// Undo the last snooze/follow-up action, restoring the prior reminder.
    pub(crate) fn undo_last(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        if let Some(prev) = &undo.previous {
            let _ = self.db.upsert_reminder(
                &prev.account_id,
                &prev.message_id,
                &prev.kind,
                prev.label.as_deref(),
                prev.due_at,
            );
        } else {
            let _ = self.db.delete_reminder(&undo.message_id);
        }
        self.toast = Some("Undone".to_string());
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn move_selected_to_folder(
        &mut self,
        message_id: &str,
        folder_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.move_to_folder(message_id, folder_id, cx);
    }

    pub(crate) fn open_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.reply_to(&id, window, cx);
    }

    pub(crate) fn open_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.forward_to(&id, window, cx);
    }

    /// Toggle the star flag on a specific message.
    pub(crate) fn toggle_flag(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let new_flag = !msg.is_flagged;
        let _ = self.db.mark_flagged(id, new_flag);
        if self.selected_message_id() == Some(id) {
            self.selected_message = Some(MessageDetail {
                is_flagged: new_flag,
                ..msg
            });
        }
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn set_message_read(&mut self, id: &str, read: bool, cx: &mut Context<Self>) {
        let _ = self.db.mark_read(id, read);
        if self.selected_message_id() == Some(id) {
            self.selected_message = self.db.get_message_detail(id).ok().flatten();
        }
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn archive_message(&mut self, id: &str, cx: &mut Context<Self>) {
        let target = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("archive"));
        match target {
            Some(target) => {
                let _ = self.db.move_message(id, &target.id);
            }
            None => {
                let _ = self.db.mark_read(id, true);
            }
        }
        if self.selected_message_id() == Some(id) {
            self.selected_message = None;
        }
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn delete_message(&mut self, id: &str, cx: &mut Context<Self>) {
        let target = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("trash"));
        match target {
            Some(target) => {
                let _ = self.db.move_message(id, &target.id);
            }
            None => {
                let _ = self.db.mark_read(id, true);
            }
        }
        if self.selected_message_id() == Some(id) {
            self.selected_message = None;
        }
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn move_to_folder(
        &mut self,
        message_id: &str,
        folder_id: &str,
        cx: &mut Context<Self>,
    ) {
        let _ = self.db.move_message(message_id, folder_id);
        if self.selected_message_id() == Some(message_id) {
            self.selected_message = None;
        }
        self.drag_over_folder_id = None;
        self.reload_messages();
        cx.notify();
    }

    pub(crate) fn reply_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let to = msg.sender_email.clone().unwrap_or_default();
        let subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = to;
        self.reply_subject = subject;
        cx.notify();
    }

    pub(crate) fn reply_all_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let parse = |raw: Option<&String>| -> Vec<String> {
            raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default()
        };
        let own = self.accounts.first().map(|a| a.email.clone());
        let mut to = vec![msg.sender_email.clone().unwrap_or_default()];
        to.extend(parse(msg.to_addresses.as_ref()));
        let mut cc = parse(msg.cc_addresses.as_ref());
        // Reply-all etiquette: everyone originally addressed goes back on
        // To/CC respectively, minus our own address and duplicates.
        let mut seen: Vec<String> = Vec::new();
        let dedup = |list: &mut Vec<String>, seen: &mut Vec<String>| {
            list.retain(|addr| {
                let addr = addr.trim();
                if addr.is_empty() || own.as_deref() == Some(addr) || seen.iter().any(|s| s == addr)
                {
                    return false;
                }
                seen.push(addr.to_string());
                true
            });
        };
        dedup(&mut to, &mut seen);
        dedup(&mut cc, &mut seen);
        let subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = to.join(", ");
        self.reply_cc = cc.join(", ");
        self.reply_subject = subject;
        cx.notify();
    }

    pub(crate) fn forward_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let subject = format!(
            "Fwd: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = String::new();
        self.reply_subject = subject;
        cx.notify();
    }

    pub(crate) fn add_attachments(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for path in paths.paths() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "attachment".to_string());
            match std::fs::read(path) {
                Ok(data) => {
                    self.composer_attachments.push(PendingAttachment {
                        size_bytes: data.len() as i64,
                        name,
                        data,
                    });
                }
                Err(e) => {
                    log::warn!("could not read dropped file {name}: {e}");
                }
            }
        }
        self.composer_drag_over = false;
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // Inline reply composer
    // ---------------------------------------------------------------------
    pub(crate) fn send_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let body = self.reply_input.read(cx).value().to_string();
        if body.trim().is_empty() {
            self.set_toast("Write a reply first.", cx);
            return false;
        }
        let to: Vec<String> = if self.reply_to.trim().is_empty() {
            Vec::new()
        } else {
            self.reply_to
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let cc: Vec<String> = self
            .reply_cc
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let subject = self.reply_subject.clone();
        let attachments: Vec<mail::smtp::Attachment> = self
            .composer_attachments
            .iter()
            .map(|a| mail::smtp::Attachment {
                filename: a.name.clone(),
                content_type: mail::smtp::guess_content_type(&a.name).to_string(),
                data: a.data.clone(),
            })
            .collect();
        let account = self.accounts.first().cloned();

        // Clear the reply composer optimistically; the send owns its copies.
        self.reply_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.composer_attachments.clear();
        self.reply_to = String::new();
        self.reply_cc = String::new();
        self.reply_subject = String::new();
        self.ai_reply_draft = None;
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match account.as_ref() {
                        Some(a) => {
                            perform_send(&db, a, &to, &cc, &[], &subject, &body, &attachments)
                        }
                        None => Ok(()),
                    }
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(()) => this.set_toast("Reply sent ✓", cx),
                    Err(e) => this.set_toast(&format!("Send failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        true
    }

    pub(crate) fn run_ai_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = msg.sender_email.clone().unwrap_or_default();
        self.reply_subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.ai_busy = true;
        self.ai_error = None;
        cx.notify();

        let sender = msg.sender_name.clone();
        let subj = msg.subject.clone();
        let body = msg.body_text.clone().unwrap_or_default();
        let message_id = msg.id.clone();
        let account_id = msg.account_id.clone();
        let ai = self.ai.clone();
        let db = self.db.clone();
        let tone = self.ai_tone.clone();
        let entity = cx.entity();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    perform_ai(
                        &ai,
                        &db,
                        AiKind::DraftReply,
                        &account_id,
                        &message_id,
                        sender.as_deref(),
                        subj.as_deref(),
                        &body,
                        &tone,
                    )
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.ai_busy = false;
                match result {
                    Ok(text) => this.ai_reply_draft = Some(text),
                    Err(e) => this.ai_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------------
    // Context menus
    // ---------------------------------------------------------------------
}
