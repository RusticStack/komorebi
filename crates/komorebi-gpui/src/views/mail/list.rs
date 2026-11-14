//! The list pane (mockup `.lh`, `.pv`, `.rows`): the view's title and its
//! account, the "..." menu (Mark all as read, Select messages), the
//! filters, a search's results line, the status line, then the rows in a
//! virtual list (`uniform_list`: only the rows in view are read from the
//! cache and built), or what an empty list says; the Undo offer has its own
//! slot at the bottom.

use std::ops::Range;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Role, Styled, Window, div,
    prelude::*, px, svg, uniform_list,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::scroll::ScrollableElement as _;

use super::model::Mail;
use super::rows::{self, RowLook};
use super::{MailView, commands, filters, menus, text};
use crate::access::Key;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, space};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::Pane;

pub fn render(view: &mut MailView, window: &mut Window, cx: &mut Context<MailView>) -> AnyElement {
    let t = cx.tokens().clone();
    let body = if view.mail.rows().is_empty() {
        empty(view, &t, cx)
    } else {
        rows(view, window, cx)
    };
    let pane = Pane::new("list-pane")
        .child(header(view, &t, cx))
        .children(filters::pills(view, cx))
        .children(filters::results(view, &t, cx))
        .children(super::status::line(&view.hub, cx))
        .children(status(view, cx))
        .child(body)
        .children(undo(view, cx));
    // The box of the pane (flex_1, or a width) is the cached view's own
    // (`panes::Shape::list_style`); the pane fills it.
    pane.into_any_element()
}

/// The view's name (or "Search results"), a folder's account under it,
/// and the "..." menu.
fn header(view: &MailView, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let mail = &view.mail;
    let title = if mail.searching() {
        text::search_results()
    } else {
        text::view_name(mail.view(), view.view_folder())
    };
    let sub = mail
        .view_account()
        .map(|a| Mail::account_label(a).to_string());
    let entity = cx.entity().downgrade();
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(space::SP_1))
        .pt(px(space::SP_4 + space::SP_0_5))
        .px(px(space::SP_5))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("list-title")
                        .role(Role::Heading)
                        .aria_label(title.clone())
                        .typo(&t.text.subtitle)
                        .text_color(t.color.ink)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(title)
                        .test_support(),
                )
                .when_some(sub, |el, sub| {
                    el.child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(t.color.ink_3)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(sub),
                    )
                }),
        )
        .child(sync_control(view, cx))
        .child(
            IconButton::new("list-more", Icon::More, text::more_actions())
                .disabled(!mail.lists_messages())
                .dropdown_menu(move |menu, window, cx| {
                    menus::list_menu(entity.clone(), menu, window, cx)
                }),
        )
        .into_any_element()
}

/// Refresh (F5), named with the last sync pass ("Refresh (F5). Updated
/// 14:05"); while an account syncs, the brand's three spots take its place
/// (mockup `.lh .tools`), so the header never grows a second busy mark.
fn sync_control(view: &MailView, cx: &mut Context<MailView>) -> AnyElement {
    let hub = view.hub.read(cx);
    if hub.sync().syncing() {
        return div()
            .size(px(crate::theme::layout::ICON_BTN))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .child(view.loader.clone())
            .into_any_element();
    }
    let updated = hub.sync().updated().map(komorebi::format::time);
    let name = super::sync_text::refresh(updated.as_deref());
    let t = cx.tokens().clone();
    // "Updated 14:05" in words beside it, unless the window is compact
    // (the Slint header; the name says it either way).
    let shown = updated
        .filter(|_| !view.compact_now)
        .map(|time| super::sync_text::updated_at(&time));
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_1))
        .children(shown.map(|words| {
            div()
                .typo(&t.text.caption)
                .text_color(t.color.ink_3)
                .whitespace_nowrap()
                .child(words)
        }))
        .child(
            IconButton::new("list-refresh", Icon::Sync, name)
                .access_key(Key::Refresh)
                .disabled(view.mail.accounts().is_empty())
                .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
        )
        .into_any_element()
}

/// The status line (an account without an Archive, a cache error).
pub(super) fn status(view: &MailView, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let line = view.mail.status()?;
    let kind = match line {
        super::model::Status::Error(_) => Kind::Error,
        _ => Kind::Info,
    };
    Some(
        div()
            .flex_none()
            .px(px(space::SP_3))
            .pt(px(space::SP_1))
            .pb(px(space::SP_2))
            .child(
                Notice::new("list-status", kind, text::status(line)).action(
                    Button::new("status-close")
                        .variant(Variant::OnInset)
                        .label(text::close())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act(window, cx, Mail::dismiss_status);
                        })),
                ),
            )
            .into_any_element(),
    )
}

/// The progress bar's height under an empty list's first download.
const BAR_H: f32 = 4.;

