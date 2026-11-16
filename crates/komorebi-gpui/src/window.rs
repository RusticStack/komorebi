//! The main window: the app-drawn title bar (`crate::titlebar`, with the
//! mail view's search in its centre) over the mail view (`views::mail`;
//! an empty paper pane when the cache could not be opened), its saved place
//! (cookbook_window_state: observed bounds, flushed on quit), the live
//! theme feeds a window owns (light and dark from Windows, animation
//! effects re-read when the window becomes active; `theme::feeds`), the
//! tab keys (`crate::tabs`) and the first-frame snapshot (`crate::snapshot`).

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Pixels, Render, Styled, Subscription, Window, WindowBounds, div, point,
    prelude::*, px, size,
};
use komorebi::db::{Db, Geometry};
use komorebi::tabs::Tabs;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use crate::access::{self, AccessHost as _};
use crate::i18n::CurrentLang;
use crate::tabs::{self, FirstTab, LastTab, NextTab, PreviousTab};
use crate::theme::{ActiveTokens as _, feeds, layout, radius, space};
use crate::titlebar::TitleBar;
use crate::tray::{self, Tray, TrayEvent};
use crate::views::contacts::ContactsView;
use crate::views::mail::hub::Hub;
use crate::views::mail::{self, MailEvent, MailView, commands as mail_keys};
use crate::views::updates::{self, UpdateEvent, Updater};
use crate::{snapshot, snapshot_drive};

#[cfg(test)]
mod access_tests;
#[cfg(test)]
mod drafts_list_tests;
mod places;
#[cfg(test)]
mod port_tests;
mod startup;
mod tab_host;
#[cfg(test)]
mod tab_tests;
mod tips;

/// The window's size when no saved place fits a display, and its minimum.
pub const DEFAULT_SIZE: (f32, f32) = layout::WINDOW_DEFAULT;
pub const MIN_SIZE: (f32, f32) = layout::WINDOW_MIN;

/// The setting key of the GPUI window's place (logical px; see
/// `komorebi::db::Settings::gpui_window`).
pub const GEOMETRY_KEY: &str = "gpui_window";

/// What the window needs from start-up. The language and the theme are
/// app globals (`crate::i18n`, `crate::theme`), set before the window opens.
pub struct Init {
    /// The cache, for saving the window's place on quit (`None`: it could
    /// not be opened; nothing is saved).
    pub db: Option<Db>,
    /// `KOMOREBI_TIMING`: when `main` started, for the first-frame line.
    pub timing: Option<Instant>,
    /// `KOMOREBI_EXIT_AFTER_FIRST_FRAME`: quit once the first frame is up.
    pub exit_after_first_frame: bool,
    /// `KOMOREBI_SNAPSHOT`: capture the first frame to a file, then quit
    /// (`snapshot.rs`). The window draws as active and its place is not
    /// saved.
    pub snapshot: Option<snapshot::Request>,
    /// The process exit code when the app quits (a failed snapshot sets 1).
    pub exit_code: Arc<AtomicU8>,
    /// What start-up found: a mail link, Start with Windows, the notices of
    /// the last run, a locked cache (`window/startup.rs`).
    pub start: Start,
}

pub struct MainWindow {
    db: Option<Rc<Db>>,
    /// The account workers and the status line, shared by the mail view,
    /// the tabs and the message windows (with the cache).
    hub: Option<Entity<Hub>>,
    /// The mail view (with the cache).
    mail: Option<Entity<MailView>>,
    /// The inboxes' unread count, as the OS title shows it.
    unread: i64,
    /// The place to save: windowed bounds, or the restore bounds while
    /// maximized.
    bounds: WindowBounds,
    timing: Option<Instant>,
    exit_after_first_frame: bool,
    snapshot: Option<snapshot::Request>,
    exit_code: Arc<AtomicU8>,
    first_frame_seen: bool,
    /// The workers, the tray and the timers were started (after the first
    /// frame, or at once for a window that starts hidden).
    work_started: bool,
    /// The window is in front (the taskbar flash, the toasts).
    active: bool,
    /// Started hidden (`--minimized`) and not shown since: GPUI applies
    /// the saved place on the first activation, so that show must not go
    /// through `ShowWindow` first (`show_window`).
    never_shown: bool,
    /// The window's keyboard focus when nothing inside has it: the tab keys
    /// (`tabs.rs`) need a focused path through the window's key context.
    focus: FocusHandle,
    /// The tabs: the Mail tab, message tabs, draft tabs.
    tabs: Tabs,
    /// The views behind the tabs, and the message windows.
    tab_views: tab_host::TabViews,
    /// The tray icon and new-mail notifications (with the cache).
    tray: Option<Entity<Tray>>,
    /// The updater's state and its bar (with the cache).
    updater: Option<Entity<Updater>>,
    /// A view mounted over the mail (Settings, Contacts, the onboarding
    /// flow), with its subscription to what it asks of us.
    place: Option<places::Mounted>,
    /// Contacts: made with the window, empty until it shows; its CardDAV
    /// passes ride on the mail check timer.
    contacts: Option<Entity<ContactsView>>,
    /// The teaching tips (`window/tips.rs`).
    tips: tips::Tips,
    _subscriptions: Vec<Subscription>,
}

pub use places::remove_data_if_asked;
pub use startup::{Start, is_locked as cache_is_locked, set_aside as set_cache_aside};

