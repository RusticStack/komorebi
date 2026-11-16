//! Settings (phase 6; mockup `settings`): the page that replaces the mail
//! view in the content area. A section list at the left (Accounts,
//! Signatures, Rules, General, Privacy, About) and the section's cards at
//! the right, 760 px wide at most.
//!
//! The shell mounts it with [`SettingsView::new`] and listens to
//! [`SettingsEvent`]: what a choice means outside Settings (the sync timer,
//! the workers of an account, the tray) is the shell's; theme and language
//! are applied here, live, as in the Slint app. The plain rules live in
//! their own modules (`choices`, `hosts`, `form`, `pins`, `roles`, `account`,
//! `wipe`, `check`) and are tested without a window.

pub mod about;
pub mod account;
pub mod accounts;
pub mod app_password;
pub mod appearance;
pub mod background;
pub mod card;
pub mod check;
pub mod choices;
pub mod editor;
pub mod form;
pub mod general;
pub mod hosts;
pub mod pins;
pub mod privacy;
pub mod roles;
pub mod servers;
pub mod services;
pub mod shortcuts;
pub mod signatures;
pub mod text;
pub mod theme_editor;
pub mod trust;
pub mod wipe;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, Render, Role, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Window, div, prelude::*,
    px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use komorebi::db::{Account, Settings};

use self::account::Change;
use self::accounts::AccountLine;
use self::editor::{AccountEditor, EditorEvent};
use self::signatures::SignaturesPage;
use crate::access;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::Db;
use crate::views::onboarding::{OnboardingEvent, OnboardingView};
use crate::views::rules::RulesView;
use crate::widgets::notice::{self, Notice};
use crate::widgets::pane::ScrollArea;
use crate::widgets::ring_style;

/// The key context of Settings.
pub const CONTEXT: &str = "Settings";

gpui_kit::actions!(settings, [Close, Link, ShowShortcuts]);

/// Show a section (its access key, `crate::access`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = settings, no_json)]
pub struct ShowSection(pub Section);

/// Settings' keys: Esc steps back (the account form, then Settings);
/// Ctrl+K is Link on a signature's page. F1 and Ctrl+/ open the Keyboard
/// shortcuts dialog from Settings (the main window binds them for the rest
/// of the app, and not in the composer for Ctrl+/).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Close, Some(CONTEXT)),
        KeyBinding::new("ctrl-k", Link, Some(CONTEXT)),
        KeyBinding::new("f1", ShowShortcuts, Some(CONTEXT)),
        KeyBinding::new("ctrl-/", ShowShortcuts, Some(CONTEXT)),
    ]);
    // The key arrives while its window is being updated, so the dialog opens
    // once that update is over.
    cx.on_action(|_: &ShowShortcuts, cx: &mut App| {
        let Some(window) = cx
            .active_window()
            .or_else(|| cx.windows().into_iter().next())
        else {
            return;
        };
        cx.defer(move |cx| {
            // Not over another dialog (the trust dialog, a confirmation,
            // the sheet itself): one dialog at a time.
            let _ = window.update(cx, |_, window, cx| {
                if !window.has_active_dialog(cx) {
                    shortcuts::open(window, cx);
                }
            });
        });
    });
}

/// The sections, in the order the list shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Accounts,
    Signatures,
    Rules,
    General,
    Appearance,
    Privacy,
    About,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Accounts,
        Section::Signatures,
        Section::Rules,
        Section::General,
        Section::Appearance,
        Section::Privacy,
        Section::About,
    ];

    /// The section's name in the language shown.
    pub fn name(self) -> String {
        use komorebi_i18n::tr;
        match self {
            Section::Accounts => tr!("Accounts"),
            Section::Signatures => tr!("Signatures"),
            Section::Rules => tr!("Rules"),
            Section::General => tr!("General"),
            Section::Appearance => tr!("Appearance"),
            Section::Privacy => tr!("Privacy"),
            Section::About => tr!("About"),
        }
    }
}

