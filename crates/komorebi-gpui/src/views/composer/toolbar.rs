//! The format bar (mockup `.fmt`): Bold, Italic and Underline; the
//! bulleted and numbered lists and Link; then the Signature menu and More
//! (strikethrough, indent, quote, alignment, a table, clear formatting,
//! importance, plain or formatted text). Toggles show the format at the
//! caret (`Editor::format_at`). A plain-text message has no formatting:
//! its bar holds the Signature menu and More only. Link opens the link row
//! under the subject (Enter inserts, Esc closes).

use gpui::{
    Context, Focusable as _, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use komorebi::compose::rich::Action;
use komorebi::db::Importance;
use komorebi::mail::rich::edit::TableOp;
use komorebi::mail::rich::{Align, ColorId, FontId, SizePt};

use super::{ComposerEvent, ComposerView, text};
use crate::access::Key;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo, layout, space};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::field::{FieldLook, TextField};
use crate::widgets::menu;

/// The formatting row's buttons: Bold, Italic, Underline, Bullets,
/// Numbers, Link. They are one Tab stop (Fluent's toolbar; the Slint app's
/// `the_formatting_row_is_one_tab_stop`), the one used last; Left and Right
/// move along the row, Home and End to its ends.
pub(crate) const FORMAT_BUTTONS: usize = 6;

/// Where a key moves the focus in the formatting row from button `at`
/// (`None`: not a key of the row). Left and Right wrap round.
pub(crate) fn format_step(key: &str, at: usize) -> Option<usize> {
    let last = FORMAT_BUTTONS - 1;
    match key {
        "left" => Some(if at == 0 { last } else { at - 1 }),
        "right" => Some(if at >= last { 0 } else { at + 1 }),
        "home" => Some(0),
        "end" => Some(last),
        _ => None,
    }
}

