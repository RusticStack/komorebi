//! The calm panel: what the reading pane's room shows while nothing is
//! selected ("Read. Reply. Rest."). The list grows wider (longer rows and
//! previews) and the panel takes the rest ([`layout::calm_w`]):
//!
//! - **Today**, while the view has unread mail: how many, the newest few
//!   unread (spot, sender, subject, time; a click opens it), and the drafts
//!   saved on this PC (a click opens one in the composer; the section is
//!   left out when there are none).
//! - **Rest**, when nothing is unread: the Rest scene (`scene.rs`) and one
//!   calm line.
//!
//! Selecting a message (or anything that fills the reading pane) returns
//! to the usual list and reading pane. The panel fades in (`CALM_MS`), at
//! once under reduce motion. Searches, the Drafts and Outbox views and the
//! one-pane width keep the usual layout ([`mode`]).

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Role, Styled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::db::Filter;
use komorebi::i18n::Lang;
use komorebi::rows::{Kind, Row, Rows};
use komorebi_i18n::{tr, trn};

use super::model::{AccountState, Mail, View};
use super::{MailView, scene, text};
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, motion, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::pane::{ListRow, Pane};
use crate::widgets::spot::SenderSpot;

/// What the room beside the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The usual list and reading pane (something is selected, or the
    /// view has nothing to summarise).
    Split,
    /// Nothing selected, unread mail waiting: the summary.
    Today,
    /// Nothing selected, nothing unread: the scene and the calm line.
    Rest,
}

/// What the mail view is doing, for [`mode`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Moment {
    /// The composer shows in the reading pane.
    pub composing: bool,
    /// A message is open.
    pub open: bool,
    /// Rows selected (two or more show the selection panel).
    pub selected: usize,
    /// An Outbox entry shows.
    pub outbox: bool,
    /// The window shows one pane at a time.
    pub one_pane: bool,
    /// The list has no rows (its own empty state takes the whole room).
    pub list_empty: bool,
    /// The view lists messages (All inboxes or a folder).
    pub lists_messages: bool,
    /// A query is in the search box.
    pub searching: bool,
    /// The view's unread mail.
    pub unread: i64,
}

/// Today, Rest, or the usual split.
pub fn mode(m: &Moment) -> Mode {
    let busy = m.composing || m.open || m.selected > 0 || m.outbox;
    if busy || m.one_pane || m.list_empty || !m.lists_messages || m.searching {
        Mode::Split
    } else if m.unread > 0 {
        Mode::Today
    } else {
        Mode::Rest
    }
}

/// The calm line under the Rest scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestLine {
    /// The inboxes: the brand's line.
    Inbox,
    /// An account shown could not be reached at its last sync: nothing new
    /// is all this PC can say.
    Offline,
    /// Another folder, by id.
    Folder(i64),
}

/// The summary's content, read from the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// The view's unread mail.
    pub unread: i64,
    /// The newest unread messages, newest first.
    pub picks: Vec<Row>,
    /// The drafts saved last, last first.
    pub drafts: Vec<Row>,
    /// All the saved drafts.
    pub draft_count: i64,
}

/// What the summary was read for: it is read again when any of it moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    view: View,
    unread: i64,
    drafts: i64,
    flags: u64,
    held: Option<u64>,
    rows: usize,
    lang: Lang,
}

impl Mail {
    /// The unread mail of the view shown: never in Sent or Drafts (their
    /// rows are never bold), none in the Drafts and Outbox views.
    pub fn view_unread(&self) -> i64 {
        if !self.lists_messages() || self.view_role().is_outgoing() {
            return 0;
        }
        let unread = self.shown_unread();
        self.view_folders()
            .iter()
            .map(|f| unread.get(f).copied().unwrap_or(0))
            .sum()
    }

