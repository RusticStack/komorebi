//! The views the window mounts over the mail (docs/GPUI-MIGRATION.md stage
//! C): Settings (with Rules in its list and the update card in About),
//! Contacts, and the first-run onboarding flow; what each asks of the
//! window, and what the window tells them back.
//!
//! - Settings and Contacts show beside the folder pane or rail, in place of
//!   the list and the reading pane (the mockup's `main two`), through
//!   [`MailView::set_place`]. Choosing a folder, the Inbox or New mail
//!   there takes them away ([`MailEvent::ClosePlace`]).
//! - The onboarding flow of a first run (no account) takes the whole body.
//!   Settings > Add account runs the same flow inside Settings.
//! - Worker state flows the other way: the hub's per-account state goes to
//!   Settings' account cards and to the flow's first download.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{AnyView, AppContext as _, Context, Entity, Focusable as _, Subscription, Window};
use komorebi::mail::render::links::Mailto;
use komorebi_i18n::tr;

use super::MainWindow;
use crate::views::contacts::{ContactsEvent, ContactsView, text as contacts_text};
use crate::views::mail::Place;
use crate::views::mail::hub::Hub;
use crate::views::mail::sync::{Said, State};
use crate::views::onboarding::{OnboardingEvent, OnboardingView};
use crate::views::rules::{RulesEvent, RulesView, text as rules_text};
use crate::views::settings::accounts::{AccountLine, AccountState};
use crate::views::settings::{
    Section, Setting, SettingsEvent, SettingsView, text as settings_text, wipe,
};
use crate::views::updates::{self, card::UpdatesCard};
use crate::widgets::dialog::{self, Confirm};

/// The view mounted, and what it is.
pub(super) struct Mounted {
    pub which: Place,
    pub view: AnyView,
    /// Beside the pane (Settings, Contacts) rather than over the whole
    /// body (the first-run flow).
    pub in_mail: bool,
    pub kind: Kind,
    _subscriptions: Vec<Subscription>,
}

pub(super) enum Kind {
    Settings(Entity<SettingsView>, Entity<RulesView>),
    Contacts,
    Onboarding(Entity<OnboardingView>),
}

/// "Remove all data" was confirmed: the data folder's cache, and the note
/// (with a marker for the list of files left) in the language shown. The
/// removal runs after the app stopped, when the cache is closed.
static WIPE: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

const LIST: &str = "\u{1}";

/// After the app stopped: remove the data if the user asked for it, and say
/// what could not be removed. True when it ran.
pub fn remove_data_if_asked() -> bool {
    let Some((path, note)) = WIPE.lock().ok().and_then(|mut w| w.take()) else {
        return false;
    };
    let left = wipe::remove_files(&path);
    if !left.is_empty() {
        let list: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
        crate::preflight::warn(&note.replace(LIST, &list.join("\n")));
    }
    true
}

impl MainWindow {
    // ------------------------------------------------------------ opening

    /// The view mounted over the mail, if any (Rules is a section of
    /// Settings).
    pub fn place(&self) -> Option<Place> {
        self.place.as_ref().map(|p| p.which)
    }

    /// Mount `place`: Settings (also on its Rules section) or Contacts
    /// beside the pane, or the onboarding flow over the body. Asking for
    /// the place that shows takes it away again.
    pub fn show_place(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let place = match place {
            Place::Rules => Place::Settings,
            other => other,
        };
        if self.place() == Some(place) && place != Place::Onboarding {
            return self.close_place(window, cx);
        }
        self.unmount(cx);
        match place {
            Place::Settings => {
                self.mount_settings(db, window, cx);
            }
            Place::Contacts => self.mount_contacts(window, cx),
            Place::Onboarding => self.mount_onboarding(db, window, cx),
            Place::Rules => unreachable!("Rules is a section of Settings"),
        }
        cx.notify();
    }

