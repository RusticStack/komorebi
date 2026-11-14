//! An account's page in Settings > Accounts (the Slint app's account form
//! when it edits: `edit_account`, `update_account`, `test_account`, the
//! certificate review, the folder roles, Download again and Remove). New
//! accounts are added by the setup flow ([`crate::views::onboarding`]).
//!
//! Save checks the servers first only when something about how they are
//! reached changed or a password was typed ([`account::needs_check`]);
//! Test connection runs the same check and saves nothing. The check runs
//! on GPUI's background executor ([`check::start`]); its task is kept here,
//! so Cancel, Back and Esc drop its answer.

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use komorebi::db::{Account, SignatureInfo};
use komorebi::find::{Fields, merge};
use komorebi::mail::discover::{self, Outcome};
use komorebi::mail::pin::Kind;
use komorebi::providers;
use komorebi_i18n::tr;

use super::account::{self, Change, SaveError};
use super::card::{CARD_MAX_W, Card, group_head, labelled, note};
use super::check::{self, CheckFailure, CheckState, Request, StepState};
use super::hosts::{FormError, typed_server};
use super::pins::{self, CertReview, FormPins, Unreadable};
use super::roles::{CHOOSABLE, RoleEdit};
use super::servers::{ServerEvent, ServerForm, find_row};
use super::services::{self, Services};
use super::text;
use super::trust;
use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::Db;
use crate::views::onboarding::flow;
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::TextField;
use crate::widgets::menu;
use crate::widgets::notice::{self, Notice};
use crate::widgets::pane::ScrollArea;
use crate::widgets::ring_style;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::{Checkbox, Toggle};

/// What the account page tells Settings.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// Back to the accounts.
    Back,
    /// Saved; what changed says what its workers need.
    Saved { id: i64, change: Change },
    /// Download again, after the question.
    Redownload(i64),
    /// Removed (its password first).
    Removed(i64),
    /// Sync this account switched.
    Enabled { id: i64, on: bool },
    /// Its notifications switched.
    Notify { id: i64, on: bool },
    /// Settings > Signatures.
    ManageSignatures,
    /// Move up (true) or Move down: the account's place in the folder pane
    /// and the lists changed.
    Moved { id: i64, up: bool },
}

/// Whether `a`'s servers are a preset's (no Find settings on its page, as
/// the Slint app's `edit-preset`).
pub fn is_preset(a: &Account) -> bool {
    providers::PRESETS
        .iter()
        .any(|p| p.is_fixed() && p.imap.0 == a.imap.host && p.smtp.0 == a.smtp.host)
}

/// Whether Server settings start open: a certificate is saved, or the SMTP
/// server needs no sign-in (the Slint app's `open-edit`).
pub fn starts_open(a: &Account) -> bool {
    a.imap_pin.is_some() || a.smtp_pin.is_some() || a.smtp_no_auth
}

/// The line under the page's buttons: why it can't be saved, or what went
/// well. Typed, so a language switch rewords it.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Form(FormError),
    Check(CheckFailure),
    OfflineCheck,
    OfflineCert,
    Unreadable(Unreadable),
    Gone,
    NotSaved(String),
    NotRemoved(String),
    Passed,
    Trusted(String),
}

impl Line {
    fn is_error(&self) -> bool {
        !matches!(self, Line::Passed | Line::Trusted(_))
    }
}

/// What a running check does when it passes.
#[derive(Clone)]
enum After {
    /// Test connection: say so.
    Say,
    /// Save: the edited account and the typed password.
    Save(Box<Account>, Option<String>),
}

