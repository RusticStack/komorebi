//! The address-book sync pane and the CardDAV passes (the Slint app's
//! `SyncPane` and `src/app/contacts_sync.rs`): per account, sync off, read
//! only, or read and write; the server address (Find reads it from the
//! account's domain, without credentials); the user name; the mail password
//! or a separate one kept in Windows Credential Manager; Find address
//! books (the first request with credentials, only on that press); the
//! books with their switches; Sync now; what the last pass said.
//!
//! What runs is the library's (`komorebi::contacts_sync`). A pass is a
//! short-lived thread per account, at most one at a time, on its own
//! connection to the cache (`Db::file`); it starts when the view opens, on
//! Sync now, after a local edit, and from [`ContactsView::timer_tick`]: never
//! before the first frame, never under `KOMOREBI_OFFLINE` (the library's
//! `plan` refuses). The way back to the view is a oneshot channel awaited in
//! `cx.spawn` (cookbook_async).

use std::time::Instant;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, ParentElement, Role, SharedString,
    Styled, WeakEntity, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use komorebi::contacts_sync as dav_sync;
use komorebi::dav::DavError;
use komorebi::db::contacts::DavAccount;

use super::{ContactsView, Side, field, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::widgets::button::{Button, IconButton};
use crate::widgets::dialog::Confirm;
use crate::widgets::field::TextField;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::ScrollArea;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Toggle;

/// What a sync request ended with, for the pane (worded in `text::note`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Offline,
    Auth,
    Redirect,
    NotFound,
    Tls,
    /// Another failure (its English detail).
    Failed(String),
    /// Find address books found none.
    NoneFound,
    /// The cache or the credential store refused (the error chain).
    Detail(String),
}

impl Note {
    pub fn of(e: &DavError) -> Self {
        match e {
            DavError::Offline => Note::Offline,
            DavError::Auth => Note::Auth,
            DavError::AuthRedirect => Note::Redirect,
            DavError::NotFound => Note::NotFound,
            DavError::Tls(_) => Note::Tls,
            other => Note::Failed(other.to_string()),
        }
    }
}

/// The pane's state while it shows.
pub(crate) struct SyncPane {
    /// The account the pane shows.
    pub(crate) account: Option<i64>,
    /// 0 off, 1 read only, 2 read and write, as chosen.
    pub(crate) mode: i64,
    /// The mode before a change to Off waits for its answer.
    pub(crate) previous_mode: i64,
    /// The mail password signs in (else the separate one).
    pub(crate) same: bool,
    pub(crate) url: Entity<InputState>,
    pub(crate) user: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) books: dav_sync::Books,
    pub(crate) note: Option<Note>,
}

type Weak = WeakEntity<ContactsView>;