impl ComposerView {
    /// A formatting command on the body's selection.
    fn format(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |b, cx| b.act(&action, cx));
        let focus = self.body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Left, Right, Home and End on the formatting row.
    fn format_row_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(at) = self.fmt_focus.iter().position(|h| h.is_focused(window)) else {
            return false;
        };
        let Some(to) = format_step(key, at) else {
            return false;
        };
        self.fmt_at = to;
        window.focus(&self.fmt_focus[to], cx);
        cx.notify();
        true
    }

    /// The formatting row's button `i` as it is drawn: the row's one Tab
    /// stop when it is the one used last.
    pub(crate) fn format_focus(&self, i: usize) -> gpui::FocusHandle {
        self.fmt_focus[i]
            .clone()
            .tab_index(0)
            .tab_stop(i == self.fmt_at)
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
        let focus: Vec<_> = (0..FORMAT_BUTTONS).map(|i| self.format_focus(i)).collect();
        let toggle =
            |i: usize, id: &'static str, icon: Icon, tip: String, on: bool, action: Action| {
                IconButton::new(id, icon, tip)
                    .small()
                    .toggled(on)
                    .track_focus(&focus[i])
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fmt_at = i;
                        this.format(action.clone(), window, cx)
                    }))
            };
        let mut bar = div()
            .id("compose-format-bar")
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .gap(px(geo::BAR_GAP))
            .px(px(geo::BAR_PAD_X))
            .py(px(geo::BAR_PAD_Y));
        if formatted {
            bar = bar
                .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                    let k = &e.keystroke;
                    if !k.modifiers.modified() && this.format_row_key(&k.key, window, cx) {
                        cx.stop_propagation();
                    }
                }))
                .child(toggle(
                    0,
                    "compose-bold",
                    Icon::Bold,
                    text::words::bold(),
                    f.b,
                    Action::Bold,
                ))
                .child(toggle(
                    1,
                    "compose-italic",
                    Icon::Italic,
                    text::words::italic(),
                    f.i,
                    Action::Italic,
                ))
                .child(toggle(
                    2,
                    "compose-underline",
                    Icon::Underline,
                    text::words::underline(),
                    f.u,
                    Action::Underline,
                ))
                .child(div().w(px(geo::BAR_GROUP_GAP)))
                .child(toggle(
                    3,
                    "compose-bullets",
                    Icon::Bullets,
                    text::words::bullets(),
                    f.list == Some(komorebi::mail::rich::ListKind::Bullet),
                    Action::Bullets,
                ))
                .child(toggle(
                    4,
                    "compose-numbers",
                    Icon::Numbers,
                    text::words::numbers(),
                    f.list == Some(komorebi::mail::rich::ListKind::Number),
                    Action::Numbers,
                ))
                .child(
                    IconButton::new("compose-link", Icon::Link, text::words::link_tip())
                        .small()
                        .access_key(Key::Link)
                        .toggled(f.link.is_some())
                        .track_focus(&focus[5])
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.fmt_at = 5;
                            this.open_link(window, cx)
                        })),
                )
                .child(div().w(px(geo::BAR_GROUP_GAP)));
        }
        let view = cx.entity().downgrade();
        let more = IconButton::new("compose-more", Icon::More, text::words::more())
            .small()
            .dropdown_menu(move |m, window, cx| {
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
                let mut m = menu::brand(m, cx);
                if formatted {
                    m = m
                        .item(item(
                            text::words::strike(),
                            f.s,
                            MoreItem::Format(Action::Strike),
                        ))
                        .item(item(
                            text::words::indent(),
                            false,
                            MoreItem::Format(Action::Indent(true)),
                        ))
                        .item(item(
                            text::words::outdent(),
                            false,
                            MoreItem::Format(Action::Indent(false)),
                        ))
                        .item(item(
                            text::words::quote(),
                            f.quote,
                            MoreItem::Format(Action::Quote),
                        ))
                        .separator()
                        .item(item(
                            text::words::align_left(),
                            f.align == Align::Left,
                            MoreItem::Format(Action::Align(Align::Left)),
                        ))
                        .item(item(
                            text::words::align_center(),
                            f.align == Align::Center,
                            MoreItem::Format(Action::Align(Align::Center)),
                        ))
                        .item(item(
                            text::words::align_right(),
                            f.align == Align::Right,
                            MoreItem::Format(Action::Align(Align::Right)),
                        ))
                        .item(item(
                            text::words::justify(),
                            f.align == Align::Justify,
                            MoreItem::Format(Action::Align(Align::Justify)),
                        ))
                        .separator();
                    let (fonts, sizes, colours) = (item.clone(), item.clone(), item.clone());
                    let (font, size, colour) = (f.font, f.size, f.color);
                    m = m
                        .submenu(text::words::font(), window, cx, move |m, _, cx| {
                            FontId::ALL.iter().fold(menu::brand(m, cx), |m, id| {
                                m.item(fonts(
                                    id.label().to_string(),
                                    *id == font,
                                    MoreItem::Format(Action::Font(*id)),
                                ))
                            })
                        })
                        .submenu(text::words::font_size(), window, cx, move |m, _, cx| {
                            SizePt::ALL.iter().filter_map(|pt| SizePt::new(*pt)).fold(
                                menu::brand(m, cx),
                                |m, pt| {
                                    m.item(sizes(
                                        pt.get().to_string(),
                                        pt == size,
                                        MoreItem::Format(Action::Size(pt)),
                                    ))
                                },
                            )
                        })
                        .submenu(text::words::font_colour(), window, cx, move |m, _, cx| {
                            std::iter::once(None)
                                .chain(ColorId::ALL.iter().copied().map(Some))
                                .fold(menu::brand(m, cx), |m, c| {
                                    m.item(colours(
                                        text::words::colour(c),
                                        c == colour,
                                        MoreItem::Format(Action::Color(c)),
                                    ))
                                })
                        })
                        .separator()
                        .item(item(text::words::insert_table(), false, MoreItem::Table));
                    if f.in_table {
                        let tables = item.clone();
                        m = m.submenu(text::words::table(), window, cx, move |m, _, cx| {
                            [
                                TableOp::RowAbove,
                                TableOp::RowBelow,
                                TableOp::ColLeft,
                                TableOp::ColRight,
                                TableOp::DeleteRow,
                                TableOp::DeleteCol,
                                TableOp::DeleteTable,
                            ]
                            .into_iter()
                            .fold(menu::brand(m, cx), |m, op| {
                                m.item(tables(
                                    text::words::table_op(op),
                                    false,
                                    MoreItem::Format(Action::Table(op)),
                                ))
                            })
                        });
                    }
                    m = m.item(item(
                        text::words::clear(),
                        false,
                        MoreItem::Format(Action::Clear),
                    ));
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
        Some(bar.child(self.signature_button(cx)).child(more))
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
        self.link_error = false;
        self.filling = true;
        self.link.update(cx, |s, cx| s.set_value("", window, cx));
        self.filling = false;
        let focus = self.link.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_open = false;
        self.link_error = false;
        let focus = self.body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Enter in the link row: the address on the selection, or an error
    /// under it when it is not a web or e-mail address.
    pub(crate) fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.link.read(cx).value().to_string();
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
                        .child(div().flex_1().child(
                            TextField::new(&self.link, text::words::link_address()).look(
                                FieldLook::Underline {
                                    label: text::words::link().into(),
                                },
                            ),
                        ))
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
    Importance(Importance),
    Rich(bool),
    Preview,
    OpenWindow,
}