    /// Settings on `section`: opened if it does not show, and left open if
    /// it does (unlike [`Self::show_place`], which toggles it).
    pub fn show_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings().is_none() {
            self.show_place(Place::Settings, window, cx);
        }
        if let Some(settings) = self.settings() {
            settings.update(cx, |s, cx| s.show(section, window, cx));
        }
    }

    /// Settings, on its Rules section.
    pub fn show_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = match self.settings() {
            Some(s) => s,
            None => {
                self.show_place(Place::Settings, window, cx);
                match self.settings() {
                    Some(s) => s,
                    None => return,
                }
            }
        };
        settings.update(cx, |s, cx| s.show(Section::Rules, window, cx));
    }

    /// The page of account `id` in Settings (the status line's fix, the
    /// pane's "Account settings").
    pub fn edit_account(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings().is_none() {
            self.show_place(Place::Settings, window, cx);
        }
        if let Some(settings) = self.settings() {
            settings.update(cx, |s, cx| s.edit_account(id, window, cx));
        }
    }

    /// The Settings view, while it shows.
    pub fn settings(&self) -> Option<Entity<SettingsView>> {
        match self.place.as_ref().map(|p| &p.kind) {
            Some(Kind::Settings(s, _)) => Some(s.clone()),
            _ => None,
        }
    }

    /// The onboarding flow of a first run, while it shows.
    pub fn onboarding(&self) -> Option<Entity<OnboardingView>> {
        match self.place.as_ref().map(|p| &p.kind) {
            Some(Kind::Onboarding(f)) => Some(f.clone()),
            _ => None,
        }
    }

    /// A snapshot action (`KOMOREBI_SNAPSHOT_ACTION`, `crate::snapshot`):
    /// the screen it names, with made-up data where it needs some (only
    /// under `KOMOREBI_OFFLINE`, which a snapshot requires).
    pub(super) fn snapshot_action(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let section = match action {
            "settings-accounts" => Some(Section::Accounts),
            "settings-signatures" => Some(Section::Signatures),
            "settings-rules" | "rule-edit" => Some(Section::Rules),
            "settings-general" => Some(Section::General),
            "settings-privacy" => Some(Section::Privacy),
            "settings-about" => Some(Section::About),
            _ => None,
        };
        if let Some(section) = section {
            if self.settings().is_none() {
                self.show_place(Place::Settings, window, cx);
            }
            if let Some(settings) = self.settings() {
                settings.update(cx, |s, cx| s.show(section, window, cx));
            }
            if section == Section::Rules
                && let Some(Kind::Settings(_, rules)) = self.place.as_ref().map(|p| &p.kind)
            {
                let rules = rules.clone();
                let edit = action == "rule-edit";
                rules.update(cx, |r, cx| r.sample(edit, window, cx));
            }
            return;
        }
        match action {
            "settings-account" => {
                let first = self
                    .db
                    .as_ref()
                    .and_then(|db| db.accounts().ok())
                    .and_then(|a| a.first().map(|a| a.id));
                if let Some(id) = first {
                    self.edit_account(id, window, cx);
                }
            }
            "contacts" | "contact-edit" | "contacts-sync" => {
                self.show_place(Place::Contacts, window, cx);
                let then = match action {
                    "contact-edit" => "edit",
                    "contacts-sync" => "sync",
                    _ => "",
                };
                if let Some(contacts) = self.contacts.clone() {
                    contacts.update(cx, |c, cx| c.sample(then, window, cx));
                }
            }
            // A new message in the Mail tab's reading pane (Ctrl+N).
            "compose" => {
                self.show_mail_tab(window, cx);
                self.with_mail(window, cx, crate::views::mail::MailView::new_mail);
            }
            "ob-welcome" | "ob-email" | "ob-sign-in" | "ob-manual" => {
                if self.onboarding().is_none() {
                    self.show_place(Place::Onboarding, window, cx);
                }
                if let Some(flow) = self.onboarding() {
                    let action = action.to_string();
                    flow.update(cx, |v, cx| {
                        if action != "ob-welcome" {
                            v.start(window, cx);
                        }
                        if action == "ob-sign-in" || action == "ob-manual" {
                            let email = if action == "ob-manual" {
                                "alice@example.com"
                            } else {
                                "ana.silva@purelymail.com"
                            };
                            v.email_input()
                                .update(cx, |s, cx| s.set_value(email, window, cx));
                            v.continue_email(window, cx);
                            if action == "ob-manual" {
                                v.go_manual(window, cx);
                            }
                        }
                    });
                }
            }
            _ => {}
        }
    }

    /// The Contacts view (made with the window; empty until it shows).
    pub fn contacts(&self) -> Option<&Entity<ContactsView>> {
        self.contacts.as_ref()
    }

    fn mount_settings(
        &mut self,
        db: crate::views::Db,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| SettingsView::new(db.clone(), window, cx));
        let rules = cx.new(|cx| RulesView::new(db, window, cx));
        let card = self
            .updater
            .clone()
            .map(|u| cx.new(|cx| UpdatesCard::new(u, cx)));
        view.update(cx, |v, cx| {
            v.set_rules(Some(rules.clone()), cx);
            v.set_updates(card.map(AnyView::from), cx);
        });
        let subscriptions = vec![
            cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                this.on_settings(event, window, cx);
            }),
            cx.subscribe(&rules, |this, _, event, cx| this.on_rules(event, cx)),
        ];
        let any: AnyView = view.clone().into();
        self.place = Some(Mounted {
            which: Place::Settings,
            view: any.clone(),
            in_mail: true,
            kind: Kind::Settings(view.clone(), rules),
            _subscriptions: subscriptions,
        });
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| m.set_place(Some((Place::Settings, any)), cx));
        }
        self.push_states(cx);
        let focus = view.focus_handle(cx);
        window.focus(&focus, cx);
    }

    fn mount_contacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(contacts) = self.contacts.clone() else {
            return;
        };
        let any: AnyView = contacts.clone().into();
        self.place = Some(Mounted {
            which: Place::Contacts,
            view: any.clone(),
            in_mail: true,
            kind: Kind::Contacts,
            _subscriptions: Vec::new(),
        });
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| m.set_place(Some((Place::Contacts, any)), cx));
        }
        contacts.update(cx, |c, cx| c.opened(window, cx));
    }

    fn mount_onboarding(
        &mut self,
        db: crate::views::Db,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flow = cx.new(|cx| OnboardingView::new(db, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&flow, window, |this, _, event, window, cx| {
                this.on_onboarding(event, window, cx);
            }),
        ];
        flow.update(cx, |f, cx| f.focus_first(window, cx));
        self.place = Some(Mounted {
            which: Place::Onboarding,
            view: flow.clone().into(),
            in_mail: false,
            kind: Kind::Onboarding(flow),
            _subscriptions: subscriptions,
        });
    }

    /// Take the mounted view away without moving the focus.
    fn unmount(&mut self, cx: &mut Context<Self>) {
        let Some(mounted) = self.place.take() else {
            return;
        };
        if matches!(mounted.kind, Kind::Contacts)
            && let Some(contacts) = &self.contacts
        {
            contacts.update(cx, |c, cx| c.closed(cx));
        }
        if mounted.in_mail
            && let Some(mail) = &self.mail
        {
            mail.update(cx, |m, cx| m.set_place(None, cx));
        }
    }

    /// Back to the mail: the list has the keys again, and the pane, the
    /// lists, the composer and the reader read what Settings changed.
    pub fn close_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.place.is_none() {
            return;
        }
        self.unmount(cx);
        match &self.mail {
            Some(mail) => {
                mail.update(cx, |m, cx| m.reload_settings(window, cx));
                let list = mail.read(cx).list_focus().clone();
                window.focus(&list, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// Ctrl+,: Settings, and back to the mail.
    pub(super) fn toggle_settings(
        &mut self,
        _: &crate::tabs::ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.place() == Some(Place::Onboarding) {
            return;
        }
        self.show_place(Place::Settings, window, cx);
    }

    /// A first run: no account yet, so the onboarding flow opens.
    pub(super) fn first_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let none = self
            .db
            .as_ref()
            .is_some_and(|db| db.accounts().is_ok_and(|a| a.is_empty()));
        if none {
            self.show_place(Place::Onboarding, window, cx);
        }
    }

    // ------------------------------------------------------- what they ask

    fn on_settings(&mut self, event: &SettingsEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SettingsEvent::Close => self.close_place(window, cx),
            SettingsEvent::Changed(setting) => self.apply_setting(setting, window, cx),
            SettingsEvent::AccountsChanged => self.accounts_changed(window, cx),
            SettingsEvent::AccountAdded(id) => self.account_added(*id, cx),
            SettingsEvent::AccountSaved { id, change } => {
                if let Some(hub) = &self.hub {
                    hub.update(cx, |hub, cx| {
                        if change.servers {
                            hub.respawn(*id, cx);
                        } else if change.password {
                            hub.retry_account(*id, cx);
                        }
                    });
                }
                self.accounts_changed(window, cx);
            }
            SettingsEvent::AccountEnabled { id, on } => {
                if let Some(hub) = &self.hub {
                    hub.update(cx, |hub, cx| {
                        if *on {
                            hub.respawn(*id, cx);
                        } else {
                            hub.remove_account(*id, cx);
                        }
                    });
                }
                self.accounts_changed(window, cx);
            }
            SettingsEvent::Redownload(id) => self.redownload(*id, window, cx),
            SettingsEvent::AccountRemoved(id) => {
                if let Some(hub) = &self.hub {
                    hub.update(cx, |hub, cx| hub.remove_account(*id, cx));
                }
                self.accounts_changed(window, cx);
            }
            SettingsEvent::OpenLink(url) => open_link(url, window, cx),
            // Every tip due again, over the mail (`window/tips.rs`).
            SettingsEvent::ShowTips => self.show_tips_again(window, cx),
            SettingsEvent::RestartNow => {
                restart();
                self.quit(cx);
            }
            SettingsEvent::WipeAndQuit => self.wipe_and_quit(cx),
        }
    }

    /// A choice in Settings that something other than the cache must learn.
    fn apply_setting(&mut self, setting: &Setting, window: &mut Window, cx: &mut Context<Self>) {
        match setting {
            Setting::CheckInterval(secs) => {
                if let Some(hub) = &self.hub {
                    let interval = Duration::from_secs(*secs);
                    hub.update(cx, |hub, _| hub.set_interval(interval));
                }
            }
            Setting::UndoSend(_) | Setting::MarkRead(_) | Setting::ComposeRich(_) => {
                if let Some(mail) = &self.mail {
                    mail.update(cx, |m, cx| m.reload_settings(window, cx));
                }
            }
            Setting::UpdateAuto(on) => {
                if let Some(updater) = &self.updater {
                    updater.update(cx, |u, cx| u.set_auto(*on, cx));
                }
            }
            Setting::DiagnosticsLog(on) => {
                let log = self
                    .db
                    .as_ref()
                    .and_then(|db| db.file())
                    .map(|p| komorebi::diag::log_path(&p));
                komorebi::diag::set(log.filter(|_| *on));
            }
            // Read again from the cache where they are used: the tray and
            // the toasts at each new message, the tabs at each change.
            Setting::FlashTaskbar(_)
            | Setting::Notify(_)
            | Setting::NotifyDetail(_)
            | Setting::CloseToTray(_)
            | Setting::Autostart(_)
            | Setting::ReopenTabs(_)
            | Setting::AccountNotify { .. } => {}
        }
    }

    /// The accounts, their order, names or folders changed.
    fn accounts_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| m.reload_settings(window, cx));
        }
        self.mark_gone_tabs(cx);
        self.push_states(cx);
    }

    /// An account was added: its workers start and it downloads its mail.
    fn account_added(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let accounts = db.accounts().unwrap_or_default();
        // The bar for this version is for those who were using Komorebi.
        if accounts.len() == 1 {
            updates::whats_new::seen(&db);
        }
        if let (Some(account), Some(hub)) = (accounts.into_iter().find(|a| a.id == id), &self.hub) {
            hub.update(cx, |hub, cx| hub.add_account(account, cx));
        }
        self.push_states(cx);
    }

    /// Download again (after the question): the account's mail is
    /// forgotten and a full sync fetches it again, with new workers.
    fn redownload(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(db), Some(hub)) = (self.db.clone(), self.hub.clone()) else {
            return;
        };
        let email = db
            .accounts()
            .ok()
            .and_then(|a| a.into_iter().find(|a| a.id == id))
            .map(|a| a.email)
            .unwrap_or_default();
        let had_workers = hub.read(cx).has_workers(id);
        // The old workers go first, so no sync of the old state writes
        // after the reset.
        hub.update(cx, |hub, cx| hub.remove_account(id, cx));
        let said = match db.redownload(id) {
            Err(e) => (
                Said::Text(settings_text::not_reset(&format!("{e:#}"))),
                true,
            ),
            Ok(()) if had_workers => (Said::Text(settings_text::redownloading(&email)), false),
            Ok(()) => (Said::Text(settings_text::redownload_later(&email)), false),
        };
        if had_workers {
            hub.update(cx, |hub, cx| hub.respawn(id, cx));
        }
        hub.update(cx, |hub, cx| hub.say(said.0, said.1, cx));
        self.accounts_changed(window, cx);
    }

    fn on_rules(&mut self, event: &RulesEvent, cx: &mut Context<Self>) {
        let Some(hub) = &self.hub else {
            return;
        };
        match event {
            RulesEvent::SieveCopied => {
                hub.update(cx, |h, cx| {
                    h.say(Said::Text(rules_text::sieve_copied()), false, cx)
                });
            }
            RulesEvent::Failed(e) => {
                hub.update(cx, |h, cx| h.say(Said::Detail(e.clone()), true, cx));
            }
        }
    }

    pub(super) fn on_contacts(
        &mut self,
        event: &ContactsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContactsEvent::Compose { account, to } => {
                self.close_place(window, cx);
                if let Some(mail) = &self.mail {
                    let mailto = Mailto {
                        to: to.clone(),
                        ..Mailto::default()
                    };
                    let account = *account;
                    mail.update(cx, |m, cx| {
                        m.compose_mailto(Some(account), mailto, window, cx)
                    });
                }
            }
            ContactsEvent::NoAccount => {
                self.say(Said::Text(contacts_text::add_account_write()), false, cx)
            }
            ContactsEvent::Copied => self.say(Said::Text(contacts_text::copied()), false, cx),
            ContactsEvent::Failed(e) => self.say(Said::Detail(e.clone()), true, cx),
        }
    }

    fn say(&self, said: Said, error: bool, cx: &mut Context<Self>) {
        if let Some(hub) = &self.hub {
            hub.update(cx, |h, cx| h.say(said, error, cx));
        }
    }

    fn on_onboarding(
        &mut self,
        event: &OnboardingEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            OnboardingEvent::Added(id) => self.account_added(*id, cx),
            OnboardingEvent::Finished(account) => {
                self.close_place(window, cx);
                if let (Some(id), Some(mail)) = (account, &self.mail) {
                    mail.update(cx, |m, cx| m.show_account_inbox(*id, window, cx));
                }
            }
            OnboardingEvent::Closed => self.close_place(window, cx),
            OnboardingEvent::OpenLink(url) => open_link(url, window, cx),
            OnboardingEvent::OpenSettings => self.show_place(Place::Settings, window, cx),
        }
    }

    // -------------------------------------------------- what they are told

    /// The workers' state, for Settings' account cards and for the first
    /// download the onboarding flow shows.
    pub(super) fn push_states(&mut self, cx: &mut Context<Self>) {
        let (Some(hub), Some(mounted)) = (self.hub.clone(), self.place.as_ref()) else {
            return;
        };
        let line = |id: i64, cx: &Context<Self>| {
            let hub = hub.read(cx);
            let sync = hub.sync();
            let state = sync.state(id).map(|s| match s {
                State::Syncing => AccountState::Syncing,
                State::Checked(at) => AccountState::Checked(*at),
                State::Unreachable => AccountState::Unreachable,
                State::Failed(p) => AccountState::Failed(p.clone()),
            });
            AccountLine {
                state,
                attention: hub.attention(id),
                download: sync.first_downloads().get(&id).copied(),
            }
        };
        match &mounted.kind {
            Kind::Settings(settings, _) => {
                let settings = settings.clone();
                let ids: Vec<i64> = settings.read(cx).accounts.iter().map(|a| a.id).collect();
                for id in ids {
                    let l = line(id, cx);
                    settings.update(cx, |s, cx| s.set_account_state(id, l, cx));
                }
                if let Some(flow) = settings.read(cx).adding().cloned() {
                    self.push_flow(&flow, &hub, cx);
                }
            }
            Kind::Onboarding(flow) => {
                let flow = flow.clone();
                self.push_flow(&flow, &hub, cx);
            }
            Kind::Contacts => {}
        }
    }

    /// The first download of the account the flow added.
    fn push_flow(&self, flow: &Entity<OnboardingView>, hub: &Entity<Hub>, cx: &mut Context<Self>) {
        let Some(id) = flow.read(cx).added() else {
            return;
        };
        let (first, progress, syncing) = {
            let sync = hub.read(cx).sync();
            (
                sync.first_downloads().contains_key(&id),
                sync.progress(id),
                sync.is_syncing(id),
            )
        };
        flow.update(cx, |f, cx| {
            if first || syncing {
                f.set_syncing(id, true, cx);
            } else {
                f.set_syncing(id, false, cx);
            }
            if let Some((done, total)) = progress {
                f.set_progress(id, done, total, cx);
            }
        });
        let has_mail =
            !flow.read(cx).has_mail() && self.db.as_ref().is_some_and(|db| inbox_has_mail(db, id));
        if has_mail {
            flow.update(cx, |f, cx| f.set_has_mail(id, cx));
        }
    }

    // ---------------------------------------------------------- quitting

    /// Really quit (also with close to tray on).
    fn quit(&mut self, cx: &mut Context<Self>) {
        if let Some(tray) = &self.tray {
            tray.update(cx, |t, _| t.quit());
        }
        if self.close_popouts(cx) {
            cx.quit();
        }
    }

    /// Remove all data: the passwords are gone already; the app stops, and
    /// once the cache is closed `main` removes the files.
    fn wipe_and_quit(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.db.as_ref().and_then(|db| db.file())
            && let Ok(mut wipe) = WIPE.lock()
        {
            *wipe = Some((path, settings_text::wipe_files_left(LIST)));
        }
        komorebi::diag::set(None);
        self.quit(cx);
    }
}