/// The account page.
pub struct AccountEditor {
    db: Db,
    old: Account,
    focus: FocusHandle,
    scroll: ScrollHandle,
    name: Entity<InputState>,
    password: Entity<InputState>,
    servers: Entity<ServerForm>,
    save_sent: bool,
    pins: FormPins,
    roles: RoleEdit,
    signatures: Vec<SignatureInfo>,
    sig: (Option<i64>, Option<i64>),
    sig_first: (Option<i64>, Option<i64>),
    enabled: bool,
    notify: bool,
    check: CheckState,
    after: Option<After>,
    line: Option<Line>,
    /// A certificate read for review, and the trust choice picked.
    review: Option<CertReview>,
    trust_choice: usize,
    reading: Option<Task<()>>,
    /// The password box's error state.
    rejected: bool,
    /// Server settings unfolded.
    advanced: bool,
    /// The servers are a preset's: no Find settings.
    preset: bool,
    /// Find settings: the lookup running, the fields when it started, what
    /// the last one filled, its note, and whether the form changed while it
    /// looked.
    find: Option<Task<()>>,
    found_last: Option<Fields>,
    find_note: String,
    find_stale: bool,
    /// The account's place among the accounts (index, count), for Move up
    /// and Move down.
    place: (usize, usize),
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for AccountEditor {}

impl Focusable for AccountEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Account `id`'s place among the accounts: (index, count).
fn place_of(db: &Db, id: i64) -> (usize, usize) {
    let accounts = db.accounts().unwrap_or_default();
    let ix = accounts.iter().position(|a| a.id == id).unwrap_or(0);
    (ix, accounts.len())
}

fn fields_of(a: &Account) -> Fields {
    let form = super::form::AccountForm::of(a);
    Fields {
        imap_host: form.imap_host,
        imap_port: form.imap_port,
        imap_starttls: form.imap_starttls,
        imap_user: form.imap_user,
        smtp_host: form.smtp_host,
        smtp_port: form.smtp_port,
        smtp_starttls: form.smtp_starttls,
        smtp_user: form.smtp_user,
        smtp_own_login: form.smtp_own_login,
    }
}

impl AccountEditor {
    /// The page of `account`.
    pub fn new(db: Db, account: Account, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        name.update(cx, |s, cx| s.set_value(account.name.clone(), window, cx));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let servers = cx.new(|cx| ServerForm::new(window, cx));
        let fields = fields_of(&account);
        let no_auth = account.smtp_no_auth;
        servers.update(cx, |s, cx| {
            s.set(&fields, window, cx);
            s.set_no_auth(no_auth, cx);
        });
        let settings = db.settings().unwrap_or_default();
        let sig = db.account_signatures(account.id).unwrap_or((None, None));
        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(&servers, |this: &mut Self, _, _: &ServerEvent, cx| {
                this.servers_edited(cx);
            }),
        );
        // Enter in Name or Password saves (the Slint page's `submit(false)`).
        for input in [&name, &password] {
            subscriptions.push(
                cx.subscribe(input, |this: &mut Self, _, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.save(cx);
                    }
                }),
            );
        }
        let place = place_of(&db, account.id);
        Self {
            advanced: starts_open(&account),
            preset: is_preset(&account),
            find: None,
            found_last: None,
            find_note: String::new(),
            find_stale: false,
            place,
            roles: RoleEdit::load(&db, account.id),
            signatures: db.signatures().unwrap_or_default(),
            pins: FormPins::for_account(&account),
            save_sent: account.save_sent,
            enabled: account.enabled,
            notify: !settings.notify_off.contains(&account.id),
            sig,
            sig_first: sig,
            db,
            old: account,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            name,
            password,
            servers,
            check: CheckState::default(),
            after: None,
            line: None,
            review: None,
            trust_choice: 0,
            reading: None,
            rejected: false,
            _subscriptions: subscriptions,
        }
    }

    /// The account the page shows.
    pub fn account(&self) -> &Account {
        &self.old
    }

    /// Whether a check (or a certificate read) runs.
    pub fn busy(&self) -> bool {
        self.check.running() || self.reading.is_some()
    }

    /// The line under the buttons.
    pub fn line(&self) -> Option<&Line> {
        self.line.as_ref()
    }

    /// The steps list, while it shows.
    pub fn steps(&self) -> Option<Vec<StepState>> {
        self.check.steps()
    }

    /// The certificate waiting for an answer.
    pub fn review(&self) -> Option<&CertReview> {
        self.review.as_ref()
    }

    /// The pins of the open form.
    pub fn pins(&self) -> &FormPins {
        &self.pins
    }

    /// The server fields.
    pub fn server_form(&self) -> &Entity<ServerForm> {
        &self.servers
    }

    /// The password box.
    pub fn password_input(&self) -> &Entity<InputState> {
        &self.password
    }

    /// The name box.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    /// Whether the password box shows its error state.
    pub fn password_rejected(&self) -> bool {
        self.rejected
    }

    /// Esc or Back: a running check is cancelled first (true: Esc was used
    /// for that); else the page closes (false).
    pub fn back(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy() {
            self.cancel(cx);
            return true;
        }
        false
    }

    /// Cancel: a running check's answer is dropped, the form stays as typed.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.check.cancel();
        self.after = None;
        self.reading = None;
        cx.notify();
    }

    /// Update password (a rejected sign-in): the password box, focused.
    pub fn focus_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.notify();
    }

    fn servers_edited(&mut self, cx: &mut Context<Self>) {
        let f = self.servers.read(cx).fields(cx);
        let imap = typed_server(&f.imap_host, &f.imap_port, f.imap_starttls);
        let smtp = typed_server(&f.smtp_host, &f.smtp_port, f.smtp_starttls);
        self.pins.retain_for(imap.as_ref(), smtp.as_ref());
        if !self.busy() {
            self.check.clear();
            self.line = None;
        }
        cx.notify();
    }

    /// The form as it stands, over the account's own values.
    fn form(&self, cx: &App) -> super::form::AccountForm {
        let f = self.servers.read(cx).fields(cx);
        super::form::AccountForm {
            name: self.name.read(cx).value().to_string(),
            password: self.password.read(cx).value().to_string(),
            imap_host: f.imap_host,
            imap_port: f.imap_port,
            imap_starttls: f.imap_starttls,
            imap_user: f.imap_user,
            smtp_host: f.smtp_host,
            smtp_port: f.smtp_port,
            smtp_starttls: f.smtp_starttls,
            smtp_user: f.smtp_user,
            smtp_own_login: f.smtp_own_login,
            smtp_no_auth: self.servers.read(cx).no_auth(),
            save_sent: self.save_sent,
            ..super::form::AccountForm::of(&self.old)
        }
    }

    /// Whether Server settings are unfolded.
    pub fn servers_open(&self) -> bool {
        self.advanced
    }

    /// Fold or unfold Server settings.
    pub fn set_servers_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.advanced = open;
        cx.notify();
    }

    /// Whether the page offers Find settings (a server that is not a
    /// preset's).
    pub fn can_find(&self) -> bool {
        !self.preset
    }

    /// Whether a lookup runs.
    pub fn finding(&self) -> bool {
        self.find.is_some()
    }

    /// The lookup's note: what it found, or that the form changed while it
    /// looked ("" before the first).
    pub fn find_note(&self) -> String {
        if self.find_stale {
            tr!("Not filled in: the form changed while Komorebi looked.")
        } else {
            self.find_note.clone()
        }
    }

    /// The account's place among the accounts (index, count).
    pub fn place(&self) -> (usize, usize) {
        self.place
    }

    /// Find settings: look the address's domain up (only on this press),
    /// then fill the servers if they are unchanged since.
    pub fn find_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() || self.busy() || self.preset {
            return;
        }
        let svc = services::get(cx);
        // The Slint app's find.rs: the offline answer goes in the find note,
        // and a new lookup clears the last one's note.
        self.find_stale = false;
        if svc.offline {
            self.find_note = text::err_offline_find();
            cx.notify();
            return;
        }
        self.find_note.clear();
        let email = self.old.email.clone();
        let from = self.servers.read(cx).fields(cx);
        let lookup = svc.lookup;
        let work = cx.background_executor().spawn({
            let email = email.clone();
            async move { lookup(&email) }
        });
        let handle = window.window_handle();
        self.find = Some(cx.spawn(async move |this, cx| {
            let out = work.await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.finish_find(from, out, window, cx));
            });
        }));
        cx.notify();
    }

    /// The lookup answered: fill the servers it found, unless they were
    /// edited while it looked (then say so and fill nothing).
    pub fn finish_find(
        &mut self,
        from: Fields,
        out: Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find = None;
        let address = discover::parse_email(&self.old.email)
            .map(|(l, d)| format!("{l}@{d}"))
            .unwrap_or_default();
        let merged = merge(&from, &out.found, &address, self.found_last.as_ref());
        let now = self.servers.read(cx).fields(cx);
        if now == from {
            self.find_stale = false;
            if merged.to != from {
                self.found_last = Some(merged.to.clone());
                let to = merged.to.clone();
                self.servers.update(cx, |s, cx| s.set(&to, window, cx));
                self.servers_edited(cx);
            }
            self.advanced = true;
        } else {
            self.find_stale = true;
        }
        self.find_note = flow::note(&out, merged.kept);
        cx.notify();
    }

    /// Move up (true) or Move down: the account's place in the folder pane
    /// and the lists.
    pub fn move_account(&mut self, up: bool, cx: &mut Context<Self>) {
        match self.db.move_account(self.old.id, up) {
            Ok(true) => {
                self.place = place_of(&self.db, self.old.id);
                cx.emit(EditorEvent::Moved {
                    id: self.old.id,
                    up,
                });
            }
            Ok(false) => {}
            Err(e) => self.line = Some(Line::NotSaved(format!("{e:#}"))),
        }
        cx.notify();
    }

    /// The edited account, or why the form can't be used.
    fn edited(&mut self, cx: &App) -> Result<Account, FormError> {
        let form = self.form(cx);
        let (imap, smtp) = form.servers()?;
        self.pins.retain_for(Some(&imap), Some(&smtp));
        account::edited(&self.old, &form, &self.pins.blobs())
    }

    fn request(&self, edited: &Account, password: Option<String>, svc: &Services) -> Request {
        Request {
            servers: (edited.imap.clone(), edited.smtp.clone()),
            pins: (edited.imap_pin.clone(), edited.smtp_pin.clone()),
            manual: true,
            logins: (
                edited.imap_login().to_string(),
                edited.smtp_login().to_string(),
            ),
            smtp_sign_in: !edited.smtp_no_auth,
            password,
            saved: edited.id,
            verify: svc.verify,
            read_password: svc.read_password,
        }
    }

    fn run(&mut self, req: Request, after: After, cx: &mut Context<Self>) {
        if services::get(cx).offline {
            self.line = Some(Line::OfflineCheck);
            cx.notify();
            return;
        }
        self.line = None;
        self.rejected = false;
        self.after = Some(after);
        let task = check::start(req, cx, |this: &mut Self, msg, cx| this.on_check(msg, cx));
        self.check.start(task);
        // The steps and the answer are under the buttons, at the page's end.
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn on_check(&mut self, msg: check::Msg, cx: &mut Context<Self>) {
        match msg {
            check::Msg::Progress(done) => self.check.progress(done),
            check::Msg::Done(result) => {
                self.check.finish(&result);
                self.scroll.scroll_to_bottom();
                let after = self.after.take();
                match result {
                    Err(f) => {
                        self.rejected = check::rejected(&f.text);
                        // The answer may be about the servers: they show.
                        self.advanced |= !self.rejected;
                        self.line = Some(Line::Check(f));
                    }
                    Ok(()) => match after {
                        Some(After::Save(edited, password)) => {
                            self.finish_save(*edited, password, cx);
                        }
                        _ => self.line = Some(Line::Passed),
                    },
                }
            }
        }
        cx.notify();
    }

    /// Test connection: every step, nothing saved.
    pub fn test(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let edited = match self.edited(cx) {
            Ok(a) => a,
            Err(e) => {
                self.advanced = true;
                self.line = Some(Line::Form(e));
                cx.notify();
                return;
            }
        };
        let password = Some(self.password.read(cx).value().to_string()).filter(|p| !p.is_empty());
        let req = self.request(&edited, password, &services::get(cx));
        self.run(req, After::Say, cx);
    }

    /// Save: checked first if the servers or the password changed.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let edited = match self.edited(cx) {
            Ok(a) => a,
            Err(e) => {
                self.advanced = true;
                self.line = Some(Line::Form(e));
                cx.notify();
                return;
            }
        };
        let password = Some(self.password.read(cx).value().to_string()).filter(|p| !p.is_empty());
        if !account::needs_check(&self.old, &edited, password.is_some()) {
            self.finish_save(edited, None, cx);
            return;
        }
        let req = self.request(&edited, password.clone(), &services::get(cx));
        self.run(req, After::Save(Box::new(edited), password), cx);
    }

    fn finish_save(&mut self, edited: Account, password: Option<String>, cx: &mut Context<Self>) {
        let svc = services::get(cx);
        match account::save(
            &self.db,
            &self.old,
            &edited,
            password.as_deref(),
            &svc.store_password,
        ) {
            Err(SaveError::Gone) => self.line = Some(Line::Gone),
            Err(SaveError::Store(e)) => self.line = Some(Line::NotSaved(format!("{e:#}"))),
            Ok(change) => {
                let id = edited.id;
                let mut problem = None;
                if let Err(e) = self.roles.apply(&self.db, id) {
                    problem = Some(format!("{e:#}"));
                }
                if self.sig != self.sig_first
                    && let Err(e) = self.db.set_account_signatures(id, self.sig.0, self.sig.1)
                {
                    problem = Some(format!("{e:#}"));
                }
                match problem {
                    Some(e) => self.line = Some(Line::NotSaved(e)),
                    None => cx.emit(EditorEvent::Saved { id, change }),
                }
            }
        }
        cx.notify();
    }

    fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        match self.db.set_account_enabled(self.old.id, on) {
            Ok(()) => {
                self.enabled = on;
                self.old.enabled = on;
                cx.emit(EditorEvent::Enabled {
                    id: self.old.id,
                    on,
                });
            }
            Err(e) => self.line = Some(Line::NotSaved(format!("{e:#}"))),
        }
        cx.notify();
    }

    fn set_notify(&mut self, on: bool, cx: &mut Context<Self>) {
        let mut off = self.db.settings().unwrap_or_default().notify_off;
        off.retain(|id| *id != self.old.id);
        if !on {
            off.push(self.old.id);
        }
        let value = off.iter().map(i64::to_string).collect::<Vec<_>>().join(" ");
        match self.db.set_setting("notify_off", &value) {
            Ok(()) => {
                self.notify = on;
                cx.emit(EditorEvent::Notify {
                    id: self.old.id,
                    on,
                });
            }
            Err(e) => self.line = Some(Line::NotSaved(format!("{e:#}"))),
        }
        cx.notify();
    }

    /// Download again: asked first; the shell replaces the workers.
    pub fn ask_redownload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.old.id;
        let this = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::q_redownload_title(&self.old.email).into(),
                message: text::q_redownload_text().into(),
                confirm: text::q_redownload_ok().into(),
                cancel: tr!("Cancel").into(),
                destructive: false,
            },
            move |_, cx| {
                let _ = this.update(cx, |_, cx| cx.emit(EditorEvent::Redownload(id)));
            },
            window,
            cx,
        );
    }

    /// Remove: always asked first, with the unsent mail that goes with it.
    pub fn ask_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unsent = account::unsent(&self.db, Some(self.old.id));
        let message = if unsent > 0 {
            format!(
                "{} {}",
                text::q_remove_text(),
                text::q_remove_unsent(unsent)
            )
        } else {
            text::q_remove_text()
        };
        let this = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::q_remove_title(&self.old.email).into(),
                message: message.into(),
                confirm: text::q_remove_ok().into(),
                cancel: tr!("Cancel").into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = this.update(cx, |this, cx| this.remove_now(cx));
            },
            window,
            cx,
        );
    }

    /// Remove, answered: the password first, then the account and its mail.
    pub fn remove_now(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
        let svc = services::get(cx);
        match account::remove(&self.db, self.old.id, &svc.forget_password) {
            Ok(()) => cx.emit(EditorEvent::Removed(self.old.id)),
            Err(e) => {
                self.line = Some(Line::NotRemoved(format!("{e:#}")));
                cx.notify();
            }
        }
    }

    /// Forget certificate.
    pub fn forget_certificate(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.pins.forget(kind);
        self.line = None;
        cx.notify();
    }

    /// Review certificate: read what the server presents, without signing
    /// in, then ask whether to trust it.
    pub fn review_certificate(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let svc = services::get(cx);
        if svc.offline {
            self.line = Some(Line::OfflineCert);
            cx.notify();
            return;
        }
        let servers = match self.form(cx).servers() {
            Ok(s) => s,
            Err(e) => {
                self.advanced = true;
                self.line = Some(Line::Form(e));
                cx.notify();
                return;
            }
        };
        let server = match kind {
            Kind::Imap => servers.0,
            Kind::Smtp => servers.1,
        };
        self.line = None;
        self.advanced = true;
        let probe = svc.probe;
        let read = cx.background_executor().spawn({
            let server = server.clone();
            async move { probe(&server, kind) }
        });
        let handle = window.window_handle();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.finish_review(kind, server, result, window, cx)
                });
            });
        }));
        cx.notify();
    }

    /// Review certificate on the line of an account whose saved certificate
    /// changed: the saved one is forgotten in the form (shown as the one
    /// before), and the new one read for the prompt.
    pub fn review_changed(
        &mut self,
        host: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = pins::changed_kind(&self.old, host);
        self.pins.forget(kind);
        self.review_certificate(kind, window, cx);
    }

    fn finish_review(
        &mut self,
        kind: Kind,
        server: komorebi::providers::Server,
        result: Result<(Vec<u8>, Option<Vec<u8>>), Unreadable>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading.take().is_none() {
            return;
        }
        match result {
            Err(e) => self.line = Some(Line::Unreadable(e)),
            Ok((der, issuer)) => {
                self.review = Some(CertReview {
                    kind,
                    server,
                    der,
                    issuer,
                });
                self.trust_choice = 0;
                self.open_trust(window, cx);
            }
        }
        cx.notify();
    }

    /// Trust: the certificate is pinned in the form only; Save keeps it.
    pub fn trust(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.review.take() else {
            return;
        };
        let blob = review.blob(self.trust_choice);
        self.pins.set(review.kind, blob, review.server.clone());
        self.check.clear();
        self.line = Some(Line::Trusted(crate::text::host(&review.server.host)));
        cx.notify();
    }

    fn open_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.review.clone() else {
            return;
        };
        let before = self.pins.before(review.kind).map(str::to_string);
        let this = cx.entity().downgrade();
        let cancel = this.clone();
        trust::open(
            &review,
            before.as_deref(),
            move |choice, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.trust_choice = choice;
                    this.trust(cx);
                });
            },
            move |cx| {
                let _ = cancel.update(cx, |this, cx| {
                    this.review = None;
                    cx.notify();
                });
            },
            window,
            cx,
        );
    }

    fn line_text(&self, line: &Line) -> String {
        match line {
            Line::Form(e) => text::form_error(*e),
            Line::Check(f) => check::failure_text(f, None, &self.old.email),
            Line::OfflineCheck => text::err_offline_check(),
            Line::OfflineCert => text::err_offline_cert(),
            Line::Unreadable(Unreadable::NoStartTls { host }) => {
                crate::text::say(&komorebi::mail::errors::Say::NoStartTls { host: host.clone() })
            }
            Line::Unreadable(Unreadable::Unread { address }) => text::cert_unreadable(address),
            Line::Gone => text::err_account_removed(),
            Line::NotSaved(e) => text::not_saved(e),
            Line::NotRemoved(e) => text::not_removed(e),
            Line::Passed => text::check_passed(),
            Line::Trusted(host) => text::cert_trusted(host),
        }
    }

    fn render_steps(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let steps = self.check.steps()?;
        let t = cx.tokens().clone();
        let c = t.color;
        let rows = steps.into_iter().enumerate().map(move |(ix, state)| {
            let label = check::step_label(ix);
            let mark = match state {
                StepState::Passed => Some(Icon::Checkmark),
                StepState::Failed => Some(Icon::Dismiss),
                _ => None,
            };
            div()
                .id(SharedString::from(format!("account-step-{ix}")))
                .role(Role::ListItem)
                .aria_label(check::step_name(&label, state))
                .flex()
                .items_center()
                .gap(px(space::SP_2))
                .typo(&t.text.caption)
                .text_color(match state {
                    StepState::Waiting | StepState::Skipped => c.ink_3,
                    _ => c.ink,
                })
                .child(
                    div()
                        .size(px(geo::STEP_MARK))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .when_some(mark, |el, icon| {
                            el.child(
                                svg()
                                    .path(icon.path())
                                    .size(px(layout::ICON))
                                    .text_color(c.ink),
                            )
                        })
                        .when(state == StepState::Running, |el| {
                            el.child(
                                div()
                                    .size(px(layout::PLACE_SPOT))
                                    .rounded(px(radius::FULL))
                                    .bg(c.ink),
                            )
                        }),
                )
                .child(label)
                .test_support()
        });
        Some(
            div()
                .id("account-steps")
                .role(Role::List)
                .aria_label(tr!("Test connection"))
                .flex()
                .flex_col()
                .gap(px(space::SP_1))
                .children(rows)
                .test_support(),
        )
    }

    fn render_line(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let line = self.line.as_ref()?;
        let kind = if line.is_error() {
            notice::Kind::Error
        } else {
            notice::Kind::Info
        };
        let mut n = Notice::new("account-line", kind, self.line_text(line));
        if let Line::Check(f) = line {
            if check::rejected(&f.text) {
                n = n.action(
                    Button::new("account-set-username")
                        .label(tr!("Set username"))
                        .variant(Variant::OnInset)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let servers = this.servers.clone();
                            servers.update(cx, |s, cx| s.focus_user(window, cx));
                        })),
                );
            }
            if let Some(kind) = f.review {
                n = n.action(
                    Button::new("account-review-certificate")
                        .label(tr!("Review certificate"))
                        .variant(Variant::OnInset)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.review_certificate(kind, window, cx)
                        })),
                );
            }
        }
        Some(div().w_full().max_w(px(CARD_MAX_W)).child(n))
    }

    fn render_cert(&self, kind: Kind, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let saved = self.pins.is_saved(kind);
        let id = match kind {
            Kind::Imap => "account-forget-imap",
            Kind::Smtp => "account-forget-smtp",
        };
        div()
            .flex()
            .items_center()
            .gap(px(space::SP_2))
            .max_w(px(geo::FORM_W))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(self.pins.line(kind)),
            )
            .when(saved, |el| {
                el.child(
                    Button::new(id)
                        .label(tr!("Forget certificate"))
                        .variant(Variant::Subtle)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.forget_certificate(kind, cx)),
                        ),
                )
            })
    }

    /// Server settings' header (the Slint page's `Expander`): folded unless
    /// a certificate is saved or the SMTP server needs no sign-in; a
    /// lookup, a review or an error unfolds it.
    fn render_servers_head(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let open = self.advanced;
        div()
            .id("account-servers")
            .role(Role::Button)
            .aria_label(tr!("Server settings"))
            .aria_description(tr!("Incoming and sending servers, ports and security"))
            .aria_expanded(open)
            .tab_index(0)
            .mt(px(geo::GROUP_TOP))
            .w_full()
            .max_w(px(CARD_MAX_W))
            .min_h(px(layout::CARD_H))
            .px(px(space::SP_4))
            .flex()
            .items_center()
            .gap(px(space::SP_4))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .hover(move |s| s.bg(c.wash))
            .focus_visible(ring_style(&t))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                let open = !this.advanced;
                this.set_servers_open(open, cx);
            }))
            .child(
                svg()
                    .path(Icon::Settings.path())
                    .size(px(layout::ICON))
                    .flex_none()
                    .text_color(c.ink_2),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(space::SP_3))
                    .child(div().typo(&t.text.body).child(tr!("Server settings")))
                    .child(
                        div()
                            .mt(px(space::SP_0_5))
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(tr!("Incoming and sending servers, ports and security")),
                    ),
            )
            .child(
                svg()
                    .path(if open {
                        Icon::ChevronUp.path()
                    } else {
                        Icon::ChevronDown.path()
                    })
                    .size(px(layout::ICON))
                    .flex_none()
                    .text_color(c.ink_2),
            )
            .test_support()
    }

    /// Server settings, unfolded: Find settings (not for a preset's
    /// servers), the fields and the certificate lines.
    fn render_servers(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if !self.advanced {
            return None;
        }
        let find = (!self.preset).then(|| {
            let can = !self.finding() && !self.busy() && discover::looks_valid(&self.old.email);
            find_row(
                "account-find",
                self.finding(),
                can,
                self.find_note(),
                cx.listener(|this, _, window, cx| this.find_settings(window, cx)),
                cx,
            )
        });
        Some(
            div()
                .pt(px(space::SP_3))
                .flex()
                .flex_col()
                .gap(px(space::SP_3))
                .children(find)
                .child(self.servers.clone())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_1))
                        .child(self.render_cert(Kind::Imap, cx))
                        .child(self.render_cert(Kind::Smtp, cx)),
                ),
        )
    }

    fn signature_card(&self, reply: bool, cx: &mut Context<Self>) -> Card {
        let (id, title, name, value) = if reply {
            (
                "account-sig-reply",
                tr!("Replies and forwards"),
                tr!("Signature for replies and forwards"),
                self.sig.1,
            )
        } else {
            (
                "account-sig-new",
                tr!("New messages"),
                tr!("Signature for new messages"),
                self.sig.0,
            )
        };
        let mut choices = vec![("none".to_string(), text::sig_none())];
        choices.extend(
            self.signatures
                .iter()
                .map(|s| (s.id.to_string(), s.name.clone())),
        );
        let current = value.map_or_else(|| "none".to_string(), |v| v.to_string());
        Card::new(id, title).control(
            Dropdown::new(id, name, current)
                .choices(choices)
                .on_inset(true)
                .on_pick(cx.listener(move |this, value: &SharedString, _, cx| {
                    let picked = value.parse::<i64>().ok();
                    if reply {
                        this.sig.1 = picked;
                    } else {
                        this.sig.0 = picked;
                    }
                    cx.notify();
                })),
        )
    }

    fn role_card(&self, ix: usize, cx: &mut Context<Self>) -> Card {
        let title = match CHOOSABLE[ix] {
            komorebi::role::Role::Sent => tr!("Sent"),
            komorebi::role::Role::Drafts => tr!("Drafts"),
            komorebi::role::Role::Trash => tr!("Trash"),
            komorebi::role::Role::Archive => tr!("folder" => "Archive"),
            _ => tr!("Junk"),
        };
        let id = SharedString::from(format!("account-role-{ix}"));
        let options = self.roles.options(ix);
        let picked = self.roles.picked.get(ix).copied().unwrap_or(0);
        Card::new(id.clone(), title.clone()).control(
            Dropdown::new(id, title, picked.to_string())
                .choices(
                    options
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| (i.to_string(), label)),
                )
                .on_inset(true)
                .on_pick(cx.listener(move |this, value: &SharedString, _, cx| {
                    if let Ok(option) = value.parse() {
                        this.roles.pick(ix, option);
                        cx.notify();
                    }
                })),
        )
    }
}