    /// The line the Rest scene says.
    pub fn rest_line(&self) -> RestLine {
        let shown = self.view_folders();
        let unreachable = self
            .folders
            .iter()
            .filter(|f| shown.contains(&f.id))
            .filter_map(|f| self.account_state.get(&f.account_id))
            .any(|s| matches!(s, AccountState::Unreachable | AccountState::Failed));
        match self.view {
            _ if unreachable => RestLine::Offline,
            View::Folder(id) if self.folder_role(id) != komorebi::role::Role::Inbox => {
                RestLine::Folder(id)
            }
            _ => RestLine::Inbox,
        }
    }

    pub fn summary_key(&self, lang: Lang) -> Key {
        Key {
            view: self.view,
            unread: self.view_unread(),
            drafts: self.draft_count,
            flags: self.flags_seq,
            held: self.held.as_ref().map(|h| h.seq),
            rows: self.rows.len(),
            lang,
        }
    }

    /// The newest unread messages of the view and the drafts saved last:
    /// two small indexed queries and a few rows.
    pub fn summary(&self, lang: Lang) -> Summary {
        let unread = self.view_unread();
        let mut ids = if unread > 0 {
            self.db
                .message_ids_filtered(&self.view_folders(), "", Filter::Unread)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(held) = &self.held {
            ids.retain(|id| !held.ids.contains(id));
        }
        ids.truncate(layout::CALM_UNREAD_ROWS);
        let messages = Rows::new(self.db.clone());
        messages.set_lang(lang);
        if self.view == View::AllInboxes && self.accounts.len() > 1 {
            messages.set_account_tags(
                self.accounts
                    .iter()
                    .map(|a| (a.id, Self::account_label(a).to_string()))
                    .collect(),
            );
        }
        let n = ids.len();
        messages.set(ids, Kind::Received);
        let picks = (0..n).filter_map(|ix| messages.row(ix)).collect();
        let mut drafts = Vec::new();
        if self.draft_count > 0 {
            let mut ids = self.db.draft_ids("").unwrap_or_default();
            ids.truncate(layout::CALM_DRAFT_ROWS);
            let rows = Rows::new(self.db.clone()).with_above_quote(komorebi::compose::above_quote);
            rows.set_lang(lang);
            let n = ids.len();
            rows.set(ids, Kind::Drafts);
            drafts = (0..n).filter_map(|ix| rows.row(ix)).collect();
        }
        Summary {
            unread,
            picks,
            drafts,
            draft_count: self.draft_count,
        }
    }

    /// A draft of the summary opens in the composer (the view stays).
    pub fn open_summary_draft(&mut self, id: i64) {
        self.effects.push(super::model::Effect::OpenDraft(id));
    }
}

// ------------------------------------------------------------- words

/// The panel's landmark name.
pub fn pane_name() -> String {
    tr!("Summary")
}

pub fn unread_title(n: i64) -> String {
    trn!("{n} unread message", "{n} unread messages", n)
}

pub fn newest_unread() -> String {
    tr!("Newest unread")
}

pub fn show_all_unread() -> String {
    tr!("Show all unread")
}

pub fn drafts_waiting(n: i64) -> String {
    trn!("{n} draft waiting", "{n} drafts waiting", n)
}

pub fn show_all_drafts() -> String {
    tr!("Show all drafts")
}

/// The calm line.
pub fn rest_words(line: RestLine, mail: &Mail) -> String {
    match line {
        RestLine::Inbox => tr!("Read. Replied. Now rest."),
        RestLine::Offline => tr!("Nothing new on this PC."),
        RestLine::Folder(id) => match mail.folder(id) {
            Some(f) => tr!("Nothing unread in {0}.", text::folder_label(f)),
            None => tr!("Read. Replied. Now rest."),
        },
    }
}

// ------------------------------------------------------------- views

fn item_id(prefix: &str, id: i64) -> ElementId {
    ElementId::Name(format!("{prefix}-{id}").into())
}

/// The calm panel, `width` px wide, in place of the reading pane.
pub fn render(
    view: &mut MailView,
    mode: Mode,
    width: f32,
    window: &mut Window,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let (fade_id, content) = match mode {
        Mode::Today => ("calm-fade-today", today(view, &t, window, cx)),
        _ => ("calm-fade-rest", rest(view, width, cx)),
    };
    let fade = t.duration(motion::CALM_MS);
    let content: AnyElement = if fade.is_zero() {
        content
    } else {
        div()
            .size_full()
            .child(content)
            .with_animation(
                fade_id,
                Animation::new(fade).with_easing(motion::ease_enter),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
    };
    div()
        .flex_none()
        .w(px(width))
        .h_full()
        .child(
            Pane::new("calm-pane").name(pane_name()).child(
                div()
                    .id("calm")
                    .size_full()
                    .child(content)
                    .track_focus(&view.reading_focus),
            ),
        )
        .into_any_element()
}

/// Today: the count, the newest unread, the drafts.
fn today(
    view: &mut MailView,
    t: &Tokens,
    _window: &mut Window,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let lang = komorebi_i18n::current();
    let key = view.mail.summary_key(lang);
    if view.calm.as_ref().is_none_or(|(k, _)| *k != key) {
        view.calm = Some((key, view.mail.summary(lang)));
    }
    let Some((_, s)) = view.calm.as_ref() else {
        return div().into_any_element();
    };
    let s = s.clone();
    let c = t.color;
    let title = unread_title(s.unread);
    let more_unread = s.unread > s.picks.len() as i64 && view.mail.filter() == Filter::All;
    let n_picks = s.picks.len();
    let picks = s.picks.into_iter().enumerate().map(|(ix, r)| {
        let id = r.id;
        // The list's name ("Newest unread") already says they are unread;
        // the name also stays apart from the message's row in the list.
        let name = text::row_name(&Row {
            unread: false,
            ..r.clone()
        });
        item(
            item_id("calm-unread", id),
            name,
            &r,
            true,
            (ix + 1, n_picks),
            t,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.act(window, cx, |m| {
                m.select_message(id);
            });
            window.focus(&this.list_focus, cx);
        }))
    });
    let n_drafts = s.drafts.len();
    let drafts = s.drafts.into_iter().enumerate().map(|(ix, mut r)| {
        let id = r.id;
        // Named as a draft ("Draft, To: Ana, Dinner, 14:05").
        r.draft = true;
        let name = text::row_name(&r);
        item(
            item_id("calm-draft", id),
            name,
            &r,
            false,
            (ix + 1, n_drafts),
            t,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.act(window, cx, |m| m.open_summary_draft(id));
        }))
    });
    let heading = |id: &'static str, said: String, token: &crate::theme::TextToken| {
        div()
            .id(id)
            .role(Role::Heading)
            .aria_label(said.clone())
            .typo(token)
            .text_color(c.ink)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(said)
            .test_support()
    };
    let list = |id: &'static str, name: String| {
        div()
            .id(id)
            .role(Role::List)
            .aria_label(name)
            .px(px(space::SP_2))
            .flex()
            .flex_col()
            .gap(px(scaled::ROW_GAP))
            .test_support()
    };
    div()
        .id("calm-today")
        .size_full()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        // The title sits on the list title's line (the list's header).
        .pt(px(space::SP_4 + space::SP_0_5))
        .pb(px(space::SP_4))
        .child(
            div()
                .px(px(space::SP_5))
                .child(heading("calm-title", title, &t.text.subtitle)),
        )
        .child(
            list("calm-unread", newest_unread())
                .mt(px(space::SP_3))
                .children(picks),
        )
        .when(more_unread, |el| {
            el.child(
                div().px(px(space::SP_3)).pt(px(space::SP_1)).child(
                    Button::new("calm-show-unread")
                        .variant(Variant::Subtle)
                        .label(show_all_unread())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act(window, cx, |m| m.set_filter(Filter::Unread));
                        })),
                ),
            )
        })
        .when(s.draft_count > 0, |el| {
            let more = s.draft_count > n_drafts as i64;
            el.child(div().mt(px(space::SP_6)).px(px(space::SP_5)).child(heading(
                "calm-drafts-title",
                drafts_waiting(s.draft_count),
                &t.text.body_strong,
            )))
            .child(
                list("calm-drafts", text::drafts())
                    .mt(px(space::SP_2))
                    .children(drafts),
            )
            .when(more, |el| {
                el.child(
                    div().px(px(space::SP_3)).pt(px(space::SP_1)).child(
                        Button::new("calm-show-drafts")
                            .variant(Variant::Subtle)
                            .label(show_all_drafts())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.act(window, cx, |m| m.set_view(View::Drafts));
                            })),
                    ),
                )
            })
        })
        .into_any_element()
}

