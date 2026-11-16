//! Onboarding (phase 7; mockup `welcome`, the Slint app's
//! `ui/onboarding.slint` and `src/app/onboarding.rs`): the first run's
//! welcome, then Your email (how the servers are found), Sign in (the
//! account check, then the account added), Make it yours (the name and the
//! signature) and You're all set, with the first download's progress.
//! Settings > Accounts > Add account opens the same flow at Your email
//! ([`OnboardingView::adding`]).
//!
//! Import from Outlook (`Step::Import`, [`import`]): the accounts and
//! signatures classic Outlook keeps on this PC, each with a checkbox. The
//! welcome offers it when something was found; Settings > Accounts opens it
//! directly ([`OnboardingView::importing`]). Continue saves the ticked
//! signatures, then walks each ticked account through Sign in with its
//! fields filled in, so only the password is typed.
//!
//! The lookup runs only on Continue, for a domain that is not a preset's,
//! and never sends the password; offline (`KOMOREBI_OFFLINE`) the provider
//! list shows at once (`providers`). Nothing here talks to a server but the check
//! and the lookup, both through [`services`] so tests fake them.

pub mod flow;
pub mod import;
pub mod providers;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, PathBuilder, Pixels, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task,
    Window, canvas, div, point, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::db::Account;
use komorebi::find::{Fields, merge};
use komorebi::i18n::{LangSetting, resolve_here};
use komorebi::import::outlook::{Found, FoundAccount, Kind as FoundKind};
use komorebi::mail::discover::{self, Outcome};
use komorebi::mail::pin::Kind;
use komorebi::providers::{self as presets, PRESETS, Security, SignIn};
use komorebi_i18n::tr;

use self::flow::{Act, Chip};
use self::providers::{PickerEvent, ProviderPicker};
use crate::icons::Icon;
use crate::theme::settings::{self as geo, mark};
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::Db;
use crate::views::settings::about::Page;
use crate::views::settings::account;
use crate::views::settings::card::labelled;
use crate::views::settings::check::{self, CheckFailure, CheckState, Kind3, Request, StepState};
use crate::views::settings::form::{AccountForm, Provider};
use crate::views::settings::general::{LANGUAGES, language_label};
use crate::views::settings::pins::{CertReview, FormPins, Unreadable};
use crate::views::settings::servers::{ServerEvent, ServerForm, find_row};
use crate::views::settings::services;
use crate::views::settings::text;
use crate::views::settings::trust;
use crate::widgets::button::{Button, Height, Variant};
use crate::widgets::field::TextField;
use crate::widgets::icon_button::IconButton;
use crate::widgets::notice::{self, Notice};
use crate::widgets::pane::ScrollArea;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Checkbox;

/// What the setup flow tells its owner (the shell on the first run,
/// Settings for Add account).
#[derive(Clone, Debug, PartialEq)]
pub enum OnboardingEvent {
    /// The account was added (its check passed): start its workers and its
    /// first download.
    Added(i64),
    /// Open your inbox (the account added, if any): close the flow on it.
    Finished(Option<i64>),
    /// Back from the first step: the flow closes, nothing added.
    Closed,
    /// A page for the link prompt (help, the privacy statement).
    OpenLink(String),
    /// The welcome's Settings button: Settings opens in place of the flow.
    OpenSettings,
}

/// The flow's steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    /// Import from Outlook: the list of what was found.
    Import,
    Email,
    SignIn,
    Yours,
    Done,
}

/// Why the address can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmailProblem {
    Invalid,
    Exists,
}

/// The first download of the account added (`ob-sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Download {
    /// No worker yet.
    None,
    /// Started, no count yet.
    Started,
    Counting {
        done: usize,
        total: usize,
    },
    Finished,
}

/// The fields the Sign in step marks as missing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Missing {
    pub password: bool,
    pub provider: bool,
    pub imap: bool,
    pub smtp: bool,
}

impl Missing {
    fn any(self) -> bool {
        self.password || self.provider || self.imap || self.smtp
    }
}

/// The setup flow.
pub struct OnboardingView {
    db: Db,
    step: Step,
    /// The first run (the welcome shows; Back from Your email returns to it).
    first_run: bool,
    focus: FocusHandle,
    scroll: ScrollHandle,
    email: Entity<InputState>,
    email_problem: Option<EmailProblem>,
    domain: String,
    chip: Chip,
    /// The preset chosen (a preset's domain, or picked after a failed
    /// lookup).
    provider: Option<usize>,
    manual: bool,
    /// The provider list shows (no provider yet, or Change pressed).
    choosing: bool,
    /// Manual setup filled in for Proton Mail Bridge.
    bridge: bool,
    picker: Entity<ProviderPicker>,
    offline: bool,
    servers: Entity<ServerForm>,
    password: Entity<InputState>,
    /// The password box's placeholder as last set.
    password_placeholder: String,
    user: Entity<InputState>,
    user_open: bool,
    missing: Missing,
    find: Option<Task<()>>,
    /// The server fields when the lookup started, and what the last one
    /// filled.
    find_from: Option<Fields>,
    found_last: Option<Fields>,
    find_note: String,
    find_stale: bool,
    /// Sign in was pressed while the lookup ran.
    pending: bool,
    check: CheckState,
    /// The wait's reassurance: 1 after 3 s, 2 after 10 s.
    slow: u8,
    slow_task: Option<Task<()>>,
    failure: Option<CheckFailure>,
    details: bool,
    pins: FormPins,
    reading: Option<Task<()>>,
    unreadable: Option<Unreadable>,
    added: Option<(i64, String)>,
    name: Entity<InputState>,
    signature: Entity<InputState>,
    /// The name to start "Your name" with, put in at the next render.
    start_name: Option<String>,
    download: Download,
    has_mail: bool,
    error: Option<String>,
    /// What Outlook has on this PC (`None` while it is read).
    outlook: Option<Found>,
    _outlook_task: Option<Task<()>>,
    import_accounts: Vec<import::AccountRow>,
    import_signatures: Vec<import::SignatureRow>,
    /// The ticked signatures being read (their picture files, off the UI
    /// thread) before they are saved and the accounts' sign-ins start.
    signatures_task: Option<Task<()>>,
    /// The ticked accounts still to sign in to, after the one shown.
    queue: VecDeque<FoundAccount>,
    /// The account from Outlook on Sign in.
    importing: Option<FoundAccount>,
    /// Its place (from 1) and the count of ticked accounts.
    import_place: (usize, usize),
    /// The signatures imported (Outlook name, id), for the defaults.
    imported: Vec<(String, i64)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<OnboardingEvent> for OnboardingView {}

impl Focusable for OnboardingView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl OnboardingView {
    /// The first run: the welcome, then the flow.
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::make(db, true, window, cx);
        this.find_outlook(cx);
        this
    }

    /// Settings > Accounts > Add account: the flow from Your email.
    pub fn adding(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::make(db, false, window, cx)
    }

    /// Settings > Accounts > Import from Outlook: the flow from the list.
    pub fn importing(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::make(db, false, window, cx);
        this.step = Step::Import;
        this.find_outlook(cx);
        this
    }

    /// Read Outlook's accounts and signatures off the UI thread.
    fn find_outlook(&mut self, cx: &mut Context<Self>) {
        let find = services::get(cx).outlook;
        let work = cx.background_executor().spawn(async move { find() });
        self._outlook_task = Some(cx.spawn(async move |this, cx| {
            let found = work.await;
            // Findings already set (a snapshot's made-up ones) stay.
            let _ = this.update(cx, |this, cx| {
                if this.outlook.is_none() {
                    this.set_outlook(found, cx);
                }
            });
        }));
    }