impl MainWindow {
    pub fn new(init: Init, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            // Move, resize, maximize: keep the place (saved on quit) and
            // redraw (the Maximize glyph turns into Restore).
            cx.observe_window_bounds(window, |this, window, cx| {
                this.bounds = window.window_bounds();
                cx.notify();
            }),
            // The title and caption glyphs dim while another window is in
            // front. Coming back (from Windows' Settings, say) re-reads the
            // appearance settings: animation effects send GPUI no message.
            cx.observe_window_activation(window, |this, window, cx| {
                this.active = window.is_window_active();
                if this.active {
                    feeds::refresh_from_windows(cx);
                }
                // Alt held while the window left: the release goes elsewhere.
                access::on_activation(window, cx);
                cx.notify();
            }),
            // Windows' light or dark app mode changed.
            cx.observe_window_appearance(window, |_, _, cx| {
                feeds::refresh_from_windows(cx);
            }),
            // A language switch redraws every window by itself (crate::i18n);
            // the OS title (taskbar, Alt+Tab, UI Automation) is not drawn.
            cx.observe_global_in::<CurrentLang>(window, |this: &mut Self, window, cx| {
                window.set_window_title(&crate::text::window_title(this.unread as usize));
                if let Some(tray) = &this.tray {
                    tray.read(cx).relabel(this.unread as usize);
                }
                this.retitle_popouts(cx);
            }),
            // Synchronous: the async debounce would never run at quit.
            // Every draft is saved too (the pane's, the tabs', the windows').
            cx.on_app_quit(|this: &mut Self, cx| {
                this.save_geometry();
                this.save_drafts(cx);
                async {}
            }),
        ];
        let focus = cx.focus_handle();
        let db = init.db.map(Rc::new);
        let hub = db.clone().map(|db| cx.new(|cx| Hub::new(db, cx)));
        let mail = db
            .clone()
            .zip(hub.clone())
            .map(|(db, hub)| cx.new(|cx| MailView::with_hub(db, hub, window, cx)));
        let mut subscriptions = subscriptions;
        if let Some(mail) = &mail {
            subscriptions.push(cx.subscribe_in(mail, window, |this, _, event, window, cx| {
                this.on_mail(event, window, cx);
            }));
            // A move, its Undo or its end changes the list's held move: the
            // message tabs and windows follow (`tab_host::follow_held`).
            subscriptions.push(cx.observe_in(mail, window, |this, _, window, cx| {
                this.follow_held(window, cx);
                // Read and important marks set there: the message tabs and
                // windows follow (`tab_host::follow_flags`).
                this.follow_flags(cx);
            }));
        }
        // A sync that removed messages (a vanished folder, a removed
        // account) marks their tabs.
        if let Some(hub) = &hub {
            subscriptions.push(cx.subscribe(hub, |this, _, event, cx| match event {
                crate::views::mail::hub::HubEvent::Changed => {
                    this.mark_gone_tabs(cx);
                    this.push_states(cx);
                    this.check_new_mail(cx);
                }
                // The check timer ran: Contacts' CardDAV passes that are due.
                crate::views::mail::hub::HubEvent::Tick => {
                    if let Some(contacts) = &this.contacts {
                        contacts.update(cx, |c, cx| c.timer_tick(cx));
                    }
                }
                // The What's new bar was closed or acted on: told.
                crate::views::mail::hub::HubEvent::WhatsNew => {
                    if let Some(db) = &this.db {
                        updates::whats_new::seen(db);
                    }
                }
                _ => {}
            }));
        }
        let updater = db.clone().map(|db| cx.new(|_| Updater::new(db)));
        if let Some(updater) = &updater {
            subscriptions.push(
                cx.subscribe_in(updater, window, |this, _, event, window, cx| match event {
                    UpdateEvent::Quit => {
                        if let Some(tray) = &this.tray {
                            tray.update(cx, |t, _| t.quit());
                        }
                        if this.close_popouts(cx) {
                            cx.quit();
                        }
                    }
                    UpdateEvent::OpenLink(url) => places::open_link(url, window, cx),
                }),
            );
            subscriptions.push(cx.observe(updater, |_, _, cx| cx.notify()));
        }
        let contacts = db
            .clone()
            .map(|db| cx.new(|cx| ContactsView::new(db, window, cx)));
        if let Some(contacts) = &contacts {
            subscriptions.push(
                cx.subscribe_in(contacts, window, |this, _, event, window, cx| {
                    this.on_contacts(event, window, cx);
                }),
            );
        }
        let tray = db.clone().map(|db| cx.new(|_| Tray::new(db)));
        if let Some(tray) = &tray {
            subscriptions.push(cx.subscribe_in(tray, window, |this, _, event, window, cx| {
                this.on_tray(event, window, cx);
            }));
        }
        // Closing the window keeps the app in the tray when the setting
        // asks for it; otherwise the message windows close with it.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.should_close(window, cx))
                .unwrap_or(true)
        });
        let unread = mail
            .as_ref()
            .map_or(0, |m| m.read(cx).mail().inbox_unread());
        window.set_window_title(&crate::text::window_title(unread as usize));
        if let (Some(mail), Some(_)) = (&mail, &init.snapshot) {
            mail.update(cx, |m, cx| snapshot_actions(m, window, cx));
        }
        // The keys go to the list when there is one.
        match &mail {
            Some(mail) => {
                let list = mail.read(cx).list_focus().clone();
                window.focus(&list, cx);
            }
            None => window.focus(&focus, cx),
        }
        let start = init.start;
        let tips = db.as_ref().map_or_else(tips::Tips::default, |db| {
            let seen = db.settings().map(|s| s.tips_seen).unwrap_or_default();
            tips::Tips::new(&seen, updates::whats_new::due(db))
        });
        subscriptions.push(Self::tips_esc(window, cx));
        let mut this = Self {
            db,
            hub,
            mail,
            unread,
            bounds: window.window_bounds(),
            timing: init.timing,
            exit_after_first_frame: init.exit_after_first_frame || init.snapshot.is_some(),
            snapshot: init.snapshot,
            exit_code: init.exit_code,
            first_frame_seen: false,
            work_started: false,
            active: window.is_window_active(),
            never_shown: false,
            focus,
            tabs: Tabs::default(),
            tab_views: Default::default(),
            tray,
            updater,
            place: None,
            contacts,
            tips,
            _subscriptions: subscriptions,
        };
        // No account yet: the onboarding flow opens.
        this.first_run(window, cx);
        if let Some(action) = this.snapshot.as_ref().map(|r| r.action) {
            this.snapshot_action(action, window, cx);
        }
        this.take_start(start, window, cx);
        this
    }

    /// What start-up handed over: the notices, the locked-cache dialog, a
    /// mail link, a start in the tray.
    fn take_start(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        self.say_start_notices(&start, cx);
        let hidden = start.hidden();
        self.never_shown = hidden;
        if let Some(db) = start.locked {
            cx.defer_in(window, move |this, window, cx| {
                this.ask_locked(db, window, cx)
            });
        }
        if let Some(mailto) = start.mailto {
            cx.defer_in(window, move |this, window, cx| {
                this.compose_mailto(mailto, window, cx)
            });
        }
        // Hidden in the tray: no frame is drawn, so the work starts now;
        // without a tray icon the window shows after all.
        if hidden {
            cx.defer_in(window, |this, window, cx| {
                this.start_work(window, cx);
                // After the icon's own deferred start.
                cx.defer_in(window, |this, window, cx| {
                    if !this.tray.as_ref().is_some_and(|t| t.read(cx).has_icon()) {
                        this.show_window(window);
                    }
                });
            });
        }
    }

    /// The window back from the tray, in front. The first show of a window
    /// that started hidden is GPUI's activation alone: it applies the saved
    /// place (bounds, maximized) as it shows the window, where a
    /// `ShowWindow` first would show it at the creation place and then
    /// move it.
    fn show_window(&mut self, window: &mut Window) {
        if std::mem::take(&mut self.never_shown) {
            window.activate_window();
        } else {
            tray::set_shown(window, true);
        }
    }

    /// A `mailto:` link (the command line, a second launch): the window in
    /// front, the Mail tab, and the composer with the fields filled (never
    /// sent).
    pub fn compose_mailto(
        &mut self,
        mailto: komorebi::mail::render::links::Mailto,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.place() != Some(mail::Place::Onboarding) {
            self.close_place(window, cx);
        }
        self.show_mail_tab(window, cx);
        self.with_mail(window, cx, |m, window, cx| {
            m.compose_mailto(None, mailto, window, cx)
        });
    }

    /// The Mail tab in front (the composer of New mail and of a link lives
    /// in its reading pane).
    fn show_mail_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.active() != 0 {
            self.select_tab(0, window, cx);
        }
    }

    /// The new inbox mail since the last look notifies (found by message
    /// id, so a read and a new message in one pass still say so).
    fn check_new_mail(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        if let Some(tray) = &self.tray {
            tray.update(cx, |t, _| {
                t.check_new(active);
            });
        }
    }

    /// What the mail view tells the window.
    fn on_mail(&mut self, event: &MailEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            MailEvent::Unread(n) => {
                let active = window.is_window_active();
                if let Some(tray) = &self.tray {
                    tray.update(cx, |t, _| t.set_unread(*n as usize, active));
                }
                let flash = self
                    .db
                    .as_ref()
                    .is_some_and(|db| db.settings().is_ok_and(|s| s.flash_taskbar));
                if tray::should_flash(flash, active, self.unread, *n) && self.snapshot.is_none() {
                    tray::flash_taskbar(window);
                }
                self.unread = *n;
                window.set_window_title(&crate::text::window_title(*n as usize));
                cx.notify();
            }
            MailEvent::Show(place) => self.show_place(*place, window, cx),
            MailEvent::EditAccount(id) => self.edit_account(*id, window, cx),
            MailEvent::ClosePlace => self.close_place(window, cx),
            MailEvent::OpenLink(url) => places::open_link(url, window, cx),
            MailEvent::OpenTab { id, background } => self.open_in_tab(*id, *background, window, cx),
            MailEvent::OpenWindow(id) => self.open_in_window(*id, window, cx),
            MailEvent::OpenSettings(section) => self.show_section(*section, window, cx),
            MailEvent::OpenDraftWindow => self.pane_draft_out(window, cx),
            MailEvent::OpenDraftTab {
                row,
                background,
                sent_by,
            } => self.open_undone_or_aside(*row, *background, *sent_by, window, cx),
            MailEvent::Changed => {
                self.mark_gone_tabs(cx);
                self.follow_held(window, cx);
            }
        }
    }

    /// What the tray asks.
    fn on_tray(&mut self, event: &TrayEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TrayEvent::Show => self.show_window(window),
            TrayEvent::CheckMail => {
                if let Some(hub) = &self.hub {
                    hub.update(cx, |hub, cx| hub.refresh(cx));
                }
            }
            TrayEvent::Quit => {
                if let Some(tray) = &self.tray {
                    tray.update(cx, |t, _| t.quit());
                }
                if self.close_popouts(cx) {
                    cx.quit();
                }
            }
            TrayEvent::OpenMessage(id) => {
                self.show_window(window);
                self.close_place(window, cx);
                self.open_in_tab(*id, false, window, cx);
            }
            TrayEvent::Mailto(m) => {
                self.show_window(window);
                self.compose_mailto(m.clone(), window, cx);
            }
        }
    }

    /// The window's close button (or Alt+F4): to the tray, or closed with
    /// its message windows.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self
            .tray
            .as_ref()
            .is_some_and(|t| t.read(cx).keeps_running())
        {
            tray::set_shown(window, false);
            return false;
        }
        // A composer window whose draft could not be saved keeps the main
        // window open too, rather than losing the draft's text.
        self.close_popouts(cx)
    }

    /// The place as it would be saved.
    pub fn geometry(&self) -> Geometry {
        to_geometry(&self.bounds)
    }

    /// The updater, when the cache is open (Settings > About reads it).
    pub fn updater(&self) -> Option<&Entity<Updater>> {
        self.updater.as_ref()
    }

    /// The hub, when the cache is open.
    pub fn hub(&self) -> Option<&Entity<Hub>> {
        self.hub.as_ref()
    }

    /// The mail view, when the cache is open.
    pub fn mail(&self) -> Option<&Entity<MailView>> {
        self.mail.as_ref()
    }

    /// A window key for the mail view (New mail, Search, Inbox, Refresh,
    /// the folders).
    fn with_mail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut MailView, &mut Window, &mut Context<MailView>),
    ) {
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| f(m, window, cx));
        }
    }

    /// Ctrl+N: the composer opens in the Mail tab's reading pane, so a
    /// message or draft tab in front gives way to the Mail tab (its body
    /// would hide the composer).
    fn new_mail(&mut self, _: &mail_keys::NewMail, window: &mut Window, cx: &mut Context<Self>) {
        if self.place() == Some(mail::Place::Onboarding) {
            return;
        }
        self.show_mail_tab(window, cx);
        self.with_mail(window, cx, MailView::new_mail);
    }

    /// F1 and Ctrl+/: the Keyboard shortcuts dialog, from anywhere.
    fn show_shortcuts(
        &mut self,
        _: &tabs::ShowShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // One dialog at a time (the trust dialog, a confirmation, the sheet).
        if !gpui_kit::component::WindowExt::has_active_dialog(window, cx) {
            crate::views::settings::shortcuts::open(window, cx);
        }
    }

    /// The Mail tab's panes in F6 order: the folders, the list, then the
    /// composer or the open message (tests; the title bar's search is the
    /// mail view's own stop).
    #[cfg(test)]
    fn panes(&self, cx: &App) -> Vec<FocusHandle> {
        let Some(mail) = &self.mail else {
            return Vec::new();
        };
        let m = mail.read(cx);
        let mut panes = vec![m.nav_focus().clone(), m.list_focus().clone()];
        let composer = m.composer().read(cx);
        if composer.is_open() {
            panes.push(composer.focus_handle(cx));
        } else if m.mail().open_id().is_some() {
            panes.push(m.reader().read(cx).focus_handle(cx));
        }
        panes
    }

    /// F6 (`forward`) and Shift+F6 from outside the mail view (the title
    /// bar's search): the mail view's own cycle round the search, the
    /// folders, the list and the reading pane. Outside the Mail tab nothing
    /// moves, unless Settings or Contacts show beside the folder pane (the
    /// mail view draws them, whatever tab is current).
    fn cycle_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.active() != 0 && self.place.as_ref().is_none_or(|p| !p.in_mail) {
            return;
        }
        self.with_mail(window, cx, |m, window, cx| {
            m.cycle_pane(!forward, window, cx)
        });
    }

    fn next_pane(&mut self, _: &tabs::NextPane, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(true, window, cx);
    }

    fn previous_pane(
        &mut self,
        _: &tabs::PreviousPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_pane(false, window, cx);
    }

    fn focus_search(
        &mut self,
        _: &mail_keys::FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_mail(window, cx, MailView::focus_search);
    }

    fn go_inbox(&mut self, _: &mail_keys::GoInbox, window: &mut Window, cx: &mut Context<Self>) {
        self.with_mail(window, cx, MailView::go_inbox);
    }

    fn refresh(&mut self, _: &mail_keys::Refresh, window: &mut Window, cx: &mut Context<Self>) {
        self.with_mail(window, cx, MailView::refresh);
    }

    fn toggle_folders(
        &mut self,
        _: &mail_keys::ToggleFolders,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_mail(window, cx, MailView::toggle_folders);
    }

    #[cfg(test)]
    pub fn tabs(&self) -> &Tabs {
        &self.tabs
    }

    #[cfg(test)]
    pub fn tabs_mut(&mut self) -> &mut Tabs {
        &mut self.tabs
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(self.tabs.step(1), window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(self.tabs.step(-1), window, cx);
    }

    fn first_tab(&mut self, _: &FirstTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.number(1) {
            self.select_tab(i, window, cx);
        }
    }

    fn last_tab(&mut self, _: &LastTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.number(9) {
            self.select_tab(i, window, cx);
        }
    }

    fn save_geometry(&mut self) {
        // A snapshot starts at its own size and leaves the cache as it was.
        if self.snapshot.is_some() {
            return;
        }
        let geometry = self.geometry();
        if let Some(db) = &self.db
            && let Err(e) = db.set_setting(GEOMETRY_KEY, &geometry.to_setting())
        {
            eprintln!("komorebi-gpui: saving the window's place: {e:#}");
        }
    }

    /// The first render: once that frame is drawn and presented, the next
    /// pass of the event loop reports it (Komorebi's clock: `main` to the
    /// first pass after the first present, as the Slint app's zero timer
    /// after its first redraw) and optionally quits.
    ///
    /// With a snapshot, the capture runs on a background thread
    /// (`PrintWindow` sends the window a message, which this thread must
    /// stay free to answer), then the app quits with the snapshot's result.
    fn after_first_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.first_frame_seen, true) {
            return;
        }
        self.start_work(window, cx);
        // The A/B harness's script, when KOMOREBI_BENCH asked for one.
        if let Some(steps) = crate::bench::script() {
            let mail = self.mail.clone();
            crate::bench::start(window, mail, steps, cx);
        }
        let (timing, exit) = (self.timing, self.exit_after_first_frame);
        if timing.is_none() && !exit {
            return;
        }
        // A development build can draw the frame off screen instead
        // (`KOMOREBI_SNAPSHOT_OFFSCREEN`): for a display that is off.
        #[cfg(feature = "gallery")]
        if let Some(request) = self.snapshot.clone()
            && komorebi::dev_var("KOMOREBI_SNAPSHOT_OFFSCREEN").is_some()
        {
            offscreen_shot(request, window, &self.exit_code, cx);
            return;
        }
        let capture = self.snapshot.clone().map(|request| {
            let shot = snapshot::Shot {
                hwnd: snapshot::window_handle(window),
                scale: window.scale_factor(),
                palette: cx.tokens().color,
            };
            (request, shot)
        });
        let exit_code = self.exit_code.clone();
        // What the snapshot does first: move the focus, walk, scroll, point
        // (`snapshot_drive`).
        let handle = window.window_handle();
        let list = self.mail.as_ref().map(|m| m.read(cx).list_focus().clone());
        cx.spawn(async move |_, cx| {
            if let Some(started) = timing {
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                eprintln!("[timing] first frame drawn: {ms:.1} ms");
            }
            if let Some((request, shot)) = capture {
                if !request.drive.is_empty() {
                    snapshot_drive::run(handle, list, &request.drive, cx).await;
                }
                let taken = cx
                    .background_executor()
                    .spawn(async move { snapshot::take(&request, &shot) })
                    .await;
                if let Err(e) = taken {
                    eprintln!("snapshot failed: {e:#}");
                    exit_code.store(snapshot::EXIT_FAILED, Ordering::Relaxed);
                }
            }
            if exit {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    }

    /// The work that waits for the window: the last session's tabs, the
    /// account workers, the tray icon, the update check and the What's new
    /// bar. Once; after the first frame, or at once for a window that
    /// starts hidden in the tray (it draws no frame until shown).
    fn start_work(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.work_started, true) {
            return;
        }
        // The last session's tabs, once the first frame is up (not in a
        // snapshot, which shows the mail as the cache has it).
        if self.snapshot.is_none() {
            cx.defer_in(window, |this, window, cx| this.restore_tabs(window, cx));
        }
        // The network work starts once the window is on screen (never in a
        // snapshot; the hub starts no workers offline).
        if self.snapshot.is_none()
            && let Some(hub) = self.hub.clone()
        {
            cx.defer(move |cx| hub.update(cx, |hub, cx| hub.start(cx)));
        }
        // The tray icon (never in a snapshot, never from a test).
        #[cfg(not(test))]
        if self.snapshot.is_none()
            && let Some(tray) = self.tray.clone()
        {
            let unread = self.unread as usize;
            cx.defer(move |cx| tray.update(cx, |t, cx| t.start(unread, cx)));
        }
        // The once-a-day update check (never offline, in a snapshot or a
        // test).
        #[cfg(not(test))]
        if self.snapshot.is_none()
            && let Some(updater) = self.updater.clone()
        {
            cx.defer(move |cx| updater.update(cx, |u, cx| u.schedule_auto(cx)));
        }
        // The teaching tips, shortly after the start (`window/tips.rs`).
        self.arm_tips(tips::DELAY, window, cx);
        // The What's new bar, shortly after the start (once a run, never
        // with a tip).
        #[cfg(not(test))]
        if self.snapshot.is_none() && self.db.is_some() {
            cx.spawn(async move |this, cx| {
                let after = updates::whats_new::AFTER;
                cx.background_executor().timer(after).await; // timer-exempt: the Slint app's What's new bar waits two seconds after start
                let _ = this.update(cx, |this, cx| this.whats_new_due_now(cx));
            })
            .detach();
        }
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The snapshot's list switches (the Slint app's, docs/ENVIRONMENT.md):
/// `KOMOREBI_SNAPSHOT_SEARCH=<text>` types a search, then
/// `KOMOREBI_SNAPSHOT_SELECT=<row>` opens that row (0 the first; `a,b,c`
/// selects several rows with Ctrl+click).
fn snapshot_actions(mail: &mut MailView, window: &mut Window, cx: &mut Context<MailView>) {
    if let Some(text) =
        komorebi::dev_var("KOMOREBI_SNAPSHOT_SEARCH").and_then(|v| v.into_string().ok())
    {
        mail.snapshot_search(&text, window, cx);
    }
    if let Some(rows) =
        komorebi::dev_var("KOMOREBI_SNAPSHOT_SELECT").and_then(|v| v.into_string().ok())
    {
        let rows: Vec<usize> = rows
            .split(',')
            .filter_map(|r| r.trim().parse().ok())
            .collect();
        mail.snapshot_select(&rows, window, cx);
    }
}

/// Draws the first frame to a PNG with GPUI's off-screen render (the
/// gallery's `Window::render_to_image`; development builds only), then
/// quits.
#[cfg(feature = "gallery")]
fn offscreen_shot(
    request: snapshot::Request,
    window: &Window,
    exit_code: &Arc<AtomicU8>,
    cx: &mut Context<MainWindow>,
) {
    let handle = window.window_handle();
    let exit_code = exit_code.clone();
    cx.spawn(async move |_, cx| {
        // Two frames: the list's rows lay out after the first.
        cx.background_executor()
            .timer(std::time::Duration::from_millis(400)) // timer-exempt: the development off-screen shot waits for the rows' first layout
            .await;
        let image = handle.update(cx, |_, window, _| window.render_to_image());
        let saved = match image {
            Ok(Ok(img)) => img.save(&request.path).map_err(|e| e.to_string()),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(e) => Err(format!("{e:#}")),
        };
        match saved {
            Ok(()) => eprintln!("snapshot (off screen): {}", request.path.display()),
            Err(e) => {
                eprintln!("snapshot failed: {e}");
                exit_code.store(snapshot::EXIT_FAILED, Ordering::Relaxed);
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// The saved place for these bounds (logical px, rounded).
pub fn to_geometry(bounds: &WindowBounds) -> Geometry {
    let b = bounds.get_bounds();
    Geometry {
        x: b.origin.x.as_f32().round() as i32,
        y: b.origin.y.as_f32().round() as i32,
        w: b.size.width.as_f32().round() as i32,
        h: b.size.height.as_f32().round() as i32,
        maximized: matches!(bounds, WindowBounds::Maximized(_)),
    }
}

/// The bounds a saved place asks for (not checked against the displays).
pub fn from_geometry(g: Geometry) -> WindowBounds {
    let bounds: Bounds<Pixels> = Bounds::new(
        point(px(g.x as f32), px(g.y as f32)),
        size(px(g.w as f32), px(g.h as f32)),
    );
    if g.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

/// Saved geometry is used only when it still meets a connected display
/// (cookbook_window_state).
pub fn fits_a_display(saved: Bounds<Pixels>, cx: &App) -> bool {
    cx.displays()
        .iter()
        .any(|display| display.bounds().intersects(&saved))
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.after_first_frame(window, cx);
        self.tips_follow_layout(window, cx);
        let tip = self.tip_layer(window, cx);
        let c = cx.tokens().color;
        // Snapshots draw the window active, whatever has the focus.
        let mut bar = TitleBar::new().always_active(self.snapshot.is_some());
        // The search, once there is an account to search (not under a
        // mounted view).
        if self.place.is_none()
            && let Some(mail) = &self.mail
            && !mail.read(cx).mail().accounts().is_empty()
        {
            // The search with its scope beside it (the Slint title bar's).
            bar = bar.center(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(crate::theme::space::SP_1))
                    .child(mail::search_field(mail, cx))
                    .child(mail::search_scope_box(mail, cx)),
            );
        }
        let tab_body = if self.place.is_none() {
            self.tab_body(window, cx)
        } else {
            None
        };
        let strip = if self.place.is_none() {
            self.tab_strip(cx)
        } else {
            None
        };
        let update_bar = match (&self.place, &self.updater) {
            (None, Some(updater)) => updates::bar(updater, cx),
            _ => None,
        };
        // The first-run flow takes the whole body; Settings and Contacts
        // show inside the mail view, beside its pane.
        let over = self.place.as_ref().filter(|p| !p.in_mail);
        let body = match (over, &self.mail) {
            (None, Some(_)) if tab_body.is_some() => div()
                .flex_1()
                .min_h_0()
                .px(px(space::PANE_GAP))
                .pb(px(space::PANE_GAP))
                .children(tab_body)
                .into_any_element(),
            (Some(place), _) => div()
                .flex_1()
                .min_h_0()
                .child(place.view.clone())
                .into_any_element(),
            (None, Some(mail)) => div()
                .flex_1()
                .min_h_0()
                .child(mail.clone())
                .into_any_element(),
            (None, None) => div()
                .flex_1()
                .px(px(space::PANE_GAP))
                .pb(px(space::PANE_GAP))
                .child(div().size_full().rounded(px(radius::SURFACE)).bg(c.pane))
                .into_any_element(),
        };
        div()
            .id("main-window")
            .key_context(tabs::CONTEXT)
            .track_focus(&self.focus)
            .access_keys()
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::first_tab))
            .on_action(cx.listener(Self::last_tab))
            .on_action(cx.listener(Self::go_to_tab))
            .on_action(cx.listener(Self::close_tab_key))
            .on_action(cx.listener(Self::reopen_tab))
            .on_action(cx.listener(Self::move_tab_left))
            .on_action(cx.listener(Self::move_tab_right))
            .on_action(cx.listener(Self::open_in_tab_key))
            .on_action(cx.listener(Self::open_in_window_key))
            .on_action(cx.listener(Self::new_mail))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::go_inbox))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::toggle_folders))
            .on_action(cx.listener(Self::toggle_settings))
            .on_action(cx.listener(Self::show_shortcuts))
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::previous_pane))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(c.backdrop)
            .children(crate::backdrop::layer(cx))
            .child(bar)
            .children(strip)
            .children(update_bar)
            .child(body)
            .children(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, assert_named, in_window, names, press};
    use crate::views::mail::Place;
    use gpui::{Role, TestAppContext};
    use komorebi::db::ThemeChoice;
    use komorebi::i18n::Lang;
    use komorebi::tabs::{Tab, TabKind};

    #[test]
    fn a_saved_place_round_trips() {
        let windowed = Geometry {
            x: -1200,
            y: 40,
            w: 1280,
            h: 800,
            maximized: false,
        };
        assert_eq!(to_geometry(&from_geometry(windowed)), windowed);
        let maximized = Geometry {
            maximized: true,
            ..windowed
        };
        assert!(matches!(
            from_geometry(maximized),
            WindowBounds::Maximized(_)
        ));
        assert_eq!(to_geometry(&from_geometry(maximized)), maximized);
        // The setting's own format, as `Settings::gpui_window` reads it.
        assert_eq!(
            Geometry::from_setting(&maximized.to_setting()),
            Some(maximized)
        );
    }

    /// The view follows the window's bounds (headless test window, no GPU).
    #[gpui::test]
    fn the_window_place_is_observed(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (view, cx) = cx
            .add_window_view(|window, cx| MainWindow::new(test_support::plain_init(), window, cx));
        cx.simulate_resize(size(px(1000.), px(700.)));
        cx.run_until_parked();
        let g = view.read_with(cx, |v, _| v.geometry());
        assert_eq!((g.w, g.h, g.maximized), (1000, 700, false));
    }

    /// What UI Automation reads on the caption: three named buttons, in the
    /// language shown (cookbook_a11y queries, by accessible name and role).
    #[gpui_kit::test]
    fn the_caption_buttons_are_named_buttons(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        test_support::set_lang(cx, Lang::PtPt);
        test_support::set_theme(cx, ThemeChoice::Dark);
        let (window, _) = test_support::open_main(cx, test_support::plain_init());
        in_window(cx, window, |window, _| {
            for name in ["Minimizar", "Maximizar", "Fechar"] {
                assert_named(window, name, Role::Button);
            }
            let buttons: Vec<String> = names(window)
                .into_iter()
                .filter(|(role, _)| *role == Some(Role::Button))
                .map(|(_, name)| name)
                .collect();
            assert_eq!(buttons, ["Minimizar", "Maximizar", "Fechar"]);
        });
    }

    /// Port of the Slint app's view test
    /// `ctrl_tab_moves_between_tabs_even_from_a_text_field`
    /// (`src/app/composer/tests/tabs_tests.rs`, docs/GPUI-TEST-PORT.md): the
    /// tab keys, pressed in the window, move between the tabs. The tabs are
    /// added to the model here (phase 5 opens them from the list); the
    /// half that presses them from the composer's To field waits for the
    /// composer (phase 4).
    #[gpui_kit::test]
    fn ctrl_tab_moves_between_tabs(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::plain_init());
        view.update(cx, |v, _| {
            for id in [2, 3] {
                let n = v.tabs_mut().len();
                v.tabs_mut()
                    .insert(n, Tab::new(TabKind::Message(id), String::new()));
            }
        });
        let active = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.tabs().active());
        assert_eq!(view.read_with(cx, |v, _| v.tabs().len()), 3);
        let steps: [(&[&str], usize); 6] = [
            (&["ctrl-tab"], 1),
            (&["ctrl-tab"], 2),
            (&["ctrl-shift-tab"], 1),
            (&["ctrl-pagedown"], 2),
            (&["ctrl-9"], 2),
            (&["ctrl-1"], 0),
        ];
        for (keys, want) in steps {
            in_window(cx, window, |window, cx| press(window, keys, cx));
            assert_eq!(active(cx), want, "after {keys:?}");
        }
        // Round the strip: from the Mail tab, back to the last one.
        in_window(cx, window, |window, cx| {
            press(window, &["ctrl-shift-tab"], cx)
        });
        assert_eq!(active(cx), 2);
    }

    /// The tab actions reach the window from anywhere in it, also when
    /// dispatched without a key (as a menu item would).
    #[gpui_kit::test]
    fn tab_actions_dispatch_without_keys(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::plain_init());
        view.update(cx, |v, _| {
            v.tabs_mut()
                .insert(1, Tab::new(TabKind::Message(7), String::new()));
        });
        in_window(cx, window, |window, cx| {
            test_support::dispatch(window, &LastTab, cx)
        });
        assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
        in_window(cx, window, |window, cx| {
            test_support::dispatch(window, &FirstTab, cx)
        });
        assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 0);
    }

    /// Settings and Contacts show beside the folder pane in place of the
    /// list and the reading pane, and go again: Esc closes Settings, the
    /// foot's button closes the view that shows.
    #[gpui_kit::test]
    fn settings_and_contacts_show_beside_the_pane_and_close(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("mounts", 5);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        let shown = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, cx| {
                (v.place(), v.mail().and_then(|m| m.read(cx).place()))
            })
        };
        assert_eq!(shown(cx), (None, None));
        for place in [Place::Settings, Place::Contacts] {
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |v, cx| v.show_place(place, window, cx))
            })
            .expect("update");
            assert_eq!(shown(cx), (Some(place), Some(place)));
            // The same place again takes it away (the foot's button).
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |v, cx| v.show_place(place, window, cx))
            })
            .expect("update");
            assert_eq!(shown(cx), (None, None), "{place:?} closes");
        }
        // Esc steps back from Settings.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |v, cx| v.show_place(Place::Settings, window, cx))
        })
        .expect("update");
        in_window(cx, window, |window, cx| press(window, &["escape"], cx));
        assert_eq!(shown(cx), (None, None));
        // Rules are a section of Settings.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |v, cx| v.show_rules(window, cx))
        })
        .expect("update");
        assert_eq!(shown(cx).0, Some(Place::Settings));
        let section = view.read_with(cx, |v, cx| v.settings().map(|s| s.read(cx).section()));
        assert_eq!(section, Some(crate::views::settings::Section::Rules));
    }

    /// Ctrl+, opens Settings and closes it again.
    #[gpui_kit::test]
    fn ctrl_comma_toggles_settings(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("ctrl-comma", 3);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        for want in [Some(Place::Settings), None] {
            in_window(cx, window, |window, cx| press(window, &["ctrl-,"], cx));
            assert_eq!(view.read_with(cx, |v, _| v.place()), want);
        }
    }

    /// A first run (no account) opens the onboarding flow over the whole
    /// body; the mail is not drawn.
    #[gpui_kit::test]
    fn a_first_run_opens_the_onboarding_flow(cx: &mut TestAppContext) {
        let cache = test_support::TempCache::new("first-run");
        test_support::init_app(cx);
        let (_window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
        assert_eq!(
            view.read_with(cx, |v, _| v.place()),
            Some(Place::Onboarding)
        );
        assert!(view.read_with(cx, |v, _| v.onboarding().is_some()));
    }

    /// The pane's "Account settings" and the status line's fixes open the
    /// account's page in Settings.
    #[gpui_kit::test]
    fn edit_account_opens_the_accounts_page(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("edit-account", 3);
        let id = db.accounts().unwrap()[0].id;
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |v, cx| v.edit_account(id, window, cx))
        })
        .expect("update");
        let editing = view.read_with(cx, |v, cx| v.settings().map(|s| s.read(cx).editing()));
        assert_eq!(editing, Some(true));
    }

    /// A seeded throwaway cache: the window saves its place there on quit.
    #[gpui::test]
    fn the_place_is_saved_to_the_cache_on_quit(cx: &mut TestAppContext) {
        let (cache, db, seeded) = test_support::seeded("place", 20);
        assert_eq!(db.message_ids(&[seeded.inbox], "").unwrap().len(), 20);
        test_support::init_app(cx);
        let init = test_support::init(Some(db));
        let (_view, vcx) = cx.add_window_view(|window, cx| MainWindow::new(init, window, cx));
        vcx.simulate_resize(size(px(900.), px(600.)));
        vcx.run_until_parked();
        cx.quit();
        let saved = cache.open().settings().unwrap().gpui_window.expect("saved");
        assert_eq!((saved.w, saved.h, saved.maximized), (900, 600, false));
    }

    /// The pane that has the keys: 0 the folders, 1 the list, 2 the reading
    /// pane (the open message or the composer).
    fn pane_with_keys(
        window: gpui::AnyWindowHandle,
        view: &Entity<MainWindow>,
        cx: &mut TestAppContext,
    ) -> Option<usize> {
        cx.update_window(window, |_, window, cx| {
            let panes = view.read(cx).panes(cx);
            panes.iter().position(|h| h.contains_focused(window, cx))
        })
        .expect("update")
    }

    /// Port of `f6_cycles_the_panes_and_the_folders_take_arrows` and
    /// `shift_f6_reaches_the_folders_from_every_pane` (their F6 halves; the
    /// folders' arrows are the list lane's NAV keys): F6 goes from the list
    /// to the reading pane (here the composer), then to the folders and
    /// round; Shift+F6 goes back the other way.
    #[gpui_kit::test]
    fn f6_cycles_the_panes(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("f6", 3);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        in_window(cx, window, |window, cx| press(window, &["ctrl-n"], cx));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            let mail = view.read(cx).mail().cloned().expect("the mail view");
            let list = mail.read(cx).list_focus().clone();
            window.focus(&list, cx);
        })
        .expect("update");
        cx.run_until_parked();
        assert_eq!(pane_with_keys(window, &view, cx), Some(1), "the list");
        // The mail view's cycle: the title bar's search is a stop too (no
        // pane of `panes`, so `None`), between the reading pane and the
        // folders.
        for (key, want) in [
            ("f6", Some(2)),
            ("f6", None),
            ("f6", Some(0)),
            ("f6", Some(1)),
            ("shift-f6", Some(0)),
            ("shift-f6", None),
            ("shift-f6", Some(2)),
            ("shift-f6", Some(1)),
        ] {
            in_window(cx, window, |window, cx| press(window, &[key], cx));
            cx.run_until_parked();
            assert_eq!(pane_with_keys(window, &view, cx), want, "after {key}");
        }
    }

    /// F1 and Ctrl+/ open the Keyboard shortcuts dialog; Ctrl+/ in the
    /// composer does not (the Slint app's `ui/app.slint` keys).
    #[gpui_kit::test]
    fn f1_and_ctrl_slash_open_the_shortcuts(cx: &mut TestAppContext) {
        use gpui_kit::component::WindowExt as _;
        let (_cache, db, _) = test_support::seeded("f1", 2);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        let dialog = |cx: &mut TestAppContext| {
            in_window(cx, window, |window, cx| window.has_active_dialog(cx))
        };
        for key in ["f1", "ctrl-/"] {
            in_window(cx, window, |window, cx| press(window, &[key], cx));
            cx.run_until_parked();
            assert!(dialog(cx), "{key} opens it");
            in_window(cx, window, |window, cx| window.close_dialog(cx));
            cx.run_until_parked();
            assert!(!dialog(cx));
        }
        in_window(cx, window, |window, cx| press(window, &["ctrl-n"], cx));
        cx.run_until_parked();
        let composing = view.read_with(cx, |v, cx| {
            v.mail()
                .is_some_and(|m| m.read(cx).composer().read(cx).is_open())
        });
        assert!(composing);
        in_window(cx, window, |window, cx| press(window, &["ctrl-/"], cx));
        cx.run_until_parked();
        assert!(!dialog(cx), "not from the composer");
    }

    /// A `mailto:` on the command line opens the composer, filled, once the
    /// window is up (the Slint app's `app::open_mailto`).
    #[gpui_kit::test]
    fn a_mail_link_at_start_opens_the_composer(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("start-mailto", 2);
        test_support::init_app(cx);
        let mut init = test_support::init(Some(db));
        init.start.mailto = Some(komorebi::mail::render::links::Mailto {
            to: "ana@example.org".into(),
            subject: "Lunch".into(),
            ..Default::default()
        });
        let (_window, view) = test_support::open_main(cx, init);
        cx.run_until_parked();
        let composing = view.read_with(cx, |v, cx| {
            v.mail()
                .is_some_and(|m| m.read(cx).composer().read(cx).is_open())
        });
        assert!(composing);
    }

    /// Stage D integration: the reader's "Privacy settings" and the
    /// composer's "Manage signatures" open Settings on their own section, and
    /// asking again leaves Settings open (it does not toggle).
    #[gpui_kit::test]
    fn privacy_and_signatures_open_their_settings_section(cx: &mut TestAppContext) {
        use crate::views::composer::ComposerEvent;
        use crate::views::reader::ReaderEvent;
        use crate::views::settings::Section;
        let (_cache, db, _) = test_support::seeded("sections", 2);
        test_support::init_app(cx);
        let (_window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        cx.run_until_parked();
        let mail = view.read_with(cx, |v, _| v.mail().cloned().expect("the mail view"));
        let section = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, cx| v.settings().map(|s| s.read(cx).section()))
        };
        assert_eq!(section(cx), None);
        let reader = mail.read_with(cx, |m, _| m.reader().clone());
        reader.update(cx, |_, cx| cx.emit(ReaderEvent::OpenPrivacy));
        cx.run_until_parked();
        assert_eq!(section(cx), Some(Section::Privacy));
        let composer = mail.read_with(cx, |m, _| m.composer().clone());
        composer.update(cx, |_, cx| cx.emit(ComposerEvent::ManageSignatures));
        cx.run_until_parked();
        assert_eq!(
            section(cx),
            Some(Section::Signatures),
            "still open, on Signatures"
        );
        composer.update(cx, |_, cx| cx.emit(ComposerEvent::ManageSignatures));
        cx.run_until_parked();
        assert_eq!(
            section(cx),
            Some(Section::Signatures),
            "asked again on Signatures: Settings stays open"
        );
    }

    /// The start-up notices go to the status line: a crash last time, a
    /// damaged cache set aside.
    #[gpui_kit::test]
    fn the_notices_of_the_last_run_are_said(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("start-notices", 2);
        test_support::init_app(cx);
        let mut init = test_support::init(Some(db));
        init.start.crashed = true;
        let (window, _) = test_support::open_main(cx, init);
        cx.run_until_parked();
        in_window(cx, window, |window, _| {
            let said = startup::text::crash_notice();
            assert!(
                names(window).iter().any(|(_, n)| n.contains(&said)),
                "{:?}",
                names(window)
            );
        });
        // It stays (the Slint app's `notice`), past the 5 s of news.
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        in_window(cx, window, |window, _| {
            let said = startup::text::crash_notice();
            assert!(names(window).iter().any(|(_, n)| n.contains(&said)));
        });
    }

    /// `--minimized`: the work starts without a frame, and with no tray
    /// icon (a test makes none) the window shows after all, through the
    /// first-show path that lets GPUI apply the saved place.
    #[gpui_kit::test]
    fn a_hidden_start_starts_the_work_and_shows_without_an_icon(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("start-hidden", 2);
        test_support::init_app(cx);
        let mut init = test_support::init(Some(db));
        init.start.minimized = true;
        let (window, view) = test_support::open_main(cx, init);
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert!(v.work_started, "the work started without a frame");
            assert!(!v.never_shown, "no tray icon: the window was shown");
        });
        // A later show (from the tray) goes through `ShowWindow` again.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |v, _| {
                v.never_shown = true;
                v.show_window(window);
                assert!(!v.never_shown, "the first show is taken once");
            })
        })
        .expect("the test window is open");
    }

    /// The damaged cache's notice comes after the crash notice and stays.
    #[gpui_kit::test]
    fn the_damaged_cache_notice_stays_after_the_crash_notice(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("start-damaged", 2);
        test_support::init_app(cx);
        let mut init = test_support::init(Some(db));
        init.start.crashed = true;
        init.start.damaged = Some("cache.damaged-1.db".into());
        let (window, _) = test_support::open_main(cx, init);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        in_window(cx, window, |window, _| {
            let said = startup::text::cache_damaged("cache.damaged-1.db");
            assert!(
                names(window).iter().any(|(_, n)| n.contains(&said)),
                "{:?}",
                names(window)
            );
        });
    }

    /// Port of `the_title_bar_has_no_search_until_the_first_account`: no
    /// account, no search in the title bar; with one, the search shows.
    #[gpui_kit::test]
    fn the_title_bar_has_no_search_until_the_first_account(cx: &mut TestAppContext) {
        let search = crate::views::mail::text::search_name();
        let empty = test_support::TempCache::new("bar-search-empty");
        test_support::init_app(cx);
        let (window, _) = test_support::open_main(cx, test_support::init(Some(empty.open())));
        in_window(cx, window, |window, _| {
            assert!(!names(window).iter().any(|(_, n)| *n == search));
        });
        let (_cache, db, _) = test_support::seeded("bar-search", 2);
        let (window, _) = test_support::open_main(cx, test_support::init(Some(db)));
        in_window(cx, window, |window, _| {
            assert!(names(window).iter().any(|(_, n)| *n == search));
        });
    }
}
