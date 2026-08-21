//! Right-click context menus for messages and folders.

use super::*;

impl App {
    pub(crate) fn message_context_menu(
        &self,
        entity: Entity<Self>,
        message_id: String,
        is_read: bool,
        is_flagged: bool,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let current_folder_id = self
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.folder_id.clone());
        let folders: Vec<(String, String, IconName)> = self
            .folders
            .iter()
            .filter(|f| Some(f.id.as_str()) != current_folder_id.as_deref())
            .map(|f| {
                (
                    f.id.clone(),
                    f.display_name.clone().unwrap_or_else(|| f.name.clone()),
                    Self::folder_icon(f),
                )
            })
            .collect();

        let label_entries: Vec<(String, String)> = self
            .labels
            .iter()
            .map(|l| (l.id.clone(), l.name.clone()))
            .collect();

        move |menu, window, cx| {
            let menu = menu
                .item(
                    PopupMenuItem::new("Reply")
                        .icon(IconName::ArrowLeft)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.reply_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Reply all")
                        .icon(IconName::ArrowLeft)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.reply_all_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Forward")
                        .icon(IconName::ArrowRight)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.forward_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Archive")
                        .icon(IconName::FolderClosed)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.archive_message(&id, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new(if is_flagged { "Unstar" } else { "Star" })
                        .icon(if is_flagged {
                            IconName::StarOff
                        } else {
                            IconName::Star
                        })
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.toggle_flag(&id, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new(if is_read {
                        "Mark as unread"
                    } else {
                        "Mark as read"
                    })
                    .icon(if is_read {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    })
                    .on_click({
                        let entity = entity.clone();
                        let id = message_id.clone();
                        let target_read = is_read;
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.set_message_read(&id, !target_read, cx);
                            });
                        }
                    }),
                )
                .separator();

            let menu =
                menu.submenu_with_icon(Some(Icon::new(IconName::Folder)), "Move to", window, cx, {
                    let folders = folders.clone();
                    let entity = entity.clone();
                    let message_id = message_id.clone();
                    move |submenu, _window, _cx| {
                        let mut m = submenu;
                        for (fid, label, icon) in &folders {
                            m = m.item(
                                PopupMenuItem::new(SharedString::from(label.clone()))
                                    .icon(Icon::new(icon.clone()))
                                    .on_click({
                                        let entity = entity.clone();
                                        let mid = message_id.clone();
                                        let fid = fid.clone();
                                        move |_, _, cx| {
                                            let mid = mid.clone();
                                            let fid = fid.clone();
                                            cx.update_entity(&entity, move |this, cx| {
                                                this.move_to_folder(&mid, &fid, cx);
                                            });
                                        }
                                    }),
                            );
                        }
                        m
                    }
                });

            let menu = if label_entries.is_empty() {
                menu
            } else {
                menu.submenu_with_icon(None, "Labels", window, cx, {
                    let entries = label_entries.clone();
                    let entity = entity.clone();
                    let message_id = message_id.clone();
                    move |submenu, _window, _cx| {
                        let mut m = submenu;
                        for (lid, lname) in &entries {
                            m = m.item(
                                PopupMenuItem::new(SharedString::from(lname.clone())).on_click({
                                    let entity = entity.clone();
                                    let mid = message_id.clone();
                                    let id = lid.clone();
                                    move |_, _, cx| {
                                        let mid = mid.clone();
                                        let lid = id.clone();
                                        cx.update_entity(&entity, move |this, cx| {
                                            this.toggle_message_label(&mid, &lid, cx);
                                        });
                                    }
                                }),
                            );
                        }
                        m
                    }
                })
            };

            menu.separator().item(
                PopupMenuItem::new("Delete")
                    .icon(IconName::Delete)
                    .on_click({
                        let entity = entity.clone();
                        let id = message_id.clone();
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.delete_message(&id, cx);
                            });
                        }
                    }),
            )
        }
    }

    pub(crate) fn folder_context_menu(
        &self,
        entity: Entity<Self>,
        folder: Folder,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let folder_id = folder.id.clone();
        let special = folder.special_use.clone();
        let is_trash = special.as_deref() == Some("trash");
        let is_custom = special.is_none();
        let trash_id = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("trash"))
            .map(|f| f.id.clone());

        move |menu, _window, _cx| {
            let menu = menu
                .item(
                    PopupMenuItem::new("Open")
                        .icon(IconName::FolderOpen)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.selected_folder_id = Some(id);
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Mark all read")
                        .icon(IconName::Check)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.mark_folder_read(&id);
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                );

            if is_trash {
                menu.separator().item(
                    PopupMenuItem::new("Empty trash")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_all_messages(&id);
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            } else if is_custom {
                menu.separator().item(
                    PopupMenuItem::new("Delete folder")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_folder(&id);
                                    this.refresh_folders();
                                    this.selected_message = None;
                                    if this.selected_folder_id.as_deref() == Some(id.as_str()) {
                                        this.selected_folder_id = None;
                                    }
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            } else {
                menu.separator().item(
                    PopupMenuItem::new("Empty folder")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            let trash_id = trash_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                let trash_id = trash_id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    if let Some(trash) = &trash_id {
                                        let _ = this.db.move_all_messages(&id, trash);
                                    }
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            }
        }
    }

    /// Right-click menu for a user label row in the sidebar.
    pub(crate) fn label_context_menu(
        &self,
        entity: Entity<Self>,
        label: DbLabel,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let label_id = label.id.clone();

        move |menu, _window, _cx| {
            menu.item(PopupMenuItem::new("Rename").on_click({
                let entity = entity.clone();
                let id = label_id.clone();
                move |_, window, cx| {
                    let id = id.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.start_rename_label(&id, window, cx);
                    });
                }
            }))
            .separator()
            .item(
                PopupMenuItem::new("Delete")
                    .icon(IconName::Delete)
                    .on_click({
                        let entity = entity.clone();
                        let id = label_id.clone();
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.delete_label_by_id(&id, cx);
                            });
                        }
                    }),
            )
        }
    }

    // ---------------------------------------------------------------------
    // Title bar
    // ---------------------------------------------------------------------
}
