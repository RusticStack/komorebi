//! Sidebar rendering: folder tree, labels, nav items, storage meter.

use super::*;

impl App {
    // ---------------------------------------------------------------------
    // Sidebar
    // ---------------------------------------------------------------------
    pub(crate) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let priority_unread = self
            .db
            .list_flagged_messages(500, 0)
            .map(|m| m.len())
            .unwrap_or(0) as i64;
        let unread_view = self
            .account_messages()
            .into_iter()
            .filter(|m| !m.is_read)
            .count() as i64;

        v_flex()
            .id("sidebar")
            .h_full()
            .w_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            // Japandi layering: the sidebar is a recessed surface behind the
            // content layer - calm and unornamented.
            .p_2()
            .gap_1()
            .overflow_y_scroll()
            .child(
                Button::new("compose")
                    .label(SharedString::from(i18n::t("action.compose", self.locale)))
                    .icon(IconName::Plus)
                    .primary()
                    .w_full()
                    .h_8()
                    .tooltip("Compose a new message (Ctrl+N)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.new_message(window, cx);
                            });
                        }
                    }),
            )
            .child(
                Button::new("sidebar-sync")
                    .label(if self.syncing { "Syncing…" } else { "Sync" })
                    .icon(IconName::Globe)
                    .ghost()
                    .w_full()
                    .h_7()
                    .justify_start()
                    .disabled(self.syncing)
                    .tooltip("Sync all accounts")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.sync_now(cx));
                        }
                    }),
            )
            .child(self.section_header(cx, "Mail"))
            .child(self.mailbox_item(cx, entity.clone(), "inbox", "Inbox", IconName::Inbox))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "nav-priority",
                PRIORITY.to_string(),
                IconName::Star,
                i18n::t("sidebar.priority", self.locale),
                priority_unread,
                None,
            ))
            .child(self.mailbox_item(
                cx,
                entity.clone(),
                "sent",
                i18n::t("sidebar.sent", self.locale),
                IconName::ArrowRight,
            ))
            .child(self.mailbox_item(
                cx,
                entity.clone(),
                "drafts",
                i18n::t("sidebar.drafts", self.locale),
                IconName::File,
            ))
            .child(self.mailbox_item(
                cx,
                entity.clone(),
                "archive",
                i18n::t("sidebar.archive", self.locale),
                IconName::FolderClosed,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "nav-snoozed",
                SNOOZED.to_string(),
                IconName::Calendar,
                i18n::t("sidebar.snoozed", self.locale),
                0,
                None,
            ))
            .child(self.mailbox_item(
                cx,
                entity.clone(),
                "trash",
                i18n::t("sidebar.trash", self.locale),
                IconName::Delete,
            ))
            .child(self.section_header_with_add(cx, entity.clone(), "Labels", true))
            .child(self.labels_list(cx, entity.clone()))
            .child(self.section_header_with_add(cx, entity.clone(), "Views", false))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-unread",
                VIEW_UNREAD.to_string(),
                IconName::Eye,
                i18n::t("reader.unread", self.locale),
                unread_view,
                None,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-attachments",
                VIEW_ATTACHMENTS.to_string(),
                IconName::File,
                i18n::t("reader.attachments", self.locale),
                0,
                None,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-mentions",
                VIEW_MENTIONS.to_string(),
                IconName::Asterisk,
                "Mentions",
                0,
                None,
            ))
            .child(div().flex_1())
            .child(self.storage_widget(cx))
            .child(
                Button::new("accounts")
                    .label("Accounts & Settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .w_full()
                    .justify_start()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.panel = Panel::Accounts;
                                cx.notify();
                            });
                        }
                    }),
            )
    }

    pub(crate) fn section_header(
        &self,
        cx: &mut Context<Self>,
        title: &'static str,
    ) -> impl IntoElement + use<> {
        h_flex()
            .px_2()
            .pt_3()
            .pb_1()
            .w_full()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().muted_foreground)
            .child(SharedString::from(title))
    }

    pub(crate) fn section_header_with_add(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        title: &'static str,
        addable: bool,
    ) -> impl IntoElement + use<> {
        h_flex()
            .px_2()
            .pt_3()
            .pb_1()
            .w_full()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().muted_foreground)
            .justify_between()
            .child(SharedString::from(title))
            .child(
                Button::new(SharedString::from(format!("add-{title}")))
                    .icon(IconName::Plus)
                    .ghost()
                    .xsmall()
                    .tooltip(SharedString::from(format!("Add {title}")))
                    .when(addable, |b| {
                        b.on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| this.toggle_labels_adding(cx));
                            }
                        })
                    }),
            )
    }

    fn mailbox_item(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        special_use: &str,
        label: impl Into<SharedString>,
        icon: IconName,
    ) -> AnyElement {
        match self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some(special_use))
            .cloned()
        {
            Some(folder) => {
                let unread = folder.unread_count;
                let id = folder.id.clone();
                self.nav_item(
                    cx,
                    entity,
                    &format!("nav-{special_use}"),
                    id,
                    icon,
                    label.into(),
                    unread,
                    Some(folder),
                )
            }
            None => div().into_any_element(),
        }
    }

    fn labels_list(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        let mut list = v_flex().gap_1();
        if self.labels.is_empty() && !self.labels_adding {
            list = list.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No labels yet"),
            );
        }
        for label in &self.labels {
            let target = format!("{LABEL_PREFIX}{}", label.id);
            list = list.child(self.label_row(cx, entity.clone(), label, target));
        }
        if self.labels_adding {
            list = list.child(self.label_add_row(entity.clone()));
        }
        list
    }

    /// Inline composer row shown while adding a label.
    fn label_add_row(&self, entity: Entity<Self>) -> AnyElement {
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .items_center()
            .child(Input::new(&self.label_input))
            .child(
                Button::new("confirm-add-label")
                    .icon(IconName::Check)
                    .ghost()
                    .xsmall()
                    .tooltip("Create label")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.create_label_from_input(window, cx);
                            });
                        }
                    }),
            )
            .child(
                Button::new("cancel-add-label")
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip("Cancel")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.toggle_labels_adding(cx));
                        }
                    }),
            )
            .into_any_element()
    }

    fn label_row(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        label: &DbLabel,
        target: String,
    ) -> AnyElement {
        let renaming = self.renaming_label.as_deref() == Some(label.id.as_str());
        if renaming {
            return h_flex()
                .px_2()
                .py_1()
                .gap_1()
                .items_center()
                .child(Input::new(&self.label_input))
                .child(
                    Button::new("confirm-rename-label")
                        .icon(IconName::Check)
                        .ghost()
                        .xsmall()
                        .tooltip("Rename")
                        .on_click({
                            let entity = entity.clone();
                            move |_, window, cx| {
                                cx.update_entity(&entity, |this, cx| {
                                    this.commit_rename_label(window, cx);
                                });
                            }
                        }),
                )
                .child(
                    Button::new("cancel-rename-label")
                        .icon(IconName::Close)
                        .ghost()
                        .xsmall()
                        .tooltip("Cancel")
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| this.cancel_rename_label(cx));
                            }
                        }),
                )
                .into_any_element();
        }

        let selected = self.selected_folder_id.as_deref() == Some(target.as_str());
        let selected_bg = cx.theme().sidebar_accent;
        let selected_fg = cx.theme().sidebar_accent_foreground;
        let hover_bg = cx.theme().muted;
        let dot = stored_label_color(label.color.as_deref(), &label.name);
        let name = label.name.clone();

        h_flex()
            .id(SharedString::from(format!("label-{}", label.id)))
            .w_full()
            .px_2()
            .py_1()
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .gap_2()
            .items_center()
            .when(selected, |d| d.bg(selected_bg).text_color(selected_fg))
            .when(!selected, |d| d.hover(|s| s.bg(hover_bg)))
            .child(
                div()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rounded_full()
                    .bg(dot)
                    .flex_none(),
            )
            .child(SharedString::from(name))
            .child(div().flex_1())
            .on_click({
                let entity = entity.clone();
                let t = target.clone();
                move |_, _, cx| {
                    let t = t.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.selected_folder_id = Some(t);
                        this.selected_message = None;
                        this.search_query.clear();
                        this.reload_messages();
                        cx.notify();
                    });
                }
            })
            .context_menu(self.label_context_menu(entity, label.clone()))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn nav_item(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        id: &str,
        target: String,
        icon: IconName,
        label: impl Into<SharedString>,
        unread: i64,
        context_folder: Option<Folder>,
    ) -> AnyElement {
        let selected = self.selected_folder_id.as_deref() == Some(target.as_str());
        let selected_bg = cx.theme().sidebar_accent;
        let selected_fg = cx.theme().sidebar_accent_foreground;
        let hover_bg = cx.theme().muted;
        let muted = cx.theme().muted_foreground;
        let label_owned: SharedString = label.into();

        let row = h_flex()
            .id(SharedString::from(id.to_string()))
            .w_full()
            .px_2()
            .py_1()
            // In-page controls use 6px radius (Japandi-softened).
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .gap_2()
            .items_center()
            .when(selected, |d| d.bg(selected_bg).text_color(selected_fg))
            .when(!selected, |d| d.hover(|s| s.bg(hover_bg)))
            .child(
                Icon::new(icon)
                    .size_4()
                    .text_color(if selected { selected_fg } else { muted }),
            )
            .child(label_owned.clone())
            .child(div().flex_1())
            .children(unread_guard(unread, cx))
            .on_click({
                let entity = entity.clone();
                let t = target.clone();
                move |_, _, cx| {
                    let t = t.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.selected_folder_id = Some(t);
                        this.selected_message = None;
                        this.search_query.clear();
                        this.reload_messages();
                        cx.notify();
                    });
                }
            });

        let row = match context_folder.as_ref() {
            Some(folder) => {
                let fid_drop = folder.id.clone();
                let fid_drag = folder.id.clone();
                row.on_drop::<DragMessage>({
                    let entity = entity.clone();
                    move |drag, _, cx| {
                        let mid = drag.message_id.clone();
                        let fid = fid_drop.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            this.move_selected_to_folder(&mid, &fid, cx);
                        });
                    }
                })
                .on_drag_move::<DragMessage>({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        let fid = fid_drag.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            this.drag_over_folder_id = Some(fid);
                            cx.notify();
                        });
                    }
                })
            }
            None => row,
        };

        match context_folder {
            Some(folder) => row
                .context_menu(self.folder_context_menu(entity, folder))
                .into_any_element(),
            None => row.into_any_element(),
        }
    }

    fn storage_widget(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .id("storage")
            .m_1()
            .p_3()
            // Top-level container uses 8px radius.
            .rounded(RADIUS_LG)
            .bg(cx.theme().muted)
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_xs().font_medium().child("Storage"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("2.4 GB of 10 GB"),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.0))
                    .rounded_full()
                    .bg(cx.theme().border)
                    .child(
                        div()
                            .h_full()
                            .w(px(56.0))
                            .rounded_full()
                            .bg(cx.theme().primary),
                    ),
            )
    }
}
