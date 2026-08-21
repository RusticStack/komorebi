//! Attachment handling: download, open, in-app preview, and card grid.

use super::*;

impl App {
    pub(crate) fn download_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
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

    pub(crate) fn download_all_attachments(
        &mut self,
        atts: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
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
    pub(crate) fn open_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
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
    pub(crate) fn preview_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
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

    pub(crate) fn close_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.take() {
            let _ = std::fs::remove_file(&preview.path);
        }
        cx.notify();
    }

    pub(crate) fn preview_section(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> Option<AnyElement> {
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

    pub(crate) fn attachments_section(
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

    pub(crate) fn attachment_card(
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