/// One message or draft of the summary: its spot, who and when, the
/// subject. A tab stop of its own, with the brand ring.
fn item(
    id: ElementId,
    name: String,
    r: &Row,
    unread: bool,
    (position, of): (usize, usize),
    t: &Tokens,
) -> ListRow {
    let c = t.color;
    let subject = if r.subject.trim().is_empty() {
        text::no_subject()
    } else {
        r.subject.clone()
    };
    let who = if unread {
        t.text.body_strong.weight
    } else {
        t.text.body.weight
    };
    ListRow::new(id, name)
        .position(position, of)
        .child(SenderSpot::new(&r.who).unread(unread))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .py(px(scaled::ROW_PAD))
                .flex()
                .flex_col()
                .typo(&t.text.body)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::SP_2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_weight(who)
                                .text_color(c.ink)
                                .child(r.sender.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .typo(&t.text.caption)
                                .text_color(c.ink_3)
                                .child(r.date.clone()),
                        ),
                )
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(c.ink_2)
                        .child(subject),
                ),
        )
}

/// Rest: the scene fills the panel, with the calm line in its sky
/// (`scene.rs`). A cached view: its frames redraw it alone, the mail view
/// and the window around it are replayed (GPUI's in-place splice).
fn rest(view: &mut MailView, width: f32, cx: &mut Context<MailView>) -> AnyElement {
    let said = rest_words(view.mail.rest_line(), &view.mail);
    let scene = view
        .rest_scene
        .get_or_insert_with(|| cx.new(|_| scene::RestScene::new()))
        .clone();
    scene.update(cx, |s, cx| s.set(said.into(), width, cx));
    div()
        .size_full()
        .child(scene.cached(gpui::StyleRefinement::default().size_full()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> Moment {
        Moment {
            lists_messages: true,
            ..Moment::default()
        }
    }

    #[test]
    fn unread_mail_with_nothing_selected_is_today() {
        let m = Moment {
            unread: 3,
            ..idle()
        };
        assert_eq!(mode(&m), Mode::Today);
    }

    #[test]
    fn nothing_unread_is_rest() {
        assert_eq!(mode(&idle()), Mode::Rest);
    }

    #[test]
    fn anything_in_the_reading_pane_is_the_usual_split() {
        let base = Moment {
            unread: 3,
            ..idle()
        };
        for m in [
            Moment { open: true, ..base },
            Moment {
                selected: 2,
                ..base
            },
            Moment {
                composing: true,
                ..base
            },
            Moment {
                outbox: true,
                ..base
            },
            Moment {
                one_pane: true,
                ..base
            },
            Moment {
                list_empty: true,
                ..base
            },
            Moment {
                searching: true,
                ..base
            },
            Moment {
                lists_messages: false,
                ..base
            },
        ] {
            assert_eq!(mode(&m), Mode::Split, "{m:?}");
        }
    }

    #[test]
    fn the_words_fit_both_languages() {
        komorebi_i18n::with_lang(Lang::En, || {
            assert_eq!(unread_title(1), "1 unread message");
            assert_eq!(unread_title(5), "5 unread messages");
            assert_eq!(drafts_waiting(2), "2 drafts waiting");
        });
        komorebi_i18n::with_lang(Lang::PtPt, || {
            assert_eq!(unread_title(5), "5 mensagens não lidas");
            assert_eq!(drafts_waiting(1), "1 rascunho à espera");
            assert_eq!(pane_name(), "Resumo");
        });
    }
}