fn click(
    entity: &Weak,
    f: impl Fn(&mut ContactsView, &mut Window, &mut Context<ContactsView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = entity.clone();
    move |_, window, cx| {
        let _ = entity.update(cx, |this, cx| f(this, window, cx));
    }
}

impl ContactsView {
    fn pane(&mut self) -> Option<&mut SyncPane> {
        match &mut self.side {
            Side::Sync(p) => Some(p),
            _ => None,
        }
    }

    /// Open the sync pane on the last account shown there (or the first).
    pub fn open_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.accounts = self.db.accounts().unwrap_or_default();
        let shown = match &self.side {
            Side::Sync(p) => p.account,
            _ => None,
        };
        let account = shown
            .filter(|id| self.accounts.iter().any(|a| a.id == *id))
            .or_else(|| self.accounts.first().map(|a| a.id));
        let pane = SyncPane {
            account,
            mode: 0,
            previous_mode: 0,
            same: true,
            url: field("", window, cx),
            user: field("", window, cx),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            books: dav_sync::Books::default(),
            note: None,
        };
        self.side = Side::Sync(Box::new(pane));
        self.details_shown = true;
        self.copied = false;
        self.side_scroll.set_offset(gpui::Point::default());
        if let Some(id) = account {
            self.show_dav_form(id, window, cx);
        }
        cx.notify();
    }

    pub fn close_sync(&mut self, cx: &mut Context<Self>) {
        self.side = Side::Detail;
        self.details_shown = self.page.shown().is_some();
        cx.notify();
    }

    /// Fill the pane from `account`'s saved settings.
    fn show_dav_form(&mut self, account: i64, window: &mut Window, cx: &mut Context<Self>) {
        let d = dav_sync::settings(&self.db, &self.accounts, account);
        let books = dav_sync::books(&self.db, account);
        let Some(p) = self.pane() else {
            return;
        };
        p.account = Some(account);
        p.mode = d.mode;
        p.previous_mode = d.mode;
        p.same = !d.separate_secret;
        p.books = books;
        p.note = None;
        let (url, user, password) = (p.url.clone(), p.user.clone(), p.password.clone());
        url.update(cx, |s, cx| s.set_value(d.url.clone(), window, cx));
        user.update(cx, |s, cx| s.set_value(d.user.clone(), window, cx));
        password.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn show_dav_books(&mut self, account: i64, cx: &mut Context<Self>) {
        let books = dav_sync::books(&self.db, account);
        if let Some(p) = self.pane()
            && p.account == Some(account)
        {
            p.books = books;
        }
        cx.notify();
    }

    /// The pane's account, by position in its picker.
    pub fn sync_pick_account(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.accounts.get(index).map(|a| a.id) {
            self.show_dav_form(id, window, cx);
        }
    }

    /// The form's account, with its fields saved (the password too, when
    /// it is a separate one, which then empties).
    fn save_dav_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<DavAccount> {
        let p = self.pane()?;
        let account = p.account?;
        let (url, user, password) = (p.url.clone(), p.user.clone(), p.password.clone());
        let form = dav_sync::Form {
            url: url.read(cx).value().to_string(),
            user: user.read(cx).value().to_string(),
            mode: p.mode,
            same: p.same,
            password: password.read(cx).value().to_string(),
        };
        match dav_sync::save(&self.db, &self.accounts, account, &form) {
            Ok((d, kept)) => {
                if kept {
                    password.update(cx, |s, cx| s.set_value("", window, cx));
                }
                Some(d)
            }
            Err(e) => {
                if let Some(p) = self.pane() {
                    p.note = Some(Note::Detail(format!("{e:#}")));
                }
                cx.notify();
                None
            }
        }
    }

    /// Sync off, read only, or read and write. Off asks first when there
    /// are synced contacts.
    pub fn sync_pick_mode(&mut self, mode: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.pane() else {
            return;
        };
        let Some(account) = p.account else {
            return;
        };
        let previous = p.previous_mode;
        p.mode = mode;
        if dav_sync::asks_to_remove(&self.db, account, previous, mode) {
            let (yes, no) = (cx.entity().downgrade(), cx.entity().downgrade());
            super::ask_two(
                Confirm {
                    title: text::remove_question().into(),
                    message: text::remove_question_text().into(),
                    confirm: text::remove().into(),
                    cancel: text::keep().into(),
                    destructive: true,
                },
                move |window, cx| {
                    let _ = yes.update(cx, |v, cx| v.sync_off(true, window, cx));
                },
                move |window, cx| {
                    let _ = no.update(cx, |v, cx| v.sync_off(false, window, cx));
                },
                window,
                cx,
            );
            cx.notify();
            return;
        }
        if self.save_dav_form(window, cx).is_some() {
            if let Some(p) = self.pane() {
                p.previous_mode = mode;
            }
            self.show_dav_books(account, cx);
            self.reload(true, cx);
        }
    }

    /// The answer to "Remove the synced contacts?": sync turns off either
    /// way; yes removes the synced contacts, no keeps them on this
    /// computer, no longer synced.
    pub fn sync_off(&mut self, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.pane().and_then(|p| p.account) else {
            return;
        };
        if !remove {
            dav_sync::turn_off_keeping(&self.db, account);
            if let Some(p) = self.pane() {
                p.previous_mode = 0;
                p.mode = 0;
            }
            cx.notify();
            return;
        }
        if self.save_dav_form(window, cx).is_some() {
            if let Some(p) = self.pane() {
                p.previous_mode = 0;
            }
            self.show_dav_books(account, cx);
            self.reload(true, cx);
        }
    }

    pub fn set_same_password(&mut self, same: bool, cx: &mut Context<Self>) {
        if let Some(p) = self.pane() {
            p.same = same;
        }
        cx.notify();
    }

    fn busy_now(&mut self, account: i64, on: bool, cx: &mut Context<Self>) {
        if on {
            self.busy.insert(account);
            if let Some(p) = self.pane()
                && p.account == Some(account)
            {
                p.note = None;
            }
        } else {
            self.busy.remove(&account);
        }
        cx.notify();
    }

    /// Find: the server address from the account's domain, without
    /// credentials.
    pub fn find_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.pane().and_then(|p| p.account) else {
            return;
        };
        let Some(domain) = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .and_then(|a| dav_sync::domain_of(&a.email))
        else {
            return;
        };
        self.busy_now(account, true, cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dav_sync::find_server(&domain));
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(found) = rx.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy_now(account, false, cx);
                let Some(p) = this.pane() else {
                    return;
                };
                if p.account != Some(account) {
                    return;
                }
                match found {
                    Ok(url) => {
                        let input = p.url.clone();
                        input.update(cx, |s, cx| s.set_value(url, window, cx));
                    }
                    Err(e) => p.note = Some(Note::of(&e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Find address books: the first request with credentials.
    pub fn find_books(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.save_dav_form(window, cx) else {
            return;
        };
        if d.mode == 0 || d.url.is_empty() {
            return;
        }
        let account = d.account_id;
        let pass = match dav_sync::password(&d) {
            Ok(p) => p,
            Err(e) => {
                if let Some(p) = self.pane() {
                    p.note = Some(Note::Detail(format!("{e:#}")));
                }
                cx.notify();
                return;
            }
        };
        self.busy_now(account, true, cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        let settings = d.clone();
        std::thread::spawn(move || {
            let _ = tx.send(dav_sync::find_books(&settings, &pass));
        });
        cx.spawn(async move |this, cx| {
            let Ok(found) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.busy_now(account, false, cx);
                match found {
                    Ok(books) if books.is_empty() => this.note_for(account, Note::NoneFound, cx),
                    Ok(books) => {
                        this.db.set_found_books(account, d.mode, &books).ok();
                        this.show_dav_books(account, cx);
                        this.start_pass(account, cx);
                    }
                    Err(e) => this.note_for(account, Note::of(&e), cx),
                }
            });
        })
        .detach();
    }

    fn note_for(&mut self, account: i64, note: Note, cx: &mut Context<Self>) {
        if let Some(p) = self.pane()
            && p.account == Some(account)
        {
            p.note = Some(note);
        }
        cx.notify();
    }

    /// Turn the book at `index` of the pane's account on or off.
    pub fn set_book(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let Some(account) = self.pane().and_then(|p| p.account) else {
            return;
        };
        if dav_sync::set_book_enabled(&self.db, account, index, on) {
            self.show_dav_books(account, cx);
            self.reload(true, cx);
            if on {
                self.start_pass(account, cx);
            }
        }
    }

    /// Sync now, for the pane's account.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.pane().and_then(|p| p.account) {
            self.start_pass(id, cx);
        }
    }

    /// One pass for `account`, unless one is running or sync is off.
    pub fn start_pass(&mut self, account: i64, cx: &mut Context<Self>) {
        if self.busy.contains(&account) {
            return;
        }
        let Some(plan) = dav_sync::plan(&self.db, account) else {
            return;
        };
        let Some(path) = self.db.file() else {
            return;
        };
        self.last_pass.insert(account, Instant::now());
        self.busy_now(account, true, cx);
        let started = Instant::now();
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dav_sync::run(&path, &plan));
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.busy_now(account, false, cx);
                match &result {
                    Ok(o) => {
                        komorebi::diag::record(account, "CARDDAV", "ok", started.elapsed());
                        if o.changed {
                            this.reload(true, cx);
                        }
                    }
                    Err(e) => {
                        komorebi::diag::record(account, "CARDDAV", "failed", started.elapsed());
                        this.note_for(account, Note::of(e), cx);
                    }
                }
                this.show_dav_books(account, cx);
            });
        })
        .detach();
    }

    /// A local edit or the view opening: a pass for each account that
    /// syncs.
    pub(crate) fn kick(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in ids {
            self.start_pass(id, cx);
        }
    }

    /// From the mail check timer (the shell's): a pass for each account
    /// whose interval has passed.
    pub fn timer_tick(&mut self, cx: &mut Context<Self>) {
        // Before the view first shows, it has not read the accounts.
        self.accounts = self.db.accounts().unwrap_or_default();
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        for id in dav_sync::due(&self.db, &ids, &self.last_pass) {
            self.start_pass(id, cx);
        }
    }
}

