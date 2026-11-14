//! Under the list's title: the All / Unread / Important pills (mockup
//! `.pv`; the Slint app's SelectorBar), and while a search runs, what it
//! found where with the ways on (Search all folders, Clear search).

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, prelude::*, px};

use super::{MailView, text};
use crate::theme::{TextStyled as _, Tokens, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::pill::FilterPills;

/// The filter pills, in the views of mail to triage.
pub fn pills(view: &MailView, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let mail = &view.mail;
    if !mail.lists_messages() || mail.list_drafts() {
        return None;
    }
    let entity = cx.entity().downgrade();
    Some(
        div()
            .flex_none()
            .px(px(space::SP_5))
            .pt(px(space::SP_3))
            .pb(px(space::SP_2))
            .child(
                FilterPills::new("filter", text::filter_name())
                    .items(text::filters())
                    .selected(text::filter_index(mail.filter()))
                    .on_select(move |ix, window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.act(window, cx, |m| m.set_filter(text::filter_at(ix)));
                        });
                    }),
            )
            .into_any_element(),
    )
}

/// While a search runs: the count and where, then the ways on.
pub fn results(view: &MailView, t: &Tokens, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let mail = &view.mail;
    if !mail.searching() {
        return None;
    }
    let all = mail.search_all() && mail.lists_messages();
    let view_name = text::view_name(mail.view(), view.view_folder());
    let line = text::results(mail.rows().len(), all, &view_name);
    let show_all = !mail.search_all() && mail.lists_messages();
    Some(
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::SP_1))
            .px(px(space::SP_5))
            .pb(px(space::SP_2))
            .child(
                div()
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(line),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(space::SP_1))
                    // The buttons' words line up with the title above.
                    .ml(px(-space::SP_4))
                    // Back from all folders to the view (the Slint ScopeBox's
                    // first entry).
                    .when(all, |el| {
                        el.child(
                            Button::new("search-here")
                                .variant(Variant::Subtle)
                                .label(text::scope_here(mail.view()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.act(window, cx, |m| m.set_search_all(false));
                                })),
                        )
                    })
                    .when(show_all, |el| {
                        el.child(
                            Button::new("search-all")
                                .variant(Variant::Subtle)
                                .label(text::search_all_folders())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.act(window, cx, |m| m.set_search_all(true));
                                })),
                        )
                    })
                    .child(
                        Button::new("clear-search")
                            .variant(Variant::Subtle)
                            .label(text::clear_search())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_search(window, cx);
                            })),
                    ),
            )
            .into_any_element(),
    )
}