/// What an empty list says, centred in the pane (the Slint empty state):
/// an icon for what is empty (a search, a filter, mail), the sentence, a
/// new account's first download as a bar, the search's scope while
/// searching, and Show all under a filter. It keeps the list's focus (and
/// its keys), so the window's keys still reach it.
fn empty(view: &MailView, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let kind = view.mail.empty();
    let said = text::empty(kind, komorebi_i18n::current());
    let searching = view.mail.searching();
    let filter = view.mail.filter();
    let icon = if searching {
        Icon::Search
    } else {
        match filter {
            komorebi::db::Filter::Flagged => Icon::Flag,
            komorebi::db::Filter::Unread => Icon::MailRead,
            komorebi::db::Filter::All => Icon::Mail,
        }
    };
    // The bar only once the count is known: before that the sentence says
    // the download started (a still part-filled bar would look like real
    // progress, and the brand's motion rules leave no running bar).
    let fill = match kind {
        super::model::Empty::Downloading(done, total) if total > 0 => {
            Some((done as f32 / total as f32).clamp(0., 1.))
        }
        _ => None,
    };
    let show_all = !searching && filter != komorebi::db::Filter::All && view.mail.lists_messages();
    let c = t.color;
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .key_context(commands::MESSAGES)
        .child(
            div()
                .id("list-empty-pane")
                .key_context(commands::LIST)
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(space::SP_2))
                .px(px(space::SP_6))
                .child(
                    svg()
                        .path(icon.path())
                        .size(px(crate::theme::layout::ICON_BTN_SM)) // 32 px, the Slint empty icon
                        .text_color(c.ink_3),
                )
                .child(
                    div()
                        .id("list-empty")
                        .role(Role::Status)
                        .aria_label(said.clone())
                        .text_center()
                        .typo(&t.text.body)
                        .text_color(c.ink_2)
                        .child(said)
                        .test_support(),
                )
                .children(fill.map(|fill| {
                    div()
                        .id("list-first-download")
                        .role(Role::ProgressIndicator)
                        .aria_label(text::downloading_your_mail())
                        .aria_numeric_value(f64::from((fill * 100.).round()))
                        .aria_min_numeric_value(0.)
                        .aria_max_numeric_value(100.)
                        .pt(px(space::SP_2))
                        .w(px(crate::theme::layout::LIST_W_COMPACT))
                        .max_w_full()
                        .child(progress_bar(fill, t))
                        .test_support()
                }))
                .when(searching, |el| {
                    el.child(
                        div()
                            .text_center()
                            .typo(&t.text.caption)
                            .text_color(c.ink_2)
                            .child(text::search_hint()),
                    )
                })
                .when(show_all, |el| {
                    el.child(
                        Button::new("list-show-all")
                            .variant(Variant::Subtle)
                            .label(text::show_all())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.act(window, cx, |m| m.set_filter(komorebi::db::Filter::All));
                            })),
                    )
                })
                .track_focus(&view.list_focus),
        )
        .into_any_element()
}

/// A thin bar filled to `fill` (0 to 1).
fn progress_bar(fill: f32, t: &Tokens) -> AnyElement {
    let c = t.color;
    div()
        .h(px(BAR_H))
        .w_full()
        .rounded(px(crate::theme::radius::FULL))
        .bg(c.line)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(gpui::relative(fill))
                .rounded(px(crate::theme::radius::FULL))
                .bg(c.ink),
        )
        .into_any_element()
}

/// The rows: a virtual list, one tab stop, its keys in [`commands::LIST`].
fn rows(view: &mut MailView, window: &mut Window, cx: &mut Context<MailView>) -> AnyElement {
    let count = view.mail.rows().len();
    let scroll = view.list_scroll.clone();
    let bar = scroll.0.borrow().base_handle.clone();
    let focused = view.list_focus.is_focused(window);
    let keyboard = focused && window.last_input_was_keyboard();
    let list = uniform_list(
        "rows",
        count,
        cx.processor(
            move |this: &mut MailView, range: Range<usize>, _window, cx| {
                let t = cx.tokens().clone();
                let checks = this.mail.checks();
                let plain = !this.mail.lists_messages() || this.mail.list_drafts();
                let open = this.mail.open_id();
                let current = this.mail.current_index();
                let total = this.mail.rows().len();
                let messages = this.mail.lists_messages() && !this.mail.list_drafts();
                let hovered = this.hovered_row;
                range
                    .filter_map(|ix| {
                        let row = this.mail.rows().row(ix)?;
                        let at = current == Some(ix);
                        let look = RowLook {
                            selected: row.selected || open == Some(row.id),
                            checks,
                            ring: keyboard && at,
                            cursor: at,
                            plain,
                            quick: messages && (hovered == Some(row.id) || (keyboard && at)),
                            position: ix + 1,
                            of: total,
                        };
                        Some(rows::render(&row, look, &t, cx))
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(&scroll)
    .size_full();
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .key_context(commands::MESSAGES)
        .child(
            div()
                .id("messages")
                .key_context(commands::LIST)
                .role(Role::List)
                .aria_label(text::messages())
                .size_full()
                .child(list)
                .test_support()
                .track_focus(&view.list_focus),
        )
        .vertical_scrollbar(&bar)
        .into_any_element()
}

/// The Undo offer's own slot (a status line never replaces it).
pub(super) fn undo(view: &MailView, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let removal = view.mail.removal()?;
    let place = removal.to.and_then(|id| view.mail.folder(id));
    let said = text::removal(&removal, place);
    Some(
        div()
            .id("undo-slot")
            .flex_none()
            .px(px(space::SP_2))
            .pb(px(space::SP_2))
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                let now = std::time::Instant::now();
                let on = *hovered;
                this.act(window, cx, |m| m.undo_hold(on, now));
            }))
            .child(
                Notice::new("undo", Kind::Info, said).action(
                    Button::new("undo-button")
                        .variant(Variant::OnInset)
                        .label(text::undo())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act(window, cx, Mail::undo);
                        })),
                ),
            )
            .into_any_element(),
    )
}