/// A field with its label above and an optional note under it.
fn labelled(t: &Tokens, label: String, body: impl IntoElement, note: Option<String>) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_1))
        .child(
            div()
                .typo(&t.text.caption)
                .text_color(t.color.ink_2)
                .child(label),
        )
        .child(body)
        .children(note.map(|n| {
            div()
                .typo(&t.text.caption)
                .text_color(t.color.ink_3)
                .child(n)
        }))
        .into_any_element()
}

pub fn render(
    view: &mut ContactsView,
    single: bool,
    _window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let entity = cx.entity().downgrade();
    let Side::Sync(p) = &view.side else {
        return div().into_any_element();
    };
    let head = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_2))
        .when(single, |el| {
            el.child(
                IconButton::new("sync-back", Icon::ChevronLeft, text::back())
                    .on_click(click(&entity, |this, window, cx| this.back(window, cx))),
            )
        })
        .child(
            div()
                .id("sync-title")
                .role(Role::Heading)
                .aria_label(text::sync_title())
                .flex_1()
                .min_w_0()
                .typo(&t.text.subtitle)
                .text_color(t.color.ink)
                .child(text::sync_title())
                .test_support(),
        )
        .when(!single, |el| {
            el.child(
                IconButton::new("sync-close", Icon::Dismiss, text::close())
                    .on_click(click(&entity, |this, _, cx| this.close_sync(cx))),
            )
        });
    let intro = div()
        .typo(&t.text.body)
        .text_color(t.color.ink_2)
        .child(text::sync_intro());
    let body = match p.account {
        None => div()
            .typo(&t.text.body)
            .text_color(t.color.ink_2)
            .child(text::add_account_first())
            .into_any_element(),
        Some(account) => {
            let url_set = !p.url.read(cx).value().is_empty();
            form(view, p, account, url_set, &t, &entity)
        }
    };
    div()
        .size_full()
        .min_h_0()
        .child(
            ScrollArea::new("sync-scroll", &view.side_scroll).child(
                div()
                    .w_full()
                    .max_w(px(layout::DIALOG_W
                        + layout::LIST_W_COMPACT / 2.
                        + space::SP_8
                        + space::SP_8))
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_4))
                    .px(px(space::SP_8))
                    .pt(px(space::SP_6))
                    .pb(px(space::SP_8))
                    .child(head)
                    .child(intro)
                    .child(body),
            ),
        )
        .into_any_element()
}