    fn make(db: Db, first_run: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: Option<String>,
                     masked: bool,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                let s = InputState::new(window, cx).masked(masked);
                match placeholder {
                    Some(p) => s.placeholder(p),
                    None => s,
                }
            })
        };
        let email = input(Some(tr!("name@example.com")), false, window, cx);
        let password = input(Some(tr!("Password or app password")), true, window, cx);
        let user = input(Some(tr!("Same as email address")), false, window, cx);
        let name = input(Some(tr!("e.g. Ana Silva")), false, window, cx);
        let signature = input(
            Some(tr!("Added below new mail, replies and forwards")),
            false,
            window,
            cx,
        );
        let servers = cx.new(|cx| ServerForm::new(window, cx).flow());
        let picker = cx.new(|cx| ProviderPicker::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &picker,
                window,
                |this, _, e: &PickerEvent, window, cx| match *e {
                    PickerEvent::Picked(i) => this.pick_provider(i, window, cx),
                    PickerEvent::AddProvider => this.go_manual(window, cx),
                    PickerEvent::Back => {
                        this.back(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &email,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } => this.continue_email(window, cx),
                    InputEvent::Change if this.email_problem.take().is_some() => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &password,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Change if this.missing.password => {
                        this.missing.password = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(&servers, |this: &mut Self, _, _: &ServerEvent, cx| {
                let f = this.servers.read(cx).fields(cx);
                let imap = crate::views::settings::hosts::typed_server(
                    &f.imap_host,
                    &f.imap_port,
                    f.imap_starttls,
                );
                let smtp = crate::views::settings::hosts::typed_server(
                    &f.smtp_host,
                    &f.smtp_port,
                    f.smtp_starttls,
                );
                this.pins.retain_for(imap.as_ref(), smtp.as_ref());
                this.missing.imap = false;
                this.missing.smtp = false;
                cx.notify();
            }),
            cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&signature, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        Self {
            db,
            step: if first_run {
                Step::Welcome
            } else {
                Step::Email
            },
            first_run,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            email,
            email_problem: None,
            domain: String::new(),
            chip: Chip::None,
            provider: None,
            manual: false,
            choosing: false,
            bridge: false,
            picker,
            offline: false,
            servers,
            password,
            password_placeholder: tr!("Password or app password"),
            user,
            user_open: false,
            missing: Missing::default(),
            find: None,
            find_from: None,
            found_last: None,
            find_note: String::new(),
            find_stale: false,
            pending: false,
            check: CheckState::default(),
            slow: 0,
            slow_task: None,
            failure: None,
            details: false,
            pins: FormPins::default(),
            reading: None,
            unreadable: None,
            added: None,
            name,
            signature,
            start_name: None,
            download: Download::None,
            has_mail: false,
            error: None,
            outlook: None,
            _outlook_task: None,
            import_accounts: Vec::new(),
            import_signatures: Vec::new(),
            signatures_task: None,
            queue: VecDeque::new(),
            importing: None,
            import_place: (0, 0),
            imported: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    // ---- what tests and the shell read --------------------------------

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn chip(&self) -> &Chip {
        &self.chip
    }

    pub fn email_problem(&self) -> Option<EmailProblem> {
        self.email_problem
    }

    pub fn manual(&self) -> bool {
        self.manual
    }

    /// The provider list shows.
    pub fn choosing(&self) -> bool {
        self.choosing && !self.manual
    }

    /// Manual setup was filled in for Proton Mail Bridge.
    pub fn bridge(&self) -> bool {
        self.bridge
    }

    /// The provider list.
    pub fn picker(&self) -> &Entity<ProviderPicker> {
        &self.picker
    }

    /// The preset picked on the provider list (an index into `PRESETS`).
    pub fn provider(&self) -> Option<usize> {
        self.provider
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    pub fn busy(&self) -> bool {
        self.check.running()
    }

    pub fn finding(&self) -> bool {
        self.find.is_some()
    }

    pub fn failure(&self) -> Option<&CheckFailure> {
        self.failure.as_ref()
    }

    pub fn missing(&self) -> Missing {
        self.missing
    }

    /// The app-password help of the provider being signed in to (a preset
    /// by the address's domain, its MX records or the list), when it wants
    /// one; never on manual setup.
    pub fn app_password_help(&self) -> Option<presets::AppPassword> {
        if self.manual {
            return None;
        }
        presets::app_password(self.provider)
    }

    /// Opens a provider's page: the shell's link prompt, then the browser.
    fn open_page(
        &self,
        cx: &Context<Self>,
    ) -> impl Fn(String, &mut Window, &mut App) + Clone + 'static {
        let view = cx.entity().downgrade();
        move |url, _, cx| {
            let _ = view.update(cx, |_, cx| cx.emit(OnboardingEvent::OpenLink(url)));
        }
    }

    /// A made-up refused sign-in, for the snapshot of the Sign in step
    /// after a failure (`ob-sign-in-gmail-failed`; offline only).
    pub fn sample_refused(&mut self, cx: &mut Context<Self>) {
        let email = self.email.read(cx).value().trim().to_string();
        let host = self
            .provider
            .and_then(|i| PRESETS.get(i))
            .map_or("imap.example.com", |p| p.imap.0);
        self.failure = Some(CheckFailure {
            step: Some(komorebi::mail::errors::Step::ImapSignIn.index()),
            text: check::CheckText::Say(komorebi::mail::errors::Say::Rejected {
                host: host.to_string(),
                login: email,
            }),
            review: None,
        });
        cx.notify();
    }

    /// The account the flow added, if any.
    pub fn added(&self) -> Option<i64> {
        self.added.as_ref().map(|(id, _)| *id)
    }

    pub fn email_input(&self) -> &Entity<InputState> {
        &self.email
    }

    pub fn password_input(&self) -> &Entity<InputState> {
        &self.password
    }

    /// The password box's placeholder.
    pub fn password_placeholder(&self) -> &str {
        &self.password_placeholder
    }

    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    pub fn signature_input(&self) -> &Entity<InputState> {
        &self.signature
    }

    pub fn server_form(&self) -> &Entity<ServerForm> {
        &self.servers
    }

    pub fn download(&self) -> Download {
        self.download
    }

    /// The wait's reassurance shown (0 none, 1 after 3 s, 2 after 10 s).
    pub fn slow(&self) -> u8 {
        self.slow
    }

    // ---- what the shell tells the flow --------------------------------

    /// The added account's worker started (`Syncing` active) or finished.
    pub fn set_syncing(&mut self, id: i64, active: bool, cx: &mut Context<Self>) {
        if self.added() != Some(id) {
            return;
        }
        self.download = match (active, self.download) {
            (true, Download::None) => Download::Started,
            (true, d) => d,
            (false, _) => Download::Finished,
        };
        cx.notify();
    }

    /// The first download's count (holding the last value; never back).
    pub fn set_progress(&mut self, id: i64, done: usize, total: usize, cx: &mut Context<Self>) {
        if self.added() != Some(id) || total == 0 {
            return;
        }
        let done = match self.download {
            Download::Counting { done: d, total: t } if t == total => done.max(d),
            _ => done,
        };
        self.download = Download::Counting { done, total };
        cx.notify();
    }

    /// The added account's Inbox has a message (told by the shell).
    pub fn has_mail(&self) -> bool {
        self.has_mail
    }

    /// The added account's Inbox has a message.
    pub fn set_has_mail(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.added() == Some(id) {
            self.has_mail = true;
            cx.notify();
        }
    }

    // ---- moving through the flow --------------------------------------

    /// Focus the step's first field.
    pub fn focus_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            Step::Welcome | Step::Import | Step::Done => window.focus(&self.focus, cx),
            Step::Email => self.email.update(cx, |s, cx| s.focus(window, cx)),
            Step::SignIn => {
                if self.manual {
                    let servers = self.servers.clone();
                    servers.update(cx, |s, cx| {
                        let host = s.imap_host.clone();
                        host.update(cx, |h, cx| h.focus(window, cx));
                    });
                } else if self.choosing {
                    self.picker.update(cx, |p, cx| p.focus(window, cx));
                } else {
                    self.password.update(cx, |s, cx| s.focus(window, cx));
                }
            }
            Step::Yours => self.name.update(cx, |s, cx| s.focus(window, cx)),
        }
    }

    fn go(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.focus_first(window, cx);
        cx.notify();
    }

    /// Add your email account (the welcome's button).
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go(Step::Email, window, cx);
    }

    /// Back (Esc): a running check is cancelled first; from manual setup to
    /// the chip; from Sign in to the email; from the email out (true: the
    /// flow handled it; false: it closes).
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy() {
            self.cancel(cx);
            return true;
        }
        self.pending = false;
        match self.step {
            // From manual setup back to the provider (or the list).
            Step::SignIn
                if self.manual && (self.chip != Chip::None || self.offline || self.bridge) =>
            {
                self.manual = false;
                self.bridge = false;
                self.failure = None;
                self.provider = match self.chip {
                    Chip::Preset(i) | Chip::Hosted(i)
                        if PRESETS.get(i).is_some_and(|p| p.is_fixed()) =>
                    {
                        Some(i)
                    }
                    _ => None,
                };
                self.choosing = self.provider.is_none() && !matches!(self.chip, Chip::Found(_));
                self.focus_first(window, cx);
                cx.notify();
                true
            }
            // The list opened over a provider: closed.
            Step::SignIn
                if self.choosing
                    && (self.provider.is_some() || matches!(self.chip, Chip::Found(_))) =>
            {
                self.choosing = false;
                self.focus_first(window, cx);
                cx.notify();
                true
            }
            Step::SignIn => {
                self.reset();
                self.manual = false;
                self.provider = None;
                self.user_open = false;
                // From Outlook: back to the list, the walk ended.
                let to = if self.importing.take().is_some() {
                    self.queue.clear();
                    self.mark_existing();
                    Step::Import
                } else {
                    Step::Email
                };
                self.go(to, window, cx);
                true
            }
            Step::Email | Step::Import if self.first_run => {
                self.go(Step::Welcome, window, cx);
                true
            }
            Step::Yours | Step::Done => true,
            Step::Email | Step::Import | Step::Welcome => {
                cx.emit(OnboardingEvent::Closed);
                false
            }
        }
    }

    /// Cancel a running check: its answer is dropped, the form stays.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.check.cancel();
        self.slow = 0;
        self.slow_task = None;
        cx.notify();
    }

    /// Back to the email step, or the flow closed: a lookup or check still
    /// running answers into nothing; the form's certificates and errors go.
    fn reset(&mut self) {
        self.check.cancel();
        self.find = None;
        self.find_from = None;
        self.found_last = None;
        self.find_note.clear();
        self.find_stale = false;
        self.slow = 0;
        self.slow_task = None;
        self.failure = None;
        self.details = false;
        self.unreadable = None;
        self.reading = None;
        self.pins = FormPins::default();
        self.chip = Chip::None;
        self.offline = false;
        self.choosing = false;
        self.bridge = false;
        self.missing = Missing::default();
    }

    /// A provider picked on the list.
    pub fn pick_provider(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.take_preset(index, false, window, cx);
        self.focus_first(window, cx);
        cx.notify();
    }

    /// Use preset `index` (by the address's domain, its MX records when
    /// `hosted`, or the list): a fixed one is the provider; Proton Mail
    /// Bridge fills manual setup with its local servers; one Komorebi
    /// cannot sign in to yet leaves the list open with why.
    fn take_preset(
        &mut self,
        index: usize,
        hosted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = PRESETS.get(index) else {
            return;
        };
        self.missing.provider = false;
        self.failure = None;
        match p.sign_in {
            SignIn::NotSupported => {
                self.provider = None;
                self.manual = false;
                self.bridge = false;
                self.chip = Chip::Unsupported(index);
                self.choosing = true;
            }
            SignIn::Bridge => {
                let fields = Fields {
                    imap_host: p.imap.0.into(),
                    imap_port: p.imap.1.to_string(),
                    imap_starttls: p.imap.2 == Security::StartTls,
                    smtp_host: p.smtp.0.into(),
                    smtp_port: p.smtp.1.to_string(),
                    smtp_starttls: p.smtp.2 == Security::StartTls,
                    ..Fields::default()
                };
                self.servers.update(cx, |s, cx| s.set(&fields, window, cx));
                if matches!(
                    self.chip,
                    Chip::Preset(_) | Chip::Hosted(_) | Chip::Found(_)
                ) {
                    self.chip = Chip::None;
                }
                self.provider = None;
                self.manual = true;
                self.bridge = true;
                self.choosing = false;
            }
            _ => {
                self.provider = Some(index);
                self.manual = false;
                self.bridge = false;
                self.chip = if hosted {
                    Chip::Hosted(index)
                } else {
                    Chip::Preset(index)
                };
                self.choosing = false;
            }
        }
    }

    /// Continue on the email step: refuse an address that is not one or is
    /// already here; a preset's domain has its settings (no lookup);
    /// offline, the provider list with its note; otherwise the lookup starts
    /// (only because of this press) and Sign in shows at once.
    pub fn continue_email(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.email.read(cx).value().to_string();
        let address = typed.trim().to_string();
        if !discover::looks_valid(&address) {
            self.email_problem = Some(EmailProblem::Invalid);
            cx.notify();
            return;
        }
        let exists = self
            .db
            .accounts()
            .unwrap_or_default()
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&address));
        if exists {
            self.email_problem = Some(EmailProblem::Exists);
            cx.notify();
            return;
        }
        self.email_problem = None;
        self.reset();
        self.domain = discover::parse_email(&address).map_or_else(String::new, |(_, d)| d);
        self.picker.update(cx, |p, cx| p.reset(window, cx));
        let svc = services::get(cx);
        if let Some(preset) = presets::guess(&address) {
            self.take_preset(preset, false, window, cx);
        } else if svc.offline {
            self.provider = None;
            self.manual = false;
            self.offline = true;
            self.chip = Chip::None;
            self.choosing = true;
        } else {
            self.provider = None;
            self.manual = false;
            self.chip = Chip::Finding;
            self.find_settings(address, window, cx);
        }
        self.go(Step::SignIn, window, cx);
    }

    fn find_settings(&mut self, email: String, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.servers.read(cx).fields(cx);
        self.find_from = Some(from.clone());
        let lookup = services::get(cx).lookup;
        let work = cx.background_executor().spawn({
            let email = email.clone();
            async move { lookup(&email) }
        });
        let handle = window.window_handle();
        self.find = Some(cx.spawn(async move |this, cx| {
            let out = work.await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.finish_find(&email, from, out, window, cx)
                });
            });
        }));
    }

    /// The lookup answered: fill the fields it found, unless they were
    /// edited meanwhile; the chip; then a Sign in that waited for it.
    pub fn finish_find(
        &mut self,
        email: &str,
        from: Fields,
        out: Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find = None;
        if self.chip == Chip::Finding {
            if let Some(i) = out.preset {
                let hosted = out.sources.contains(&discover::Source::Mx);
                self.take_preset(i, hosted, window, cx);
            } else {
                self.chip = flow::chip_of(&out);
                self.choosing = self.chip == Chip::NotFound;
            }
        }
        let address = discover::parse_email(email)
            .map(|(l, d)| format!("{l}@{d}"))
            .unwrap_or_default();
        let merged = merge(&from, &out.found, &address, self.found_last.as_ref());
        if merged.to != from {
            self.found_last = Some(merged.to.clone());
        }
        let now = self.servers.read(cx).fields(cx);
        if now == from {
            self.find_stale = false;
            let to = merged.to.clone();
            self.servers.update(cx, |s, cx| s.set(&to, window, cx));
        } else {
            self.find_stale = true;
        }
        self.find_note = flow::note(&out, merged.kept);
        if self.pending {
            self.pending = false;
            if matches!(self.chip, Chip::Found(_)) || self.provider.is_some() {
                self.submit(window, cx);
            }
        }
        cx.notify();
    }

    /// Set up manually (the list's Add provider, a failure's next step).
    pub fn go_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.manual = true;
        self.bridge = false;
        self.provider = None;
        self.missing.provider = false;
        self.failure = None;
        self.focus_first(window, cx);
        cx.notify();
    }

    fn form(&self, cx: &App) -> AccountForm {
        let f = self.servers.read(cx).fields(cx);
        let found = matches!(self.chip, Chip::Found(_)) && self.provider.is_none();
        let provider = if self.manual || found {
            Some(Provider::Other)
        } else {
            self.provider.map(Provider::Preset)
        };
        let user = self.user.read(cx).value().to_string();
        let preset = matches!(provider, Some(Provider::Preset(_)));
        AccountForm {
            id: 0,
            provider,
            name: String::new(),
            email: self.email.read(cx).value().trim().to_string(),
            password: {
                let typed = self.password.read(cx).value().to_string();
                // Google's 16 letters come as four groups with spaces.
                if self.app_password_help().is_some_and(|h| h.always()) {
                    presets::clean_app_password(&typed)
                } else {
                    typed
                }
            },
            imap_host: f.imap_host,
            imap_port: f.imap_port,
            imap_starttls: f.imap_starttls,
            smtp_host: f.smtp_host,
            smtp_port: f.smtp_port,
            smtp_starttls: f.smtp_starttls,
            save_sent: true,
            signature: String::new(),
            imap_user: if preset { user.clone() } else { f.imap_user },
            smtp_user: if preset { user } else { f.smtp_user },
            smtp_own_login: !preset && f.smtp_own_login,
            smtp_no_auth: self.manual && self.servers.read(cx).no_auth(),
        }
    }

    /// Find settings on manual setup: the lookup again, for the address
    /// typed, filling only what is unchanged when it answers.
    pub fn find_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() || self.busy() {
            return;
        }
        let email = self.email.read(cx).value().trim().to_string();
        if !discover::looks_valid(&email) {
            return;
        }
        if services::get(cx).offline {
            self.find_note = tr!("You're offline, so we can't look up settings. Enter them below.");
            cx.notify();
            return;
        }
        self.find_stale = false;
        self.find_settings(email, window, cx);
        cx.notify();
    }

    /// Forget certificate (a manual server's line).
    pub fn forget_certificate(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.pins.forget(kind);
        cx.notify();
    }

    /// The certificates pinned in the form.
    pub fn pins(&self) -> &FormPins {
        &self.pins
    }

    /// Sign in: the missing fields marked, else the check (after the
    /// lookup, if it still runs).
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.step != Step::SignIn {
            return;
        }
        let form = self.form(cx);
        self.missing = Missing {
            password: form.password.is_empty(),
            provider: !self.manual
                && self.provider.is_none()
                && !matches!(self.chip, Chip::Found(_) | Chip::Finding),
            imap: self.manual && form.imap_host.trim().is_empty(),
            smtp: self.manual && form.smtp_host.trim().is_empty(),
        };
        if self.missing.any() {
            cx.notify();
            return;
        }
        if !self.manual && self.chip == Chip::Finding {
            self.pending = true;
            cx.notify();
            return;
        }
        let new = match form.new_account() {
            Ok(n) => n,
            Err(e) => {
                self.error = Some(text::form_error(e));
                cx.notify();
                return;
            }
        };
        self.error = None;
        self.failure = None;
        self.details = false;
        self.unreadable = None;
        let svc = services::get(cx);
        let pins = self.pins.blobs();
        let req = Request {
            servers: (new.imap.clone(), new.smtp.clone()),
            pins: pins.clone(),
            manual: form.preset().is_none(),
            logins: (
                if new.imap_user.is_empty() {
                    new.email.clone()
                } else {
                    new.imap_user.clone()
                },
                if new.smtp_user.is_empty() {
                    new.email.clone()
                } else {
                    new.smtp_user.clone()
                },
            ),
            smtp_sign_in: !new.smtp_no_auth,
            password: Some(form.password.clone()),
            saved: 0,
            verify: svc.verify,
            read_password: svc.read_password,
        };
        let password = form.password.clone();
        let task = check::start(req, cx, move |this: &mut Self, msg, cx| match msg {
            check::Msg::Progress(done) => {
                this.check.progress(done);
                cx.notify();
            }
            check::Msg::Done(result) => {
                this.check.finish(&result);
                this.slow = 0;
                this.slow_task = None;
                match result {
                    Err(f) => this.failure = Some(f),
                    Ok(()) => this.finish_add(&new, &password, pins.clone(), cx),
                }
                cx.notify();
            }
        });
        self.check.start(task);
        self.start_slow(cx);
        let _ = window;
        cx.notify();
    }

    fn start_slow(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().clone();
        self.slow = 0;
        self.slow_task = Some(cx.spawn(async move |this, cx| {
            timer.timer(Duration::from_secs(3)).await; // timer-exempt: the wait's reassurance after 3 s, as the Slint flow
            if this
                .update(cx, |this, cx| {
                    this.slow = 1;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            timer.timer(Duration::from_secs(7)).await; // timer-exempt: and after 10 s
            let _ = this.update(cx, |this, cx| {
                this.slow = 2;
                cx.notify();
            });
        }));
    }

    /// The check passed: the account, its password and pins, then Make it
    /// yours (the name started from the address) while its first download
    /// starts.
    fn finish_add(
        &mut self,
        new: &komorebi::db::NewAccount,
        password: &str,
        pins: account::Pins,
        cx: &mut Context<Self>,
    ) {
        let svc = services::get(cx);
        match account::add(
            &self.db,
            new,
            "",
            password,
            pins,
            &svc.store_password,
            &svc.forget_password,
        ) {
            Ok(id) => {
                self.added = Some((id, new.email.clone()));
                self.download = Download::None;
                self.has_mail = false;
                self.step = Step::Yours;
                self.scroll.set_offset(point(px(0.), px(0.)));
                // From Outlook: its sender name, and its default signatures
                // when they were imported.
                let outlook_name = self.importing.as_ref().map(|a| {
                    import::apply_defaults(&self.db, id, a, &self.imported);
                    a.display_name.trim().to_string()
                });
                // The name box is filled at the next render (it needs the
                // window).
                self.start_name = Some(
                    outlook_name
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| flow::name_from_address(&new.email)),
                );
                cx.emit(OnboardingEvent::Added(id));
            }
            Err(e) => self.error = Some(text::not_saved(&format!("{e:#}"))),
        }
    }

    /// Done (`save`) writes the name and the signature with one
    /// `update_account` (the signature becomes a named one, the account's
    /// default); Skip writes nothing. Then the finish.
    pub fn personalise(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if save
            && let Some((id, _)) = &self.added
            && let Some(old) = self
                .db
                .accounts()
                .unwrap_or_default()
                .into_iter()
                .find(|a| a.id == *id)
        {
            let name = self.name.read(cx).value().trim().to_string();
            let signature = self.signature.read(cx).value().to_string();
            let edited = Account {
                name,
                signature: signature.clone(),
                ..old
            };
            let saved = self.db.update_account(&edited).and_then(|_| {
                self.db
                    .adopt_signature_text(edited.id, &signature)
                    .map(drop)
            });
            if let Err(e) = saved {
                self.error = Some(text::not_saved(&format!("{e:#}")));
            }
        }
        // From Outlook: the next ticked account, then You're all set.
        if self.importing.is_some() && self.error.is_none() {
            self.next_import(window, cx);
            return;
        }
        self.go(Step::Done, window, cx);
    }

    /// Open your inbox.
    pub fn open_inbox(&mut self, cx: &mut Context<Self>) {
        cx.emit(OnboardingEvent::Finished(self.added()));
    }

    /// Add another account: the flow again from Your email.
    pub fn add_another(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset();
        self.importing = None;
        self.queue.clear();
        self.added = None;
        self.download = Download::None;
        self.has_mail = false;
        self.manual = false;
        self.provider = None;
        self.user_open = false;
        self.error = None;
        for s in [
            &self.email,
            &self.password,
            &self.user,
            &self.name,
            &self.signature,
        ] {
            s.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.servers
            .update(cx, |s, cx| s.set(&Fields::default(), window, cx));
        self.first_run = false;
        self.go(Step::Email, window, cx);
    }

    /// Review certificate (a manual server's untrusted one).
    pub fn review_certificate(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let svc = services::get(cx);
        let Ok((imap, smtp)) = self.form(cx).servers() else {
            return;
        };
        let server = match kind {
            Kind::Imap => imap,
            Kind::Smtp => smtp,
        };
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
                    this.reading = None;
                    match result {
                        Err(e) => this.unreadable = Some(e),
                        Ok((der, issuer)) => {
                            let review = CertReview {
                                kind,
                                server,
                                der,
                                issuer,
                            };
                            let weak = cx.entity().downgrade();
                            let chosen = review.clone();
                            trust::open(
                                &review,
                                None,
                                move |choice, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        let blob = chosen.blob(choice);
                                        this.pins.set(chosen.kind, blob, chosen.server.clone());
                                        this.failure = None;
                                        cx.notify();
                                    });
                                },
                                |_| {},
                                window,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }

    // ---- Import from Outlook ------------------------------------------

    /// What Outlook has on this PC (`None` while it is read).
    pub fn outlook(&self) -> Option<&Found> {
        self.outlook.as_ref()
    }

    /// The list's account rows.
    pub fn import_accounts(&self) -> &[import::AccountRow] {
        &self.import_accounts
    }

    /// The list's signature rows.
    pub fn import_signature_rows(&self) -> &[import::SignatureRow] {
        &self.import_signatures
    }

    /// The account from Outlook on Sign in: it, its place (from 1) and the
    /// count.
    pub fn importing_account(&self) -> Option<(&FoundAccount, usize, usize)> {
        let (place, total) = self.import_place;
        self.importing.as_ref().map(|a| (a, place, total))
    }

    /// Outlook's accounts and signatures (read, or made up for a snapshot
    /// or a test): the list's rows, addresses already here unticked.
    pub fn set_outlook(&mut self, found: Found, cx: &mut Context<Self>) {
        let existing: Vec<String> = self
            .db
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.email)
            .collect();
        let (accounts, signatures) = import::rows(&found, &existing);
        self.import_accounts = accounts;
        self.import_signatures = signatures;
        self.outlook = Some(found);
        cx.notify();
    }

    /// Mark the accounts added since the list was made (unticked).
    fn mark_existing(&mut self) {
        let existing: Vec<String> = self
            .db
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.email)
            .collect();
        for r in &mut self.import_accounts {
            if komorebi::import::outlook::already_added(
                &r.account.email,
                existing.iter().map(String::as_str),
            ) {
                r.exists = true;
                r.ticked = false;
            }
        }
    }

    /// Import from Outlook (the welcome's button).
    pub fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go(Step::Import, window, cx);
    }

    /// Tick or untick a found account (one already here stays unticked).
    pub fn tick_account(&mut self, ix: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.import_accounts.get_mut(ix)
            && !row.exists
        {
            row.ticked = on;
            cx.notify();
        }
    }

    /// Tick or untick a found signature.
    pub fn tick_signature(&mut self, ix: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.import_signatures.get_mut(ix) {
            row.ticked = on;
            cx.notify();
        }
    }

    /// Whether anything is ticked (Continue is enabled).
    pub fn import_ticked(&self) -> bool {
        self.import_accounts.iter().any(|r| r.ticked)
            || self.import_signatures.iter().any(|r| r.ticked)
    }

    /// Continue on the list: the ticked signatures are saved, then each
    /// ticked account goes through Sign in, filled in.
    pub fn continue_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Signatures still being read: this Continue is already under way.
        if !self.import_ticked() || self.signatures_task.is_some() {
            return;
        }
        let sigs: Vec<_> = self
            .import_signatures
            .iter()
            .filter(|r| r.ticked)
            .map(|r| r.signature.clone())
            .collect();
        self.queue = self
            .import_accounts
            .iter()
            .filter(|r| r.ticked)
            .map(|r| r.account.clone())
            .collect();
        self.import_place = (0, self.queue.len());
        if sigs.is_empty() {
            self.next_import(window, cx);
            return;
        }
        // Saved once: a second Continue does not save them again.
        for r in &mut self.import_signatures {
            r.ticked = false;
        }
        // Their picture files are read and decoded off the UI thread; then
        // they are saved, and the accounts' sign-ins start (an account's
        // defaults need them).
        let read = cx
            .background_executor()
            .spawn(async move { import::read_signatures(sigs) });
        let handle = window.window_handle();
        self.signatures_task = Some(cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.signatures_task = None;
                    let saved = import::save_signatures(&this.db, read);
                    this.imported.extend(saved);
                    this.next_import(window, cx);
                });
            });
        }));
        cx.notify();
    }

    /// The next ticked account, or the end: You're all set when one was
    /// added, else Your email (first run) or out (Settings).
    fn next_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(next) = self.queue.pop_front() else {
            self.importing = None;
            if self.added.is_some() {
                self.go(Step::Done, window, cx);
            } else if self.first_run {
                self.reset();
                self.go(Step::Email, window, cx);
            } else {
                cx.emit(OnboardingEvent::Closed);
            }
            return;
        };
        self.import_place.0 += 1;
        self.start_import_account(&next, window, cx);
        self.importing = Some(next);
    }

    /// Sign in for an account from Outlook: its address, its provider or
    /// its servers, its sign-in name.
    fn start_import_account(
        &mut self,
        a: &FoundAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `added` stays: the last account added is the one You're all set
        // names when the rest are skipped.
        self.reset();
        self.error = None;
        self.email_problem = None;
        self.manual = false;
        self.provider = None;
        self.user_open = false;
        for s in [&self.password, &self.user, &self.name, &self.signature] {
            s.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let email = a.email.clone();
        self.email
            .update(cx, |s, cx| s.set_value(email.clone(), window, cx));
        self.servers
            .update(cx, |s, cx| s.set(&Fields::default(), window, cx));
        self.domain = discover::parse_email(&email).map_or_else(String::new, |(_, d)| d);
        self.picker.update(cx, |p, cx| p.reset(window, cx));
        match import::prefill(a) {
            import::Prefill::Preset(i) => {
                self.take_preset(i, false, window, cx);
                if let Some(user) = import::preset_user(a) {
                    self.user.update(cx, |s, cx| s.set_value(user, window, cx));
                    self.user_open = true;
                }
            }
            import::Prefill::Manual(fields) => {
                self.servers.update(cx, |s, cx| s.set(&fields, window, cx));
                self.manual = true;
            }
            import::Prefill::Lookup => {
                if services::get(cx).offline {
                    self.offline = true;
                    self.choosing = true;
                } else {
                    self.chip = Chip::Finding;
                    self.find_settings(email, window, cx);
                }
            }
        }
        self.go(Step::SignIn, window, cx);
    }

    /// Skip this account (Sign in for an account from Outlook).
    pub fn skip_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.importing.is_none() {
            return;
        }
        self.check.cancel();
        self.next_import(window, cx);
    }

    fn pick_language(&mut self, setting: LangSetting, cx: &mut Context<Self>) {
        crate::i18n::set_language(resolve_here(setting), cx);
        let _ = self.db.set_setting("language", setting.key());
        cx.notify();
    }

    // ---- drawing ------------------------------------------------------

    fn render_welcome(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let (x, bottom, fall_w, fall_h, fall_right, fall_top) = if compact {
            (
                geo::WELCOME_X_COMPACT,
                geo::WELCOME_BOTTOM_COMPACT,
                geo::FALL_W_COMPACT,
                geo::FALL_H_COMPACT,
                geo::FALL_RIGHT_COMPACT,
                geo::FALL_TOP_COMPACT,
            )
        } else {
            (
                geo::WELCOME_X,
                geo::WELCOME_BOTTOM,
                geo::FALL_W,
                geo::FALL_H,
                geo::FALL_RIGHT,
                geo::FALL_TOP,
            )
        };
        let fall_color = c.fall;
        let fall = canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window, _| paint_fall(bounds, fall_color, window),
        )
        .absolute()
        .right(px(fall_right))
        .top(px(fall_top))
        .w(px(fall_w))
        .h(px(fall_h));
        let lang = crate::views::settings::general::LANGUAGES;
        let current = self.db.settings().unwrap_or_default().language;
        let language = Dropdown::new(
            "welcome-language",
            tr!("settings language" => "Language"),
            current.key(),
        )
        .choices(
            lang.iter()
                .map(|l| (l.key().to_string(), language_label(*l))),
        )
        .on_pick(cx.listener(|this, value: &SharedString, _, cx| {
            if let Some(l) = LANGUAGES.iter().find(|l| l.key() == value.as_ref()) {
                this.pick_language(*l, cx);
            }
        }));
        let link =
            |id: &'static str, label: String, name: String, page: Page, cx: &mut Context<Self>| {
                Button::new(id)
                    .label(label)
                    .name(name)
                    .variant(Variant::Subtle)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let url = match page {
                            Page::Guide => format!(
                                "{}#add-an-account",
                                crate::views::settings::about::REPOSITORY
                            ),
                            p => p.url(),
                        };
                        cx.emit(OnboardingEvent::OpenLink(url));
                    }))
            };
        div()
            .id("welcome")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(c.paper)
            .child(fall)
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .bottom(px(bottom))
                    .w(px(geo::WELCOME_W))
                    .flex()
                    .flex_col()
                    .child(
                        svg()
                            .path(Icon::Seal.path())
                            .size(px(geo::WELCOME_SEAL))
                            .mb(px(geo::WELCOME_SEAL_GAP))
                            .text_color(c.ink),
                    )
                    .child(
                        div()
                            .id("welcome-title")
                            .role(Role::Heading)
                            .aria_level(1)
                            .typo(&t.text.display)
                            .text_color(c.ink)
                            .child(tr!("Your mail, calm and close at hand")),
                    )
                    .child(
                        div()
                            .mt(px(geo::LEDE_GAP))
                            .max_w(px(geo::LEDE_W))
                            .text_size(px(geo::LEDE_SIZE))
                            .line_height(px(geo::LEDE_LINE))
                            .text_color(c.ink_2)
                            .child(tr!("A small, fast email app for Windows. Your messages stay on this PC and on your mail server.")),
                    )
                    .child(self.render_values(cx))
                    .child(
                        div()
                            .mt(px(geo::CTA_GAP))
                            .flex()
                            .flex_wrap()
                            .gap(px(space::SP_2))
                            .child(
                                Button::new("welcome-start")
                                    .label(tr!("Add your email account"))
                                    .variant(Variant::Primary)
                                    .height(Height::Large)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.start(window, cx)),
                                    ),
                            )
                            // Only when Outlook has something to offer.
                            .when(self.outlook.as_ref().is_some_and(|f| !f.is_empty()), |el| {
                                el.child(
                                    Button::new("welcome-import")
                                        .label(tr!("Import from Outlook"))
                                        // The new Outlook keeps nothing on
                                        // this PC: only the classic one.
                                        .tooltip(tr!(
                                            "Your accounts and signatures from classic Outlook on this PC."
                                        ))
                                        .height(Height::Large)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_import(window, cx)
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(geo::FINE_GAP))
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(tr!("It takes about a minute.")),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .right(px(geo::FOOT_RIGHT))
                    .bottom(px(geo::FOOT_BOTTOM))
                    .flex()
                    .items_center()
                    .gap(px(geo::FOOT_GAP))
                    .child(link(
                        "welcome-help",
                        tr!("Need help?"),
                        tr!("Need help? How to add an account"),
                        Page::Guide,
                        cx,
                    ))
                    .child(link(
                        "welcome-privacy",
                        tr!("Privacy"),
                        tr!("Privacy statement"),
                        Page::Privacy,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(language)
                    .child(
                        IconButton::new("welcome-settings", Icon::Settings, tr!("Settings"))
                            .tooltip(tr!("Settings"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(OnboardingEvent::OpenSettings)
                            })),
                    ),
            )
    }

    /// The welcome's three value rows (the Slint welcome's): private, fast,
    /// works with your provider.
    fn render_values(&self, cx: &App) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let rows = [
            (
                Icon::Eye,
                tr!("Private by design"),
                tr!("No ads, no tracking, no AI reading your mail."),
            ),
            (
                Icon::Sync,
                tr!("Fast from the start"),
                tr!("Opens in a blink, works offline with what's downloaded."),
            ),
            (
                Icon::Mail,
                tr!("Works with your provider"),
                tr!("Gmail, iCloud, Fastmail, Postale.io and more, or your own server."),
            ),
        ];
        div()
            .id("welcome-values")
            .role(Role::List)
            .mt(px(geo::CTA_GAP))
            .flex()
            .flex_col()
            .gap(px(space::SP_3))
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(move |(ix, (icon, title, line))| {
                        div()
                            .id(SharedString::from(format!("welcome-value-{ix}")))
                            .role(Role::ListItem)
                            .aria_label(format!("{title}. {line}"))
                            .flex()
                            .items_start()
                            .gap(px(space::SP_3))
                            .child(
                                svg()
                                    .path(icon.path())
                                    .mt(px(space::SP_0_5))
                                    .size(px(layout::ICON))
                                    .flex_none()
                                    .text_color(c.ink_2),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .typo(&t.text.body_strong)
                                            .text_color(c.ink)
                                            .child(title),
                                    )
                                    .child(
                                        div().typo(&t.text.caption).text_color(c.ink_2).child(line),
                                    ),
                            )
                            .test_support()
                    }),
            )
            .test_support()
    }

    fn render_steps_indicator(&self, cx: &App) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let current = match self.step {
            Step::Welcome | Step::Import | Step::Email => 0,
            Step::SignIn => 1,
            _ => 2,
        };
        let labels = [tr!("Your email"), tr!("Sign in"), tr!("Make it yours")];
        let count = labels.len();
        div()
            .id("onboarding-steps")
            .role(Role::List)
            .aria_label(tr!("Steps to add an account"))
            .flex()
            .gap(px(space::SP_4))
            .children(labels.into_iter().enumerate().map(move |(ix, label)| {
                let on = ix == current;
                let past = ix < current;
                div()
                    .id(SharedString::from(format!("onboarding-step-{ix}")))
                    .role(Role::ListItem)
                    .aria_label(flow::step_name(&label, past, on))
                    .aria_selected(on)
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(count)
                    .flex()
                    .items_center()
                    .gap(px(geo::STEP_GAP))
                    .typo(if on {
                        &t.text.body_strong
                    } else {
                        &t.text.caption
                    })
                    .text_color(if on || past { c.ink } else { c.ink_3 })
                    .child(
                        div()
                            .size(px(geo::STEP_DOT))
                            .rounded(px(radius::FULL))
                            .when(on || past, |el| el.bg(c.ink))
                            .when(!on && !past, |el| {
                                el.border(px(layout::FOCUS_RING)).border_color(c.ink_3)
                            }),
                    )
                    .child(label)
                    .test_support()
            }))
            .test_support()
    }

    /// A line of the import list: its checkbox, then what it is, under the
    /// label (`ob-import-<kind>-<ix>`).
    fn import_row(
        id: SharedString,
        label: String,
        lines: Vec<String>,
        checked: bool,
        disabled: bool,
        on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> gpui::AnyElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let name = if lines.is_empty() {
            label.clone()
        } else {
            format!("{label}. {}", lines.join(" "))
        };
        div()
            .id(id.clone())
            .role(Role::ListItem)
            .aria_label(name)
            .py(px(space::SP_2))
            .flex()
            .flex_col()
            .gap(px(space::SP_0_5))
            .child(
                Checkbox::new(SharedString::from(format!("{id}-check")), label, checked)
                    .disabled(disabled)
                    .on_change(on_change),
            )
            .children(lines.into_iter().map(move |line| {
                div()
                    .pl(px(layout::CHECK + space::SP_3))
                    .typo(&t.text.caption)
                    .text_color(c.ink_2)
                    .child(line)
            }))
            .test_support()
            .into_any_element()
    }

    fn render_import(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let weak = cx.entity().downgrade();
        let heading = |id: &'static str, label: String| {
            div()
                .id(id)
                .role(Role::Heading)
                .aria_level(2)
                .pt(px(space::SP_2))
                .typo(&t.text.body_strong)
                .text_color(c.ink)
                .child(label)
        };
        let mut col = div().flex().flex_col().gap(px(geo::FIELD_GAP)).child(
            div()
                .id("onboarding-import-title")
                .role(Role::Heading)
                .aria_level(1)
                .typo(&t.text.title)
                .text_color(c.ink)
                .child(tr!("Import from Outlook")),
        );
        let found = self.outlook.as_ref();
        match found {
            None => {
                col = col.child(
                    div()
                        .id("onboarding-import-finding")
                        .role(Role::Status)
                        .typo(&t.text.body)
                        .text_color(c.ink_2)
                        .child(tr!("Looking for Outlook on this PC\u{a0}…")),
                );
            }
            Some(f) if f.is_empty() => {
                let line = if f.new_outlook_only {
                    tr!(
                        "The new Outlook keeps your accounts and signatures online, so there's nothing on this PC to import. Add your account by its address instead."
                    )
                } else {
                    tr!("No Outlook accounts or signatures were found on this PC.")
                };
                col = col.child(Notice::new(
                    "onboarding-import-none",
                    notice::Kind::Info,
                    line,
                ));
            }
            Some(_) => {
                col = col.child(
                    div()
                        .typo(&t.text.body)
                        .text_color(c.ink_2)
                        .child(tr!("Komorebi found these in Outlook on this PC. Your passwords and mail aren't copied: you type each password once, and your mail downloads again from the server.")),
                );
            }
        }
        if !self.import_accounts.is_empty() {
            let rows = self.import_accounts.iter().enumerate().map(|(ix, r)| {
                let a = &r.account;
                let mut lines = Vec::new();
                if !a.display_name.is_empty() {
                    lines.push(a.display_name.clone());
                }
                if r.exists {
                    lines.push(tr!("Already in Komorebi."));
                } else if import::unsupported(a) {
                    let name = match import::prefill(a) {
                        import::Prefill::Preset(i) => PRESETS.get(i).map_or("", |p| p.name),
                        _ => "",
                    };
                    lines.push(text::find_unsupported(name));
                } else if a.kind == FoundKind::Pop {
                    lines.push(tr!(
                        "A POP account in Outlook. Komorebi sets it up with IMAP."
                    ));
                } else if let Some(s) = &a.incoming {
                    lines.push(crate::text::host(&s.host));
                }
                let weak = weak.clone();
                Self::import_row(
                    SharedString::from(format!("ob-import-account-{ix}")),
                    a.email.clone(),
                    lines,
                    r.ticked,
                    r.exists,
                    move |on, _, cx| {
                        let _ = weak.update(cx, |v, cx| v.tick_account(ix, on, cx));
                    },
                    cx,
                )
                .into_any_element()
            });
            col = col
                .child(heading("onboarding-import-accounts", tr!("Accounts")))
                .child(
                    div()
                        .id("onboarding-import-account-list")
                        .role(Role::List)
                        .aria_label(tr!("Accounts in Outlook"))
                        .flex()
                        .flex_col()
                        .children(rows.collect::<Vec<_>>())
                        .test_support(),
                );
        }
        if !self.import_signatures.is_empty() {
            let rows = self.import_signatures.iter().enumerate().map(|(ix, r)| {
                let s = &r.signature;
                let mut lines = Vec::new();
                let first = komorebi::mail::html::html_to_text(&s.html)
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .to_string();
                if !first.is_empty() {
                    lines.push(first);
                }
                // Its pictures on this PC come with it; one on the web is
                // never fetched.
                let web = komorebi::import::outlook::web_pictures(&s.html);
                if web > 0 {
                    lines.push(komorebi_i18n::trn!(
                        "{n} picture from the web isn't imported.",
                        "{n} pictures from the web aren't imported.",
                        web
                    ));
                }
                let weak = weak.clone();
                Self::import_row(
                    SharedString::from(format!("ob-import-signature-{ix}")),
                    s.name.clone(),
                    lines,
                    r.ticked,
                    false,
                    move |on, _, cx| {
                        let _ = weak.update(cx, |v, cx| v.tick_signature(ix, on, cx));
                    },
                    cx,
                )
                .into_any_element()
            });
            col = col
                .child(heading("onboarding-import-signatures", tr!("Signatures")))
                .child(
                    div()
                        .id("onboarding-import-signature-list")
                        .role(Role::List)
                        .aria_label(tr!("Signatures in Outlook"))
                        .flex()
                        .flex_col()
                        .children(rows.collect::<Vec<_>>())
                        .test_support(),
                );
        }
        let empty = found.is_some_and(Found::is_empty);
        col.children(
            self.error
                .clone()
                .map(|e| Notice::new("onboarding-import-error", notice::Kind::Error, e)),
        )
        .child(
            div()
                .pt(px(space::SP_6))
                .flex()
                .justify_between()
                .gap(px(space::SP_2))
                .child(
                    Button::new("onboarding-import-back")
                        .label(tr!("Back"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.back(window, cx);
                        })),
                )
                .child(if empty {
                    Button::new("onboarding-import-add")
                        .label(tr!("Add your email account"))
                        .variant(Variant::Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                } else {
                    Button::new("onboarding-import-continue")
                        .label(tr!("Continue"))
                        .variant(Variant::Primary)
                        .disabled(!self.import_ticked())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.continue_import(window, cx)),
                        )
                }),
        )
    }

    fn render_email(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let problem = self.email_problem.map(|p| match p {
            EmailProblem::Invalid => text::err_email(),
            EmailProblem::Exists => tr!("This account is already in Komorebi."),
        });
        div()
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(
                div()
                    .id("onboarding-email-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .typo(&t.text.title)
                    .text_color(c.ink)
                    .child(tr!("What's your email address?")),
            )
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(tr!("We'll look up the right settings for you.")),
            )
            .child(labelled(
                tr!("Email address"),
                TextField::new(&self.email, tr!("Email address")),
                problem.map(SharedString::from),
                cx,
            ))
            .child(
                div().flex().justify_end().gap(px(space::SP_2)).child(
                    Button::new("onboarding-continue")
                        .label(tr!("Continue"))
                        .variant(Variant::Primary)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.continue_email(window, cx)),
                        ),
                ),
            )
    }

    fn render_chip(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.manual || self.chip == Chip::None {
            return None;
        }
        let t = cx.tokens().clone();
        let c = t.color;
        let (title, detail) = match &self.chip {
            Chip::Finding => (
                tr!("Looking up settings for {0}\u{a0}…", self.domain),
                String::new(),
            ),
            Chip::Preset(i) => PRESETS.get(*i).map_or_else(Default::default, |p| {
                (
                    p.name.to_string(),
                    tr!("Settings ready · {0}", providers::note(p.sign_in)),
                )
            }),
            Chip::Hosted(i) => PRESETS.get(*i).map_or_else(Default::default, |p| {
                (
                    tr!("Your mail is hosted by {0}", p.name),
                    tr!("Settings ready · {0}", providers::note(p.sign_in)),
                )
            }),
            Chip::Unsupported(i) => PRESETS.get(*i).map_or_else(Default::default, |p| {
                (
                    tr!("{0} isn't supported yet", p.name),
                    tr!("It needs Microsoft sign-in, which Komorebi doesn't have yet. Choose another provider below."),
                )
            }),
            Chip::Found(host) => (tr!("Settings found"), tr!("{0} · encrypted", host)),
            Chip::NotFound => (
                tr!("We couldn't find settings for {0}", self.domain),
                tr!("Choose your provider below, or add it yourself."),
            ),
            Chip::None => (String::new(), String::new()),
        };
        // Change opens the list (it is open already after a failed lookup).
        let can_change = self.chip != Chip::Finding && !self.busy() && !self.choosing;
        let icon = match self.chip {
            Chip::Finding => None,
            Chip::NotFound | Chip::Unsupported(_) => Some(Icon::Info),
            _ => Some(Icon::Checkmark),
        };
        Some(
            div()
                .id("onboarding-chip")
                .role(Role::Status)
                .aria_label(if detail.is_empty() {
                    title.clone()
                } else {
                    format!("{title}, {detail}")
                })
                .flex()
                .items_center()
                .gap(px(space::SP_3))
                .p(px(space::SP_3))
                .rounded(px(radius::CARD))
                .bg(c.inset)
                .child(
                    div()
                        .size(px(layout::ICON))
                        .flex_none()
                        .when_some(icon, |el, icon| {
                            el.child(
                                svg()
                                    .path(icon.path())
                                    .size(px(layout::ICON))
                                    .text_color(c.ink),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().typo(&t.text.body_strong).child(title))
                        .when(!detail.is_empty(), |el| {
                            el.child(
                                div()
                                    .typo(&t.text.caption)
                                    .text_color(c.ink_2)
                                    .child(detail),
                            )
                        }),
                )
                .when(can_change, |el| {
                    el.child(
                        Button::new("onboarding-chip-change")
                            .label(tr!("change the servers" => "Change"))
                            .name(tr!("Change provider"))
                            .variant(Variant::OnInset)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choosing = true;
                                this.picker.update(cx, |p, cx| p.reset(window, cx));
                                this.focus_first(window, cx);
                                cx.notify();
                            })),
                    )
                })
                .test_support(),
        )
    }

    /// The provider list (`providers::ProviderPicker`): after a failed
    /// lookup, offline, for a provider not supported yet, or on Change.
    fn render_providers(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.manual || !self.choosing {
            return None;
        }
        let note = self
            .missing
            .provider
            .then(|| SharedString::from(tr!("Choose your provider.")));
        Some(labelled(tr!("Provider"), self.picker.clone(), note, cx))
    }

    /// A manual server's certificate line, with Forget certificate.
    fn render_cert(&self, kind: Kind, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let id = match kind {
            Kind::Imap => "onboarding-forget-imap",
            Kind::Smtp => "onboarding-forget-smtp",
        };
        div()
            .flex()
            .items_center()
            .gap(px(space::SP_2))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(self.pins.line(kind)),
            )
            .child(
                Button::new(id)
                    .label(tr!("Forget certificate"))
                    .variant(Variant::Subtle)
                    .on_click(cx.listener(move |this, _, _, cx| this.forget_certificate(kind, cx))),
            )
    }

    fn render_wait(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        if !self.busy() {
            return None;
        }
        let t = cx.tokens().clone();
        let c = t.color;
        let states = self.check.steps().unwrap_or_default();
        let names = [
            tr!("Connecting securely"),
            tr!("Checking your password"),
            tr!("Getting ready to send"),
        ];
        let rows = names.into_iter().enumerate().map(move |(ix, name)| {
            let state = flow::stage(&states, ix);
            div()
                .id(SharedString::from(format!("onboarding-stage-{ix}")))
                .role(Role::ListItem)
                .aria_label(check::step_name(&name, state))
                .flex()
                .items_center()
                .gap(px(space::SP_2))
                .typo(&t.text.body)
                .text_color(if state == StepState::Waiting {
                    c.ink_2
                } else {
                    c.ink
                })
                .child(
                    div()
                        .size(px(geo::STEP_MARK))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .when(state == StepState::Passed, |el| {
                            el.child(
                                svg()
                                    .path(Icon::Checkmark.path())
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
                .child(name)
                .test_support()
        });
        let slow = match self.slow {
            1 => Some(tr!(
                "Still working on it\u{a0}… This can take a moment on some servers."
            )),
            2 => Some(tr!(
                "Taking longer than usual. You can go back and check the address."
            )),
            _ => None,
        };
        Some(
            div()
                .id("onboarding-wait")
                .role(Role::List)
                .pt(px(space::SP_6))
                .flex()
                .flex_col()
                .gap(px(space::SP_2))
                .children(rows)
                .when_some(slow, |el, s| {
                    el.child(div().typo(&t.text.caption).text_color(c.ink_2).child(s))
                })
                .test_support(),
        )
    }

    fn render_failure(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.busy() {
            return None;
        }
        let t = cx.tokens().clone();
        // A provider that wants an app password: a refused sign-in says so
        // (by the provider's flag, whatever the server's words were); the
        // button to make one is under the field.
        let mut app_password = None;
        let (title, message, detail, act) = if let Some(f) = &self.failure {
            let kind = check::error_kind(&f.text, f.review.is_some());
            let email = self.email.read(cx).value().trim().to_string();
            let long = check::failure_text(f, self.provider, &email);
            if kind == Kind3::Rejected {
                app_password = self.app_password_help();
            }
            let (title, message) = match kind {
                Kind3::Rejected => match &app_password {
                    Some(help) => crate::views::settings::app_password::refused(help),
                    None => (
                        tr!("That password didn't work"),
                        tr!(
                            "Check it and try again. If your provider uses two-step sign-in, use an app password."
                        ),
                    ),
                },
                Kind3::Unreachable => (
                    tr!("Can't reach your mail server"),
                    tr!("Check your internet connection, then try again."),
                ),
                Kind3::Certificate => (
                    tr!("This server's certificate isn't trusted"),
                    tr!("Review it before you continue."),
                ),
                Kind3::Other => (tr!("Couldn't sign in"), long.clone()),
            };
            let detail = (kind != Kind3::Other).then_some(long);
            (
                title,
                message,
                detail,
                flow::act(kind, self.user_open, f.review.is_some(), self.manual),
            )
        } else if let Some(e) = &self.unreadable {
            let text = match e {
                Unreadable::NoStartTls { host } => {
                    crate::text::say(&komorebi::mail::errors::Say::NoStartTls {
                        host: host.clone(),
                    })
                }
                Unreadable::Unread { address } => text::cert_unreadable(address),
            };
            (tr!("Couldn't sign in"), text, None, Act::None)
        } else if let Some(e) = &self.error {
            (tr!("Couldn't sign in"), e.clone(), None, Act::None)
        } else {
            return None;
        };
        let review = self.failure.as_ref().and_then(|f| f.review);
        let mut n = Notice::new(
            "onboarding-failure",
            notice::Kind::Error,
            format!("{title}. {message}"),
        );
        let button = match act {
            Act::None => None,
            Act::SetUsername => Some(("onboarding-set-username", tr!("Set username"))),
            Act::ReviewCertificate => Some(("onboarding-review", tr!("Review certificate"))),
            Act::TryAgain => Some(("onboarding-try-again", tr!("Try again"))),
            Act::SetUpManually => Some(("onboarding-manual", tr!("Set up manually"))),
        };
        // An app password's refusal: Create app password sits just above,
        // under the field, and the sign-in is the address (Set username
        // would not help), so the notice has no button of its own.
        if let Some((id, label)) = button.filter(|_| app_password.is_none()) {
            n = n.action(
                Button::new(id)
                    .label(label)
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(move |this, _, window, cx| match act {
                        Act::SetUsername => {
                            this.user_open = true;
                            if this.manual {
                                let servers = this.servers.clone();
                                servers.update(cx, |s, cx| s.focus_user(window, cx));
                            } else {
                                this.user.update(cx, |s, cx| s.focus(window, cx));
                            }
                            cx.notify();
                        }
                        Act::ReviewCertificate => {
                            if let Some(kind) = review {
                                this.review_certificate(kind, window, cx);
                            }
                        }
                        Act::TryAgain => this.submit(window, cx),
                        Act::SetUpManually => this.go_manual(window, cx),
                        Act::None => {}
                    })),
            );
        }
        let details = self.details;
        Some(
            div()
                .pt(px(space::SP_4))
                .flex()
                .flex_col()
                .gap(px(space::SP_2))
                .child(n)
                .when_some(detail, |el, detail| {
                    el.child(
                        div().flex().child(
                            Button::new("onboarding-details")
                                .label(if details {
                                    tr!("Hide details")
                                } else {
                                    tr!("Show details")
                                })
                                .variant(Variant::Subtle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details = !this.details;
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(details, |el| {
                        el.child(
                            div()
                                .typo(&t.text.caption)
                                .text_color(t.color.ink)
                                .child(detail),
                        )
                    })
                }),
        )
    }

    fn render_sign_in(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let email = self.email.read(cx).value().trim().to_string();
        let busy = self.busy();
        let title = if self.manual {
            tr!("Server settings")
        } else {
            tr!("Sign in to {0}", email)
        };
        let password_note = if self.missing.password {
            tr!("Enter your password.")
        } else {
            tr!("Saved in Windows Credential Manager, never in Komorebi's files.")
        };
        let mut col = div().flex().flex_col().gap(px(geo::FIELD_GAP)).child(
            div()
                .id("onboarding-sign-in-title")
                .role(Role::Heading)
                .aria_level(1)
                .typo(&t.text.title)
                .text_color(c.ink)
                .truncate()
                .child(title),
        );
        // An account from Outlook: where it is in the walk, and what Outlook
        // had that Komorebi does differently.
        let from_outlook = self.importing_account().map(|(a, place, total)| {
            let mut line = tr!("Account {0} of {1} from Outlook.", place, total);
            if a.kind == FoundKind::Pop {
                line.push(' ');
                line.push_str(&tr!(
                    "Outlook used POP for this account. Komorebi uses IMAP, so check the settings."
                ));
            } else if self.manual {
                line.push(' ');
                line.push_str(&tr!(
                    "The server settings are Outlook's. Type your password to sign in."
                ));
            }
            line
        });
        let unsupported = from_outlook.is_some() && matches!(self.chip, Chip::Unsupported(_));
        if let Some(line) = from_outlook.clone() {
            col = col.child(Notice::new(
                "onboarding-from-outlook",
                notice::Kind::Info,
                line,
            ));
        }
        if self.manual {
            col = col.child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(tr!("Your provider's help pages list these. If you run your own server, use its IMAP and SMTP details.")),
            );
            if self.bridge {
                col = col.child(Notice::new(
                    "onboarding-bridge",
                    notice::Kind::Info,
                    tr!("Proton Mail Bridge must be running on this PC. Sign in with the password Bridge shows, not your Proton password."),
                ));
            }
            if self.offline {
                col = col.child(Notice::new(
                    "onboarding-offline",
                    notice::Kind::Info,
                    tr!("You're offline, so we can't look up settings. Enter them below."),
                ));
            }
        } else if self.offline {
            col = col.child(Notice::new(
                "onboarding-offline",
                notice::Kind::Info,
                tr!("You're offline, so we can't look up settings. Choose your provider, or add it yourself."),
            ));
        }
        col = col
            .children(self.render_chip(cx))
            .children(self.render_providers(cx));
        if self.manual {
            col = col.child(self.servers.clone());
            if self.missing.imap {
                col = col.child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(c.ink)
                        .child(tr!("Enter the incoming (IMAP) server.")),
                );
            }
            if self.missing.smtp {
                col = col.child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(c.ink)
                        .child(tr!("Enter the sending (SMTP) server.")),
                );
            }
            for kind in [Kind::Imap, Kind::Smtp] {
                if self.pins.is_saved(kind) {
                    col = col.child(self.render_cert(kind, cx));
                }
            }
            let note = if self.find_stale {
                tr!("Not filled in: the form changed while Komorebi looked.")
            } else {
                self.find_note.clone()
            };
            let can_find = !self.finding() && !busy && discover::looks_valid(&email);
            col = col.child(find_row(
                "onboarding-find",
                self.finding(),
                can_find,
                note,
                cx.listener(|this, _, window, cx| this.find_again(window, cx)),
                cx,
            ));
        }
        // A provider Komorebi cannot sign in to yet: its explanation above,
        // no password to type.
        if !unsupported {
            match self.app_password_help() {
                // Gmail and the like: the field is the app password, a line
                // says why, and a button opens the page that makes one.
                Some(help) => {
                    use crate::views::settings::app_password;
                    let note = if self.missing.password && help.always() {
                        tr!("Enter your app password.")
                    } else if self.missing.password {
                        tr!("Enter your password.")
                    } else {
                        app_password::needs(&help)
                    };
                    col = col.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(geo::LABEL_GAP))
                            .child(labelled(
                                app_password::field_label(&help),
                                TextField::new(&self.password, app_password::field_label(&help)),
                                None,
                                cx,
                            ))
                            .child(app_password::line("onboarding", note, cx))
                            .child(div().pt(px(space::SP_1)).child(app_password::links(
                                "onboarding",
                                &help,
                                self.open_page(cx),
                            ))),
                    );
                }
                None => {
                    col = col.child(labelled(
                        tr!("Password"),
                        TextField::new(&self.password, tr!("Password")),
                        Some(password_note.into()),
                        cx,
                    ));
                }
            }
        }
        if !self.manual && self.user_open && !unsupported {
            col = col.child(labelled(
                tr!("Username"),
                TextField::new(&self.user, tr!("Username")),
                Some(tr!("The name you sign in with, if it isn't your email address.").into()),
                cx,
            ));
        }
        col.children(self.render_wait(cx))
            .children(self.render_failure(cx))
            .child(
                div()
                    .pt(px(space::SP_6))
                    .flex()
                    .justify_between()
                    .gap(px(space::SP_2))
                    .child(
                        Button::new("onboarding-back")
                            .label(tr!("Back"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.back(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SP_2))
                            .when(from_outlook.is_some(), |el| {
                                el.child(
                                    Button::new("onboarding-skip-account")
                                        .label(tr!("Skip this account"))
                                        .variant(if unsupported {
                                            Variant::Primary
                                        } else {
                                            Variant::Secondary
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.skip_import(window, cx)
                                        })),
                                )
                            })
                            .when(!unsupported, |el| {
                                el.child(
                                    Button::new("onboarding-sign-in")
                                        .label(tr!("Sign in"))
                                        .variant(Variant::Primary)
                                        .disabled(busy || self.pending)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit(window, cx)
                                        })),
                                )
                            }),
                    ),
            )
    }

    fn render_yours(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let address = self
            .added
            .as_ref()
            .map_or(String::new(), |(_, e)| e.clone());
        let name = self.name.read(cx).value().trim().to_string();
        let from = if name.is_empty() {
            address.clone()
        } else {
            format!("{name} <{address}>")
        };
        let sig = flow::first_line(&self.signature.read(cx).value());
        div()
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(
                div()
                    .id("onboarding-yours-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .typo(&t.text.title)
                    .text_color(c.ink)
                    .child(tr!("Make it yours")),
            )
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(tr!("How should your name appear when you send mail? You can change this later in Settings.")),
            )
            .child(labelled(tr!("Your name"), TextField::new(&self.name, tr!("Your name")), None, cx))
            .child(labelled(
                tr!("Signature (optional)"),
                TextField::new(&self.signature, tr!("Signature (optional)")),
                None,
                cx,
            ))
            .child(
                div()
                    .id("onboarding-from")
                    .p(px(space::SP_3))
                    .rounded(px(radius::CARD))
                    .bg(c.inset)
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_1))
                    .child(div().typo(&t.text.body).text_color(c.ink).truncate().child(tr!("From: {0}", from)))
                    .when(!sig.is_empty(), |el| {
                        el.child(div().typo(&t.text.caption).text_color(c.ink_2).truncate().child(sig))
                    }),
            )
            .children(self.error.clone().map(|e| Notice::new("onboarding-yours-error", notice::Kind::Error, e)))
            .child(
                div()
                    .pt(px(space::SP_6))
                    .flex()
                    .justify_end()
                    .gap(px(space::SP_2))
                    .child(
                        Button::new("onboarding-skip")
                            .label(tr!("Skip"))
                            .on_click(cx.listener(|this, _, window, cx| this.personalise(false, window, cx))),
                    )
                    .child(
                        Button::new("onboarding-done")
                            .label(tr!("Done"))
                            .variant(Variant::Primary)
                            .on_click(cx.listener(|this, _, window, cx| this.personalise(true, window, cx))),
                    ),
            )
    }

    fn render_done(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let address = self
            .added
            .as_ref()
            .map_or(String::new(), |(_, e)| e.clone());
        let lang = crate::i18n::lang(cx);
        let progress = match self.download {
            Download::Counting { done, total } => Some((
                tr!(
                    "Downloading your mail\u{a0}… {0} of {1}. Newest messages arrive first.",
                    komorebi::format::thousands(lang, done),
                    komorebi::format::thousands(lang, total)
                ),
                Some(done as f32 / total as f32),
            )),
            Download::Started => Some((tr!("Downloading your mail\u{a0}…"), None)),
            _ => None,
        };
        let track = c.inset;
        let bar = c.ink;
        div()
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(
                svg()
                    .path(Icon::Seal.path())
                    .size(px(geo::WELCOME_SEAL))
                    .text_color(c.ink),
            )
            .child(
                div()
                    .id("onboarding-done-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .typo(&t.text.title)
                    .text_color(c.ink)
                    .child(tr!("You're all set")),
            )
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(tr!("account" => "{0} is ready. Your mail keeps arriving in the background.", address)),
            )
            .when_some(progress, |el, (label, fraction)| {
                el.child(
                    div()
                        .id("onboarding-progress")
                        .role(Role::ProgressIndicator)
                        .aria_label(tr!("Downloading your mail"))
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_2))
                        .child(
                            div()
                                .w_full()
                                .h(px(space::SP_1))
                                .rounded(px(radius::FULL))
                                .bg(track)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded(px(radius::FULL))
                                        .bg(bar)
                                        .when_some(fraction, |el, f| el.w(gpui::relative(f.clamp(0., 1.))))
                                        .when(fraction.is_none(), |el| el.w(gpui::relative(0.))),
                                ),
                        )
                        .child(div().typo(&t.text.caption).text_color(c.ink_2).child(label))
                        .test_support(),
                )
            })
            .when(self.has_mail, |el| {
                el.child(div().typo(&t.text.body).text_color(c.ink).child(tr!("Your newest mail is here.")))
            })
            .child(
                div()
                    .pt(px(space::SP_6))
                    .flex()
                    .gap(px(space::SP_2))
                    .child(
                        Button::new("onboarding-open-inbox")
                            .label(tr!("Open your inbox"))
                            .variant(Variant::Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.open_inbox(cx))),
                    )
                    .child(
                        Button::new("onboarding-add-another")
                            .label(tr!("Add another account"))
                            .on_click(cx.listener(|this, _, window, cx| this.add_another(window, cx))),
                    ),
            )
    }
}

