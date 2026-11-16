//! The composer's format bar (mockup `.fmt`): the shared bar of
//! `format_bar.rs` (Gmail's order: font, size, Bold, Italic, Underline,
//! colour, alignment, the lists, the indents, quote, strikethrough, clear
//! formatting), then Link, the Signature menu and More (what left the bar,
//! a table, importance, Preview, plain or formatted text, Open in new
//! window). A plain-text message has no formatting: its bar holds the
//! Signature menu and More only. Link opens the link row under the
//! subject (Enter inserts, Esc closes).

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    Styled, Window, div, prelude::*, px,
};
use gpui_kit::component::menu::PopupMenuItem;
use komorebi::compose::rich::Action;
use komorebi::db::Importance;
use komorebi::mail::rich::edit::TableOp;

use super::format_bar::{Ctl, FormatBar};
use super::{ComposerEvent, ComposerView, text};
use crate::access::Key;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo, layout, space};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::field::{FieldLook, TextField};
use crate::widgets::menu;

impl ComposerView {
    /// A formatting command on the body's selection.
    pub(crate) fn format(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |b, cx| b.act(&action, cx));
        let focus = self.body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    pub(crate) fn render_toolbar(
        &mut self,
        formatted: bool,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let f = self.body.read(cx).format().unwrap_or_default();
        let importance = self
            .drafts
            .active()
            .map_or(Importance::Normal, |d| d.importance);
        let previewing = self.previewing();
        let windowed = self.windowed;
        // A stored quote shown in full can be turned into text to edit.
        let quote_shown = self
            .body
            .read(cx)
            .editor()
            .is_some_and(|e| e.has_quote() && !e.quote_folded());
        let in_table = f.in_table;
        let merged = f.table_merged;
        // A selected picture: its size, its alt text, its original.
        let picture = self.body.read(cx).selected_picture().is_some();
        let original = self.body.read(cx).has_original();
        let link_on = f.link.is_some();
        let view = cx.entity().downgrade();
        let on_action = {
            let v = view.clone();
            move |action: &Action, window: &mut Window, cx: &mut gpui::App| {
                let action = action.clone();
                let _ = v.update(cx, |this, cx| this.format(action, window, cx));
            }
        };
        let ctls: &'static [Ctl] = if formatted { &Ctl::ALL } else { &[] };
        let mut bar = FormatBar::new("compose", ctls, f, &self.fmt, cx.entity_id(), on_action)
            .padding(geo::BAR_PAD_X, geo::BAR_PAD_Y);
        if formatted {
            let v = view.clone();
            bar = bar.pinned(move |focus| {
                IconButton::new("compose-link", Icon::Link, text::words::link_tip())
                    .small()
                    .access_key(Key::Link)
                    .toggled(link_on)
                    .track_focus(&focus)
                    .on_click(move |_, window, cx| {
                        let _ = v.update(cx, |this, cx| this.open_link(window, cx));
                    })
                    .into_any_element()
            });
        }
        bar = bar.pinned(self.signature_button(cx));
        let bar = bar.more(move |m, window, cx| {
            let v = view.clone();
            let item = move |label: String, checked: bool, act: MoreItem| {
                let v = v.clone();
                PopupMenuItem::new(label)
                    .checked(checked)
                    .on_click(move |_, window, cx| {
                        let act = act.clone();
                        let _ = v.update(cx, |this, cx| this.more(act, window, cx));
                    })
            };
            let mut m = m;
            if formatted {
                m = m.item(item(text::words::insert_table(), false, MoreItem::Table));
                if in_table {
                    let tables = item.clone();
                    m = m.submenu(text::words::table(), window, cx, move |m, _, cx| {
                        TableOp::offered(merged).iter().copied().fold(
                            menu::brand(m, cx),
                            |m, op| {
                                m.item(tables(
                                    text::words::table_op(op),
                                    false,
                                    MoreItem::Format(Action::Table(op)),
                                ))
                            },
                        )
                    });
                }
                if picture {
                    let pics = item.clone();
                    m = m.submenu(text::words::picture(), window, cx, move |m, _, cx| {
                        let mut m = menu::brand(m, cx)
                            .item(pics(text::words::alt_text(), false, MoreItem::AltText))
                            .item(pics(
                                text::words::picture_bigger(),
                                false,
                                MoreItem::Format(Action::SizeStep(true)),
                            ))
                            .item(pics(
                                text::words::picture_smaller(),
                                false,
                                MoreItem::Format(Action::SizeStep(false)),
                            ))
                            .item(pics(
                                text::words::picture_own_size(),
                                false,
                                MoreItem::OwnSize,
                            ));
                        if original {
                            m = m.item(pics(
                                text::words::use_original(),
                                false,
                                MoreItem::UseOriginal,
                            ));
                        }
                        m
                    });
                }
                if quote_shown {
                    m = m.item(item(
                        text::words::edit_quote(),
                        false,
                        MoreItem::Format(Action::EditQuote),
                    ));
                }
                m = m.separator();
            }
            m = m
                .item(item(
                    text::words::high(),
                    importance == Importance::High,
                    MoreItem::Importance(Importance::High),
                ))
                .item(item(
                    text::words::low(),
                    importance == Importance::Low,
                    MoreItem::Importance(Importance::Low),
                ))
                .separator()
                .item(item(text::words::preview(), previewing, MoreItem::Preview))
                .item(item(
                    if formatted {
                        text::words::to_plain()
                    } else {
                        text::words::to_formatted()
                    },
                    false,
                    MoreItem::Rich(!formatted),
                ));
            // A pane's or a tab's draft can go to a window of its own.
            if !windowed {
                m = m.separator().item(item(
                    crate::views::mail::text::command(
                        crate::views::mail::text::Command::OpenWindow,
                    ),
                    false,
                    MoreItem::OpenWindow,
                ));
            }
            m
        });
        Some(bar)
    }