fn form(
    view: &ContactsView,
    p: &SyncPane,
    account: i64,
    url_set: bool,
    t: &Tokens,
    entity: &Weak,
) -> AnyElement {
    let busy = view.busy.contains(&account);
    let at = view
        .accounts
        .iter()
        .position(|a| a.id == account)
        .unwrap_or(0);
    let pick_account = entity.clone();
    let accounts = Dropdown::new("sync-account", text::account(), at.to_string())
        .choices(
            view.accounts
                .iter()
                .enumerate()
                .map(|(i, a)| (i.to_string(), a.email.clone())),
        )
        .on_pick(move |v, window, cx| {
            if let Ok(i) = v.parse::<usize>() {
                let _ = pick_account.update(cx, |this, cx| this.sync_pick_account(i, window, cx));
            }
        });
    let pick_mode = entity.clone();
    let mode = Dropdown::new("sync-mode", text::sync_books(), p.mode.to_string())
        .choices((0..=2).map(|m| (m.to_string(), text::mode(m))))
        .on_pick(move |v, window, cx| {
            if let Ok(m) = v.parse::<i64>() {
                let _ = pick_mode.update(cx, |this, cx| this.sync_pick_mode(m, window, cx));
            }
        });
    let same = {
        let entity = entity.clone();
        Toggle::new("sync-same", text::same_password(), p.same).on_change(move |on, _, cx| {
            let _ = entity.update(cx, |this, cx| this.set_same_password(on, cx));
        })
    };
    let books = p.books.lines.iter().enumerate().map(|(i, (name, on))| {
        let entity = entity.clone();
        div()
            .w_full()
            .min_h(px(layout::CONTROL_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_3))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .typo(&t.text.body)
                    .text_color(t.color.ink)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name.clone()),
            )
            .child(
                Toggle::new(("sync-book", i), name.clone(), *on).on_change(move |on, _, cx| {
                    let _ = entity.update(cx, |this, cx| this.set_book(i, on, cx));
                }),
            )
    });
    let status = if busy {
        Some(text::working())
    } else {
        p.books.status().map(|(count, at)| text::status(count, at))
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_4))
        .child(labelled(
            t,
            text::account(),
            div().flex().child(accounts),
            None,
        ))
        .child(labelled(
            t,
            text::sync_books(),
            div().flex().child(mode),
            None,
        ))
        .child(labelled(
            t,
            text::server_address(),
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(space::SP_2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(TextField::new(&p.url, text::server_address())),
                )
                .child(
                    Button::new("sync-find")
                        .label(text::find())
                        .disabled(busy)
                        .on_click(click(entity, |this, window, cx| {
                            this.find_server(window, cx)
                        })),
                ),
            Some(text::server_note()),
        ))
        .child(labelled(
            t,
            text::user_name(),
            TextField::new(&p.user, text::user_name()),
            None,
        ))
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(space::SP_3))
                .child(
                    div()
                        .flex_1()
                        .typo(&t.text.body)
                        .text_color(t.color.ink)
                        .child(text::same_password()),
                )
                .child(same),
        )
        .when(!p.same, |el| {
            el.child(labelled(
                t,
                text::password(),
                TextField::new(&p.password, text::password()),
                Some(text::password_note()),
            ))
        })
        .children(
            p.note
                .as_ref()
                .map(|n| Notice::new("sync-note", Kind::Error, text::note(n))),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(space::SP_2))
                .child(
                    Button::new("sync-find-books")
                        .primary()
                        .label(text::find_books())
                        .disabled(busy || p.mode == 0 || !url_set)
                        .on_click(click(entity, |this, window, cx| {
                            this.find_books(window, cx)
                        })),
                )
                .child(
                    Button::new("sync-now")
                        .icon(Icon::Sync)
                        .label(text::sync_now())
                        .disabled(busy || p.books.lines.is_empty())
                        .on_click(click(entity, |this, _, cx| this.sync_now(cx))),
                ),
        )
        .child(
            div()
                .id("sync-books")
                .role(Role::List)
                .aria_label(text::sync_books())
                .w_full()
                .flex()
                .flex_col()
                .children(books)
                .test_support(),
        )
        .children(status.map(|s| {
            div()
                .id("sync-status")
                .role(Role::Status)
                .aria_label(SharedString::from(s.clone()))
                .typo(&t.text.caption)
                .text_color(t.color.ink_2)
                .child(s)
                .test_support()
        }))
        .into_any_element()
}
