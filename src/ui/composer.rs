//! Full-page compose panel: fields, attachments, send pipeline glue.

use super::*;

impl App {
    // ---------------------------------------------------------------------
    // Compose
    // ---------------------------------------------------------------------
    pub(crate) fn compose_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let status = self.compose_status.clone();
        let to_input = self.to_input.clone();
        let cc_input = self.cc_input.clone();
        let bcc_input = self.bcc_input.clone();
        let subject_input = self.subject_input.clone();
        let body_input = self.body_input.clone();
        let attachments = self.composer_attachments.clone();
        let drag_over = self.composer_drag_over;

        v_flex()
            .id("compose")
            .flex_1()
            .h_full()
            .p_4()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            // Back button for consistent
                            // navigation from compose back to mail.
                            .child(
                                Button::new("compose-back")
                                    .icon(IconName::ChevronLeft)
                                    .ghost()
                                    .tooltip("Back to mail")
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                this.flush_draft(cx);
                                                this.panel = Panel::Mail;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(div().text_xl().font_semibold().child("New message")),
                    )
                    .child(
                        Button::new("close-compose")
                            .icon(IconName::Close)
                            .ghost()
                            .tooltip("Close")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.flush_draft(cx);
                                        this.panel = Panel::Mail;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Label::new("To")
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                Button::new("toggle-cc-bcc")
                                    .label(if self.compose_cc_bcc_open {
                                        "Hide Cc/Bcc"
                                    } else {
                                        "Add Cc/Bcc"
                                    })
                                    .link()
                                    .xsmall()
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                this.compose_cc_bcc_open =
                                                    !this.compose_cc_bcc_open;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            ),
                    )
                    .child(Input::new(&to_input))
                    .when(self.compose_cc_bcc_open, |col| {
                        col.child(Input::new(&cc_input))
                            .child(Input::new(&bcc_input))
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Subject")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(Input::new(&subject_input)),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h(px(240.0))
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(if drag_over {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().background)
                    .when(drag_over, |d| d.bg(cx.theme().drop_target))
                    .child(Input::new(&body_input).border_0().h_full())
                    .on_drop::<ExternalPaths>({
                        let entity = entity.clone();
                        move |paths, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.add_attachments(paths, cx);
                            });
                        }
                    })
                    .on_drag_move::<ExternalPaths>({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if !this.composer_drag_over {
                                    this.composer_drag_over = true;
                                    cx.notify();
                                }
                            });
                        }
                    }),
            )
            .child(self.attachment_row(cx, entity.clone(), attachments))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("send")
                            .label("Send")
                            .icon(IconName::ArrowRight)
                            .primary()
                            .on_click({
                                let entity = entity.clone();
                                move |_, window, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        let _ = this.send_composed(window, cx);
                                    });
                                }
                            }),
                    )
                    .child(Button::new("cancel-compose").label("Discard").on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.discard_draft(cx);
                                this.clear_composer(window, cx);
                                this.panel = Panel::Mail;
                                cx.notify();
                            });
                        }
                    })),
            )
            .children(status.map(|s| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(s))
            }))
    }

    fn attachment_row(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        attachments: Vec<PendingAttachment>,
    ) -> impl IntoElement + use<> {
        if attachments.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Drag files anywhere onto the message body to attach them.");
        }
        let mut chips = h_flex().gap_1().flex_wrap();
        for (ix, att) in attachments.into_iter().enumerate() {
            let name = att.name.clone();
            let size = att.size_bytes;
            chips = chips.child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .rounded(RADIUS_SM)
                    .bg(cx.theme().muted)
                    .text_sm()
                    .items_center()
                    .child(Icon::new(IconName::File).size_3p5())
                    .child(SharedString::from(format!(
                        "{name} ({})",
                        format_bytes(size)
                    )))
                    .child(
                        Button::new(SharedString::from(format!("detach-{ix}")))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Remove attachment")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, move |this, cx| {
                                        if ix < this.composer_attachments.len() {
                                            this.composer_attachments.remove(ix);
                                        }
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            );
        }
        chips
    }

    fn send_composed(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let to_raw = self.to_input.read(cx).value().to_string();
        let cc_raw = self.cc_input.read(cx).value().to_string();
        let bcc_raw = self.bcc_input.read(cx).value().to_string();
        let subject = self.subject_input.read(cx).value().to_string();
        let body = self.body_input.read(cx).value().to_string();

        let parse_addresses = |raw: &str| -> Vec<String> {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let to = parse_addresses(&to_raw);
        let cc = parse_addresses(&cc_raw);
        let bcc = parse_addresses(&bcc_raw);

        if to.is_empty() {
            self.compose_status = Some("Enter at least one recipient.".to_string());
            cx.notify();
            return false;
        }

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

        // The outgoing message consumes its draft; on failure it is re-saved.
        if let Some(id) = self.draft_id.take() {
            let _ = self.db.delete_draft(&id);
        }
        self.draft_dirty = false;

        // Clear the composer optimistically; the send owns its copies.
        self.to_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.cc_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.bcc_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_cc_bcc_open = false;
        self.subject_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.body_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.composer_attachments.clear();
        self.compose_status = Some("Sending…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        let account_id = account.as_ref().map(|a| a.id.clone());
        let to_for_draft = to.join(", ");
        let subject_for_draft = subject.clone();
        let body_for_draft = body.clone();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match account.as_ref() {
                        Some(a) => {
                            perform_send(&db, a, &to, &cc, &bcc, &subject, &body, &attachments)
                        }
                        None => Ok(()),
                    }
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(()) => this.compose_status = Some("Sent ✓".to_string()),
                    Err(e) => {
                        // Keep the message as a draft so nothing is lost.
                        if let Ok(id) = this.db.save_draft(
                            account_id.as_deref().unwrap_or_default(),
                            Some(&subject_for_draft),
                            Some(&to_for_draft),
                            Some(&body_for_draft),
                        ) {
                            this.draft_id = Some(id);
                        }
                        this.compose_status = Some(format!("Send failed: {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        true
    }

    // ---------------------------------------------------------------------
    // Accounts
    // ---------------------------------------------------------------------
    /// Two-toggle TLS/STARTTLS picker for manual server settings.
    pub(crate) fn security_buttons(&self, entity: Entity<Self>, is_imap: bool) -> AnyElement {
        let current = if is_imap {
            self.setup_imap_security
        } else {
            self.setup_smtp_security
        };
        let mut row = h_flex().gap_1();
        for sec in [SetupSecurity::Tls, SetupSecurity::StartTls] {
            let selected = current == sec;
            let entity = entity.clone();
            row = row.child(
                Button::new(SharedString::from(format!(
                    "{}sec-{}",
                    if is_imap { "imap" } else { "smtp" },
                    sec.as_str()
                )))
                .label(SharedString::from(sec.label()))
                .when(selected, |b| b.primary())
                .when(!selected, |b| b.outline())
                .xsmall()
                .on_click(move |_, _, cx| {
                    cx.update_entity(&entity, move |this, cx| {
                        if is_imap {
                            this.setup_imap_security = sec;
                        } else {
                            this.setup_smtp_security = sec;
                        }
                        cx.notify();
                    });
                }),
            );
        }
        row.into_any_element()
    }

    /// Read-only summary of a successful autodiscovery result.
    pub(crate) fn discovered_summary(
        &self,
        cx: &mut Context<Self>,
        d: &DiscoveredSettings,
    ) -> AnyElement {
        let imap = d
            .imap
            .as_ref()
            .map(|s| format!("{}:{} ({})", s.host, s.port, s.security.as_str()))
            .unwrap_or_else(|| " - ".to_string());
        let smtp = d
            .smtp
            .as_ref()
            .map(|s| format!("{}:{} ({})", s.host, s.port, s.security.as_str()))
            .unwrap_or_else(|| " - ".to_string());
        v_flex()
            .gap_1()
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .p_2()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .child(SharedString::from(format!(
                        "{} · {}% confidence",
                        d.provider_label, d.confidence
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!("IMAP {imap} · SMTP {smtp}"))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!("Auth: {}", d.auth_mode))),
            )
            .children(d.warnings.iter().map(|w| {
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(SharedString::from(w.clone()))
            }))
            .into_any_element()
    }
}