/// Whether the account's Inbox has a message yet.
fn inbox_has_mail(db: &komorebi::db::Db, account: i64) -> bool {
    db.folders().is_ok_and(|folders| {
        folders
            .iter()
            .find(|f| f.account_id == account && f.role == komorebi::role::Role::Inbox)
            .is_some_and(|f| db.count_messages(f.id).is_ok_and(|n| n > 0))
    })
}

/// A web link from Settings or the What's new bar: the prompt first, then
/// the browser.
pub(super) fn open_link(url: &str, window: &mut Window, cx: &mut gpui::App) {
    if !url.starts_with("https://") {
        return;
    }
    let url = url.to_string();
    dialog::confirm(
        Confirm {
            title: tr!("Open this link in your browser?").into(),
            message: url.clone().into(),
            confirm: tr!("Open").into(),
            cancel: tr!("Cancel").into(),
            destructive: false,
        },
        move |_, cx| cx.open_url(&url),
        window,
        cx,
    );
}

/// Start a new copy of the app with the same arguments; it waits for this
/// process to end (`KOMOREBI_RESTART_AFTER`) before it takes the one-app
/// lock and opens the cache.
pub(super) fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env("KOMOREBI_RESTART_AFTER", std::process::id().to_string())
            .spawn()
            .ok();
    }
}