impl Render for AccountEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let compact = window.viewport_size().width < px(layout::COMPACT_BELOW_W);
        let a = self.old.clone();
        let shown = if a.name.trim().is_empty() {
            a.email.clone()
        } else {
            a.name.clone()
        };
        let busy = self.busy();
        let crumb = div()
            .flex()
            .items_center()
            .gap(px(space::SP_2))
            .max_w(px(CARD_MAX_W))
            .child(
                Button::new("account-back")
                    .label(tr!("Accounts"))
                    .name(tr!("Back"))
                    .icon(Icon::ChevronLeft)
                    .variant(Variant::Subtle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel(cx);
                        cx.emit(EditorEvent::Back);
                    })),
            )
            .child(
                div()
                    .id("account-heading")
                    .role(Role::Heading)
                    .aria_level(1)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typo(&t.text.large)
                    .text_color(c.ink)
                    .child(shown),
            )
            .when(self.place.1 > 1, |el| {
                let (ix, count) = self.place;
                let view = cx.entity().downgrade();
                el.child(
                    IconButton::new(
                        "account-more",
                        Icon::More,
                        tr!("More options for this account"),
                    )
                    .tooltip(tr!("More options for this account"))
                    .disabled(busy)
                    .dropdown_menu(move |m, _, cx| {
                        let (up, down) = (view.clone(), view.clone());
                        menu::brand(m, cx)
                            .item(
                                PopupMenuItem::new(tr!("Move up"))
                                    .disabled(ix == 0)
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            up.update(cx, |this, cx| this.move_account(true, cx));
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(tr!("Move down"))
                                    .disabled(ix + 1 >= count)
                                    .on_click(move |_, _, cx| {
                                        let _ = down
                                            .update(cx, |this, cx| this.move_account(false, cx));
                                    }),
                            )
                    }),
                )
            });
        let this = cx.entity().downgrade();
        let sync = Card::new("account-sync", tr!("Sync this account"))
            .icon(Icon::Sync)
            .note(tr!("When this is off, Komorebi doesn't connect to the account: no new mail comes in and nothing is sent. Its downloaded mail stays readable."))
            .control(
                Toggle::new("account-sync-toggle", tr!("Sync {0}", a.email), self.enabled)
                    .state_label(tr!("On"), tr!("Off"))
                    .on_change({
                        let this = this.clone();
                        move |on, _, cx| {
                            let _ = this.update(cx, |this, cx| this.set_enabled(on, cx));
                        }
                    }),
            );
        let notify = Card::new("account-notify", tr!("Notifications for this account"))
            .icon(Icon::Alert)
            .note(tr!("New mail in this account can show a Windows notification. Notifications for new mail in Settings > General applies to every account."))
            .control(
                Toggle::new(
                    "account-notify-toggle",
                    tr!("Notifications for {0}", a.email),
                    self.notify,
                )
                .state_label(tr!("On"), tr!("Off"))
                .on_change({
                    let this = this.clone();
                    move |on, _, cx| {
                        let _ = this.update(cx, |this, cx| this.set_notify(on, cx));
                    }
                }),
            );
        let email_box = div()
            .h(px(layout::CONTROL_H_LG))
            .px(px(space::SP_3))
            .flex()
            .items_center()
            .rounded(px(radius::FULL))
            .bg(c.inset)
            .text_color(c.ink_2)
            .typo(&t.text.body)
            .child(a.email.clone());
        let fields = div()
            .w_full()
            .max_w(px(geo::FORM_W))
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(labelled(
                tr!("Name"),
                TextField::new(&self.name, tr!("Name")),
                Some(tr!("Shown to recipients").into()),
                cx,
            ))
            .child(labelled(
                tr!("Email"),
                email_box,
                Some(
                    tr!("The address can't be changed. To use another one, add another account.")
                        .into(),
                ),
                cx,
            ))
            .child(labelled(
                tr!("Password"),
                div()
                    .rounded(px(radius::FULL))
                    .when(self.rejected, |el| {
                        el.border(px(layout::FOCUS_RING)).border_color(c.ink)
                    })
                    .child(TextField::new(&self.password, tr!("Password"))),
                Some(tr!("Leave empty to keep the current one").into()),
                cx,
            ));
        let roles = (!self.roles.names.is_empty()).then(|| {
            (0..CHOOSABLE.len())
                .map(|ix| self.role_card(ix, cx).into_any_element())
                .collect::<Vec<_>>()
        });
        let save_sent = Checkbox::new(
            "account-save-sent",
            tr!("Save a copy of sent mail in the Sent folder"),
            self.save_sent,
        )
        .on_change({
            let this = this.clone();
            move |on, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.save_sent = on;
                    cx.notify();
                });
            }
        });
        let redownload = Card::new("account-redownload", tr!("Download again"))
            .icon(Icon::ArrowDown)
            .note(tr!("Forgets the copy of this account's mail on this computer and downloads it once more. Mail on the server is not affected."))
            .control(
                Button::new("account-redownload-button")
                    .label(tr!("Download again"))
                    .name(tr!("Download the mail of {0} again", a.email))
                    .variant(Variant::OnInset)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_redownload(window, cx))),
            );
        let remove = Card::new("account-remove", tr!("Remove this account"))
            .icon(Icon::Delete)
            .note(tr!("Removes the account and its downloaded mail from this computer. Mail on the server is not affected."))
            .control(
                Button::new("account-remove-button")
                    .label(tr!("Remove"))
                    .name(tr!("Remove {0}", a.email))
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_remove(window, cx))),
            );
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap(px(space::SP_2))
            .child(
                Button::new("account-save")
                    .label(tr!("Save"))
                    .variant(Variant::Primary)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                Button::new("account-test")
                    .label(tr!("Test connection"))
                    .tooltip(tr!(
                        "Checks the servers and the sign-in without saving anything"
                    ))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
            )
            .when(busy, |el| {
                el.child(
                    Button::new("account-cancel")
                        .label(tr!("Cancel"))
                        .tooltip(tr!("Stops the check; the form stays as typed"))
                        .variant(Variant::Subtle)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
            })
            .when(self.reading.is_some(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .typo(&t.text.caption)
                        .text_color(c.ink_2)
                        .child(tr!("Reading the certificate\u{a0}…")),
                )
            });
        let pad_x = if compact {
            geo::PAGE_PAD_X_COMPACT
        } else {
            geo::PAGE_PAD_X
        };
        div()
            .id("account-editor")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .child(
                ScrollArea::new("account-scroll", &self.scroll).child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .px(px(pad_x))
                        .py(px(geo::PAGE_PAD_Y))
                        .child(crumb)
                        .child(div().mt(px(space::SP_6)).child(sync))
                        .child(notify)
                        .child(group_head(tr!("account page group" => "Account"), cx))
                        .child(fields)
                        .child(group_head(tr!("Signature"), cx))
                        .child(self.signature_card(false, cx))
                        .child(self.signature_card(true, cx))
                        .child(
                            div().pt(px(space::SP_1)).child(
                                Button::new("account-manage-signatures")
                                    .label(tr!("Manage signatures"))
                                    .variant(Variant::Subtle)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(EditorEvent::ManageSignatures)
                                    })),
                            ),
                        )
                        .child(group_head(tr!("Folders"), cx))
                        .child(note(tr!("Where sent mail and drafts are saved, and where Delete, Archive and Report junk move mail. Automatic follows what the server marks. Nothing is created on the server."), cx))
                        .child(div().py(px(space::SP_3)).child(save_sent))
                        .children(roles.into_iter().flatten())
                        .child(self.render_servers_head(cx))
                        .children(self.render_servers(cx))
                        .child(group_head(tr!("Downloaded mail"), cx))
                        .child(redownload)
                        .child(remove)
                        .child(
                            div()
                                .mt(px(space::SP_6))
                                .flex()
                                .flex_col()
                                .gap(px(space::SP_3))
                                .children(self.render_steps(cx))
                                .children(self.render_line(cx))
                                .child(buttons),
                        ),
                ),
            )
            .test_support()
    }
}