/// A choice saved in General, Privacy or About that the shell applies (the
/// theme and the language are applied here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    CheckInterval(u64),
    UndoSend(u64),
    MarkRead(i64),
    FlashTaskbar(bool),
    Notify(bool),
    NotifyDetail(bool),
    CloseToTray(bool),
    Autostart(bool),
    ReopenTabs(bool),
    DiagnosticsLog(bool),
    UpdateAuto(bool),
    ComposeRich(bool),
    /// An account's own notifications switch.
    AccountNotify {
        id: i64,
        on: bool,
    },
}

/// What Settings tells the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsEvent {
    /// Esc or Back on the first page: show the mail again.
    Close,
    /// A saved choice (theme and language are already applied).
    Changed(Setting),
    /// The accounts, their order, their names or their folders changed:
    /// the folder pane and the lists read them again.
    AccountsChanged,
    /// An account was added (its check passed): the shell starts its
    /// workers and its first download.
    AccountAdded(i64),
    /// An account was saved; what changed says what its workers need.
    AccountSaved { id: i64, change: Change },
    /// An account was switched on or off.
    AccountEnabled { id: i64, on: bool },
    /// Download again (after the question): the shell replaces the
    /// account's workers, resets, and syncs in full.
    Redownload(i64),
    /// An account was removed (its password is gone, its mail with it).
    AccountRemoved(i64),
    /// A link for the link prompt (the privacy statement, the guide).
    OpenLink(String),
    /// About > Show tips again.
    ShowTips,
    /// Privacy > Restart now: the cache's encryption changes at the next
    /// start; the shell starts a new copy and quits (the Slint app's
    /// `cachelock::restart`).
    RestartNow,
    /// Remove all data: the passwords are gone; close the cache and the
    /// window, then `wipe::remove_files` the data folder.
    WipeAndQuit,
}

/// A line under the page heading, from a typed value (never the
/// sentence, so a language switch rewords it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    NotSaved(String),
    NotRemoved(String),
    NotReset(String),
}

impl Message {
    fn text(&self) -> String {
        match self {
            Message::NotSaved(e) => text::not_saved(e),
            Message::NotRemoved(e) => text::not_removed(e),
            Message::NotReset(e) => text::not_reset(e),
        }
    }
}

/// The Settings view.
pub struct SettingsView {
    pub(crate) db: Db,
    pub(crate) section: Section,
    pub(crate) s: Settings,
    pub(crate) accounts: Vec<Account>,
    pub(crate) message: Option<Message>,
    /// An account's page, while it shows.
    pub(crate) editor: Option<Entity<AccountEditor>>,
    /// Add account: the setup flow, inside Settings, while it shows.
    pub(crate) adding: Option<Entity<OnboardingView>>,
    /// Appearance > Customise: the colour editor, while it shows.
    pub(crate) customising: Option<Entity<theme_editor::ThemeEditor>>,
    /// Appearance: why the last theme file was not taken.
    pub(crate) theme_note: Option<String>,
    /// Appearance > Background: its sliders, once the page has shown.
    pub(crate) background_ui: Option<background::BackgroundUi>,
    /// What the shell reported of each account's workers.
    pub(crate) lines: HashMap<i64, AccountLine>,
    /// About > Updates: the shell's updater view.
    pub(crate) updates: Option<AnyView>,
    pub(crate) sigs: Entity<SignaturesPage>,
    /// Rules (phase 8) are the data lane's view, mounted by the shell.
    rules: Option<Entity<RulesView>>,
    pub(crate) remote_senders: Vec<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsView {
    /// Settings over cache `db` (the window's connection).
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sigs = cx.new(|cx| SignaturesPage::new(db.clone(), window, cx));
        let mut view = Self {
            s: db.settings().unwrap_or_default(),
            accounts: db.accounts().unwrap_or_default(),
            remote_senders: db.remote_allow_list().unwrap_or_default(),
            db,
            section: Section::Accounts,
            message: None,
            editor: None,
            adding: None,
            customising: None,
            theme_note: None,
            background_ui: None,
            lines: HashMap::new(),
            updates: None,
            sigs: sigs.clone(),
            rules: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: Vec::new(),
        };
        view._subscriptions.push(cx.subscribe(
            &sigs,
            |_, _, e: &signatures::SignaturesEvent, cx| match e {
                signatures::SignaturesEvent::Changed => cx.emit(SettingsEvent::AccountsChanged),
            },
        ));
        view
    }

