//! Attachment pills (mockup `.att`): 40 px inset pills with the file's
//! glyph, its name and its size in ink-3; a click opens it, the save
//! glyph saves it (both handed to the view's owner, which has the files
//! and the dialogs). They wrap, as many to a row as fit, each 180 to 280 px
//! wide, as the Slint reader lays them out.

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window,
    div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use komorebi::i18n::Lang;
use komorebi::mail::mime::AttachmentInfo;
use komorebi_i18n::tr;

use super::format::{self, FileKind};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, reader, space};
use crate::widgets::button::IconButton;
use crate::widgets::ring_style;

/// What a press on a pill asks for, by attachment index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Open(usize),
    Save(usize),
}

type OnPress = Rc<dyn Fn(Press, &mut Window, &mut App)>;

fn icon(kind: FileKind) -> Icon {
    match kind {
        FileKind::Picture => Icon::Image,
        FileKind::Calendar => Icon::Calendar,
        FileKind::Message => Icon::Mail,
        FileKind::Document | FileKind::File => Icon::Document,
    }
}

/// The pills of a message's attachments.
#[derive(IntoElement)]
pub struct Attachments {
    items: Vec<AttachmentInfo>,
    lang: Lang,
    on_press: OnPress,
}

impl Attachments {
    pub fn new(
        items: Vec<AttachmentInfo>,
        lang: Lang,
        on_press: impl Fn(Press, &mut Window, &mut App) + 'static,
    ) -> Self {
        Attachments {
            items,
            lang,
            on_press: Rc::new(on_press),
        }
    }
}

impl RenderOnce for Attachments {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let lang = self.lang;
        let pills = self.items.into_iter().map(|a| {
            let (open, save) = (self.on_press.clone(), self.on_press.clone());
            let index = a.index;
            let size: SharedString = format::size(lang, a.size).into();
            let name: SharedString = a.name.clone().into();
            div()
                .id(ElementId::Name(format!("attachment-{index}").into()))
                .role(Role::Button)
                .aria_label(name.clone())
                .aria_description(size.clone())
                .tab_index(0)
                .h(px(reader::ATTACHMENT_H))
                .min_w(px(reader::ATTACHMENT_MIN_W))
                .max_w(px(reader::ATTACHMENT_MAX_W))
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(space::SP_2_5))
                .pl(px(space::SP_3))
                .pr(px(space::SP_1))
                .rounded(px(radius::FULL))
                .bg(c.inset)
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(&t))
                .typo(&t.text.preview)
                .text_color(c.ink)
                .on_click(move |_, window, cx| open(Press::Open(index), window, cx))
                .child(
                    svg()
                        .path(icon(format::file_kind(&a.name)).path())
                        .size(px(layout::ICON))
                        .flex_none()
                        .text_color(c.ink_2),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_baseline()
                        .gap(px(space::SP_1))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .typo(&t.text.caption)
                                .text_color(c.ink_3)
                                .child(size),
                        ),
                )
                .child(
                    IconButton::new(
                        ElementId::Name(format!("attachment-save-{index}").into()),
                        Icon::Save,
                        tr!("Save {0}", name),
                    )
                    .small()
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        save(Press::Save(index), window, cx)
                    }),
                )
                .test_support()
        });
        div()
            .id("attachments")
            .w_full()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(reader::ATTACHMENT_GAP))
            .mt(px(reader::ATTACHMENT_TOP))
            .children(pills)
    }
}
