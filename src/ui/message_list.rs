//! Message list panel: resizable list+reader split, rows, empty states.

use super::*;

impl App {
    // ---------------------------------------------------------------------

    // Message list

    // ---------------------------------------------------------------------

    pub(crate) fn mail_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let list = self.message_list(cx).into_any_element();

        let reader = self.reader(cx).into_any_element();

        let list_width = self.list_width;

        let entity = cx.entity();

        h_resizable("mail")
            .on_resize({
                let entity = entity.clone();

                move |state, _, cx| {
                    if let Some(width) = state.read(cx).sizes().first().copied() {
                        cx.update_entity(&entity, |this, cx| {
                            this.list_width = width;

                            cx.notify();
                        });
                    }
                }
            })
            .child(
                resizable_panel()
                    .size(list_width)
                    .size_range(px(300.0)..px(560.0))
                    .child(list),
            )
            .child(resizable_panel().child(reader))
    }

    pub(crate) fn message_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;

        let item_count = self.messages.len();

        let search = self.search_input.clone();

        let entity = cx.entity();

        v_flex()
            .id("message-list")
            .h_full()
            .w_full()
            .child(
                // Search header.
                h_flex().w_full().px_3().py_2().gap_2().child(
                    Input::new(&search)
                        .small()
                        .w_full()
                        .prefix(Icon::new(IconName::Search).small().text_color(muted))
                        .suffix(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(RADIUS_SM)
                                .bg(cx.theme().muted)
                                .text_xs()
                                .text_color(muted)
                                .child("/"),
                        ),
                ),
            )
            .children(self.nl_search_row(cx, entity.clone()))
            .child(
                // Focused / Other tabs + filter.
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        TabBar::new("focused-tabs")
                            .pill()
                            .children([Tab::new().label("Focused"), Tab::new().label("Other")])
                            .selected_index(self.focused_tab)
                            .on_click({
                                let entity = entity.clone();

                                move |idx, _, cx| {
                                    let idx = *idx;

                                    cx.update_entity(&entity, move |this, cx| {
                                        this.focused_tab = idx;

                                        this.reload_messages();

                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("filter")
                            .label("Filter")
                            .icon(IconName::ChevronsUpDown)
                            .ghost()
                            .small()
                            .tooltip("Filter messages")
                            .dropdown_menu(|menu, _, _| {
                                menu.item(PopupMenuItem::new("Unread").icon(IconName::Eye))
                                    .item(PopupMenuItem::new("Has attachment").icon(IconName::File))
                                    .item(PopupMenuItem::new("Starred").icon(IconName::Star))
                                    .separator()
                                    .item(PopupMenuItem::new("From sender…").icon(IconName::User))
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pb_1()
                    .items_center()
                    .justify_between()
                    .child(
                        Label::new(SharedString::from(self.current_view_label()))
                            .text_lg()
                            .font_semibold(),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            // Live sync status - users always know the state of their mail.
                            // users always know the state of their mail.
                            .when(self.sync_busy(), |d| {
                                d.child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            Icon::new(IconName::LoaderCircle)
                                                .size_3p5()
                                                .text_color(muted),
                                        )
                                        .child(div().text_xs().text_color(muted).child("Syncing…")),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(SharedString::from(format!("{} messages", item_count))),
                            ),
                    ),
            )
            .child(if self.messages.is_empty() {
                self.empty_list(cx).into_any_element()
            } else {
                uniform_list(
                    "message-rows",
                    item_count,
                    cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                        let entity = cx.entity();

                        let mut items = Vec::with_capacity(range.len());

                        for ix in range {
                            if let Some(msg) = this.messages.get(ix) {
                                items.push(this.message_row(msg, entity.clone(), cx));
                            }
                        }

                        items
                    }),
                )
                .flex_1()
                .w_full()
                .into_any_element()
            })
    }

    fn empty_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label = match self.selected_folder_id.as_deref() {
            Some(SNOOZED) => "Nothing snoozed. Snoozed conversations return here when due.",

            Some(VIEW_MENTIONS) => "No mentions found.",

            Some(VIEW_ATTACHMENTS) => "No messages with attachments.",

            Some(VIEW_UNREAD) => "You're all caught up.",

            Some(id) if id.starts_with(LABEL_PREFIX) => "No messages with this label yet.",

            _ => "No messages in this view.",
        };

        // Calm, centered empty state - Ma (negative space) as a feature.

        v_flex()
            .id("empty")
            .flex_1()
            .w_full()
            .h_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(
                Icon::new(IconName::Inbox)
                    .size(px(40.0))
                    .text_color(cx.theme().muted_foreground.opacity(0.4)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(label)),
            )
    }

    fn message_row(
        &self,

        msg: &MessageRow,

        entity: Entity<Self>,

        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_message_id() == Some(msg.id.as_str());

        let msg_id = msg.id.clone();

        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());

        let subject = msg
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_string());

        let snippet = msg.snippet.clone().unwrap_or_default();

        let date = format_list_date(msg.date_received);

        let unread = !msg.is_read;

        let flagged = msg.is_flagged;

        let label = msg.ai_category.clone();

        let border = cx.theme().border;

        let list_hover = cx.theme().list_hover;

        let list_active = cx.theme().list_active;

        let primary = cx.theme().primary;

        let muted = cx.theme().muted_foreground;

        h_flex()
            .id(SharedString::from(format!("msg-{msg_id}")))
            .w_full()
            .pl_2()
            .pr_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .cursor_pointer()
            .gap_2()
            .relative()
            .when(selected, |d| d.bg(list_active))
            .when(!selected, |d| d.hover(|s| s.bg(list_hover)))
            // Accent indicator on the left edge marks the selected row,
            // reinforcing the current location.
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(3.0))
                        .bg(primary),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w(px(8.0))
                    .items_center()
                    .justify_center()
                    .h_full()
                    .child(
                        Badge::new()
                            .dot()
                            .color(primary)
                            .when(!unread, |b| b.color(gpui::transparent_black())),
                    ),
            )
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
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .when(unread, |d| d.font_semibold())
                                    .child(SharedString::from(sender.clone())),
                            )
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap_1()
                                    .items_center()
                                    .children(star_marker(flagged, cx))
                                    .children(if self.reminder_ids.contains(&msg.id) {
                                        Some(
                                            Icon::new(IconName::Calendar)
                                                .size_3p5()
                                                .text_color(cx.theme().warning)
                                                .into_any_element(),
                                        )
                                    } else {
                                        None
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(SharedString::from(date)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .when(unread, |d| d.font_semibold())
                            .child(SharedString::from(subject)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(SharedString::from(snippet)),
                            )
                            .children(label.map(|l| label_badge(cx, &l))),
                    ),
            )
            .on_click({
                let entity = entity.clone();

                let mid = msg_id.clone();

                move |_, _, cx| {
                    let id = mid.clone();

                    cx.update_entity(&entity, move |this, cx| {
                        this.open_message(&id);

                        cx.notify();
                    });
                }
            })
            .on_drag(
                DragMessage {
                    message_id: msg_id.clone(),
                },
                |drag, _, _, cx| {
                    cx.new(|_| DragMessagePreview {
                        message_id: drag.message_id.clone(),
                    })
                },
            )
            .context_menu(self.message_context_menu(entity, msg_id, msg.is_read, msg.is_flagged))
            .into_any_element()
    }

    /// "Ask AI" row under the search box when there is a query to translate.
    fn nl_search_row(&self, _cx: &mut Context<Self>, entity: Entity<Self>) -> Option<AnyElement> {
        if self.search_query.trim().len() < 3 {
            return None;
        }
        Some(
            h_flex()
                .w_full()
                .px_3()
                .pb_1()
                .child(
                    Button::new("ask-ai-search")
                        .label(if self.nl_busy {
                            "Translating…"
                        } else {
                            "Ask AI for filters"
                        })
                        .ghost()
                        .xsmall()
                        .disabled(self.nl_busy)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| this.run_nl_search(cx));
                            }
                        }),
                )
                .into_any_element(),
        )
    }

    /// Translate the current search text into structured filters via the
    /// configured AI endpoint, then apply them as the active search.
    pub(crate) fn run_nl_search(&mut self, cx: &mut Context<Self>) {
        if self.nl_busy || self.search_query.trim().len() < 3 {
            return;
        }
        self.nl_busy = true;
        cx.notify();

        let base_url = self.ai_config.base_url.clone();
        let model = self.ai_config.model.clone();
        let provider = self.ai_config.provider.clone();
        let db = self.db.clone();
        let query = self.search_query.clone();
        let entity = cx.entity();
        cx.spawn(async move |_this, cx| {
            const SYSTEM: &str = "Convert the user's mail search request into Komorebi \
                 search tokens. Allowed tokens ONLY: from:<text>, subject:<text>, \
                 is:unread, is:flagged, plus plain keywords. Output exactly one line \
                 of space-separated tokens with no explanation.";
            let result = cx
                .background_executor()
                .spawn(async move {
                    let api_key = crate::ai::resolve_api_key(&db, &provider).ok().flatten();
                    crate::ai::client::complete(
                        &base_url,
                        &model,
                        SYSTEM,
                        &query,
                        api_key.as_deref(),
                    )
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.nl_busy = false;
                match result {
                    Ok(raw) => {
                        let tokens: Vec<&str> = raw
                            .split_whitespace()
                            .filter(|t| {
                                t.starts_with("from:")
                                    || t.starts_with("subject:")
                                    || t.starts_with("is:")
                                    || !t.contains(':')
                            })
                            .collect();
                        let translated = tokens.join(" ");
                        if translated.is_empty() {
                            this.set_toast("The model returned no usable filters.", cx);
                        } else {
                            this.search_query = translated.clone();
                            this.reload_messages();
                            this.set_toast(&format!("Filters: {translated}"), cx);
                        }
                    }
                    Err(e) => this.set_toast(&format!("AI search failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
