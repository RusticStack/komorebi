//! Reader pane: conversation/thread view, toolbar, quick actions, inline composer, attachments.

use super::*;

impl App {
    // ---------------------------------------------------------------------
    // Reader
    // ---------------------------------------------------------------------
    pub(crate) fn reader(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        if self.selected_message.is_none() {
            return div()
                .id("reader")
                .h_full()
                .w_full()
                .flex_col()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .p_8()
                        .child(
                            Icon::new(IconName::Inbox)
                                .size(px(48.0))
                                .text_color(cx.theme().muted_foreground.opacity(0.3)),
                        )
                        .child(
                            Label::new("Select a conversation")
                                .text_base()
                                .font_medium(),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Messages appear here for focused reading."),
                        ),
                );
        }
        let msg = self.selected_message.clone().expect("checked above");

        let subject = msg
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_string());
        let flagged = msg.is_flagged;
        let category = msg.ai_category.clone();
        let message_id = msg.id.clone();
        let toast = self.toast.clone();
        let reminder = self.db.get_reminder(&message_id).ok().flatten();
        let has_undo = self.undo.is_some();
        let mut tags = category_tags(cx, category.as_deref(), flagged);
        if let Some(r) = &reminder {
            let label = r.label.clone().unwrap_or_default();
            let text = match r.kind.as_str() {
                "snooze" => format!("Snoozed until {label}"),
                _ => format!("Follow up · {label}"),
            };
            tags.push(reminder_tag(cx, &text));
        }

        let scroll = v_flex()
            .id("reader-scroll")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .p_4()
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        // Subject + tags + star.
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_semibold()
                                            .child(SharedString::from(subject)),
                                    )
                                    .child(h_flex().gap_1().items_center().children(tags)),
                            )
                            .child(
                                Icon::new(if flagged {
                                    IconName::Star
                                } else {
                                    IconName::StarOff
                                })
                                .size_4()
                                .text_color(if flagged {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                }),
                            ),
                    )
                    .child(self.conversation_view(cx, entity.clone()))
                    .child(self.attachments_section(cx, entity.clone(), &message_id))
                    .children(self.preview_section(cx, entity.clone()))
                    .child(self.quick_actions(cx, entity.clone()))
                    .child(self.inline_composer(cx, entity.clone()))
                    .children(toast.map(|t| {
                        let mut row = h_flex()
                            .w_full()
                            .p_2()
                            .rounded(RADIUS_SM)
                            .bg(cx.theme().muted)
                            .gap_2()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(t)),
                            );
                        if has_undo {
                            row = row.child(
                                Button::new("toast-undo").label("Undo").small().on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.undo_last(cx);
                                        });
                                    }
                                }),
                            );
                        }
                        row.child(
                            Button::new("toast-dismiss")
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.toast = None;
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                    })),
            );

        div()
            .id("reader")
            .h_full()
            .w_full()
            .flex_col()
            .child(self.reader_toolbar(cx, entity, flagged))
            .child(scroll)
    }

    // ---------------------------------------------------------------------
    // Conversation (thread) rendering
    // ---------------------------------------------------------------------
    fn conversation_view(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> AnyElement {
        if self.thread_messages.len() > 1 {
            self.thread_view(cx, entity)
        } else {
            self.single_message_view(cx)
        }
    }

    fn single_message_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(msg) = self.selected_message.as_ref() else {
            return div().into_any_element();
        };
        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());
        let sender_email = msg.sender_email.clone().unwrap_or_default();
        let date = format_full_date(msg.date_received);
        let to = msg.to_addresses.clone().unwrap_or_default();
        let unread = !msg.is_read;
        let body = msg.body_text.clone().unwrap_or_default();

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(Avatar::new().name(SharedString::from(sender.clone())))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_baseline()
                                    .child(div().font_semibold().child(SharedString::from(sender)))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(SharedString::from(recipient_line(
                                                &to,
                                                &sender_email,
                                            ))),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(date)),
                            ),
                    )
                    .child(h_flex().gap_1().children(if unread {
                        Some(
                            Badge::new()
                                .color(cx.theme().primary)
                                .child("Unread")
                                .into_any_element(),
                        )
                    } else {
                        None
                    })),
            )
            .children(self.remote_content_banner(cx))
            .child(render_message_body(
                &body,
                msg.body_html.as_deref(),
                self.allow_remote_content,
                cx,
            ))
            .into_any_element()
    }

    fn thread_view(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> AnyElement {
        let count = self.thread_messages.len();
        v_flex()
            .id("thread-view")
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!(
                        "{count} messages in this conversation"
                    ))),
            )
            .children(
                self.thread_messages
                    .iter()
                    .map(|m| self.thread_message_card(cx, entity.clone(), m))
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }

    fn thread_message_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        msg: &MessageDetail,
    ) -> AnyElement {
        let expanded = self.expanded_ids.contains(&msg.id);
        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());
        let date = format_full_date(msg.date_received);
        let body = msg.body_text.clone().unwrap_or_default();

        v_flex()
            .id(SharedString::from(format!("thread-msg-{}", msg.id)))
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .id(SharedString::from(format!("thread-head-{}", msg.id)))
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .on_click({
                        let entity = entity.clone();
                        let id = msg.id.clone();
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.toggle_thread_message(&id, cx);
                            });
                        }
                    })
                    .child(
                        Avatar::new()
                            .name(SharedString::from(sender.clone()))
                            .small(),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(SharedString::from(sender)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(date)),
                            ),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .children(if expanded {
                Some(
                    v_flex()
                        .px_3()
                        .pb_2()
                        .child(render_message_body(
                            &body,
                            msg.body_html.as_deref(),
                            self.allow_remote_content,
                            cx,
                        ))
                        .into_any_element(),
                )
            } else {
                None
            })
            .into_any_element()
    }

    fn toggle_thread_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.expanded_ids.contains(&id.to_string()) {
            self.expanded_ids.retain(|x| x != id);
        } else {
            self.expanded_ids.push(id.to_string());
        }
        cx.notify();
    }

    fn reader_toolbar(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        flagged: bool,
    ) -> impl IntoElement + use<> {
        h_flex()
            .id("reader-toolbar")
            .w_full()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .gap_1()
            .child(
                Button::new("toolbar-back")
                    .icon(IconName::ChevronLeft)
                    .ghost()
                    .tooltip("Back to list")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.selected_message = None;
                                cx.notify();
                            });
                        }
                    }),
            )
            // Separator between navigation and message actions for clarity.
            .child(div().w(px(1.0)).h(px(16.0)).bg(cx.theme().border).mx_1())
            .child(
                Button::new("toolbar-archive")
                    .icon(IconName::FolderClosed)
                    .ghost()
                    .tooltip("Archive (A)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.archive_selected(cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-delete")
                    .icon(IconName::Delete)
                    .ghost()
                    .tooltip("Delete")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.delete_selected(cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-unread")
                    .icon(IconName::EyeOff)
                    .ghost()
                    .tooltip("Mark unread (U)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(id) = this.selected_message_id().map(str::to_string) {
                                    this.set_message_read(&id, false, cx);
                                }
                            });
                        }
                    }),
            )
            .child(
                Button::new("toolbar-snooze")
                    .icon(IconName::Calendar)
                    .ghost()
                    .tooltip("Snooze")
                    .dropdown_menu({
                        let entity = entity.clone();
                        move |menu, _, _| {
                            let mut m = menu;
                            for (label, secs) in SNOOZE_OPTIONS {
                                m = m.item(
                                    PopupMenuItem::new(SharedString::from(*label)).on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            let label = *label;
                                            let secs = *secs;
                                            cx.update_entity(&entity, move |this, cx| {
                                                if let Some(id) =
                                                    this.selected_message_id().map(str::to_string)
                                                {
                                                    this.snooze_message(
                                                        &id,
                                                        label,
                                                        chrono::Utc::now().timestamp() + secs,
                                                        cx,
                                                    );
                                                }
                                            });
                                        }
                                    }),
                                );
                            }
                            m
                        }
                    }),
            )
            .child(
                Button::new("toolbar-flag")
                    .icon(if flagged {
                        IconName::Star
                    } else {
                        IconName::StarOff
                    })
                    .ghost()
                    .tooltip("Star (S)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.toggle_selected_flag(cx));
                        }
                    }),
            )
            .child(div().flex_1())
            // Separator before the reply/forward group.
            .child(div().w(px(1.0)).h(px(16.0)).bg(cx.theme().border).mx_1())
            .child(
                Button::new("toolbar-reply")
                    .icon(IconName::ArrowLeft)
                    .ghost()
                    .tooltip("Reply (R)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_reply(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-forward")
                    .icon(IconName::ArrowRight)
                    .ghost()
                    .tooltip("Forward (F)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_forward(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-more")
                    .icon(IconName::EllipsisVertical)
                    .ghost()
                    .tooltip("More")
                    .dropdown_menu({
                        let entity = entity.clone();
                        let sender_email = self
                            .selected_message
                            .as_ref()
                            .and_then(|m| m.sender_email.clone())
                            .unwrap_or_default();
                        move |menu, _, _| {
                            let email = sender_email.clone();
                            menu.item(
                                PopupMenuItem::new("Mark as unread")
                                    .icon(IconName::EyeOff)
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                if let Some(id) =
                                                    this.selected_message_id().map(str::to_string)
                                                {
                                                    this.set_message_read(&id, false, cx);
                                                }
                                            });
                                        }
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("Copy sender")
                                    .icon(IconName::Copy)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            email.clone(),
                                        ));
                                    }),
                            )
                        }
                    }),
            )
    }

    fn quick_actions(
        &self,
        _cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("qa-reply")
                    .label("Reply")
                    .icon(IconName::ArrowLeft)
                    .outline()
                    .small()
                    .tooltip("Reply to sender (R)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_reply(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("qa-reply-all")
                    .label("Reply all")
                    .icon(IconName::Undo2)
                    .outline()
                    .small()
                    .tooltip("Reply to all recipients")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(id) = this.selected_message_id().map(str::to_string) {
                                    this.reply_all_to(&id, window, cx);
                                }
                            });
                        }
                    }),
            )
            .child(
                Button::new("qa-forward")
                    .label("Forward")
                    .icon(IconName::ArrowRight)
                    .outline()
                    .small()
                    .tooltip("Forward (F)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_forward(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("qa-ai-reply")
                    .label("AI Reply")
                    .icon(IconName::Bot)
                    .primary()
                    .small()
                    .tooltip("Draft a smart reply in your tone")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.run_ai_reply(window, cx));
                        }
                    }),
            )
    }

    fn inline_composer(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        let reply_input = self.reply_input.clone();
        let reply_to = self.reply_to.clone();
        let ai_draft = self.ai_reply_draft.clone();
        let mode = self.reply_mode;

        v_flex()
            .id("inline-composer")
            .mt_1()
            // The inline composer is an elevated card on top
            // of the reader body. 8px radius for a top-level container.
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .gap_1()
            .p_2()
            .child(
                TabBar::new("composer-tabs")
                    .underline()
                    .children([Tab::new().label("Reply"), Tab::new().label("Internal note")])
                    .selected_index(if mode == ReplyMode::Note { 1 } else { 0 })
                    .on_click({
                        let entity = entity.clone();
                        move |idx, _, cx| {
                            let idx = *idx;
                            cx.update_entity(&entity, move |this, cx| {
                                this.reply_mode = if idx == 1 {
                                    ReplyMode::Note
                                } else {
                                    ReplyMode::Reply
                                };
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(if reply_to.is_empty() {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Reply to sender")
                            .into_any_element()
                    } else {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(format!("To: {reply_to}")))
                            .into_any_element()
                    })
                    .child(
                        div()
                            .w_full()
                            .min_h(px(96.0))
                            .rounded(RADIUS_SM)
                            .bg(cx.theme().input)
                            .child(Input::new(&reply_input).border_0()),
                    ),
            )
            .children(ai_draft.map(|text| self.ai_draft_bar(cx, entity.clone(), text)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(self.formatting_bar(cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("discard-reply")
                                    .icon(IconName::Delete)
                                    .ghost()
                                    .tooltip("Discard")
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                this.clear_reply(window, cx);
                                                this.reply_open = false;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("send-reply")
                                    .label("Send")
                                    .icon(IconName::ArrowRight)
                                    .primary()
                                    .small()
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                let _ = this.send_reply(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
    }

    fn ai_draft_bar(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        text: String,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .p_2()
            .rounded(RADIUS_SM)
            .bg(cx.theme().muted)
            .gap_2()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("AI drafted a reply in your tone."),
            )
            .child(
                Button::new("insert-ai-draft")
                    .label("Insert")
                    .primary()
                    .small()
                    .on_click(move |_, window, cx| {
                        let text = text.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            let value = this.reply_input.read(cx).value().to_string();
                            let merged = if value.trim().is_empty() {
                                text.clone()
                            } else {
                                format!("{value}\n\n{text}")
                            };
                            this.reply_input.update(cx, |s, cx| {
                                s.set_value(SharedString::from(merged), window, cx)
                            });
                            this.ai_reply_draft = None;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn formatting_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // Japandi honesty: show only what actually works. Rich-text editing
        // is not yet implemented; the attachment drop target in the compose
        // body handles file attachments. This bar stays minimal.
        h_flex().gap_1().items_center().child(
            div()
                .text_xs()
                .text_color(_cx.theme().muted_foreground)
                .child("Drop files to attach"),
        )
    }

    // ---------------------------------------------------------------------
    // Attachment actions
    // ---------------------------------------------------------------------
    fn download_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<PathBuf> = cx
                .background_executor()
                .spawn(async move {
                    let data = load_attachment_data(&db, &att)?;
                    let dir = dirs::download_dir()
                        .filter(|d| d.is_dir())
                        .unwrap_or_else(std::env::temp_dir);
                    write_blob(&dir, &att.display_name, &data)
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(path) => this.set_toast(&format!("Saved {}", path.display()), cx),
                    Err(e) => this.set_toast(&format!("Download failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn download_all_attachments(&mut self, atts: Vec<Attachment>, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<usize> = cx
                .background_executor()
                .spawn(async move {
                    let dir = dirs::download_dir()
                        .filter(|d| d.is_dir())
                        .unwrap_or_else(std::env::temp_dir);
                    let mut saved = 0usize;
                    for att in &atts {
                        if let Some(data) = db.get_attachment_data(&att.id)? {
                            write_blob(&dir, &att.display_name, &data)?;
                            saved += 1;
                        }
                    }
                    Ok(saved)
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(n) => this.set_toast(&format!("Saved {n} attachment(s)."), cx),
                    Err(e) => this.set_toast(&format!("Download failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Write the blob to a temp file and hand it to the OS default handler.
    /// This is also the safe path for PDFs (no in-app untrusted rendering).
    fn open_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<()> = cx
                .background_executor()
                .spawn(async move {
                    let data = load_attachment_data(&db, &att)?;
                    let path = write_blob(&std::env::temp_dir(), &att.display_name, &data)?;
                    open::that(&path)?;
                    Ok(())
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                if let Err(e) = result {
                    this.set_toast(&format!("Could not open attachment: {e}"), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Preview an image in-app, or open PDFs/other files in the OS viewer.
    fn preview_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        if is_image_name(&att.display_name) {
            let db = self.db.clone();
            let entity = cx.entity();
            cx.spawn(async move |_weak, cx| {
                let result: anyhow::Result<(String, PathBuf)> = cx
                    .background_executor()
                    .spawn(async move {
                        let data = load_attachment_data(&db, &att)?;
                        let path = write_blob(&std::env::temp_dir(), &att.display_name, &data)?;
                        Ok((att.display_name.clone(), path))
                    })
                    .await;
                let _ = entity.update(cx, move |this, cx| {
                    match result {
                        Ok((name, path)) => {
                            this.preview = Some(AttachmentPreview { name, path });
                        }
                        Err(e) => this.set_toast(&format!("Preview failed: {e}"), cx),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            if is_pdf_name(&att.display_name) {
                self.set_toast("PDF opens in your system viewer.", cx);
            }
            self.open_attachment(att, cx);
        }
    }

    fn close_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.take() {
            let _ = std::fs::remove_file(&preview.path);
        }
        cx.notify();
    }

    fn preview_section(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> Option<AnyElement> {
        let preview = self.preview.as_ref()?;
        let name = preview.name.clone();
        let path = preview.path.clone();
        Some(
            v_flex()
                .id("attachment-preview")
                .gap_2()
                .rounded(RADIUS_LG)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .p_2()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(name)),
                        )
                        .child(
                            Button::new("close-preview")
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .tooltip("Close preview")
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.close_preview(cx);
                                        });
                                    }
                                }),
                        ),
                )
                .child(
                    gpui::img(path)
                        .object_fit(gpui::ObjectFit::ScaleDown)
                        .w_full()
                        .max_h(px(320.0))
                        .rounded(RADIUS_SM)
                        .bg(cx.theme().background),
                )
                .into_any_element(),
        )
    }

    fn attachments_section(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        message_id: &str,
    ) -> AnyElement {
        let atts = self.db.list_attachments(message_id).unwrap_or_default();
        if atts.is_empty() {
            return div().into_any_element();
        }
        let all = atts.clone();
        v_flex()
            .id("attachments")
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child(SharedString::from(format!(
                                "{} {}",
                                atts.len(),
                                if atts.len() == 1 {
                                    "Attachment"
                                } else {
                                    "Attachments"
                                }
                            ))),
                    )
                    .child(
                        h_flex().gap_1().child(
                            Button::new("download-all")
                                .icon(IconName::ArrowDown)
                                .ghost()
                                .small()
                                .tooltip("Download all")
                                .on_click({
                                    let entity = entity.clone();
                                    let all = all.clone();
                                    move |_, _, cx| {
                                        let all = all.clone();
                                        cx.update_entity(&entity, move |this, cx| {
                                            this.download_all_attachments(all, cx);
                                        });
                                    }
                                }),
                        ),
                    ),
            )
            .child(
                h_flex().gap_2().flex_wrap().children(
                    atts.iter()
                        .map(|att| self.attachment_card(cx, entity.clone(), att))
                        .collect::<Vec<_>>(),
                ),
            )
            .into_any_element()
    }

    fn attachment_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        att: &Attachment,
    ) -> AnyElement {
        let (color, short) = file_type_style(&att.display_name);
        let fg = cx.theme().muted_foreground;
        let preview_label = if is_image_name(&att.display_name) {
            "Preview image"
        } else {
            "Preview in system viewer"
        };
        h_flex()
            .gap_2()
            .p_2()
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .items_center()
            .child(
                div()
                    .w(px(36.0))
                    .h(px(36.0))
                    .rounded(RADIUS_SM)
                    .bg(color)
                    .text_color(komorebi_hex(0xffffff))
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .child(SharedString::from(short)),
                    ),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .child(SharedString::from(att.display_name.clone())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(fg)
                            .child(SharedString::from(format_bytes(att.size_bytes))),
                    ),
            )
            .child(
                h_flex()
                    .gap_0()
                    .child(
                        Button::new(SharedString::from(format!("preview-{}", att.id)))
                            .icon(IconName::Eye)
                            .ghost()
                            .xsmall()
                            .tooltip(preview_label)
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.preview_attachment(att, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("download-{}", att.id)))
                            .icon(IconName::ArrowDown)
                            .ghost()
                            .xsmall()
                            .tooltip("Download")
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.download_attachment(att, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("open-{}", att.id)))
                            .icon(IconName::ExternalLink)
                            .ghost()
                            .xsmall()
                            .tooltip("Open in system viewer")
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.open_attachment(att, cx);
                                    });
                                }
                            }),
                    ),
            )
            .into_any_element()
    }
}