    fn more(&mut self, item: MoreItem, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            MoreItem::Format(action) => self.format(action, window, cx),
            MoreItem::Table => {
                self.body.update(cx, |b, cx| b.insert_table(2, 2, cx));
            }
            MoreItem::Importance(i) => {
                let now = self
                    .drafts
                    .active()
                    .map_or(Importance::Normal, |d| d.importance);
                // The one on is turned off.
                let next = if now == i { Importance::Normal } else { i };
                self.set_importance(next, cx);
            }
            MoreItem::AltText => self.open_alt(window, cx),
            MoreItem::OwnSize => {
                self.body.update(cx, |b, cx| b.picture_own_size(cx));
            }
            MoreItem::UseOriginal => {
                self.body.update(cx, |b, cx| b.use_original(cx));
            }
            MoreItem::Rich(on) => self.set_rich(on, window, cx),
            MoreItem::Preview => self.toggle_preview(window, cx),
            MoreItem::OpenWindow => cx.emit(ComposerEvent::OpenInWindow),
        }
    }

    /// Link (Ctrl+K): the link row under the subject, on the selection.
    pub(crate) fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body.read(cx).is_formatted() {
            return;
        }
        self.link_open = true;
        self.link_alt = false;
        self.link_error = false;
        self.filling = true;
        self.link.update(cx, |s, cx| s.set_value("", window, cx));
        self.filling = false;
        let focus = self.link.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// More > Picture > Alt text…: the link row, as the picture's alt text
    /// (what the plain-text part and screen readers get).
    pub(crate) fn open_alt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(alt) = self.body.read(cx).picture_alt() else {
            return;
        };
        self.link_open = true;
        self.link_alt = true;
        self.link_error = false;
        self.filling = true;
        self.link.update(cx, |s, cx| s.set_value(alt, window, cx));
        self.filling = false;
        let focus = self.link.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_open = false;
        self.link_alt = false;
        self.link_error = false;
        let focus = self.body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Enter in the link row: the address on the selection, or an error
    /// under it when it is not a web or e-mail address.
    pub(crate) fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.link.read(cx).value().to_string();
        if self.link_alt {
            self.body.update(cx, |b, cx| b.set_picture_alt(&url, cx));
            return self.close_link(window, cx);
        }
        if url.trim().is_empty() {
            return self.close_link(window, cx);
        }
        if self.body.update(cx, |b, cx| b.link(&url, cx)) {
            self.close_link(window, cx);
        } else {
            self.link_error = true;
            cx.notify();
        }
    }

    pub(crate) fn render_link(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.link_open {
            return None;
        }
        let t = cx.tokens().clone();
        let (label, name) = if self.link_alt {
            (text::words::alt_text_field(), text::words::alt_text_field())
        } else {
            (text::words::link(), text::words::link_address())
        };
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .ml(px(geo::INSET_LEFT))
                .mr(px(geo::INSET_RIGHT))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::SP_2))
                        .child(div().flex_1().child(TextField::new(&self.link, name).look(
                            FieldLook::Underline {
                                label: label.into(),
                            },
                        )))
                        .child(
                            Button::new("compose-link-insert")
                                .variant(Variant::Primary)
                                .label(text::words::insert())
                                .tooltip(text::words::insert_link_tip())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.insert_link(window, cx)),
                                ),
                        )
                        .child(
                            IconButton::new(
                                "compose-link-cancel",
                                Icon::Dismiss,
                                text::words::cancel_tip(),
                            )
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_link(window, cx)),
                            ),
                        ),
                )
                .when(self.link_error, |el| {
                    el.child(
                        div()
                            .pl(px(layout::FIELD_LABEL_W + geo::FIELD_GAP))
                            .pt(px(space::SP_1))
                            .typo(&t.text.caption)
                            .text_color(t.color.ink)
                            .child(SharedString::from(text::words::link_bad())),
                    )
                }),
        )
    }
}

/// An entry of the More menu.
#[derive(Clone)]
enum MoreItem {
    Format(Action),
    Table,
    AltText,
    OwnSize,
    UseOriginal,
    Importance(Importance),
    Rich(bool),
    Preview,
    OpenWindow,
}
