//! The draft's attachments (mockup `.att`): inset pills with the file's
//! glyph, its name, its size and a remove button, and their total size
//! ("Attachments: 12 MB"). Komorebi sets no limit on attachments: the
//! total is information only, and the server's own refusal of a message
//! too large for it is the one limit (`mail::smtp`).

use gpui::{
    Context, ElementId, InteractiveElement as _, IntoElement, ParentElement, Role, SharedString,
    Styled, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use komorebi::compose;

use super::{ComposerView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo, layout, radius, space};
use crate::widgets::button::IconButton;

/// The glyph of a file by its name's extension.
fn icon_for(name: &str) -> Icon {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "tif" | "tiff" | "svg" => {
            Icon::Image
        }
        "ics" | "vcs" => Icon::Calendar,
        "eml" | "msg" => Icon::Mail,
        _ => Icon::Document,
    }
}

impl ComposerView {
    pub(crate) fn render_attachments(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let d = self.drafts.active()?;
        if d.attachments.is_empty() {
            return None;
        }
        let t = cx.tokens().clone();
        let c = t.color;
        let lang = komorebi_i18n::current();
        let total = compose::attachments_size(&d.attachments);
        let pills: Vec<_> = d
            .attachments
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let name = a.file.filename.clone();
                let size = text::size(lang, a.file.data.len());
                div()
                    .id(ElementId::Name(format!("compose-attachment-{i}").into()))
                    .role(Role::Group)
                    .aria_label(SharedString::from(format!("{name}, {size}")))
                    .h(px(layout::CONTROL_H_LG))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SP_2_5))
                    .pl(px(space::SP_3))
                    .pr(px(space::SP_1))
                    .rounded(px(radius::FULL))
                    .bg(c.inset)
                    .typo(&t.text.preview)
                    .child(
                        svg()
                            .path(icon_for(&name).at(layout::ICON_LG))
                            .size(px(layout::ICON_LG))
                            .flex_none()
                            .text_color(c.ink_2),
                    )
                    .child(div().text_color(c.ink).child(name.clone()))
                    .child(div().text_color(c.ink_3).typo(&t.text.caption).child(size))
                    .child(
                        IconButton::new(
                            ElementId::Name(format!("compose-attachment-{i}-remove").into()),
                            Icon::Dismiss,
                            text::words::remove(&name),
                        )
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_attachment(i, cx))),
                    )
                    .test_support()
            })
            .collect();
        let total = text::attach_total(&text::size(
            lang,
            usize::try_from(total).unwrap_or(usize::MAX),
        ));
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(geo::ATTACH_GAP))
                .px(px(geo::INSET_LEFT))
                .pt(px(space::SP_2))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(geo::ATTACH_GAP))
                        .children(pills),
                )
                .child(
                    div()
                        .id("compose-attach-total")
                        .role(Role::Status)
                        .aria_label(SharedString::from(total.clone()))
                        .typo(&t.text.caption)
                        .text_color(c.ink_3)
                        .child(total)
                        .test_support(),
                ),
        )
    }
}