    /// The Rules page, the data lane's view (phase 8): its section shows
    /// in the list once it is mounted.
    pub fn set_rules(&mut self, rules: Option<Entity<RulesView>>, cx: &mut Context<Self>) {
        self.rules = rules;
        cx.notify();
    }

    /// About > Updates: the shell's updater view (the shell lane owns
    /// `views/updates`).
    pub fn set_updates(&mut self, updates: Option<AnyView>, cx: &mut Context<Self>) {
        self.updates = updates;
        cx.notify();
    }

    /// The section shown.
    pub fn section(&self) -> Section {
        self.section
    }

    /// Whether an account's page shows.
    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }

    /// The account page, while it shows.
    pub fn editor(&self) -> Option<&Entity<AccountEditor>> {
        self.editor.as_ref()
    }

    /// The setup flow of Add account, while it shows.
    pub fn adding(&self) -> Option<&Entity<OnboardingView>> {
        self.adding.as_ref()
    }

    /// The signatures page.
    pub fn signatures(&self) -> &Entity<SignaturesPage> {
        &self.sigs
    }

    /// Show a section (closing the account form).
    pub fn show(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.adding = None;
        self.close_theme_editor(cx);
        self.section = section;
        self.message = None;
        self.reload(cx);
        // The rules page reads its rules and its activity as it opens.
        if section == Section::Rules
            && let Some(rules) = &self.rules
        {
            rules.update(cx, |r, cx| r.reload(cx));
        }
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Read the cache again (after the shell or another view changed it).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.s = self.db.settings().unwrap_or_default();
        self.accounts = self.db.accounts().unwrap_or_default();
        self.remote_senders = self.db.remote_allow_list().unwrap_or_default();
        self.sigs.update(cx, |s, cx| s.reload(cx));
        cx.notify();
    }

    /// Add account: the setup flow from its email step, inside Settings.
    pub fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_flow(false, window, cx);
    }

    /// Import from Outlook: the setup flow from its list, inside Settings.
    pub fn import_outlook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_flow(true, window, cx);
    }

    fn open_flow(&mut self, import: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.section = Section::Accounts;
        self.editor = None;
        let db = self.db.clone();
        let flow = cx.new(|cx| {
            if import {
                OnboardingView::importing(db, window, cx)
            } else {
                OnboardingView::adding(db, window, cx)
            }
        });
        self._subscriptions.push(cx.subscribe_in(
            &flow,
            window,
            |this, _, e: &OnboardingEvent, window, cx| this.adding_event(e, window, cx),
        ));
        flow.update(cx, |f, cx| f.focus_first(window, cx));
        self.adding = Some(flow);
        self.message = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    fn adding_event(&mut self, e: &OnboardingEvent, window: &mut Window, cx: &mut Context<Self>) {
        match e {
            OnboardingEvent::Added(id) => {
                cx.emit(SettingsEvent::AccountAdded(*id));
                cx.emit(SettingsEvent::AccountsChanged);
            }
            OnboardingEvent::Finished(_) | OnboardingEvent::Closed => {
                self.adding = None;
                window.focus(&self.focus, cx);
            }
            OnboardingEvent::OpenLink(url) => cx.emit(SettingsEvent::OpenLink(url.clone())),
            // Settings is already open around the flow.
            OnboardingEvent::OpenSettings => {}
        }
        self.reload(cx);
    }

    /// Open the page of account `id` (the folder pane's "Account settings",
    /// a fix on its line).
    pub fn edit_account(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.section = Section::Accounts;
        self.adding = None;
        self.reload(cx);
        let Some(account) = self.accounts.iter().find(|a| a.id == id).cloned() else {
            return;
        };
        let db = self.db.clone();
        let editor = cx.new(|cx| AccountEditor::new(db, account, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            |this, _, e: &EditorEvent, window, cx| this.editor_event(e, window, cx),
        ));
        self.editor = Some(editor);
        self.message = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    fn editor_event(&mut self, e: &EditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        match e {
            EditorEvent::Back => {
                self.editor = None;
                window.focus(&self.focus, cx);
            }
            EditorEvent::Saved { id, change } => {
                self.editor = None;
                window.focus(&self.focus, cx);
                cx.emit(SettingsEvent::AccountSaved {
                    id: *id,
                    change: *change,
                });
                cx.emit(SettingsEvent::AccountsChanged);
            }
            EditorEvent::Redownload(id) => {
                self.editor = None;
                cx.emit(SettingsEvent::Redownload(*id));
            }
            EditorEvent::Removed(id) => {
                self.editor = None;
                self.lines.remove(id);
                window.focus(&self.focus, cx);
                cx.emit(SettingsEvent::AccountRemoved(*id));
                cx.emit(SettingsEvent::AccountsChanged);
            }
            EditorEvent::Enabled { id, on } => {
                cx.emit(SettingsEvent::AccountEnabled { id: *id, on: *on });
                cx.emit(SettingsEvent::AccountsChanged);
            }
            EditorEvent::Notify { id, on } => {
                cx.emit(SettingsEvent::Changed(Setting::AccountNotify {
                    id: *id,
                    on: *on,
                }));
            }
            EditorEvent::Moved { .. } | EditorEvent::IconChanged => {
                cx.emit(SettingsEvent::AccountsChanged)
            }
            EditorEvent::ManageSignatures => {
                self.show(Section::Signatures, window, cx);
                return;
            }
            EditorEvent::OpenLink(url) => {
                cx.emit(SettingsEvent::OpenLink(url.clone()));
                return;
            }
        }
        self.reload(cx);
    }

    /// Esc: the account form closes first, then Settings.
    fn close(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(flow) = self.adding.clone() {
            if !flow.update(cx, |f, cx| f.back(window, cx)) {
                self.adding = None;
                window.focus(&self.focus, cx);
                cx.notify();
            }
            return;
        }
        if self.customising.is_some() {
            self.close_theme_editor(cx);
            window.focus(&self.focus, cx);
            return;
        }
        if self.editor.is_none()
            && self.section == Section::Signatures
            && self.sigs.update(cx, |s, cx| s.close(cx))
        {
            window.focus(&self.focus, cx);
            return;
        }
        match &self.editor {
            Some(editor) => {
                let editor = editor.clone();
                if !editor.update(cx, |e, cx| e.back(window, cx)) {
                    self.editor = None;
                    window.focus(&self.focus, cx);
                    cx.notify();
                }
            }
            None => cx.emit(SettingsEvent::Close),
        }
    }

    /// Save `key` and read the settings again; a failure is the page's
    /// message.
    pub(crate) fn save(&mut self, key: &str, value: &str, cx: &mut Context<Self>) -> bool {
        let saved = self.db.set_setting(key, value);
        self.s = self.db.settings().unwrap_or_default();
        let ok = match saved {
            Ok(()) => {
                self.message = None;
                true
            }
            Err(e) => {
                self.message = Some(Message::NotSaved(format!("{e:#}")));
                false
            }
        };
        cx.notify();
        ok
    }

    fn sections(&self) -> Vec<Section> {
        Section::ALL
            .into_iter()
            .filter(|s| *s != Section::Rules || self.rules.is_some())
            .collect()
    }

    fn render_nav(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let items = self.sections();
        let count = items.len();
        let on = self.section;
        let rows = items.into_iter().enumerate().map(|(ix, section)| {
            let selected = section == on;
            let row = div()
                .id(SharedString::from(format!("settings-nav-{ix}")))
                .role(Role::Tab)
                .aria_label(section.name())
                .aria_selected(selected)
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(count)
                .tab_index(0)
                .relative()
                .h(px(crate::theme::metrics::scaled::control(
                    &t,
                    layout::CONTROL_H_MD,
                )))
                .flex()
                .flex_none()
                .items_center()
                .pl(px(space::SP_6))
                .pr(px(space::SP_3))
                .rounded(px(radius::FULL))
                .typo(if selected {
                    &t.text.body_strong
                } else {
                    &t.text.body
                })
                .text_color(c.ink)
                .when(selected, |el| {
                    el.bg(c.wash).child(
                        // The place spot, as the folder pane's (8 px in,
                        // centred at any text size).
                        div()
                            .absolute()
                            .left(px(space::SP_2))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .size(px(layout::PLACE_SPOT))
                                    .rounded(px(radius::FULL))
                                    .bg(c.accent),
                            ),
                    )
                })
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(&t))
                .child(section.name())
                .on_click(cx.listener(move |this, _, window, cx| this.show(section, window, cx)));
            // Its access key: the letter for UI Automation, the chip while
            // Alt is held.
            access::decorate(row, access::Key::Section(section), cx).test_support()
        });
        div()
            .id("settings-nav")
            .role(Role::TabList)
            .aria_label(text::settings_title())
            .flex_none()
            .flex()
            .gap(px(space::SP_1))
            .when(compact, |el| {
                el.flex_row()
                    .flex_wrap()
                    .px(px(space::SP_4))
                    .pt(px(space::SP_4))
            })
            .when(!compact, |el| {
                el.flex_col()
                    .w(px(layout::SETNAV_W))
                    .px(px(space::SP_2))
                    .py(px(space::SP_5))
                    .child(
                        div()
                            // The title starts where the pages' names do.
                            .px(px(space::SP_6))
                            .pb(px(space::SP_3))
                            .typo(&t.text.subtitle)
                            .text_color(c.ink)
                            .child(text::settings_title()),
                    )
            })
            .children(rows)
            .test_support()
    }

    fn render_message(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let message = self.message.as_ref()?;
        let t = cx.tokens();
        Some(
            div()
                .w_full()
                .max_w(px(card::CARD_MAX_W))
                .pt(px(space::SP_3))
                .text_color(t.color.ink)
                .child(Notice::new(
                    "settings-message",
                    notice::Kind::Error,
                    message.text(),
                )),
        )
    }

    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(editor) = &self.editor {
            return editor.clone().into_any_element();
        }
        if let Some(flow) = &self.adding {
            return flow.clone().into_any_element();
        }
        if let Some(editor) = &self.customising {
            return editor.clone().into_any_element();
        }
        match self.section {
            Section::Accounts => self.render_accounts(cx).into_any_element(),
            Section::Signatures => self.sigs.clone().into_any_element(),
            Section::Rules => match &self.rules {
                Some(rules) => rules.clone().into_any_element(),
                None => div().into_any_element(),
            },
            Section::General => self.render_general(cx).into_any_element(),
            Section::Appearance => self.render_appearance(window, cx).into_any_element(),
            Section::Privacy => self.render_privacy(window, cx).into_any_element(),
            Section::About => self.render_about(cx).into_any_element(),
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let compact = window.viewport_size().width < px(layout::COMPACT_BELOW_W);
        let page = self.render_page(window, cx);
        // The account form and the rules page scroll themselves.
        let own_scroll =
            self.editor.is_some() || self.adding.is_some() || self.section == Section::Rules;
        let pad_x = if compact {
            geo::PAGE_PAD_X_COMPACT
        } else {
            geo::PAGE_PAD_X
        };
        let body = if own_scroll {
            div().size_full().min_h_0().child(page).into_any_element()
        } else {
            ScrollArea::new("settings-scroll", &self.scroll)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .px(px(pad_x))
                        .py(px(geo::PAGE_PAD_Y))
                        .child(page)
                        .children(self.render_message(cx)),
                )
                .into_any_element()
        };
        // A pane: see-through like the others over a background, with the
        // blurred copy behind it when Blur behind panes is on.
        let behind = crate::backdrop::pane_layer(gpui::Pixels::ZERO, cx);
        let fill = if behind.is_some() {
            t.color.clear
        } else {
            t.color.pane
        };
        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(|this, a: &ShowSection, window, cx| {
                if this.sections().contains(&a.0) {
                    this.show(a.0, window, cx);
                }
            }))
            .role(Role::Pane)
            .aria_label(text::settings_title())
            .size_full()
            .flex()
            .when(compact, |el| el.flex_col())
            .when(!compact, |el| el.flex_row())
            .overflow_hidden()
            .relative()
            .bg(fill)
            .text_color(t.color.ink)
            .typo(&t.text.body)
            .children(behind)
            .child(self.render_nav(compact, cx))
            .child(div().flex_1().min_w_0().min_h_0().child(body))
            .test_support()
    }
}