/// The mark's K, falling off the window's right edge (`.fall`): the stem,
/// the leaf and the leg of the seal's cut-out, in the fall colour.
fn paint_fall(bounds: Bounds<Pixels>, color: gpui::Hsla, window: &mut Window) {
    let scale = bounds.size.width.as_f32() / mark::VIEW_W;
    let p = |x: f32, y: f32| point(px(x), px(y));
    let r = |v: f32| p(v, v);
    let mut b = PathBuilder::fill();
    // The stem: a pill.
    let (x0, y0, x1, y1) = mark::STEM;
    let sr = mark::STEM_R;
    b.move_to(p(x0 + sr, y0));
    b.arc_to(r(sr), px(0.), false, true, p(x1, y0 + sr));
    b.line_to(p(x1, y1 - sr));
    b.arc_to(r(sr), px(0.), false, true, p(x0, y1 - sr));
    b.line_to(p(x0, y0 + sr));
    b.arc_to(r(sr), px(0.), false, true, p(x0 + sr, y0));
    b.close();
    // The leaf: two arcs between its tips.
    let (fx, fy) = mark::LEAF_FROM;
    let (tx, ty) = mark::LEAF_TO;
    b.move_to(p(fx, fy));
    b.arc_to(r(mark::LEAF_R), px(0.), false, true, p(tx, ty));
    b.arc_to(r(mark::LEAF_R), px(0.), false, true, p(fx, fy));
    b.close();
    // The leg: a pill along its diagonal.
    let [a, c, d, e] = mark::LEG;
    let lr = mark::LEG_R;
    b.move_to(p(a.0, a.1));
    b.line_to(p(c.0, c.1));
    b.arc_to(r(lr), px(0.), false, false, p(d.0, d.1));
    b.line_to(p(e.0, e.1));
    b.arc_to(r(lr), px(0.), false, false, p(a.0, a.1));
    b.close();
    b.translate(p(-mark::VIEW_X, -mark::VIEW_Y));
    b.scale(scale);
    b.translate(bounds.origin);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

impl Render for OnboardingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(name) = self.start_name.take() {
            self.name.update(cx, |s, cx| s.set_value(name, window, cx));
            self.focus_first(window, cx);
        }
        // Gmail and the like: the box says to paste the app password.
        let placeholder = if self.app_password_help().is_some_and(|h| h.always()) {
            crate::views::settings::app_password::placeholder()
        } else {
            tr!("Password or app password")
        };
        if self.password_placeholder != placeholder {
            self.password_placeholder = placeholder.clone();
            self.password
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let t = cx.tokens().clone();
        let compact = window.viewport_size().width < px(layout::COMPACT_BELOW_W);
        let root = div()
            .id("onboarding")
            .track_focus(&self.focus)
            .role(Role::Pane)
            .aria_label(tr!("Add account"))
            .size_full()
            .bg(t.color.paper)
            .text_color(t.color.ink)
            .typo(&t.text.body);
        if self.step == Step::Welcome {
            return root.child(self.render_welcome(compact, cx)).test_support();
        }
        let page = match self.step {
            Step::Import => self.render_import(cx).into_any_element(),
            Step::Email => self.render_email(cx).into_any_element(),
            Step::SignIn => self.render_sign_in(cx).into_any_element(),
            Step::Yours => self.render_yours(cx).into_any_element(),
            _ => self.render_done(cx).into_any_element(),
        };
        let indicator = (!matches!(self.step, Step::Done | Step::Import))
            .then(|| self.render_steps_indicator(cx));
        root.child(
            ScrollArea::new("onboarding-scroll", &self.scroll).child(
                div().w_full().flex().justify_center().child(
                    div()
                        .w_full()
                        .max_w(px(geo::FLOW_W))
                        .px(px(geo::PAGE_PAD_X_COMPACT))
                        .pt(px(geo::FLOW_TOP))
                        .pb(px(geo::FLOW_PAD))
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_6))
                        .children(indicator)
                        .child(page),
                ),
            ),
        )
        .test_support()
    }
}
