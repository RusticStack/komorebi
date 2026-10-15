//! UI controller: owns the main-thread state, feeds the Slint models and
//! routes user actions to the cache and the mail workers.

use crate::db::{self, Account, Db, Filter, Folder, MarkRead, NewAccount};
use crate::mail::mime::{self, Body};
use crate::mail::render::links::Mailto;
use crate::mail::smtp::{OutAttachment, Outgoing};
use crate::mail::{self, Attention, Class, Event, Job, Notify, Outcome, Problem, Workers};
use crate::providers::{self, Security, Server};
use crate::role::Role;
use crate::rows::{self, Rows, Selection};
use crate::tr::{self, t};
use crate::trust;
use crate::{
    AccountForm, AccountRow, AppWindow, AttachmentRow, BodyEdit, ComposeField, Discovery,
    FolderOption, FormatAction, MenuCommand, NavItem, ServerPorts, Severity, Suggestion, Theme,
    format, secrets,
};
use komorebi::tree::{self, Fold};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use triage::{UndoClock, UndoOffer};

const RELOAD_DEBOUNCE: Duration = Duration::from_millis(150);
/// Searching a scope larger than this waits for a pause in typing.
const SEARCH_DEBOUNCE_ABOVE: u32 = 5_000;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(100);
/// Longer bodies are cut for display (the text widget lays out all of it).
const MAX_BODY_BYTES: usize = 300_000;
/// The software renderer has no first-frame hook, so network work starts
/// after a short delay that lets the first frame paint.
const START_SYNC_AFTER: Duration = Duration::from_millis(250);
/// Raw messages up to this size are parsed on the UI thread, as before, so
/// a text message opens at once; larger ones are parsed on the render
/// worker.
const SYNC_PARSE_MAX: usize = 256 * 1024;
/// Fonts load on the render worker this long after start, off the launch path.
const WARM_RENDER_AFTER: Duration = Duration::from_secs(1);
/// After the window closes, how long actions already handed to the
/// workers (a held move sent at close) may take to reach the server.
const CLOSE_GRACE: Duration = Duration::from_secs(3);
/// The wake check runs this often; a wall-clock gap between two checks
/// above [`RESUME_GAP`] means the computer slept (or the clock jumped).
const RESUME_TICK: Duration = Duration::from_secs(60);
const RESUME_GAP: Duration = Duration::from_secs(120);
/// After a wake, the network gets this long to come back before the sync.
const RESUME_SETTLE: Duration = Duration::from_secs(3);

mod cert;
mod composer;
pub(crate) use composer::above_quote;
mod files;
mod find;
#[cfg(debug_assertions)]
mod flow;
mod invite;
mod notifications;
mod onboarding;
pub use onboarding::sample_onboard;
mod original;
mod outbox;
mod reader;
mod roles;
mod settings;
mod signatures;
mod tabs;
mod text;
#[cfg(test)]
mod text_calls;
mod tips;
mod triage;
mod updates;
mod windows;
mod wipe;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` against the app state. Never call this while a native modal
/// dialog is open (the dialog pumps events, which could re-enter).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => app.as_mut().map(f),
        Err(_) => None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    AllInboxes,
    Folder(i64),
    /// The saved local drafts.
    Drafts,
    /// Sent messages until their send settles.
    Outbox,
}

impl View {
    /// `selected-folder` in the window: the folder id, 0 All inboxes, -1
    /// the local drafts, -2 the Outbox.
    fn code(self) -> i32 {
        match self {
            View::AllInboxes => 0,
            View::Folder(id) => id as i32,
            View::Drafts => -1,
            View::Outbox => -2,
        }
    }

    /// The `last_view` setting.
    fn key(self) -> String {
        match self {
            View::AllInboxes => "all".into(),
            View::Folder(id) => format!("f{id}"),
            View::Drafts => "drafts".into(),
            View::Outbox => "outbox".into(),
        }
    }
}

/// The view a start opens on: the one saved in `last` (`last_view`) while
/// it still exists (a folder of an account still here, the Outbox while it
/// holds mail, All inboxes while there is more than one account), else the
/// Inbox: All inboxes, or with one account that account's Inbox.
fn start_view(last: &str, accounts: &[Account], folders: &[Folder], outbox: i64) -> View {
    let inbox = || match accounts {
        [one] => folders
            .iter()
            .find(|f| f.account_id == one.id && f.role == Role::Inbox)
            .map_or(View::AllInboxes, |f| View::Folder(f.id)),
        _ => View::AllInboxes,
    };
    match last {
        _ if accounts.is_empty() => View::AllInboxes,
        "drafts" => View::Drafts,
        "outbox" if outbox > 0 => View::Outbox,
        "all" if accounts.len() > 1 => View::AllInboxes,
        f => f
            .strip_prefix('f')
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|&id| {
                folders
                    .iter()
                    .any(|x| x.id == id && accounts.iter().any(|a| a.id == x.account_id))
            })
            .map_or_else(inbox, View::Folder),
    }
}

/// The folder pane menu's commands ([`App::nav_menu`]).
const NAV_REFRESH: i32 = 0;
const NAV_MARK_READ: i32 = 1;
const NAV_COLLAPSE: i32 = 3;
const NAV_SETTINGS: i32 = 4;
/// Go to the folder in the entry's `id` (a rail tile's Inbox, Sent, Archive).
const NAV_OPEN: i32 = 6;
/// Hide folders / Show folders: fold the full pane to the rail (from
/// 1040 px; the window runs it, `toggle-pane`).
const NAV_FOLD: i32 = 7;
/// The list's "..." menu: Mark all as read.
const LIST_MARK_READ: i32 = 1;
const LIST_SELECT: i32 = 2;

/// A role's name for a folder the account doesn't have (a greyed entry).
fn role_label(role: Role) -> String {
    match role {
        Role::Sent => t!(invoke_role_sent),
        Role::Archive => t!(invoke_role_archive),
        _ => t!(invoke_role_inbox),
    }
}

/// The action button on the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StatusAction {
    #[default]
    None,
    /// Open this account's settings to update its password.
    EditAccount(i64),
    /// Let this account's workers try again (they waited for the user).
    RetryAccount(i64),
    /// This account's saved certificate changed: open its page and read
    /// the new one into the Trust this certificate? prompt.
    ReviewCertificate(i64),
    /// This account's server can't be reached: one attempt now.
    TryNow(i64),
    /// Open the changelog (the What's new bar, after an update).
    WhatsNew,
}

/// An account's last sync, for its line in Settings (this session only).
#[derive(Clone, Debug, PartialEq, Eq)]
enum AccountState {
    Syncing,
    /// Unix seconds of the last sync that went through.
    Checked(i64),
    /// The server could not be reached.
    Unreachable,
    Failed(Problem),
}

/// What the in-app prompt (`q-*`) asks before acting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Question {
    /// Send an unconfirmed message again.
    Resend(i64),
    DiscardOutbox(i64),
    /// Remove an account that still has drafts or outbox rows.
    RemoveAccount(i64),
    /// Answer the invitation in this message (with a reply, or with
    /// "Don't send a reply", only here).
    Rsvp(i64, crate::mail::ical::Answer),
    /// Forget an account's downloaded mail and download it again.
    Redownload(i64),
    /// Delete every account's password and all local data, then close.
    RemoveAllData,
    /// Trust the certificate just read from a server (`App::cert_review`).
    TrustCert,
    /// Delete the messages in `App::delete_wait` from Trash (for good on
    /// the server) or, `trash` false, from Junk (to Trash).
    DeleteFrom {
        trash: bool,
    },
    /// Delete a signature (Settings > Signatures).
    DeleteSignature(i64),
}

impl Question {
    /// It loses something for good (or may send twice): its prompt has no
    /// default button and starts on Cancel (docs/WINUI3.md "ContentDialog").
    fn destructive(self) -> bool {
        matches!(
            self,
            Question::Resend(_)
                | Question::DiscardOutbox(_)
                | Question::RemoveAccount(_)
                | Question::RemoveAllData
                | Question::TrustCert
                | Question::DeleteFrom { .. }
                | Question::DeleteSignature(_)
        )
    }
}

struct Open {
    msg: db::Message,
    body: Option<Body>,
}

/// The opened message waiting to be marked read: one at a time, so a
/// message passed on the way (arrowing through the list) stays unread.
#[derive(Debug, Default)]
struct ReadWait(Option<i64>);

impl ReadWait {
    /// Message `id` opened: how long until it is marked read (zero: at
    /// once), or `None` when it isn't (already read, or `Never`).
    fn opened(&mut self, id: i64, unread: bool, mode: MarkRead) -> Option<Duration> {
        self.0 = None;
        match mode {
            _ if !unread => None,
            MarkRead::Now => Some(Duration::ZERO),
            MarkRead::After(delay) => {
                self.0 = Some(id);
                Some(delay)
            }
            MarkRead::Never => None,
        }
    }

    fn cancel(&mut self) {
        self.0 = None;
    }

    /// The delay ran out with `open` showing: the message to mark read,
    /// if it is still the one that waited.
    fn due(&mut self, open: Option<i64>) -> Option<i64> {
        self.0.take().filter(|&id| Some(id) == open)
    }
}

#[derive(Clone, Copy)]
enum Flag {
    Seen,
    Flagged,
}

impl Flag {
    fn name(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
        }
    }

    fn of(self, m: &db::Brief) -> bool {
        match self {
            Flag::Seen => m.seen,
            Flag::Flagged => m.flagged,
        }
    }
}

/// Where Archive, Delete and Move send messages.
#[derive(Clone, Copy)]
enum Dest {
    /// The account's Archive folder.
    Archive,
    /// The account's Trash folder; from Trash itself, deleted.
    Trash,
    Folder(i64),
    /// The account's Junk folder.
    Junk,
    /// From Junk back to the Inbox, marked read.
    NotJunk,
}

/// A move, delete or junk report in its undo window: the messages are
/// gone from the list but still in the cache, and their server jobs wait.
struct Held {
    seq: u64,
    ids: HashSet<i64>,
    jobs: Vec<(i64, Job)>,
    /// Unread messages among them, by folder (left out of the counts).
    unread: HashMap<i64, i64>,
    /// All of them, by folder (left out of the server Drafts' total).
    total: HashMap<i64, i64>,
    /// The message that was open, reopened by Undo.
    open: Option<i64>,
    /// The message tabs that closed with it (their places), reopened by
    /// Undo; and the tab it was done from, shown again.
    tabs: Vec<(usize, i64)>,
    from_tab: Option<i64>,
    /// Message windows closed with it (Undo does not open them again, and
    /// says so).
    windows: bool,
}

struct App {
    ui: Weak<AppWindow>,
    db: Rc<Db>,
    db_path: PathBuf,
    notify: Notify,
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    workers: HashMap<i64, Workers>,
    /// The folder pane's folded accounts and parent folders (the
    /// `nav_folded` setting).
    folded: HashSet<Fold>,
    /// The accounts whose folders have had their default folds (the
    /// `nav_folds_init` setting).
    folds_init: HashSet<i64>,
    /// The default folds are applied in memory but not yet written: the
    /// two writes wait until after the first frame (a first launch).
    folds_unsaved: bool,
    /// The next pane rebuild puts the keyboard row on the pill (the view
    /// changed, or the pane is new).
    nav_follow: bool,
    view: View,
    search: String,
    /// The Unread / Flagged toggle under the search box.
    filter: Filter,
    /// The "All folders" search scope (it applies while searching).
    search_all: bool,
    nav: Rc<VecModel<NavItem>>,
    /// The narrow window's rail: the unified items and one tile per
    /// account (its Inbox).
    rail: Rc<VecModel<NavItem>>,
    rows: Rc<Rows>,
    open: Option<Open>,
    /// When an opened message is marked read (`mark_read_delay_ms`).
    mark_read: MarkRead,
    read_wait: ReadWait,
    read_timer: Timer,
    syncing: HashSet<i64>,
    /// Accounts whose interactive worker was signed in ahead of use.
    warmed: HashSet<i64>,
    /// Each account's last sync, shown in Settings.
    account_state: HashMap<i64, AccountState>,
    /// The problem last shown for each account ([`Problem::key`]): the same
    /// one is not shown again until a sync goes through (or Refresh).
    shown_problem: HashMap<i64, (Class, String)>,
    /// Accounts whose server can't be reached, with the line saying so: it
    /// stays (Caution) until the account syncs again or the bar is closed,
    /// and comes back when a later status line goes.
    standing: Vec<(i64, Problem)>,
    /// Header downloads in progress (done, total), by account.
    progress: HashMap<i64, (usize, usize)>,
    /// The status line shows a download's progress (any other status
    /// replaces it; it never replaces another status).
    progress_shown: Cell<bool>,
    /// Accounts in their first download (just added): the empty list shows
    /// its progress.
    first_sync: HashSet<i64>,
    /// The account the onboarding flow added, while the flow shows its
    /// first download, and the last progress seen (folder, done).
    ob_account: Option<i64>,
    ob_progress: Option<(i64, usize)>,
    /// A just-added account and the view at that moment: its Inbox opens
    /// as soon as its folders arrive, unless the view changed meanwhile.
    land_on: Option<(i64, View)>,
    /// The form whose check runs: the preset chosen (`None`: Other server
    /// or an edited account) and the address, for what a rejected password
    /// says.
    check_form: (Option<usize>, String),
    /// The wake check's last run, by the wall clock.
    last_tick: SystemTime,
    resume_timer: Timer,
    drafts: composer::Drafts,
    /// Saved drafts (counted after the first paint).
    draft_count: i64,
    autosave_timer: Timer,
    /// The latest composer edit, for the autosave.
    last_edit: Instant,
    /// When each draft row's last copy to the server was asked for.
    server_saves: HashMap<i64, Instant>,
    /// Sends in their undo window, by outbox row.
    pending_sends: HashMap<i64, composer::PendingSend>,
    /// Sends started (tells a send from a later one of the same row).
    send_seq: u64,
    /// The composer's Preview renders: the latest one wins; the width and
    /// theme it is wanted at, and the timer that waits out a resize.
    preview_seq: u64,
    preview_want: (f32, bool),
    preview_timer: Timer,
    /// The one-time caption about the formatting marks was shown.
    format_hint_seen: bool,
    /// The formatted editors' pictures (`composer::rich`).
    ed: composer::rich::EdUi,
    /// New messages start formatted, in this font, size and colour
    /// (settings `compose_*`).
    compose_rich: bool,
    compose_defaults: (
        komorebi::mail::rich::FontId,
        komorebi::mail::rich::SizePt,
        Option<komorebi::mail::rich::ColorId>,
    ),
    /// The signatures, by name (Settings > Signatures, the composer's
    /// Signature menu and the account page's boxes).
    sig_list: Vec<db::SignatureInfo>,
    /// The signature open in Settings (`None`: a new one), its logo, and
    /// whether it changed since it opened.
    sig_open: Option<i64>,
    sig_logo: Option<komorebi::mail::rich::Logo>,
    sig_dirty: bool,
    /// How long Send waits so it can be undone (setting `undo_send_secs`;
    /// 0 sends at once).
    undo_delay: Duration,
    /// Builds a sent message off the UI thread (tests: by hand).
    builder: Rc<dyn Fn(composer::BuildJob)>,
    /// Deletes an account's saved password (tests: a fake store; under
    /// `KOMOREBI_OFFLINE` nothing, as the store is never touched there).
    forget_password: Rc<dyn Fn(i64) -> anyhow::Result<()>>,
    /// Saves a new account's password (tests: a fake store, as for
    /// `forget_password`).
    store_password: Rc<PasswordStore>,
    /// A new account's workers start (not under `KOMOREBI_OFFLINE`, nor in
    /// tests).
    workers_on: bool,
    /// Checks an account's servers and sign-ins step by step (tests: a stub).
    verifier: Verifier,
    /// Counts the account checks; an answer from an older one is dropped.
    check_seq: u64,
    /// The certificates pinned in the open account form.
    form_pins: cert::FormPins,
    /// Counts the "Find settings" lookups; an answer from an older one, or
    /// from before the form closed, is dropped.
    find_seq: u64,
    /// The server fields the last lookup filled in the open form.
    found_last: Option<find::Fields>,
    /// The folders offered for the roles of the open account form, and the
    /// picks.
    role_edit: roles::RoleEdit,
    /// A certificate read from a server, while its prompt is open.
    cert_review: Option<cert::CertReview>,
    /// "Remove all data" was confirmed: the files go once the window has
    /// closed ([`remove_data_if_asked`]).
    remove_data: bool,
    /// Rows in the outbox (counted after the first paint).
    outbox_count: i64,
    /// The outbox row shown in the reader pane.
    outbox_open: Option<i64>,
    status_action: Cell<StatusAction>,
    question: Option<Question>,
    /// "Change" was pressed on the open invite: its answer buttons show
    /// again.
    invite_change: bool,
    /// An action waiting for the open message's body to download.
    raw_wait: Option<(i64, original::RawUse)>,
    /// The last move or delete, while it can be undone.
    held: Option<Held>,
    held_seq: u64,
    /// What the Undo slot at the bottom of the list offers.
    undo: Cell<UndoOffer>,
    /// The held move's undo window, and the timer that ends it.
    undo_clock: UndoClock,
    undo_timer: Timer,
    /// Counts an undo-send offer down ("Sending in 9 s").
    count_timer: Timer,
    /// The messages a confirmed Trash / Junk delete applies to.
    delete_wait: Vec<i64>,
    /// "Select messages": the rows show check boxes.
    select_mode: bool,
    /// The Move flyout's filter, and the folders used last (setting
    /// `move_recent`, the last first).
    move_filter: String,
    move_recent: Vec<i64>,
    /// Messages opened in the Unread filter: they stay listed (read now)
    /// until the view or the filter changes, as in Outlook.
    kept_unread: HashSet<i64>,
    /// Files dropped on the window, attached together once the drop ends.
    dropped: Vec<PathBuf>,
    reader: reader::Reader,
    started_sync: bool,
    /// The app theme (setting `theme`).
    theme: db::ThemeChoice,
    /// Windows' appearance settings, as last shown.
    system: komorebi::theme::SystemTheme,
    /// Time between new-mail checks (setting `check_interval_secs`). A
    /// steady-state pass is one LIST-STATUS command (~0.1 s, ~1 KB), so it
    /// can run often; some servers (Postale) never push new mail over IDLE,
    /// which makes this the new-mail latency there.
    check_interval: Duration,
    /// When every folder was last synced in full (start, Refresh, hourly).
    last_full_sync: Option<Instant>,
    /// Flash the taskbar button on new mail in the background (setting
    /// `flash_taskbar`).
    flash: bool,
    /// New-mail notifications, the tray icon and background mode.
    notes: notifications::Notes,
    /// Settings > About > Updates and its bar (`updates.rs`).
    updates: updates::Updates,
    /// When the last sync pass ended (unix seconds), for "Updated 14:05".
    last_updated: Option<i64>,
    /// The language choice in Settings (setting `language`) and the
    /// language it gives on this computer.
    language: komorebi::i18n::LangSetting,
    lang: komorebi::i18n::Lang,
    /// The window has the keyboard focus.
    focused: bool,
    /// The inboxes' unread total, as the title shows it.
    inbox_unread: Option<i32>,
    /// The window's place while neither maximized nor minimized (saved on
    /// close).
    normal_geometry: Option<db::Geometry>,
    reload_timer: Timer,
    search_timer: Timer,
    sync_timer: Timer,
    status_timer: Timer,
    /// The teaching tips and the What's new bar (`tips.rs`).
    onboarding: komorebi::onboard::Onboarding,
    /// The list has shown rows with an account: the tips' delay ran.
    tips_armed: bool,
    /// The tips and the bar start by themselves (not in snapshot runs,
    /// whose actions ask for them, nor in tests).
    tips_auto: bool,
    tip_timer: Timer,
    /// The main window's tabs (`tabs.rs`) and the timer that saves them.
    tabs: tabs::Tabs,
    tabs_timer: Timer,
    /// Open the saved tabs again at start (setting `reopen_tabs`).
    reopen_tabs: bool,
    /// The read-only message windows (`windows.rs`).
    popouts: windows::PopOuts,
    /// Composer windows (`windows::ComposerWin`), one draft each.
    cwins: Vec<windows::ComposerWin>,
    /// The draft whose composer window an event is running for
    /// ([`App::in_composer`]).
    scoped: Option<composer::DraftId>,
    /// Outbox rows sent from a composer window: Undo reopens them in one.
    sent_from_window: HashSet<i64>,
    /// A draft started now opens in a tab of its own whatever shows (a
    /// reply from a message window).
    draft_tab: bool,
}

/// Set up the controller on `ui` over the cache at `db_path`. Returns where
/// the window was when it last closed, if that was saved.
/// The cache opened off the UI thread (`main` starts it beside the window).
pub type OpeningDb = std::thread::JoinHandle<anyhow::Result<(Db, Option<PathBuf>)>>;

#[cfg(test)]
pub fn install(
    ui: &AppWindow,
    db_path: PathBuf,
    mark: &dyn Fn(&str),
) -> anyhow::Result<Option<db::Geometry>> {
    install_with(ui, db_path, mark, None)
}

/// [`install`] with the cache possibly already opening on another thread; a
/// lost thread opens it here, and its error is the same one.
pub fn install_with(
    ui: &AppWindow,
    db_path: PathBuf,
    mark: &dyn Fn(&str),
    opening: Option<OpeningDb>,
) -> anyhow::Result<Option<db::Geometry>> {
    tr::register(ui);
    let theme = ui.global::<Theme>();
    theme.on_readable(settings::readable);
    ui.global::<ServerPorts>()
        .on_port_for_security(|imap, old, new, port| {
            let security = |v: i32| {
                if v == 1 {
                    Security::StartTls
                } else {
                    Security::Tls
                }
            };
            settings::port_for_security(imap, security(old), security(new), &port).into()
        });
    let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts\";
    if std::path::Path::new(&(fonts.clone() + "SegoeIcons.ttf")).exists() {
        theme.set_icon_font("Segoe Fluent Icons".into());
    }
    if std::path::Path::new(&(fonts + "SegUIVar.ttf")).exists() {
        theme.set_text_font("Segoe UI Variable Text".into());
        theme.set_display_font("Segoe UI Variable Display".into());
    }

    let open_dir = files::open_dir(&db_path);
    let started = std::time::SystemTime::now();
    let (db, damaged) = match opening.and_then(|h| h.join().ok()) {
        Some(opened) => opened?,
        None => Db::open_ui(&db_path)?,
    };
    let db = Rc::new(db);
    let settings = db.settings()?;
    // The language, before anything is shown (the window exists, so the
    // catalogs do). Never left to Slint's own pick from the system locale:
    // it would show Portugal's text to a Brazilian Windows.
    let language = komorebi::i18n::dev_setting().unwrap_or(settings.language);
    let wanted = komorebi::i18n::resolve_here(language);
    let lang = if komorebi::i18n::select(wanted) {
        wanted
    } else {
        komorebi::i18n::Lang::En
    };
    if settings.diagnostics_log {
        komorebi::diag::set(Some(komorebi::diag::log_path(&db_path)));
    }
    // Windows' appearance settings, before the window first shows.
    let system = komorebi::theme::read();
    settings::apply_system_theme(ui, &system, None, settings.theme);
    // A send cut off by the last exit may have gone out: never resent.
    db.recover_outbox(mail::INTERRUPTED)?;
    mark("database open");
    let notify: Notify = Arc::new(|event: Event| {
        on_ui(Box::new(move |app| app.on_event(event)));
    });

    let render_notify: reader::Notify = Arc::new(|reply: reader::Reply| {
        on_ui(Box::new(move |app| app.on_render(reply)));
    });
    // Remote images, only those the user chose to load (refused under
    // KOMOREBI_OFFLINE).
    let fetch: reader::Fetch = Arc::new(|url: &str| komorebi::fetch::get(url).ok());
    let reader = reader::Reader::new(
        ui,
        Box::new(reader::Worker::new(render_notify, fetch)),
        Box::new(reader::SystemShell),
    );
    reader.on_mailto(|m| {
        with_app(|a| a.compose_mailto(m));
    });

    let nav = Rc::new(VecModel::default());
    let rail = Rc::new(VecModel::default());
    let rows = Rc::new(Rows::new(db.clone()));
    rows.set_lang(lang);
    ui.set_nav_items(ModelRc::from(nav.clone()));
    ui.set_rail_items(ModelRc::from(rail.clone()));
    ui.set_rows(ModelRc::from(rows.clone()));

    let update_last = settings.update_last_check.clone();
    let mut app = App {
        ui: ui.as_weak(),
        db,
        db_path,
        notify,
        accounts: Vec::new(),
        folders: Vec::new(),
        workers: HashMap::new(),
        folded: tree::parse_folds(&settings.nav_folded),
        folds_init: tree::parse_ids(&settings.nav_folds_init),
        folds_unsaved: false,
        nav_follow: true,
        view: View::AllInboxes,
        search: String::new(),
        filter: Filter::All,
        search_all: false,
        nav,
        rail,
        rows,
        open: None,
        mark_read: settings.mark_read,
        read_wait: ReadWait::default(),
        read_timer: Timer::default(),
        syncing: HashSet::new(),
        warmed: HashSet::new(),
        account_state: HashMap::new(),
        shown_problem: HashMap::new(),
        standing: Vec::new(),
        progress: HashMap::new(),
        progress_shown: Cell::new(false),
        first_sync: HashSet::new(),
        ob_account: None,
        ob_progress: None,
        land_on: None,
        check_form: (None, String::new()),
        last_tick: SystemTime::now(),
        resume_timer: Timer::default(),
        drafts: composer::Drafts::default(),
        draft_count: 0,
        autosave_timer: Timer::default(),
        last_edit: Instant::now(),
        server_saves: HashMap::new(),
        pending_sends: HashMap::new(),
        send_seq: 0,
        preview_seq: 0,
        preview_want: (0.0, false),
        preview_timer: Timer::default(),
        format_hint_seen: settings.format_hint_seen,
        ed: composer::rich::EdUi::default(),
        compose_rich: settings.compose_rich,
        compose_defaults: (
            settings.compose_font,
            settings.compose_size,
            settings.compose_color,
        ),
        sig_list: Vec::new(),
        sig_open: None,
        sig_logo: None,
        sig_dirty: false,
        undo_delay: Duration::from_secs(settings.undo_send_secs),
        builder: Rc::new(composer::spawn_build),
        // KOMOREBI_OFFLINE never touches the credential store.
        forget_password: if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            Rc::new(|_| Ok(()))
        } else {
            Rc::new(secrets::delete_password)
        },
        store_password: Rc::new(secrets::set_password),
        workers_on: komorebi::dev_var("KOMOREBI_OFFLINE").is_none(),
        verifier: mail::verify_account,
        check_seq: 0,
        form_pins: cert::FormPins::default(),
        find_seq: 0,
        found_last: None,
        role_edit: roles::RoleEdit::default(),
        cert_review: None,
        remove_data: false,
        outbox_count: 0,
        outbox_open: None,
        status_action: Cell::default(),
        question: None,
        invite_change: false,
        raw_wait: None,
        held: None,
        held_seq: 0,
        undo: Cell::default(),
        undo_clock: UndoClock::default(),
        undo_timer: Timer::default(),
        count_timer: Timer::default(),
        delete_wait: Vec::new(),
        select_mode: false,
        move_filter: String::new(),
        move_recent: triage::parse_recent(&settings.move_recent),
        kept_unread: HashSet::new(),
        dropped: Vec::new(),
        reader,
        started_sync: false,
        theme: settings.theme,
        system,
        check_interval: Duration::from_secs(settings.check_interval_secs),
        last_full_sync: None,
        flash: settings.flash_taskbar,
        notes: notifications::Notes::from_settings(&settings),
        updates: updates::Updates::new(settings.update_auto.as_deref()),
        last_updated: None,
        language,
        lang,
        focused: true,
        inbox_unread: None,
        normal_geometry: None,
        reload_timer: Timer::default(),
        search_timer: Timer::default(),
        sync_timer: Timer::default(),
        status_timer: Timer::default(),
        onboarding: komorebi::onboard::Onboarding::default(),
        tips_armed: false,
        tips_auto: komorebi::dev_var("KOMOREBI_SNAPSHOT").is_none(),
        tip_timer: Timer::default(),
        tabs: tabs::Tabs::default(),
        tabs_timer: Timer::default(),
        reopen_tabs: settings.reopen_tabs,
        popouts: windows::PopOuts::new(settings.popout_size),
        cwins: Vec::new(),
        scoped: None,
        sent_from_window: HashSet::new(),
        draft_tab: false,
    };
    // The view the window last showed (a folder still there), else the
    // Inbox; with the drafts and outbox counted, so the pane is whole in
    // the first frame.
    app.draft_count = app.db.count_drafts()?;
    app.outbox_count = app.db.count_outbox()?;
    app.view = start_view(
        &settings.last_view,
        &app.db.accounts()?,
        &app.db.folders()?,
        app.outbox_count,
    );
    ui.set_selected_folder(app.view.code());
    ui.set_nav_collapsed(settings.nav_collapsed);
    app.load_accounts()?;
    // The welcome's Language box shows the choice before Settings opens.
    ui.set_g_language(app.language.index());
    app.onboarding = tips::onboarding(&settings, !app.accounts.is_empty());
    // KOMOREBI_OFFLINE keeps the app off the network and away from the
    // credential store (demos, timing runs).
    if komorebi::dev_var("KOMOREBI_OFFLINE").is_none() {
        app.start_workers();
    }
    app.reload_rows();
    ui.set_about_version(env!("CARGO_PKG_VERSION").into());
    ui.set_about_data(
        std::path::absolute(&app.db_path)
            .unwrap_or_else(|_| app.db_path.clone())
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    mark("model ready");
    wire(ui);
    app.show_compose_defaults();
    app.show_updates();
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    if let Some(aside) = damaged {
        let name = aside.file_name().unwrap_or_default().to_string_lossy();
        let text = t!(invoke_cache_damaged, name.as_ref().into());
        // After the launch notices, so this one stays up.
        Timer::single_shot(Duration::ZERO, move || notice(&text));
    }
    #[cfg(debug_assertions)]
    if let Some(dir) = komorebi::dev_var("KOMOREBI_FLOW") {
        flow::start(dir.into());
    }

    Timer::single_shot(START_SYNC_AFTER, start_sync);
    updates::schedule_auto(update_last);
    Timer::single_shot(tips::WHATS_NEW_AFTER, || {
        with_app(|a| a.whats_new_due_now());
    });
    Timer::single_shot(START_SYNC_AFTER, || {
        with_app(|a| a.count_drafts_and_outbox());
    });
    // The tabs of the last session, after the first frame: only the
    // setting was read before it (titles in one query now).
    let saved_tabs = if settings.reopen_tabs {
        settings.tabs.clone()
    } else {
        String::new()
    };
    if !saved_tabs.is_empty() {
        Timer::single_shot(START_SYNC_AFTER, move || {
            with_app(|a| a.restore_tabs(&saved_tabs));
        });
    }
    Timer::single_shot(START_SYNC_AFTER, || {
        with_app(|a| a.start_notifications());
    });
    Timer::single_shot(WARM_RENDER_AFTER, move || {
        with_app(|a| a.reader.warm());
        // What an earlier run opened goes, off the launch path (what this
        // run opened meanwhile stays).
        let clear = move || files::clear_open_dir(&open_dir, started);
        background("clear-open", Box::new(clear)).ok();
    });
    Ok(settings.window)
}

/// After the window closed, if "Remove all data" was confirmed: close the
/// cache and remove the files, saying on screen what could not be removed.
/// Returns whether it ran (the close-time actions are then skipped: they
/// would write to the cache).
pub fn remove_data_if_asked(ui: AppWindow) -> bool {
    // Said in the language shown, made while the window (and with it the
    // catalog) exists: the list of files is only known once it is gone.
    const LIST: &str = "\u{1}";
    let note = t!(invoke_wipe_files_left, LIST.into());
    let Some(path) = close_for_removal(ui) else {
        return false;
    };
    let left = wipe::remove_files(&path);
    if !left.is_empty() {
        let list: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Komorebi")
            .set_description(note.replace(LIST, &list.join("\n")))
            .show();
    }
    true
}

/// If "Remove all data" was confirmed: drop the app state and the window
/// (its list keeps the old model until it next draws), which closes the
/// cache: the app's connection and the list's. Returns the cache's path.
fn close_for_removal(ui: AppWindow) -> Option<PathBuf> {
    let path = with_app(|a| a.remove_data.then(|| a.db_path.clone())).flatten()?;
    APP.with(|cell| cell.borrow_mut().take());
    ui.hide().ok();
    drop(ui);
    Some(path)
}

/// After the window closed: wait a moment (at most [`CLOSE_GRACE`]) for
/// the actions handed to the workers, such as a move sent at close, to
/// reach the server. Then refresh the cache's planner statistics.
pub fn finish_actions() {
    let until = Instant::now() + CLOSE_GRACE;
    while Instant::now() < until
        && with_app(|a| a.workers.values().any(Workers::busy)).unwrap_or(false)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    with_app(|a| {
        // A quit inside the first-frame grace still keeps the default folds.
        a.flush_default_folds();
        a.db.optimize();
    });
}

/// Run `f` on a short-lived thread named `name`. Boxed, as is `on_ui`'s
/// argument, so callers share one copy of the spawn code.
fn background(name: &str, f: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
}

/// What checks an account's servers and sign-ins ([`mail::verify_account`]).
type Verifier = fn(
    &Server,
    &Server,
    mail::Pins,
    (&str, &str),
    &str,
    &mut dyn FnMut(mail::errors::Step),
) -> Result<(), mail::errors::StepError>;

/// A failed account check: the step that failed (none: before the first),
/// the sentence, and the server whose untrusted certificate can be reviewed.
struct CheckFailure {
    step: Option<usize>,
    text: CheckText,
    review: Option<mail::pin::Kind>,
}

/// What a failed check says.
enum CheckText {
    /// A failed step.
    Say(mail::Say),
    /// The saved password couldn't be read, so nothing was tried.
    NoSavedPassword,
}

/// Saves an account's password (`secrets::set_password`; tests: a fake).
type PasswordStore = dyn Fn(i64, &str) -> anyhow::Result<()>;

/// What runs on the UI thread once an account check passed.
type AfterCheck = Box<dyn FnOnce(&mut App) + Send>;

/// Run `f` against the app state on the UI thread, from another thread.
fn on_ui(f: Box<dyn FnOnce(&mut App) + Send>) {
    slint::invoke_from_event_loop(move || {
        with_app(f);
    })
    .ok();
}

/// A `mailto:` link from the command line (or a second launch): a prefilled
/// composer in the main window. Never sent.
pub fn open_mailto(m: Mailto) {
    with_app(|a| {
        a.show_main_window();
        a.compose_mailto(m);
    });
}

/// Windows asks the app to end (the installer's Restart Manager, or a
/// sign-out): really quit, saving first, even when it lives in the tray.
pub fn end_session() {
    with_app(|a| {
        if !a.notes.quitting {
            a.quit_now();
        }
    });
}

/// Show a status line that stays until the user dismisses it (or another
/// status replaces it).
pub fn notice(text: &str) {
    with_app(|a| {
        a.set_status(text, false);
        a.status_timer.stop();
    });
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=tips`, `tips:<tip>`): the tips of
/// this layout from the first, or at the tip named (`folders`, `search`,
/// `help`; nothing is marked seen). Returns false when none applies here.
pub fn sample_tips(tip: Option<&str>) -> bool {
    with_app(|a| match tip.map(komorebi::onboard::Tip::from_token) {
        None => a.start_tips().is_some(),
        Some(Some(tip)) => a.start_tips_at(tip).is_some(),
        Some(None) => false,
    })
    .unwrap_or(false)
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=update:<state>`): Settings > About
/// in a made-up update state (see `updates.rs`).
pub fn sample_update(state: &str) {
    with_app(|a| a.sample_update(state));
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=whats-new`): the What's new bar, if
/// this cache is due one (an account, and another version saved or none).
pub fn sample_whats_new() -> bool {
    with_app(|a| a.show_whats_new_if_due()).unwrap_or(false)
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=windows:n`): the list's first `n`
/// messages in message windows of their own (for measuring their memory).
pub fn open_windows(n: usize) {
    with_app(|a| {
        let ids: Vec<i64> = (0..n).filter_map(|i| a.rows.id_at(i)).collect();
        for id in ids {
            a.open_window(id);
        }
    });
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=composer-window`): the composer
/// showing moves to a composer window of its own, the size of the main
/// one; that window is returned for the snapshot.
pub fn sample_composer_window() -> Option<AppWindow> {
    with_app(|a| {
        let id = a.drafts.active_id()?;
        a.open_draft_window(id);
        let ui = a.composer_window(id)?;
        ui.window().set_size(a.ui().window().size());
        Some(ui)
    })
    .flatten()
}

/// Snapshot QA: made-up attachments in the composer showing.
/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=discard` and `guard`): open the
/// newest saved draft in the composer.
pub fn open_newest_draft() {
    with_app(|a| {
        if let Some(&row) = a.db.draft_ids("").unwrap_or_default().first() {
            a.open_saved(row);
        } else {
            // A cache with no saved draft (the seeded one): a made-up
            // message typed into a new one, as a user would.
            a.compose_new();
            for (field, text) in [
                (ComposeField::To, "Ana Silva <ana@example.org>"),
                (ComposeField::Subject, "Trip notes"),
                (
                    ComposeField::Body,
                    "Hi Ana,\n\nThe notes from the trip are below.",
                ),
            ] {
                a.set_draft_field(field, text);
            }
        }
    });
}

/// Snapshots (`outbox:permanent`, `outbox:transient`): a made-up message
/// whose send failed, written to the throwaway cache (offline only) and
/// opened in the Outbox. Nothing is sent: the row is failed already.
pub fn sample_outbox(kind: &str) {
    with_app(|a| {
        let Some(account) = a.accounts.first().map(|x| x.id) else {
            return;
        };
        let (state, error) = match kind {
            "transient" => (
                db::Outbox::Failed,
                "transient error (421): 4.7.0 smtp.example.org Service not available, try again later",
            ),
            _ => (
                db::Outbox::Failed,
                "permanent error (550): 5.1.1 <ana.silva@example.org>: Recipient address rejected: User unknown in virtual mailbox table",
            ),
        };
        // Made-up text in the language shown, as its user would write.
        let (subject, body) = match a.lang {
            komorebi::i18n::Lang::PtPt => (
                "Notas da viagem",
                "Olá Ana,\n\nAs **notas** da viagem estão abaixo, com as fotografias \
                 de Sintra.\nAté segunda-feira.\n\n-- \nSeed",
            ),
            komorebi::i18n::Lang::En => (
                "Trip notes",
                "Hi Ana,\n\nThe **notes** from the trip are below, with the photos \
                 from Sintra.\nSee you on Monday.\n\n-- \nSeed",
            ),
        };
        let row = db::DraftRow {
            account_id: account,
            to: "Ana Silva <ana.silva@example.org>".into(),
            subject: subject.into(),
            body: body.into(),
            ..Default::default()
        };
        let stored = a.db.insert_draft(&row).and_then(|id| {
            a.db.outbox_send(id)?;
            a.db.store_raw(id, b"(made up)", &AtomicBool::new(false))?;
            a.db.outbox_queue(id)?;
            a.db.claim_outbox(id)?;
            a.db.outbox_result(id, state, None, error)?;
            Ok(id)
        });
        match stored {
            Ok(id) => {
                a.refresh_draft_count();
                a.set_view(View::Outbox);
                a.open_outbox(id);
            }
            Err(e) => a.set_status(&text::detail(&e), true),
        }
    });
}

/// Snapshots (`drop-hover`): a file dragged over the window.
pub fn sample_file_hover() {
    with_app(|a| a.ui().set_file_hover(true));
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=menu:<name>`): print a menu's
/// entries to stderr, from the same lists the menus show: `list` (the
/// list's "..." menu), `pane` (the folder pane's row menu on All inboxes,
/// each account line and the first folder of each account), `rail` (each
/// rail tile's menu), `row` (the row menu on what is selected), `more`
/// (the reader's More menu) or `move` (the Move flyout). Each item shows
/// its accelerator text and check mark. Nothing is opened.
pub fn dump_menu(name: &str) {
    let line = |e: &MenuCommand| {
        let state = if e.enabled { "enabled" } else { "disabled" };
        let mark = match (e.checkable, e.checked) {
            (true, true) => "(x) ",
            (true, false) => "( ) ",
            _ => "",
        };
        match e.title.split_once('\t') {
            Some((title, keys)) => eprintln!("  {mark}{title} [{state}]  {keys}"),
            None => eprintln!("  {mark}{} [{state}]", e.title),
        }
    };
    let print = |title: &str, groups: &[&[MenuCommand]]| {
        eprintln!("menu {title}");
        let mut first = true;
        for group in groups.iter().filter(|g| !g.is_empty()) {
            if !first {
                eprintln!("  ----");
            }
            first = false;
            group.iter().for_each(line);
        }
    };
    with_app(|a| match name {
        // The account page's "…" (the first account's): Move up / down.
        "account" => {
            let ui = a.ui();
            let n = a.accounts.len();
            let entry = |title: String, enabled: bool| MenuCommand {
                title: title.into(),
                enabled,
                ..Default::default()
            };
            if n < 2 {
                eprintln!("menu account: not shown (one account)");
            } else {
                let at = usize::try_from(ui.get_a_edit_index()).unwrap_or(0);
                print(
                    "account",
                    &[&[
                        entry(t!(invoke_menu_move_up), at > 0),
                        entry(t!(invoke_menu_move_down), at + 1 < n),
                    ]],
                );
            }
        }
        "list" => {
            let title = format!("list: {}", a.ui().get_list_title());
            print(&title, &[&a.list_menu()]);
        }
        // The row menu and the More menu, groups between separators, with
        // the Move to submenu where it sits (and how many folders it has).
        "row" | "more" => {
            a.menu_opening();
            let moves = a.ui().get_move_targets().row_count();
            let state = if a.ui().get_move_possible() {
                "enabled"
            } else {
                "disabled"
            };
            let submenu = MenuCommand {
                title: format!("{} > ({moves} folders) [{state}]", t!(invoke_menu_move_to)).into(),
                enabled: true,
                ..Default::default()
            };
            let (title, groups): (String, Vec<Vec<MenuCommand>>) = if name == "row" {
                let m = a.row_menu();
                let [g1, g2, mut g3, g4, g5, g6] = m.groups;
                g3.push(submenu);
                (
                    format!(
                        "row: {} selected",
                        a.ui().get_selected_count().max(i32::from(a.open.is_some()))
                    ),
                    vec![g1, g2, g3, g4, g5, g6],
                )
            } else {
                if a.open.is_none() {
                    eprintln!("menu:more: no open message (add KOMOREBI_SNAPSHOT_SELECT=0)");
                }
                let m = a.more_menu();
                let [mut g1, g2, g3, g4, g5] = m.groups;
                g1.push(submenu);
                ("more".to_string(), vec![g1, g2, g3, g4, g5])
            };
            eprintln!("menu {title}");
            for (i, group) in groups.iter().enumerate() {
                if i > 0 && !group.is_empty() {
                    eprintln!("  ----");
                }
                for e in group {
                    if e.title.contains(" > (") {
                        eprintln!("  {}", e.title);
                    } else {
                        line(e);
                    }
                }
            }
        }
        // The Move flyout: recent folders first, then the tree by depth.
        "move" => {
            a.move_filter.clear();
            a.show_move_targets();
            let ui = a.ui();
            eprintln!(
                "menu move: {} [{}]",
                t!(invoke_menu_move_to),
                if ui.get_move_possible() {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            for f in ui.get_move_targets().iter() {
                let lead = if f.recent {
                    "* ".to_string()
                } else {
                    "  ".repeat(f.level as usize + 1)
                };
                eprintln!("  {lead}{}", f.label);
            }
        }
        // The search scope: its two words, the one in force checked.
        "scope" => {
            let ui = a.ui();
            let first = if a.view == View::AllInboxes {
                t!(invoke_scope_these_inboxes)
            } else {
                t!(invoke_scope_this_folder)
            };
            let enabled = a.lists_messages();
            let mark = |on: bool| if on { "x" } else { " " };
            eprintln!("menu scope: {}", ui.get_search_view());
            let state = if enabled { "enabled" } else { "disabled" };
            eprintln!("  ({}) {first} [{state}]", mark(!a.search_all));
            eprintln!(
                "  ({}) {} [{state}]",
                mark(a.search_all),
                t!(invoke_all_folders)
            );
        }
        "pane" | "rail" => {
            let rows: Vec<NavItem> = if name == "rail" {
                a.rail.iter().filter(|r| !r.initial.is_empty()).collect()
            } else {
                let mut accounts = HashSet::new();
                a.nav
                    .iter()
                    .filter(|r| match r.kind {
                        0 | 1 => true,
                        2 => accounts.insert(r.account),
                        _ => false,
                    })
                    .collect()
            };
            if rows.is_empty() {
                eprintln!("menu:{name}: no rows with a menu");
            }
            for row in rows {
                if let Some((main, extra)) = a.nav_menu(&row) {
                    print(&format!("{name}: {}", row.label), &[&main, &extra]);
                }
            }
        }
        _ => eprintln!("menu:{name}: unknown menu (list, pane, rail, row, more or move)"),
    });
}

/// Snapshots (`delete-confirm`): the Trash question on what is selected.
pub fn sample_delete_confirm() {
    with_app(|a| {
        a.delete_wait = a.targets().iter().map(|m| m.id).collect();
        a.ask(Question::DeleteFrom { trash: true });
    });
}

/// Snapshots (`not-downloaded`): the open message as if its body were not
/// downloaded and its server out of reach (nothing is written).
pub fn sample_not_downloaded() {
    with_app(|a| {
        if let Some(open) = a.open.as_mut() {
            open.msg.raw = None;
            open.body = None;
            a.reader.close();
            a.show_open();
            a.ui().set_r_unreachable(true);
        }
    });
}

pub fn sample_attachments() {
    with_app(|a| a.sample_attachments());
}

/// A formatted message for the snapshots (made up; nothing is sent): a
/// greeting, bold, italic, underlined and coloured words, a link, both
/// lists and a quote; with `table`, a 3 x 3 table too.
pub fn sample_rich(table: bool) {
    with_app(|a| a.sample_rich(table));
}

/// Settings > Signatures with two made-up signatures (written to the
/// throwaway cache, offline only); `edit`: the first one's page.
pub fn sample_signatures(edit: bool) {
    with_app(|a| a.sample_signatures(edit));
}

/// A file is dragged over the window (or the drag left it): the composer
/// shows where it goes.
pub fn file_hover(on: bool) {
    with_app(|a| a.ui().set_file_hover(on));
}

/// A file was dropped on the window. A drop of several files arrives one
/// file at a time; they are attached together right after.
pub fn file_dropped(path: PathBuf) {
    with_app(|a| {
        if a.dropped.is_empty() {
            Timer::single_shot(Duration::ZERO, || {
                with_app(|a| {
                    let paths = std::mem::take(&mut a.dropped);
                    a.attach_dropped(paths);
                });
            });
        }
        a.dropped.push(path);
    });
}

/// Start network work once the window is on screen.
fn start_sync() {
    with_app(|app| {
        if app.started_sync {
            return;
        }
        app.started_sync = true;
        app.sync_all(true);
        app.resume_outbox();
        app.start_sync_timer();
        app.last_tick = SystemTime::now();
        app.resume_timer
            .start(TimerMode::Repeated, RESUME_TICK, || {
                with_app(|a| a.wake_check());
            });
    });
}

/// Whether two wake checks at wall-clock times `last` and `now` are
/// further apart than the computer could be awake between them (a clock
/// set back is no wake).
fn woke(last: SystemTime, now: SystemTime) -> bool {
    now.duration_since(last).is_ok_and(|gap| gap > RESUME_GAP)
}

/// Snapshot QA: the first account's status as when its server can't be
/// reached.
pub fn sample_offline() {
    with_app(|a| {
        if let Some(id) = a.accounts.first().map(|acc| acc.id) {
            let e = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut));
            a.on_event(Event::Syncing {
                account_id: id,
                active: false,
                error: Some(Problem::of(&e.context("connecting to the server"))),
            });
        }
    });
}

/// Snapshot QA: the first account as when its saved certificate changed
/// (made up; nothing is sent): the status line with Review certificate and
/// the account's card in Settings.
pub fn sample_cert_changed() {
    with_app(|a| {
        let Some(acc) = a.accounts.first().cloned() else {
            return;
        };
        let problem = Problem::of(&mail::pin::PinMismatch(acc.imap.host.clone()).into());
        let short = t!(invoke_say_cert_changed_short, text::host(&acc.imap.host));
        a.set_status(&text::account_line(&acc.email, &short), true);
        a.set_status_action(problem_action(acc.id, Some(Attention::Secure), &problem));
        a.account_state
            .insert(acc.id, AccountState::Failed(problem));
        a.show_account_rows();
    });
}

/// Snapshot QA: a made-up account (ana@example.com, with an empty Inbox)
/// added to the throwaway cache as if just added, its first download at
/// 1,234 of 5,000. Nothing connects (snapshots run offline).
pub fn sample_fresh_inbox() {
    with_app(|a| {
        let server = |host: &str| Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        let new = NewAccount {
            email: "ana@example.com".into(),
            name: "Ana Silva".into(),
            imap: server("imap.example.com"),
            smtp: server("smtp.example.com"),
            save_sent: true,
            imap_user: String::new(),
            smtp_user: String::new(),
        };
        let Ok(id) = a.db.add_account(&new) else {
            return;
        };
        a.db.upsert_folder(id, "INBOX", "Inbox", 0, Role::Inbox)
            .ok();
        if a.load_accounts().is_err() {
            return;
        }
        a.first_sync.insert(id);
        a.syncing.insert(id);
        a.land_on = Some((id, a.view));
        a.land();
        let folder = a
            .folders
            .iter()
            .find(|f| f.account_id == id)
            .map_or(0, |f| f.id);
        a.on_event(Event::Progress {
            account_id: id,
            folder_id: folder,
            done: 1234,
            total: 5000,
        });
    });
}

/// The certificate the certificate snapshots show: the throwaway test
/// identity in debug builds (so the details are real), a placeholder in
/// release ones (the details line is left out).
fn sample_certificate() -> Vec<u8> {
    #[cfg(debug_assertions)]
    {
        native_tls::Certificate::from_pem(include_bytes!("../tests/fixtures/tls/test-cert.pem"))
            .and_then(|c| c.to_der())
            .unwrap_or_default()
    }
    #[cfg(not(debug_assertions))]
    {
        b"synthetic certificate".to_vec()
    }
}

/// Snapshot QA: the edit form of the first account in a certificate state
/// (made-up; nothing is sent): `failed` (the secure connection failed on an
/// untrusted certificate: the sentence and Review certificate), `prompt`
/// (the Trust this certificate? prompt), `again` (the prompt as when a
/// forgotten certificate is reviewed again) or `pinned` (a certificate saved
/// for each server).
pub fn sample_cert(state: &str) {
    use mail::errors::{Class, Step, describe};
    use mail::pin::Kind;
    with_app(|a| {
        let Some(acc) = a.accounts.first().cloned() else {
            return;
        };
        let der = sample_certificate();
        match state {
            "failed" => {
                let say = describe(Step::Secure, Class::TlsCert, &acc.imap.host, 993, "alice");
                a.show_check_steps(0, Some(Step::Secure.index()));
                a.ui().set_a_error(text::say(&say).into());
                a.show_cert_state(1);
            }
            "pinned" => {
                a.form_pins.set(Kind::Imap, der.clone(), acc.imap);
                a.form_pins.set(Kind::Smtp, der, acc.smtp);
                a.show_cert_state(0);
            }
            _ => {
                if state == "again" {
                    let old = b"the old certificate".to_vec();
                    a.form_pins.set(Kind::Imap, old, acc.imap.clone());
                    a.form_pins.forget(Kind::Imap);
                }
                a.check_seq += 1;
                let seq = a.check_seq;
                a.finish_review(seq, Kind::Imap, acc.imap, Ok(der));
            }
        }
    });
}

/// Snapshots: the first account's mail as when a first sync is
/// downloading it (made-up numbers: 1,234 of 5,000 in its first folder).
pub fn sample_progress() {
    with_app(|a| {
        let Some(id) = a.accounts.first().map(|acc| acc.id) else {
            return;
        };
        let folder = a
            .folders
            .iter()
            .find(|f| f.account_id == id)
            .map_or(0, |f| f.id);
        a.on_event(Event::Progress {
            account_id: id,
            folder_id: folder,
            done: 1234,
            total: 5000,
        });
    });
}

/// The window gained or lost the keyboard focus.
pub fn window_focused(focused: bool) {
    with_app(|a| a.focused = focused);
}

/// Windows says an appearance setting changed: read them all again (a
/// burst of messages comes as one call). Retried shortly while the app is
/// busy (under a native dialog's message loop).
pub fn system_theme_changed() {
    let system = komorebi::theme::read();
    if with_app(|a| a.system_changed(system)).is_none() {
        Timer::single_shot(Duration::from_millis(250), system_theme_changed);
    }
}

/// The window moved or was resized while neither maximized nor minimized.
pub fn window_placed(g: db::Geometry) {
    with_app(|a| a.normal_geometry = Some(g));
}

fn wire(ui: &AppWindow) {
    ui.on_nav_clicked(|item| {
        with_app(|a| a.nav_clicked(item));
    });
    ui.on_nav_toggle(|item| {
        with_app(|a| a.nav_toggle(item));
    });
    ui.on_nav_context(|item| with_app(|a| a.nav_context(&item)).unwrap_or(false));
    ui.on_nav_command(|item, command, id| {
        with_app(|a| a.nav_command(&item, command, id));
    });
    ui.on_list_menu_opening(|| {
        with_app(|a| a.list_menu_opening());
    });
    ui.on_list_command(|command| {
        with_app(|a| a.list_command(command));
    });
    ui.on_message_selected(|id| {
        with_app(|a| a.row_clicked(id as i64));
    });
    ui.on_select_offset(|d| {
        with_app(|a| a.select_offset(d));
    });
    ui.on_select_edge(|end| {
        with_app(|a| a.select_edge(end));
    });
    ui.on_open_current(|| with_app(|a| a.open_current()).unwrap_or(false));
    ui.on_list_escape(|| {
        with_app(|a| a.escape());
    });
    ui.on_back_to_list(|| {
        with_app(|a| a.back_to_list());
    });
    ui.on_clear_search(|| {
        with_app(|a| a.clear_search());
    });
    ui.on_set_filter(|filter| {
        with_app(|a| a.set_filter(filter));
    });
    ui.on_mark_all_read(|| {
        with_app(|a| a.mark_all_read());
    });
    ui.on_set_search_all(|all| {
        with_app(|a| a.set_search_all(all));
    });
    ui.on_show_in_folder(|| {
        with_app(|a| a.show_in_folder());
    });
    ui.on_message_toggled(|id| {
        let id = id as i64;
        with_app(|a| a.change_selection(id, |s, _| s.toggle(id)));
    });
    ui.on_message_extended(|id| {
        let id = id as i64;
        with_app(|a| a.change_selection(id, |s, rows| s.extend_to(rows, id)));
    });
    ui.on_select_extend(|d| {
        with_app(|a| a.select_rows(|s, rows| s.extend_by(rows, d as isize)));
    });
    ui.on_select_all_rows(|| {
        with_app(|a| a.select_rows(Selection::all));
    });
    ui.on_mark_read(|on| {
        with_app(|a| a.mark(Flag::Seen, on));
    });
    ui.on_mark_flagged(|on| {
        with_app(|a| a.mark(Flag::Flagged, on));
    });
    ui.on_refresh(|| {
        with_app(|a| a.refresh());
    });
    ui.on_search_edited(|text| {
        with_app(|a| a.search_edited(&text));
    });
    ui.on_reply(|| {
        with_app(|a| a.reply(false));
    });
    ui.on_reply_all(|| {
        with_app(|a| a.reply(true));
    });
    ui.on_forward(|| {
        with_app(|a| a.forward());
    });
    ui.on_edit_server_draft(|| {
        with_app(|a| a.edit_server_draft());
    });
    ui.on_archive(|| {
        with_app(|a| a.archive());
    });
    ui.on_delete(|| {
        with_app(|a| a.delete());
    });
    ui.on_toggle_read(|| {
        with_app(|a| a.toggle_read());
    });
    ui.on_toggle_flag(|| {
        with_app(|a| a.toggle_flag());
    });
    ui.on_move_to(|id| {
        with_app(|a| a.move_to(id as i64));
    });
    ui.on_junk(|report| {
        with_app(|a| a.remove_to(if report { Dest::Junk } else { Dest::NotJunk }));
    });
    ui.on_message_context(|id| with_app(|a| a.message_context(id as i64)).unwrap_or(false));
    ui.on_menu_opening(|| {
        with_app(|a| a.menu_opening());
    });
    ui.on_message_quick(|id, action| {
        with_app(|a| a.quick_action(id as i64, action));
    });
    ui.on_clear_selection(|| {
        with_app(|a| a.clear_selection());
    });
    ui.on_move_filter_edited(|text| {
        with_app(|a| a.move_filter_edited(&text));
    });
    ui.on_go_inbox(|| {
        with_app(|a| a.go_inbox());
    });
    ui.on_undo_clicked(|| {
        with_app(|a| a.undo_clicked());
    });
    ui.on_undo_hold(|on| {
        with_app(|a| a.undo_hold(on));
    });
    ui.on_r_retry_body(|| {
        with_app(|a| a.retry_body());
    });
    ui.on_menu_respond(|kind| {
        with_app(|a| a.menu_respond(kind));
    });
    ui.on_save_attachment(save_attachment);
    ui.on_save_eml(|| {
        with_app(|a| a.source_action(original::RawUse::SaveEml));
    });
    ui.on_show_original(|| {
        with_app(|a| a.source_action(original::RawUse::ShowOriginal));
    });
    ui.on_forward_attached(|| {
        with_app(|a| a.source_action(original::RawUse::Forward));
    });
    ui.on_copy_message_id(|| {
        with_app(|a| a.copy_message_id());
    });
    ui.on_invite(|action| {
        if action == 4 {
            invite::save_invite();
        } else {
            with_app(|a| a.invite_action(action));
        }
    });
    ui.on_unsubscribe(|| {
        with_app(|a| a.unsubscribe());
    });
    ui.on_open_attachment(|index| {
        with_app(|a| a.open_attachment(index));
    });
    ui.on_compose(|| {
        with_app(|a| a.compose_new());
    });
    ui.on_send(|| {
        with_app(|a| a.send());
    });
    ui.on_c_send_anyway(|| {
        with_app(|a| a.send_anyway());
    });
    ui.on_o_edit(|| {
        with_app(|a| a.outbox_edit());
    });
    ui.on_o_retry(|| {
        with_app(|a| a.outbox_retry());
    });
    ui.on_o_discard(|| {
        with_app(|a| a.outbox_discard());
    });
    ui.on_q_alt_answer(|| {
        with_app(|a| a.answer_alt());
    });
    ui.on_q_answer(|accepted| {
        with_app(|a| a.answer(accepted));
    });
    ui.on_discard(|| {
        with_app(|a| a.discard());
    });
    ui.on_discard_confirmed(|| {
        with_app(|a| a.discard_confirmed());
    });
    ui.on_attach(composer::attach_files);
    ui.on_c_paste_files(|| with_app(|a| a.paste_files()).unwrap_or(false));
    ui.on_remove_attachment(|i| {
        with_app(|a| a.remove_attachment(i));
    });
    ui.on_c_edited(|field, text| {
        with_app(|a| a.draft_edited(field, &text));
    });
    ui.on_c_left(|field| {
        with_app(|a| a.draft_left(field));
    });
    ui.on_c_suggest_accept(|index| {
        with_app(|a| a.accept_suggestion(index));
    });
    ui.on_c_suggest_close(|| {
        with_app(|a| a.close_suggestions());
    });
    ui.on_c_from_changed(|index| {
        with_app(|a| a.draft_account(index));
    });
    ui.on_c_reveal(|field| {
        with_app(|a| a.reveal_cc(field));
    });
    ui.on_c_set_importance(|code| {
        with_app(|a| a.draft_importance(code));
    });
    ui.on_c_close(|| {
        with_app(|a| a.close_composer());
    });
    ui.on_c_unfold_quote(|| {
        with_app(|a| a.unfold_quote());
    });
    ui.on_c_preview_render(|width, dark, first| {
        with_app(|a| a.render_preview(width, dark, first));
    });
    ui.on_c_format(|action, anchor, cursor| {
        with_app(|a| a.format_body(action, anchor, cursor)).unwrap_or_default()
    });
    ui.on_c_link(|url, anchor, cursor| {
        with_app(|a| a.link_body(&url, anchor, cursor)).unwrap_or_default()
    });
    ui.on_c_unformat(|| with_app(|a| a.unformat_body()).unwrap_or_default());
    ui.on_save_account(|form| {
        with_app(|a| a.save_account(form));
    });
    ui.on_test_account(|form| {
        with_app(|a| a.test_account(form));
    });
    ui.on_edit_account(|id| {
        with_app(|a| a.edit_account(id as i64));
    });
    ui.on_find_settings(|email, fields| {
        with_app(|a| a.find_settings(email.to_string(), fields));
    });
    ui.global::<Discovery>()
        .on_email_ok(|email| komorebi::mail::discover::looks_valid(&email));
    ui.global::<Discovery>()
        .on_guess_provider(|email| providers::guess(&email).map_or(-1, |i| i as i32));
    ui.on_cancel_check(|| {
        with_app(|a| a.cancel_check());
    });
    onboarding::wire(ui);
    composer::rich::wire(ui);
    composer::recipients::wire(ui);
    signatures::wire(ui);
    ui.on_update_action(|which| {
        with_app(|a| a.update_action(which));
    });
    ui.on_welcome_link(|which| {
        with_app(|a| a.welcome_link(which));
    });
    ui.on_review_certificate(|form, which| {
        with_app(|a| a.review_certificate(form, which));
    });
    ui.on_forget_certificate(|which| {
        with_app(|a| a.forget_certificate(which));
    });
    ui.on_account_form_changed(|form| {
        with_app(|a| a.form_changed(form));
    });
    ui.on_account_form_opened(|| {
        with_app(|a| a.form_opened());
    });
    ui.on_remove_account(|id| {
        with_app(|a| a.remove_account(id as i64));
    });
    ui.on_settings_closed(|| {
        with_app(|a| {
            // A check still running answers into nothing: it neither shows
            // up when the dialog opens again nor adds the account.
            a.check_seq += 1;
            if let Some(ui) = a.ui.upgrade() {
                ui.set_a_busy(false);
                ui.set_a_error(SharedString::new());
                ui.set_a_ok(SharedString::new());
                ui.set_a_test(ModelRc::default());
                ui.set_a_edit_id(0);
                ui.set_a_edit(AccountForm::default());
                ui.set_a_reading(false);
                ui.set_a_rejected(false);
            }
            a.form_opened();
            a.cert_review = None;
        });
    });
    ui.on_r_load_remote(|always| {
        with_app(|a| a.load_remote(always));
    });
    ui.on_settings_opened(|| {
        with_app(|a| {
            a.show_remote_senders();
            a.show_general();
            // The accounts' cards show a download's progress there.
            if a.progress_shown.get() {
                a.set_status("", false);
            }
        });
    });
    ui.on_general_changed(|which, value| {
        with_app(|a| a.general_changed(which, value));
    });
    ui.on_account_enabled(|id, on| {
        with_app(|a| a.set_account_enabled(id as i64, on));
    });
    ui.on_account_notify(|id, on| {
        with_app(|a| a.set_account_notify(id as i64, on));
    });
    ui.on_move_account(|id, up| {
        with_app(|a| a.move_account(id as i64, up));
    });
    ui.on_remove_all_data(|| {
        with_app(|a| a.ask(Question::RemoveAllData));
    });
    ui.on_redownload_account(|id| {
        with_app(|a| a.ask(Question::Redownload(id as i64)));
    });
    ui.on_review_changed_certificate(|id| {
        with_app(|a| a.review_changed_certificate(i64::from(id)));
    });
    ui.on_open_logs_folder(|| {
        with_app(|a| {
            // The logs folder once a log was kept, else the data folder
            // that holds it.
            let logs = komorebi::diag::log_path(&a.db_path)
                .parent()
                .map(std::path::Path::to_path_buf)
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| a.ui().get_about_data().to_string().into());
            let open = move || {
                reader::shell_execute(logs.as_os_str());
            };
            background("open-folder", Box::new(open)).ok();
        });
    });
    ui.on_r_remote_undo(|| {
        with_app(|a| a.undo_remote_always());
    });
    ui.on_open_data_folder(|| {
        with_app(|a| {
            let dir = a.ui().get_about_data().to_string();
            // Explorer can take a moment to answer: off the UI thread.
            let open = move || {
                reader::shell_execute(std::ffi::OsStr::new(&dir));
            };
            background("open-folder", Box::new(open)).ok();
        });
    });
    ui.on_tip_got_it(|| {
        with_app(|a| a.tip_got_it());
    });
    ui.on_tip_skip(|| {
        with_app(|a| a.tip_skip());
    });
    ui.on_nav_collapse_changed(|collapsed| {
        with_app(|a| {
            if let Err(e) =
                a.db.set_setting("nav_collapsed", if collapsed { "1" } else { "0" })
            {
                a.set_status(&text::detail(&e), true);
            }
        });
    });
    ui.on_layout_changed(|compact| {
        with_app(|a| a.layout_changed(compact));
    });
    ui.on_show_tips_again(|| {
        with_app(|a| a.show_tips_again());
    });
    ui.on_open_repository(|| {
        use reader::Shell;
        reader::SystemShell.open_url(env!("CARGO_PKG_REPOSITORY"));
    });
    ui.on_forget_remote(|address| {
        with_app(|a| {
            if let Err(e) = a.db.forget_remote(&address) {
                a.set_status(&text::detail(&e), true);
            }
            a.show_remote_senders();
        });
    });
    ui.on_forget_all_remote(|| {
        with_app(|a| {
            if let Err(e) = a.db.forget_all_remote() {
                a.set_status(&text::detail(&e), true);
            }
            a.show_remote_senders();
        });
    });
    ui.on_dismiss_status(|| {
        with_app(|a| a.dismiss_status());
    });
    ui.on_choose_role(|role, option| {
        with_app(|a| a.choose_role(role, option));
    });
    ui.on_status_action_clicked(|| {
        with_app(|a| match a.status_action.get() {
            StatusAction::None => {}
            StatusAction::EditAccount(id) => {
                a.set_status("", false);
                a.edit_account(id);
            }
            StatusAction::RetryAccount(id) => {
                a.set_status("", false);
                a.retry_account(id);
            }
            StatusAction::ReviewCertificate(id) => {
                a.set_status("", false);
                a.review_changed_certificate(id);
            }
            StatusAction::TryNow(id) => a.try_now(id),
            StatusAction::WhatsNew => {
                a.whats_new_seen();
                a.set_status("", false);
                a.welcome_link(2);
            }
        });
    });
    ui.on_tab_activate(|i| {
        with_app(|a| a.activate_tab(i.max(0) as usize));
    });
    ui.on_tab_close(|i| {
        with_app(|a| a.close_tab(i.max(0) as usize));
    });
    ui.on_tab_move(|from, to| {
        with_app(|a| a.move_tab(from.max(0) as usize, to.max(0) as usize));
    });
    ui.on_tab_command(|i, command| {
        with_app(|a| a.tab_command(i.max(0) as usize, command));
    });
    ui.on_tab_key(|key| {
        with_app(|a| a.tab_key(key));
    });
    ui.on_open_tab(|id, background| {
        with_app(|a| a.open_in_tab(i64::from(id), background));
    });
    ui.on_open_current_tab(|| {
        with_app(|a| a.open_current_in_tab());
    });
    ui.on_open_window(|| {
        with_app(|a| a.open_current_window());
    });
    ui.on_bring_popped(|| {
        with_app(|a| a.bring_popped());
    });
    // The message windows close first (none outlives this one); drafts
    // are saved and held moves sent before the window goes, and where the
    // window is and its tabs, for the next start.
    // With "Keep running in the notification area" on (and the tray icon
    // up), the window only hides; otherwise the app quits.
    ui.window().on_close_requested(|| {
        let to_tray = with_app(|a| {
            a.before_close();
            a.notes.close_to_tray && a.notes.has_tray() && !a.notes.quitting
        })
        .unwrap_or(false);
        if !to_tray {
            with_app(|a| a.close_composer_windows());
            slint::quit_event_loop().ok();
        }
        slint::CloseRequestResponse::HideWindow
    });
}

impl App {
    /// What closing the window saves: drafts, held moves, where the window
    /// is and its tabs; the message windows close.
    fn before_close(&mut self) {
        self.close_all_popouts();
        self.flush_held();
        self.save_all_drafts();
        self.save_geometry();
        self.save_tabs();
        self.save_popout_size();
    }

    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window alive")
    }

    // ---------------------------------------------------------- accounts

    fn load_accounts(&mut self) -> anyhow::Result<()> {
        self.accounts = self.db.accounts()?;
        self.folders = self.db.folders()?;
        let ui = self.ui();
        ui.set_has_accounts(!self.accounts.is_empty());
        self.show_account_rows();
        let from: Vec<SharedString> = self
            .accounts
            .iter()
            .map(|a| {
                if a.name.is_empty() {
                    a.email.as_str().into()
                } else {
                    format!("{} <{}>", a.name, a.email).into()
                }
            })
            .collect();
        let from = ModelRc::new(VecModel::from(from));
        for w in &self.cwins {
            w.ui.set_c_from_options(from.clone());
        }
        ui.set_c_from_options(from);
        // The accounts may have moved: the open draft's From follows.
        if let Some(d) = self.drafts.active_id().and_then(|id| self.drafts.get(id))
            && let Some(i) = self.accounts.iter().position(|a| a.id == d.account_id)
        {
            ui.set_c_from_index(i as i32);
        }
        self.rebuild_nav();
        self.settle_view();
        Ok(())
    }

    /// The accounts in Settings, each with its status line.
    fn show_account_rows(&self) {
        let last = self.accounts.len().saturating_sub(1);
        let rows: Vec<AccountRow> = self
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let attention = self.attention(a.id);
                let state = self.account_state.get(&a.id);
                let review = matches!(state, Some(AccountState::Failed(p)) if p.changed.is_some());
                let warn = attention.is_some() || review;
                let progress = self.progress.get(&a.id).copied();
                let status = if !a.enabled {
                    t!(invoke_account_off)
                } else if let Some((done, total)) = progress {
                    t!(
                        invoke_account_downloading,
                        format::thousands(self.lang, done).into(),
                        format::thousands(self.lang, total).into()
                    )
                } else {
                    account_status(self.account_state.get(&a.id), attention, &a.imap.host)
                };
                AccountRow {
                    id: a.id as i32,
                    email: a.email.as_str().into(),
                    detail: format!("{} · {}", a.imap.host, a.smtp.host).into(),
                    status: status.into(),
                    warn,
                    review,
                    caution: a.enabled && matches!(state, Some(AccountState::Unreachable)),
                    name: a.name.trim().into(),
                    enabled: a.enabled,
                    notify: !self.notes.off.contains(&a.id),
                    dot: settings::account_color(i),
                    first: i == 0,
                    last: i == last,
                    done: progress.map_or(0, |p| p.0.min(i32::MAX as usize) as i32),
                    total: progress.map_or(0, |p| p.1.min(i32::MAX as usize) as i32),
                }
            })
            .collect();
        let ui = self.ui();
        ui.set_accounts(ModelRc::new(VecModel::from(rows)));
        // The account page follows its account (moved, or removed: -1
        // closes it).
        let editing = i64::from(ui.get_a_edit_id());
        if editing != 0 {
            let at = self.accounts.iter().position(|a| a.id == editing);
            ui.set_a_edit_index(at.map_or(-1, |i| i as i32));
        }
    }

    /// What the account's workers wait for the user to fix: the sign-in
    /// was rejected (or there is no saved password), or the secure
    /// connection failed (a changed or untrusted certificate). Nothing
    /// connects until the account is edited, Retry or Refresh.
    fn attention(&self, id: i64) -> Option<Attention> {
        self.workers.get(&id).and_then(Workers::attention)
    }

    /// Whether the account's pane row and rail tile carry the caution
    /// badge: its server was out of reach at the last sync, until one goes
    /// through.
    fn unreachable_badge(&self, id: i64) -> bool {
        matches!(self.account_state.get(&id), Some(AccountState::Unreachable))
    }

    /// Try now on the line saying a server can't be reached: one attempt
    /// at once. The line goes; it comes back if the server is still out of
    /// reach, and the badge stays until a sync goes through.
    fn try_now(&mut self, id: i64) {
        self.standing.retain(|(a, _)| *a != id);
        self.set_status("", false);
        self.shown_problem.remove(&id);
        if let Some(w) = self.workers.get(&id) {
            w.send(Job::SyncAll { full: true });
        }
    }

    /// Retry on the status line: the account's workers make one new attempt.
    fn retry_account(&mut self, id: i64) {
        self.shown_problem.remove(&id);
        if let Some(w) = self.workers.get(&id) {
            w.credentials_changed();
            w.send(Job::SyncAll { full: true });
        }
        self.show_account_rows();
        self.rebuild_nav();
    }

    fn spawn_workers(&mut self, account: Account) {
        let id = account.id;
        let workers = Workers::spawn(account, self.db_path.clone(), self.notify.clone());
        self.workers.insert(id, workers);
    }

    /// Replace an account's workers (its servers or settings changed) and
    /// sync it in full. The old ones finish their current job and log out.
    fn respawn_workers(&mut self, id: i64) {
        self.workers.remove(&id);
        self.syncing.remove(&id);
        self.warmed.remove(&id);
        self.account_state.remove(&id);
        self.standing.retain(|(a, _)| *a != id);
        if let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.id == id && a.enabled)
            .cloned()
        {
            self.spawn_workers(account);
            self.workers[&id].send(Job::SyncAll { full: true });
        }
        self.ui().set_syncing(!self.syncing.is_empty());
    }

    /// Sync every account; `full` ignores folder STATUS (start, Refresh).
    fn sync_all(&mut self, full: bool) {
        if full {
            self.last_full_sync = Some(Instant::now());
        }
        for w in self.workers.values() {
            w.send(Job::SyncAll { full });
        }
    }

    /// (Re)start the new-mail check timer at the chosen interval. Each
    /// check is a STATUS-gated pass, and a full one at least hourly.
    fn start_sync_timer(&self) {
        self.sync_timer
            .start(TimerMode::Repeated, self.check_interval, || {
                with_app(|app| {
                    let full = app
                        .last_full_sync
                        .is_none_or(|t| t.elapsed() >= settings::FULL_SYNC_EVERY);
                    app.sync_all(full);
                    app.drain_outbox();
                });
            });
    }

    /// The Refresh button: also retries a rejected sign-in, and shows any
    /// problem again.
    fn refresh(&mut self) {
        self.shown_problem.clear();
        for w in self.workers.values() {
            w.credentials_changed();
        }
        self.sync_all(true);
        self.drain_outbox();
        self.show_account_rows();
        self.rebuild_nav();
    }

    /// The settings form's Add account or Save.
    fn save_account(&mut self, form: AccountForm) {
        if form.id == 0 {
            self.add_account(form);
        } else {
            self.update_account(form);
        }
    }

    fn add_account(&mut self, form: AccountForm) {
        let ui = self.ui();
        ui.set_a_error_kind(onboarding::kind::OTHER);
        let email = form.email.trim().to_string();
        let password = form.password.to_string();
        self.check_form = (form_preset(&form), email.clone());
        let parsed = (|| -> Result<NewAccount, text::FormError> {
            if !email.contains('@') {
                return Err(text::FormError::Email);
            }
            let (imap, smtp) = form_servers(&form)?;
            let (imap_user, smtp_user) = settings::form_logins(&form, &email)?;
            Ok(NewAccount {
                email: email.clone(),
                name: form.name.trim().to_string(),
                imap,
                smtp,
                // A preset's own default can only turn the copy off.
                save_sent: form.save_sent && providers::save_sent_default(form_preset(&form)),
                imap_user,
                smtp_user,
            })
        })();
        let new = match parsed {
            Ok(n) => n,
            Err(e) => {
                ui.set_a_error(text::form_error(e).into());
                return;
            }
        };
        if self
            .accounts
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&new.email))
        {
            ui.set_a_error(t!(invoke_err_account_exists).into());
            return;
        }
        let signature = form.signature.to_string();
        let logins = (
            db::login_or(&new.imap_user, &new.email).to_string(),
            db::login_or(&new.smtp_user, &new.email).to_string(),
        );
        let servers = (new.imap.clone(), new.smtp.clone());
        let pins = self.pins_for(&form, (&servers.0, &servers.1), None);
        let given = password.clone();
        let saved_pins = pins.clone();
        self.run_check(
            servers,
            pins,
            cert::is_manual(&form),
            logins,
            Some(given),
            0,
            Some(Box::new(move |app| {
                app.finish_add_account(new, signature, password, saved_pins)
            })),
        );
    }

    /// Check the servers and the sign-ins on a thread, step by step, and
    /// show the steps under the form's buttons. Nothing is saved here: when
    /// every step passed, `then` runs (Add and Save); a check without one
    /// ("Test connection") only says so. `password`: the typed one, or
    /// else the one saved for account `saved`.
    #[allow(clippy::too_many_arguments)]
    fn run_check(
        &mut self,
        servers: (Server, Server),
        pins: (Option<Vec<u8>>, Option<Vec<u8>>),
        manual: bool,
        logins: (String, String),
        password: Option<String>,
        saved: i64,
        then: Option<AfterCheck>,
    ) {
        let ui = self.ui();
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            ui.set_a_error(t!(invoke_err_offline_check).into());
            return;
        }
        self.check_seq += 1;
        let seq = self.check_seq;
        let verify = self.verifier;
        ui.set_a_error_kind(onboarding::kind::OTHER);
        ui.set_a_busy(true);
        ui.set_a_reading(false);
        ui.set_a_rejected(false);
        ui.set_a_error(SharedString::new());
        ui.set_a_ok(SharedString::new());
        self.show_cert_state(0);
        self.show_check_steps(0, None);
        let check = move || {
            let result: Result<(), CheckFailure> = (|| {
                let password = match password {
                    Some(p) => p,
                    None => secrets::get_password(saved).map_err(|_| CheckFailure {
                        step: None,
                        text: CheckText::NoSavedPassword,
                        review: None,
                    })?,
                };
                let logins = (logins.0.as_str(), logins.1.as_str());
                let pins = (pins.0.as_deref(), pins.1.as_deref());
                verify(
                    &servers.0,
                    &servers.1,
                    pins,
                    logins,
                    &password,
                    &mut |step| {
                        let done = step.index() + 1;
                        on_ui(Box::new(move |app| app.check_progress(seq, done)));
                    },
                )
                .map_err(|e| CheckFailure {
                    step: Some(e.step.index()),
                    text: CheckText::Say(e.say()),
                    // Only a manual server's untrusted certificate can be
                    // reviewed; a preset's is never offered for trust.
                    review: cert::review_of(&e).filter(|_| manual),
                })
            })();
            on_ui(Box::new(move |app| app.finish_check(seq, result, then)));
        };
        background("check-account", Box::new(check)).expect("spawning account check");
    }

    /// The steps list: the first `done` steps passed and the next one is
    /// running (`failed`: none is, that one failed and the rest were not
    /// tried). States: 0 waiting, 1 running, 2 passed, 3 failed, 4 skipped.
    fn show_check_steps(&self, done: usize, failed: Option<usize>) {
        let count = mail::errors::Step::ALL.len();
        let states: Vec<i32> = (0..count)
            .map(|i| match failed {
                Some(f) if i == f => 3,
                Some(f) if i > f => 4,
                Some(_) => 2,
                None if i < done => 2,
                None if i == done => 1,
                None => 0,
            })
            .collect();
        self.ui().set_a_test(ModelRc::new(VecModel::from(states)));
    }

    fn check_progress(&self, seq: u64, done: usize) {
        if seq == self.check_seq && self.ui().get_a_busy() {
            self.show_check_steps(done, None);
        }
    }

    fn finish_check(
        &mut self,
        seq: u64,
        result: Result<(), CheckFailure>,
        then: Option<AfterCheck>,
    ) {
        if seq != self.check_seq {
            return;
        }
        let ui = self.ui();
        ui.set_a_busy(false);
        match result {
            Err(CheckFailure { step, text, review }) => {
                match step {
                    Some(step) => self.show_check_steps(0, Some(step)),
                    None => ui.set_a_test(ModelRc::default()),
                }
                let rejected = matches!(text, CheckText::Say(mail::Say::Rejected { .. }));
                ui.set_a_error_kind(onboarding::error_kind(&text, review.is_some()));
                ui.set_a_error(
                    match text {
                        CheckText::Say(mail::Say::Rejected { host, login }) => {
                            let (preset, email) = &self.check_form;
                            text::rejected(*preset, email, &host, &login)
                        }
                        CheckText::Say(say) => text::say(&say),
                        CheckText::NoSavedPassword => t!(invoke_err_saved_password),
                    }
                    .into(),
                );
                // The Password box's error state, and Set username.
                ui.set_a_rejected(rejected);
                self.show_cert_state(review.map_or(0, cert::number));
            }
            Ok(()) => {
                self.show_check_steps(mail::errors::Step::ALL.len(), None);
                match then {
                    Some(then) => then(self),
                    None => ui.set_a_ok(t!(invoke_check_passed).into()),
                }
            }
        }
    }

    /// Cancel, Back or Esc while a check runs: its answer is dropped (so
    /// it adds or saves nothing), and the form stays as typed.
    fn cancel_check(&mut self) {
        self.check_seq += 1;
        let ui = self.ui();
        ui.set_a_busy(false);
        ui.set_a_reading(false);
        ui.set_a_test(ModelRc::default());
    }

    /// The welcome's links, through the link prompt as a link in a message
    /// would: 0 the README's setup section, 1 the privacy statement.
    fn welcome_link(&mut self, which: i32) {
        let repo = env!("CARGO_PKG_REPOSITORY");
        let url = match which {
            0 => format!("{repo}#add-an-account"),
            2 => format!("{repo}/blob/main/CHANGELOG.md"),
            3 => format!("{repo}#readme"),
            4 => format!("{repo}/issues"),
            _ => format!("{repo}/blob/main/PRIVACY.md"),
        };
        self.reader.follow(url);
    }

    /// "Test connection": the same check as Add and Save, saving nothing.
    fn test_account(&mut self, form: AccountForm) {
        let ui = self.ui();
        let id = form.id as i64;
        let email = if id == 0 {
            form.email.trim().to_string()
        } else if let Some(a) = self.accounts.iter().find(|a| a.id == id) {
            a.email.clone()
        } else {
            ui.set_a_error(t!(invoke_err_account_removed).into());
            return;
        };
        let parsed = (|| -> Result<_, text::FormError> {
            if !email.contains('@') {
                return Err(text::FormError::Email);
            }
            let servers = form_servers(&form)?;
            let logins = settings::form_logins(&form, &email)?;
            Ok((servers, logins))
        })();
        let ((imap, smtp), (imap_user, smtp_user)) = match parsed {
            Ok(p) => p,
            Err(e) => {
                ui.set_a_error(text::form_error(e).into());
                return;
            }
        };
        let password = Some(form.password.to_string()).filter(|p| !p.is_empty());
        if password.is_none() && id == 0 {
            ui.set_a_error(t!(invoke_err_password_to_test).into());
            return;
        }
        self.check_form = (form_preset(&form), email.clone());
        let logins = (
            db::login_or(&imap_user, &email).to_string(),
            db::login_or(&smtp_user, &email).to_string(),
        );
        let old = self.accounts.iter().find(|a| a.id == id).cloned();
        let pins = self.pins_for(&form, (&imap, &smtp), old.as_ref());
        self.run_check(
            (imap, smtp),
            pins,
            cert::is_manual(&form),
            logins,
            password,
            id,
            None,
        );
    }

    fn finish_add_account(
        &mut self,
        new: NewAccount,
        signature: String,
        password: String,
        pins: (Option<Vec<u8>>, Option<Vec<u8>>),
    ) {
        let ui = self.ui();
        let saved = self.db.add_account(&new).and_then(|id| {
            let with_signature = Account {
                id,
                email: new.email.clone(),
                name: new.name.clone(),
                imap: new.imap.clone(),
                smtp: new.smtp.clone(),
                save_sent: new.save_sent,
                signature,
                enabled: true,
                imap_user: new.imap_user.clone(),
                smtp_user: new.smtp_user.clone(),
                imap_pin: pins.0,
                smtp_pin: pins.1,
            };
            (self.store_password)(id, &password)
                .and_then(|()| self.db.update_account(&with_signature).map(drop))
                .inspect_err(|_| {
                    (self.forget_password)(id).ok();
                    self.db.remove_account(id).ok();
                })?;
            Ok(id)
        });
        match saved {
            Ok(id) => {
                if let Err(e) = self.load_accounts() {
                    self.set_status(&text::detail(&e), true);
                }
                if self.accounts.len() == 1 {
                    self.first_account_added();
                }
                self.first_sync.insert(id);
                if self.workers_on
                    && let Some(account) = self.accounts.iter().find(|a| a.id == id).cloned()
                {
                    self.spawn_workers(account);
                    self.workers[&id].send(Job::SyncAll { full: true });
                }
                // The mail behind the flow moves to the new account's Inbox
                // once its folders arrive (the list then shows the
                // download); the flow goes on to Make it yours.
                self.land_on = Some((id, self.view));
                self.land();
                self.show_empty();
                self.onboard_added(id, &new.email);
            }
            Err(e) => ui.set_a_error(t!(invoke_not_saved, format!("{e:#}").into()).into()),
        }
    }

    /// Open a just-added account's Inbox once it is known, if the view is
    /// still the one it was added from.
    fn land(&mut self) {
        let Some((id, from)) = self.land_on else {
            return;
        };
        if !self.accounts.iter().any(|a| a.id == id) || self.view != from {
            self.land_on = None;
            return;
        }
        if let Some(inbox) = self
            .folders
            .iter()
            .find(|f| f.account_id == id && f.role == Role::Inbox)
            .map(|f| f.id)
        {
            self.land_on = None;
            self.set_view(View::Folder(inbox));
        }
    }

    /// Edit (or "Update password"): the settings form, filled with the
    /// account's settings. The password is never shown.
    fn edit_account(&mut self, id: i64) {
        let Some(a) = self.accounts.iter().find(|a| a.id == id) else {
            return;
        };
        let ui = self.ui();
        let account = a.clone();
        ui.set_a_edit(AccountForm {
            id: a.id as i32,
            provider: providers::PRESETS.len() as i32,
            name: a.name.as_str().into(),
            email: a.email.as_str().into(),
            password: SharedString::new(),
            imap_host: settings::host_for_display(&a.imap.host).into(),
            imap_port: a.imap.port.to_string().into(),
            imap_starttls: a.imap.security == Security::StartTls,
            smtp_host: settings::host_for_display(&a.smtp.host).into(),
            smtp_port: a.smtp.port.to_string().into(),
            smtp_starttls: a.smtp.security == Security::StartTls,
            save_sent: a.save_sent,
            signature: a.signature.as_str().into(),
            imap_user: a.imap_user.as_str().into(),
            smtp_user: a.smtp_user.as_str().into(),
            smtp_own_login: a.smtp_user != a.imap_user,
        });
        ui.set_a_edit_id(id as i32);
        self.show_account_signatures(id);
        ui.set_a_edit_index(
            self.accounts
                .iter()
                .position(|a| a.id == id)
                .map_or(-1, |i| i as i32),
        );
        // A preset's servers: no Find settings on its page.
        ui.set_a_edit_preset(
            providers::PRESETS
                .iter()
                .any(|p| p.imap.0 == account.imap.host && p.smtp.0 == account.smtp.host),
        );
        ui.set_a_error(SharedString::new());
        self.seed_pins(&account);
        self.show_role_choices(id);
        ui.set_show_settings(true);
    }

    /// Save an edited account. A new password or other servers are
    /// checked first (IMAP, then SMTP) on a thread; the name, signature and
    /// Sent choice save at once.
    fn update_account(&mut self, form: AccountForm) {
        let ui = self.ui();
        let id = form.id as i64;
        let Some(old) = self.accounts.iter().find(|a| a.id == id).cloned() else {
            ui.set_a_error(t!(invoke_err_account_removed).into());
            return;
        };
        let (imap, smtp) = match form_servers(&form) {
            Ok(servers) => servers,
            Err(e) => {
                ui.set_a_error(text::form_error(e).into());
                return;
            }
        };
        let (imap_user, smtp_user) = match settings::form_logins(&form, &old.email) {
            Ok(logins) => logins,
            Err(e) => {
                ui.set_a_error(text::form_error(e).into());
                return;
            }
        };
        let pins = self.pins_for(&form, (&imap, &smtp), Some(&old));
        self.check_form = (None, old.email.clone());
        let edited = Account {
            name: form.name.trim().to_string(),
            imap,
            smtp,
            imap_user,
            smtp_user,
            imap_pin: pins.0.clone(),
            smtp_pin: pins.1.clone(),
            save_sent: form.save_sent,
            signature: form.signature.to_string(),
            ..old.clone()
        };
        let password = Some(form.password.to_string()).filter(|p| !p.is_empty());
        let same_servers = edited.imap == old.imap
            && edited.smtp == old.smtp
            && edited.imap_user == old.imap_user
            && edited.smtp_user == old.smtp_user
            && edited.imap_pin == old.imap_pin
            && edited.smtp_pin == old.smtp_pin;
        if password.is_none() && same_servers {
            return self.finish_update_account(old, edited, None);
        }
        let logins = (
            edited.imap_login().to_string(),
            edited.smtp_login().to_string(),
        );
        let servers = (edited.imap.clone(), edited.smtp.clone());
        let given = password.clone();
        self.run_check(
            servers,
            pins,
            cert::is_manual(&form),
            logins,
            given,
            id,
            Some(Box::new(move |app| {
                app.finish_update_account(old, edited, password)
            })),
        );
    }

    fn finish_update_account(&mut self, old: Account, edited: Account, password: Option<String>) {
        let ui = self.ui();
        ui.set_a_busy(false);
        let id = edited.id;
        // Removed while the check ran: never save a password for it.
        if !self.accounts.iter().any(|a| a.id == id) {
            ui.set_a_error(t!(invoke_err_account_removed).into());
            return;
        }
        if let Some(p) = &password
            && let Err(e) = secrets::set_password(id, p)
        {
            ui.set_a_error(t!(invoke_not_saved, format!("{e:#}").into()).into());
            return;
        }
        if let Err(e) = self.db.update_account(&edited) {
            ui.set_a_error(t!(invoke_not_saved, format!("{e:#}").into()).into());
            return;
        }
        // The folders chosen for the roles, before the lists are read again.
        self.apply_role_choices(id);
        if let Err(e) = self.load_accounts() {
            self.set_status(&text::detail(&e), true);
        }
        // The workers hold the account as it was: servers and the Sent
        // choice need new ones; a new password only lifts the latch.
        let settings_changed = edited.imap != old.imap
            || edited.smtp != old.smtp
            || edited.imap_user != old.imap_user
            || edited.smtp_user != old.smtp_user
            || edited.imap_pin != old.imap_pin
            || edited.smtp_pin != old.smtp_pin
            || edited.save_sent != old.save_sent;
        if settings_changed && self.workers.contains_key(&id) {
            self.respawn_workers(id);
        } else if (password.is_some() || self.attention(id).is_some())
            && let Some(w) = self.workers.get(&id)
        {
            // A new password, or Save on an account that waited for the
            // user: one more try.
            w.credentials_changed();
            w.send(Job::SyncAll { full: true });
        }
        self.show_account_rows();
        self.rebuild_nav();
        ui.set_a_edit_id(0);
        ui.set_show_settings(false);
        self.set_status(
            &t!(invoke_account_saved, edited.email.as_str().into()),
            false,
        );
    }

    /// Remove asks first, saying how many unsent drafts and Outbox
    /// messages go with it.
    fn remove_account(&mut self, id: i64) {
        self.ask(Question::RemoveAccount(id));
    }

    fn remove_account_now(&mut self, id: i64) {
        // The password first: a credential is never left behind for an
        // account that is gone.
        if let Err(e) = (self.forget_password)(id) {
            self.set_status(&t!(invoke_not_removed, format!("{e:#}").into()), true);
            return;
        }
        // A held move goes now (one of this account's is dropped with it).
        self.flush_held();
        // Its folded lines go with it.
        let gone: HashSet<i64> = self
            .folders
            .iter()
            .filter(|f| f.account_id == id)
            .map(|f| f.id)
            .collect();
        let before = self.folded.len();
        self.folded.retain(|f| match f {
            Fold::Account(a) => *a != id,
            Fold::Folder(f) => !gone.contains(f),
        });
        if self.folded.len() != before {
            self.save_folds();
        }
        // Dropping the workers closes their channels; the threads log out.
        self.workers.remove(&id);
        self.syncing.remove(&id);
        self.warmed.remove(&id);
        self.account_state.remove(&id);
        if let Err(e) = self.db.remove_account(id) {
            self.set_status(&text::detail(&e), true);
            return;
        }
        if self.ui().get_a_edit_id() == id as i32 {
            self.ui().set_a_edit_id(0);
        }
        // Its drafts and outbox went with it.
        if self.drafts.drop_account(id) && self.ui().get_composing() {
            self.ui().set_composing(false);
        }
        self.prune_draft_tabs();
        self.pending_sends.retain(|_, p| p.account != id);
        self.refresh_draft_count();
        self.outbox_changed();
        if let View::Folder(f) = self.view
            && self.folders.iter().any(|x| x.id == f && x.account_id == id)
        {
            self.view = View::AllInboxes;
        }
        // A message tab keeps what it shows, with the moved-or-deleted
        // warning ([App::refresh_message_tabs]); the Mail tab's pane empties.
        if self.in_mail_tab() && self.open.as_ref().is_some_and(|o| o.msg.account_id == id) {
            self.close_reader();
        }
        self.load_accounts().ok();
        self.reload_rows();
        self.refresh_message_tabs();
        self.ui().set_syncing(!self.syncing.is_empty());
    }

    /// Remove all data, confirmed: every account's password first (if one
    /// can't be deleted, nothing else is touched), then nothing more is
    /// sent or synced and the window closes; the files go after that
    /// ([`remove_data_if_asked`]).
    fn remove_all_data_now(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        if let Err(e) = wipe::forget_passwords(&ids, &*self.forget_password) {
            self.set_status(&t!(invoke_stopped_no_files, format!("{e:#}").into()), true);
            return;
        }
        // Sends in their undo window are never sent, a held move never
        // reaches the server, and the workers stop (their channels close).
        for p in self.pending_sends.values() {
            p.cancel();
        }
        self.pending_sends.clear();
        self.held = None;
        self.workers.clear();
        // No timer starts workers again before the window closes.
        self.accounts.clear();
        for timer in [&self.autosave_timer, &self.sync_timer, &self.resume_timer] {
            timer.stop();
        }
        komorebi::diag::set(None);
        self.remove_data = true;
        self.close_all_popouts();
        slint::quit_event_loop().ok();
    }

    // -------------------------------------------------------- navigation

    /// Each folder's unread mail as the list shows it: held messages (in
    /// a Delete's or Move's undo window) are out of the list, so out of
    /// the counts.
    fn shown_unread(&self) -> HashMap<i64, i64> {
        let mut unread = self.db.unread_counts().unwrap_or_default();
        for (folder, n) in self.held.iter().flat_map(|h| &h.unread) {
            if let Some(count) = unread.get_mut(folder) {
                *count = (*count - n).max(0);
            }
        }
        unread
    }

    fn rebuild_nav(&mut self) {
        let started = Instant::now();
        let unread = self.shown_unread();
        let count = |f: &Folder| -> i32 {
            if f.role.shows_unread() {
                unread.get(&f.id).copied().unwrap_or(0) as i32
            } else {
                0
            }
        };
        let inbox_unread: i32 = self
            .folders
            .iter()
            .filter(|f| f.role == Role::Inbox)
            .map(count)
            .sum();
        self.show_unread(inbox_unread);
        let ui = self.ui();
        if self.accounts.is_empty() {
            update_model(&self.nav, Vec::new());
            update_model(&self.rail, Vec::new());
            ui.set_nav_pill(-1);
            ui.set_rail_pill(-1);
            ui.set_nav_cursor(0);
            return;
        }
        // Folds of accounts and folders that are gone.
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        if tree::prune_folds(&mut self.folded, &ids, &self.folders) {
            self.save_folds();
        }
        self.apply_default_folds(&unread);
        // The keyboard row's line, to keep the row on it (a row of the
        // rail while the rail shows).
        let in_rail = ui.get_rail();
        let cursor_line = usize::try_from(ui.get_nav_cursor())
            .ok()
            .and_then(|i| {
                if in_rail {
                    self.rail.row_data(i)
                } else {
                    self.nav.row_data(i)
                }
            })
            .map(|r| (r.kind, r.id));
        // All inboxes with more than one account (one account opens on its
        // Inbox); Drafts always (the local drafts, the one place a draft is
        // found).
        let mut items = Vec::new();
        if self.accounts.len() > 1 {
            items.push(NavItem {
                kind: 0,
                label: t!(invoke_all_inboxes).into(),
                // An inbox tray (Segoe Fluent Icons and MDL2 alike).
                icon: "\u{EC59}".into(),
                unread: inbox_unread,
                expanded: true,
                parent: -1,
                ..Default::default()
            });
        }
        items.push(NavItem {
            kind: 3,
            label: t!(invoke_drafts).into(),
            icon: Role::Drafts.icon().into(),
            count: self.draft_count as i32,
            expanded: true,
            parent: -1,
            ..Default::default()
        });
        // Shown while messages wait in the outbox.
        if self.outbox_count > 0 || self.view == View::Outbox {
            items.push(NavItem {
                kind: 4,
                label: t!(invoke_outbox).into(),
                // Upload: unlike Sent's paper plane, also in the compact rail.
                icon: "\u{E898}".into(),
                count: self.outbox_count as i32,
                expanded: true,
                parent: -1,
                ..Default::default()
            });
        }
        let nodes = tree::tree(&self.folders);
        let n = self.folders.len();
        // Each folder's unread with its subfolders' (shown on a folded
        // parent): children come after their parents.
        let mut sum: Vec<i32> = self.folders.iter().map(count).collect();
        for node in nodes.iter().rev() {
            if let Some(p) = node.parent
                && let (Some(&own), true) = (sum.get(node.index), p < n)
            {
                sum[p] = sum[p].saturating_add(own);
            }
        }
        // The row showing each folder: its own, or while it is folded
        // away, its nearest shown line's; and whether its subfolders show.
        let mut row = vec![0usize; n];
        let mut reach = vec![false; n];
        let lang = self.lang;
        // The rail: the unified items, then one tile per account that
        // opens its Inbox (id 0 until the account has one).
        let mut rail = items.clone();
        // All inboxes by its short rail name (two lines at most).
        if let Some(all) = rail.iter_mut().find(|r| r.kind == 0) {
            all.label = t!(invoke_rail_all_inboxes).into();
        }
        for (i, a) in self.accounts.iter().enumerate() {
            let line = items.len();
            let open = !self.folded.contains(&Fold::Account(a.id));
            let label = match a.name.trim() {
                "" => a.email.as_str(),
                name => name,
            };
            let head = NavItem {
                kind: 1,
                id: a.id as i32,
                label: label.into(),
                unread: self
                    .folders
                    .iter()
                    .filter(|f| f.account_id == a.id && f.role == Role::Inbox)
                    .map(count)
                    .sum(),
                expanded: open,
                parent: -1,
                account: a.id as i32,
                initial: label
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>())
                    .unwrap_or_default()
                    .into(),
                sub: a.email.as_str().into(),
                warn: self.attention(a.id).is_some(),
                off: !a.enabled,
                caution: a.enabled && self.unreachable_badge(a.id),
                note: if a.enabled && self.unreachable_badge(a.id) {
                    t!(invoke_nav_unreachable, text::host(&a.imap.host)).into()
                } else {
                    SharedString::new()
                },
                // Its tile's colour, shown even with one account.
                dot: settings::account_color(i),
                ..Default::default()
            };
            let inbox = self
                .folders
                .iter()
                .find(|f| f.account_id == a.id && f.role == Role::Inbox);
            rail.push(NavItem {
                kind: 2,
                id: inbox.map_or(0, |f| f.id as i32),
                icon: SharedString::new(),
                expanded: false,
                sub: inbox
                    .map_or_else(|| t!(invoke_role_inbox), |f| text::folder_label(lang, f))
                    .into(),
                ..head.clone()
            });
            items.push(head);
            for node in &nodes {
                let Some(f) = self
                    .folders
                    .get(node.index)
                    .filter(|f| f.account_id == a.id)
                else {
                    continue;
                };
                let (above, shown) = match node.parent {
                    Some(p) if p < n => (row[p], reach[p]),
                    _ => (line, open),
                };
                if !shown {
                    row[node.index] = above;
                    continue;
                }
                let folded = node.has_children && self.folded.contains(&Fold::Folder(f.id));
                row[node.index] = items.len();
                reach[node.index] = !folded;
                items.push(NavItem {
                    kind: 2,
                    id: f.id as i32,
                    label: text::folder_label(lang, f).into(),
                    icon: f.role.icon().into(),
                    unread: if folded {
                        sum.get(node.index).copied().unwrap_or(0)
                    } else {
                        count(f)
                    },
                    // The server's Drafts: its total, quietly (as the
                    // local Drafts shows its own), less the copies of
                    // local drafts, which the list leaves out.
                    count: if f.role == Role::Drafts {
                        let held: i64 = self.held.iter().filter_map(|h| h.total.get(&f.id)).sum();
                        let copies = self.db.draft_copies(&[f.id]).map_or(0, |c| c.len() as i64);
                        self.db
                            .count_messages(f.id)
                            .map_or(0, |n| (i64::from(n) - held - copies).max(0) as i32)
                    } else {
                        0
                    },
                    level: node.level as i32,
                    expanded: node.has_children && !folded,
                    has_children: node.has_children,
                    parent: above as i32,
                    account: a.id as i32,
                    sub: if node.depth > tree::MAX_LEVEL {
                        self.folder_path(&nodes, node.index).into()
                    } else {
                        SharedString::new()
                    },
                    off: !a.enabled,
                    ..Default::default()
                });
            }
        }
        // The pill: on the open view's row, or the line it is folded into.
        let pill = match self.view {
            View::AllInboxes => items.iter().position(|r| r.kind == 0),
            View::Drafts => items.iter().position(|r| r.kind == 3),
            View::Outbox => items.iter().position(|r| r.kind == 4),
            View::Folder(id) => self.folders.iter().position(|f| f.id == id).map(|i| row[i]),
        };
        let pill = pill.map_or(-1, |r| r as i32);
        // The rail's pill: on the open view's row, or the tile of the
        // account the open folder belongs to.
        let rail_pill = match self.view {
            View::AllInboxes => rail.iter().position(|r| r.kind == 0),
            View::Drafts => rail.iter().position(|r| r.kind == 3),
            View::Outbox => rail.iter().position(|r| r.kind == 4),
            View::Folder(id) => self.folders.iter().find(|f| f.id == id).and_then(|f| {
                rail.iter()
                    .position(|r| !r.initial.is_empty() && i64::from(r.account) == f.account_id)
            }),
        }
        .map_or(-1, |r| r as i32);
        let follow = std::mem::take(&mut self.nav_follow);
        let cursor = if in_rail {
            cursor_line
                .filter(|_| !follow)
                .and_then(|line| rail.iter().position(|r| (r.kind, r.id) == line))
                .map_or(rail_pill.max(0), |r| r as i32)
                .clamp(0, rail.len().saturating_sub(1) as i32)
        } else if follow {
            pill.max(0)
        } else {
            match cursor_line {
                Some(line) => match items.iter().position(|r| (r.kind, r.id) == line) {
                    Some(r) => r as i32,
                    // Folded away: the line it went into.
                    None if line.0 == 2 => self
                        .folders
                        .iter()
                        .position(|f| f.id == i64::from(line.1))
                        .map_or(pill.max(0), |i| row[i] as i32),
                    None => pill.max(0),
                },
                None => pill.max(0),
            }
        };
        let last = items.len().saturating_sub(1) as i32;
        // The words of the rail's fixed names: its width follows the
        // longest (account names may elide; these never do).
        let words: Vec<SharedString> = rail
            .iter()
            .filter(|r| r.initial.is_empty())
            .map(|r| r.label.to_string())
            .chain([
                t!(invoke_rail_new_mail),
                t!(invoke_rail_settings),
                t!(invoke_rail_folders),
            ])
            .flat_map(|l| {
                l.split_whitespace()
                    .map(SharedString::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        if ui.get_rail_words().iter().ne(words.iter().cloned()) {
            ui.set_rail_words(ModelRc::new(VecModel::from(words)));
        }
        update_model(&self.nav, items);
        update_model(&self.rail, rail);
        ui.set_nav_pill(pill);
        ui.set_rail_pill(rail_pill);
        ui.set_nav_cursor(cursor.clamp(0, last));
        if komorebi::dev_var("KOMOREBI_TIMING").is_some() {
            eprintln!(
                "[timing] rebuild_nav ({n} folders): {:.2} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Fold the long parts of each account's tree the first time its
    /// folders are here ([`tree::default_folds`]), once per account (the
    /// `nav_folds_init` setting); `unread` is the folders' unread mail.
    fn apply_default_folds(&mut self, unread: &HashMap<i64, i64>) {
        let ids: HashSet<i64> = self.accounts.iter().map(|a| a.id).collect();
        let mut changed = self.folds_init.iter().any(|id| !ids.contains(id));
        self.folds_init.retain(|id| ids.contains(id));
        let n = self.accounts.len();
        for (position, a) in self.accounts.iter().enumerate() {
            if self.folds_init.contains(&a.id) || !self.folders.iter().any(|f| f.account_id == a.id)
            {
                continue;
            }
            self.folded.extend(tree::default_folds(
                a.id,
                position,
                n,
                &self.folders,
                unread,
            ));
            self.folds_init.insert(a.id);
            changed = true;
        }
        // Written after the first frame, not on the way to it (two
        // commits, docs/PERF.md 6.3); the in-memory state is used meanwhile.
        if changed && !self.folds_unsaved {
            self.folds_unsaved = true;
            Timer::single_shot(START_SYNC_AFTER, || {
                with_app(|a| a.flush_default_folds());
            });
        }
    }

    /// Write the folds [`Self::apply_default_folds`] left unsaved.
    fn flush_default_folds(&mut self) {
        if std::mem::take(&mut self.folds_unsaved) {
            self.save_folds();
            self.db
                .set_setting("nav_folds_init", &tree::format_ids(&self.folds_init))
                .ok();
        }
    }

    /// With one account, All inboxes is not in the pane: its view moves to
    /// that account's Inbox once there is one.
    fn settle_view(&mut self) {
        if let ([one], View::AllInboxes) = (self.accounts.as_slice(), self.view)
            && let Some(inbox) = self
                .folders
                .iter()
                .find(|f| f.account_id == one.id && f.role == Role::Inbox)
        {
            let inbox = inbox.id;
            self.set_view(View::Folder(inbox));
        }
    }

    /// A folder's full path in the pane ("Projects / Alpha / ..."), for
    /// folders deeper than the levels drawn.
    fn folder_path(&self, nodes: &[tree::TreeNode], index: usize) -> String {
        let parent: HashMap<usize, usize> = nodes
            .iter()
            .filter_map(|n| Some((n.index, n.parent?)))
            .collect();
        let mut parts = Vec::new();
        let mut at = Some(index);
        while let Some(i) = at.filter(|_| parts.len() <= nodes.len()) {
            if let Some(f) = self.folders.get(i) {
                parts.push(text::folder_label(self.lang, f));
            }
            at = parent.get(&i).copied();
        }
        parts.reverse();
        parts.join(" / ")
    }

    /// Save the folded lines (a convenience: a failed write only loses
    /// them at the next start).
    fn save_folds(&self) {
        self.db
            .set_setting("nav_folded", &tree::format_folds(&self.folded))
            .ok();
    }

    /// Fold or open an account line or a parent folder. The view and the
    /// selected folder stay; the keyboard row stays on its line (or goes
    /// to the line it was folded into).
    fn nav_toggle(&mut self, item: NavItem) {
        let fold = match item.kind {
            1 => Fold::Account(i64::from(item.id)),
            2 if item.has_children => Fold::Folder(i64::from(item.id)),
            _ => return,
        };
        if !self.folded.remove(&fold) {
            self.folded.insert(fold);
        }
        self.save_folds();
        self.rebuild_nav();
    }

    fn nav_clicked(&mut self, item: NavItem) {
        match item.kind {
            0 => self.set_view(View::AllInboxes),
            3 => self.set_view(View::Drafts),
            4 => self.set_view(View::Outbox),
            1 => self.nav_toggle(item),
            // A rail tile of an account with no Inbox yet.
            2 if item.id <= 0 => {}
            _ => self.set_view(View::Folder(item.id as i64)),
        }
    }

    /// The folders a folder pane row's Mark all as read covers: every
    /// Inbox (All inboxes), the account's Inbox (as its folded count), or
    /// the folder itself.
    fn nav_menu_folders(&self, item: &NavItem) -> Vec<i64> {
        let inboxes = |account: Option<i64>| {
            self.folders
                .iter()
                .filter(|f| f.role == Role::Inbox && account.is_none_or(|a| f.account_id == a))
                .map(|f| f.id)
                .collect()
        };
        match item.kind {
            0 => inboxes(None),
            1 => inboxes(Some(i64::from(item.id))),
            2 if item.id > 0 => vec![i64::from(item.id)],
            _ => Vec::new(),
        }
    }

    /// Every parent folder, of every account or of one.
    fn parents_of(&self, account: Option<i64>) -> Vec<i64> {
        tree::tree(&self.folders)
            .iter()
            .filter(|n| n.has_children)
            .filter_map(|n| self.folders.get(n.index))
            .filter(|f| account.is_none_or(|a| f.account_id == a))
            .map(|f| f.id)
            .collect()
    }

    /// The folder pane's row menu for `item` (`menu:pane` and `menu:rail`
    /// print the same lists), as its two groups: none for a row without a
    /// menu (Drafts, Outbox). A rail tile goes to its account's Inbox, Sent
    /// or Archive; the other rows have Mark all as read (the same function
    /// as the list's "..." menu: no prompt, no Undo, as in Windows Mail)
    /// and Collapse all; an account line also Refresh and Account settings.
    /// Never "Empty folder" (it would expunge on the server).
    fn nav_menu(&self, item: &NavItem) -> Option<(Vec<MenuCommand>, Vec<MenuCommand>)> {
        let entry = |title: String, enabled: bool, command: i32| MenuCommand {
            title: title.into(),
            enabled,
            command,
            ..Default::default()
        };
        if item.kind == 2 && !item.initial.is_empty() {
            let account = i64::from(item.account);
            let go = |role: Role| {
                let folder = self
                    .folders
                    .iter()
                    .find(|f| f.account_id == account && f.role == role);
                MenuCommand {
                    title: folder
                        .map_or_else(|| role_label(role), |f| text::folder_label(self.lang, f))
                        .into(),
                    enabled: folder.is_some(),
                    command: NAV_OPEN,
                    id: folder.map_or(0, |f| f.id as i32),
                    ..Default::default()
                }
            };
            let main = vec![go(Role::Inbox), go(Role::Sent), go(Role::Archive)];
            return Some((main, self.fold_entry().into_iter().collect()));
        }
        if !matches!(item.kind, 0..=2) || (item.kind == 2 && item.id <= 0) {
            return None;
        }
        let unread = self.shown_unread();
        // A server Drafts folder's rows are never shown unread.
        let drafts = item.kind == 2
            && self
                .folders
                .iter()
                .any(|f| f.id == i64::from(item.id) && f.role == Role::Drafts);
        let has_unread = !drafts
            && self
                .nav_menu_folders(item)
                .iter()
                .any(|f| unread.get(f).is_some_and(|&n| n > 0));
        let account = (item.kind == 1).then(|| i64::from(item.id));
        // Nothing to collapse under a folded account line.
        let open_parent = account.is_none_or(|a| !self.folded.contains(&Fold::Account(a)))
            && self
                .parents_of(account)
                .iter()
                .any(|id| !self.folded.contains(&Fold::Folder(*id)));
        let collapse = if item.kind == 1 {
            entry(
                t!(invoke_menu_collapse_all_folders),
                open_parent,
                NAV_COLLAPSE,
            )
        } else {
            entry(t!(invoke_menu_collapse_all), open_parent, NAV_COLLAPSE)
        };
        let main = vec![
            entry(t!(invoke_menu_mark_all_read), has_unread, NAV_MARK_READ),
            collapse,
        ];
        let extra = if item.kind == 1 {
            vec![
                entry(t!(invoke_menu_refresh), !item.off, NAV_REFRESH),
                entry(t!(invoke_menu_account_settings), true, NAV_SETTINGS),
            ]
        } else {
            Vec::new()
        };
        let mut extra = extra;
        extra.extend(self.fold_entry());
        Some((main, extra))
    }

    /// The pane menu's Hide folders / Show folders, from 1040 px (below it
    /// the pane is always the rail, opened over the content by its toggle).
    fn fold_entry(&self) -> Option<MenuCommand> {
        let ui = self.ui.upgrade()?;
        if ui.get_compact() {
            return None;
        }
        let title = if ui.get_nav_collapsed() {
            t!(invoke_menu_show_folders)
        } else {
            t!(invoke_menu_hide_folders)
        };
        Some(MenuCommand {
            title: title.into(),
            enabled: true,
            command: NAV_FOLD,
            ..Default::default()
        })
    }

    /// Right-click, "..." or the Menu key on a folder pane row: puts its
    /// menu's entries in place; false for a row without a menu.
    fn nav_context(&mut self, item: &NavItem) -> bool {
        let Some((main, extra)) = self.nav_menu(item) else {
            return false;
        };
        let ui = self.ui();
        ui.set_nav_menu_main(ModelRc::new(VecModel::from(main)));
        ui.set_nav_menu_extra(ModelRc::new(VecModel::from(extra)));
        true
    }

    /// A command of the folder pane's menu on `item` ([`App::nav_menu`]).
    /// `id` is the folder [`NAV_OPEN`] goes to.
    fn nav_command(&mut self, item: &NavItem, command: i32, id: i32) {
        let account = i64::from(item.account);
        match command {
            NAV_REFRESH if item.kind == 1 => self.retry_account(account),
            NAV_MARK_READ => {
                let folders = self.nav_menu_folders(item);
                self.mark_folders_read(&folders);
            }
            NAV_COLLAPSE => {
                let only = (item.kind == 1).then_some(account);
                for id in self.parents_of(only) {
                    self.folded.insert(Fold::Folder(id));
                }
                self.save_folds();
                self.rebuild_nav();
            }
            NAV_SETTINGS if item.kind == 1 => self.edit_account(account),
            NAV_OPEN if id > 0 => self.set_view(View::Folder(i64::from(id))),
            _ => {}
        }
    }

    /// The list's "..." menu, for the view shown (`menu:list` prints the
    /// same list): Mark all as read. Not
    /// while a search is showing: it would mark the whole view read, not
    /// the results listed (the folder pane's row menu still offers it).
    fn list_menu(&self) -> Vec<MenuCommand> {
        let unread = self.shown_unread();
        let has_unread = self
            .view_folders()
            .iter()
            .any(|f| unread.get(f).is_some_and(|&n| n > 0));
        // Not in a server Drafts folder: its rows are never shown unread.
        let drafts = matches!(self.view, View::Folder(_)) && self.view_role() == Role::Drafts;
        vec![
            MenuCommand {
                title: t!(invoke_menu_mark_all_read).into(),
                enabled: has_unread && self.lists_messages() && !drafts && !self.searching(),
                command: LIST_MARK_READ,
                ..Default::default()
            },
            // Check boxes on the rows; checked while they show.
            MenuCommand {
                title: t!(invoke_menu_select_messages).into(),
                enabled: self.lists_messages(),
                command: LIST_SELECT,
                checkable: true,
                checked: self.select_mode,
                ..Default::default()
            },
        ]
    }

    fn list_menu_opening(&self) {
        self.ui()
            .set_list_menu(ModelRc::new(VecModel::from(self.list_menu())));
    }

    fn list_command(&mut self, command: i32) {
        match command {
            LIST_MARK_READ => self.mark_all_read(),
            LIST_SELECT => self.set_select_mode(!self.select_mode),
            _ => {}
        }
    }

    fn set_view(&mut self, view: View) {
        // The folders belong to the Mail tab.
        self.go_to_mail_tab();
        if self.view == view {
            return;
        }
        self.view = view;
        // The query stays and runs in the new view (docs/WINUI3.md
        // "AutoSuggestBox"); only the scope goes back to the view.
        self.search_timer.stop();
        self.filter = Filter::All;
        self.search_all = false;
        self.kept_unread.clear();
        self.set_select_mode(false);
        let ui = self.ui();
        ui.set_list_filter(0);
        ui.set_search_all(false);
        ui.set_selected_folder(view.code());
        self.clear_popped();
        self.close_reader();
        self.outbox_open = None;
        ui.set_o_show(false);
        // The next start opens here (a convenience: a failed write only
        // loses it).
        self.db.set_setting("last_view", &view.key()).ok();
        self.reload_rows();
        // The Outbox entry stays only while it holds mail; the keyboard
        // row follows the pill.
        self.nav_follow = true;
        self.rebuild_nav();
        ui.invoke_focus_list();
    }

    fn view_folders(&self) -> Vec<i64> {
        match self.view {
            View::AllInboxes => self
                .folders
                .iter()
                .filter(|f| f.role == Role::Inbox)
                .map(|f| f.id)
                .collect(),
            View::Folder(id) => vec![id],
            View::Drafts | View::Outbox => Vec::new(),
        }
    }

    /// A search of every cached folder is running (the "All folders" scope
    /// with search text, in a view of messages).
    fn searching_all(&self) -> bool {
        self.search_all && self.lists_messages() && !self.search.trim().is_empty()
    }

    /// The folders the list shows: the view's, or every folder while
    /// searching all of them. Folder actions (Mark all as read) keep to
    /// [`App::view_folders`].
    fn list_folders(&self) -> Vec<i64> {
        if self.searching_all() {
            self.folders.iter().map(|f| f.id).collect()
        } else {
            self.view_folders()
        }
    }

    fn view_role(&self) -> Role {
        match self.view {
            View::AllInboxes => Role::Inbox,
            View::Drafts => Role::Drafts,
            View::Outbox => Role::Sent,
            View::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map_or(Role::Other, |f| f.role),
        }
    }

    // ------------------------------------------------------ message list

    fn reload_rows(&mut self) {
        let started = Instant::now();
        let (ids, kind) = match self.view {
            View::Drafts => (self.db.draft_ids(&self.search), rows::Kind::Drafts),
            View::Outbox => (self.db.outbox_ids(&self.search), rows::Kind::Outbox),
            _ => (
                self.list_ids(),
                if self.searching_all() {
                    self.rows.set_folders(
                        self.folders
                            .iter()
                            .map(|f| (f.id, (self.folder_tag(f).into(), f.role.is_outgoing())))
                            .collect(),
                    );
                    rows::Kind::Mixed
                } else if self.view_role().is_outgoing() {
                    rows::Kind::Outgoing
                } else {
                    rows::Kind::Received
                },
            ),
        };
        if !self.search.trim().is_empty() && komorebi::dev_var("KOMOREBI_TIMING").is_some() {
            eprintln!(
                "[timing] search: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        let mut ids = ids.unwrap_or_else(|e| {
            self.set_status(&text::detail(&e), true);
            Vec::new()
        });
        if let Some(held) = self.held.as_ref().filter(|_| self.lists_messages()) {
            ids.retain(|id| !held.ids.contains(id));
        }
        // The server's copies of local drafts: the Drafts entry lists the
        // drafts themselves, so none shows twice.
        let drafts_folders: Vec<i64> = if self.lists_messages() {
            let shown = self.list_folders();
            self.folders
                .iter()
                .filter(|f| f.role == Role::Drafts && shown.contains(&f.id))
                .map(|f| f.id)
                .collect()
        } else {
            Vec::new()
        };
        if let Ok(copies) = self.db.draft_copies(&drafts_folders)
            && !copies.is_empty()
        {
            ids.retain(|id| !copies.contains(id));
        }
        // With more than one account, All inboxes tags each row with its
        // account's name (docs/WINUI3.md "ListView item").
        let tags = if self.view == View::AllInboxes && self.accounts.len() > 1 {
            self.accounts
                .iter()
                .map(|a| {
                    let name = match a.name.trim() {
                        "" => a.email.as_str(),
                        name => name,
                    };
                    (a.id, SharedString::from(name))
                })
                .collect()
        } else {
            HashMap::new()
        };
        self.rows.set_account_tags(tags);
        self.ui()
            .set_result_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
        let shown = ids.len();
        self.rows.set(ids, kind);
        self.rows_shown(shown);

        self.show_list_title();
        self.show_empty();
        self.sync_selection_index();
    }

    /// The view's message ids under the filter and the search; in the
    /// Unread filter, the messages opened there stay (read by now).
    fn list_ids(&self) -> anyhow::Result<Vec<i64>> {
        let folders = self.list_folders();
        if self.filter != Filter::Unread || self.kept_unread.is_empty() {
            return self
                .db
                .message_ids_filtered(&folders, &self.search, self.filter);
        }
        let unread: HashSet<i64> = self
            .db
            .message_ids_filtered(&folders, &self.search, Filter::Unread)?
            .into_iter()
            .collect();
        let mut ids = self
            .db
            .message_ids_filtered(&folders, &self.search, Filter::All)?;
        ids.retain(|id| unread.contains(id) || self.kept_unread.contains(id));
        Ok(ids)
    }

    /// A row's tag in a search of all folders: "Account · Folder".
    fn folder_tag(&self, f: &Folder) -> String {
        let folder = text::folder_label(self.lang, f);
        match self.accounts.iter().find(|a| a.id == f.account_id) {
            Some(a) => {
                let name = match a.name.trim() {
                    "" => a.email.as_str(),
                    name => name,
                };
                t!(invoke_account_folder, name.into(), folder.into())
            }
            None => folder,
        }
    }

    /// A query is in the search box (it runs in the view, or in every
    /// folder with the "All folders" scope).
    fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    /// The list's title: the view, or "Search results" while a query runs
    /// (the scope is said by the results line and the scope control; the
    /// pane's pill stays on the view). The search box's placeholder names
    /// the view.
    fn show_list_title(&self) {
        let view = match self.view {
            View::AllInboxes => t!(invoke_all_inboxes),
            View::Drafts => t!(invoke_drafts),
            View::Outbox => t!(invoke_outbox),
            View::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map(|f| text::folder_label(self.lang, f))
                .unwrap_or_default(),
        };
        let ui = self.ui();
        let title = if self.searching() {
            t!(invoke_search_results)
        } else {
            view.clone()
        };
        ui.set_list_title(title.into());
        ui.set_search_view(view.into());
        // A folder's account, under its name (several accounts can each
        // have an Inbox or a Sent).
        let account = match self.view {
            View::Folder(id) if !self.searching() => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| self.accounts.iter().find(|a| a.id == f.account_id))
                .map(|a| match a.name.trim() {
                    "" => a.email.clone(),
                    name => name.to_string(),
                }),
            _ => None,
        };
        ui.set_list_subtitle(account.unwrap_or_default().into());
        // A server Drafts folder: its messages are drafts, not mail to
        // triage (no filters, no unread bold).
        ui.set_list_drafts(
            matches!(self.view, View::Folder(_)) && self.view_role() == Role::Drafts,
        );
    }

    /// What the list says while it is empty.
    fn show_empty(&self) {
        let folders = self.view_folders();
        let failed = self
            .folders
            .iter()
            .filter(|f| folders.contains(&f.id))
            .filter_map(|f| self.account_state.get(&f.account_id))
            .find(|s| matches!(s, AccountState::Unreachable | AccountState::Failed(_)));
        let syncing = !self.syncing.is_empty();
        // The accounts the view shows (All inboxes: every one, even before
        // its folders arrive).
        let shown: Vec<i64> = match self.view {
            View::AllInboxes => self.accounts.iter().map(|a| a.id).collect(),
            _ => self
                .folders
                .iter()
                .filter(|f| folders.contains(&f.id))
                .map(|f| f.account_id)
                .collect(),
        };
        let fresh = shown
            .iter()
            .find(|id| self.first_sync.contains(id))
            .map(|id| self.progress.get(id).copied().unwrap_or((0, 0)));
        let synced = matches!(self.view, View::Folder(_))
            && shown
                .iter()
                .any(|id| matches!(self.account_state.get(id), Some(AccountState::Checked(_))));
        let state = ListState {
            syncing,
            failed,
            fresh: fresh.filter(|_| self.search.trim().is_empty() && self.filter == Filter::All),
            synced,
        };
        let text = empty_text(self.view, &self.search, self.filter, &state, self.lang);
        let ui = self.ui();
        ui.set_list_empty(text.into());
        let (kind, value) = match state.fresh {
            None => (0, 0.0),
            Some((_, 0)) => (1, 0.0),
            Some((done, total)) => (2, done as f32 / total as f32),
        };
        ui.set_list_fresh(kind);
        ui.set_list_fresh_value(value);
    }

    /// The Unread (1) / Flagged (2) toggles; 0 shows everything.
    fn set_filter(&mut self, filter: i32) {
        if !self.lists_messages() {
            return;
        }
        self.filter = match filter {
            1 => Filter::Unread,
            2 => Filter::Flagged,
            _ => Filter::All,
        };
        self.kept_unread.clear();
        self.ui().set_list_filter(filter.clamp(0, 2));
        self.reload_rows();
    }

    /// The search scope toggle: every cached folder of every account, or
    /// the view shown. It stays until the view changes.
    fn set_search_all(&mut self, all: bool) {
        if !self.lists_messages() {
            return;
        }
        self.search_all = all;
        self.ui().set_search_all(all);
        self.search_timer.stop();
        self.reload_rows();
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.search_timer.stop();
        self.ui().set_search_text(SharedString::new());
        self.reload_rows();
    }

    /// "Show in folder" (the row menu while searching): the search clears
    /// and the message opens in its own folder, among its neighbours.
    fn show_in_folder(&mut self) {
        if !self.reading() || !self.lists_messages() {
            return;
        }
        let [target] = self.targets()[..] else {
            return;
        };
        self.search.clear();
        self.search_timer.stop();
        self.search_all = false;
        let ui = self.ui();
        ui.set_search_text(SharedString::new());
        ui.set_search_all(false);
        let view = View::Folder(target.folder_id);
        if self.view == view {
            self.reload_rows();
        } else {
            self.set_view(view);
        }
        self.select_message(target.id);
    }

    /// Esc in the list: clears the search, else the selection and the
    /// message open, else the status line.
    fn escape(&mut self) {
        // A message tab never closes on Esc (it would be lost by accident).
        if !self.reading() || !self.in_mail_tab() {
            return;
        }
        if !self.search.is_empty() || !self.ui().get_search_text().is_empty() {
            return self.clear_search();
        }
        // With nothing open or selected, the status line goes; never one
        // with an offer (Undo, Resume draft), which Esc would take away.
        let ui = self.ui();
        if self.open.is_none()
            && ui.get_selected_count() == 0
            && !ui.get_status().is_empty()
            && ui.get_status_action().is_empty()
        {
            return self.dismiss_status();
        }
        // "Select messages" goes with the selection.
        if self.select_mode && self.open.is_none() && ui.get_selected_count() == 0 {
            return self.set_select_mode(false);
        }
        self.back_to_list();
    }

    /// The reader's back button (one pane below 720 px), and Esc's last
    /// step: the open Outbox entry or message closes and the list takes
    /// the focus. Nothing on the server changes.
    fn back_to_list(&mut self) {
        // From a message tab (one pane): back to the Mail tab.
        if !self.in_mail_tab() {
            return self.go_to_mail_tab();
        }
        if !self.reading() {
            return;
        }
        if self.outbox_open.take().is_some() {
            let ui = self.ui();
            ui.set_o_show(false);
            self.sync_selection_index();
            ui.invoke_focus_list();
            return;
        }
        self.close_reader();
    }

    /// The list's Mark all as read: every message of the view.
    fn mark_all_read(&mut self) {
        if !self.reading() || !self.lists_messages() {
            return;
        }
        let folders = self.view_folders();
        self.mark_folders_read(&folders);
    }

    /// Mark every message of these folders read (whatever view is open):
    /// in the cache at once, then one job per folder (one STORE, up to the
    /// highest UID cached, so mail that arrived since the last sync stays
    /// unread).
    fn mark_folders_read(&mut self, folders: &[i64]) {
        for &folder_id in folders {
            let Some(account) = self
                .folders
                .iter()
                .find(|f| f.id == folder_id)
                .map(|f| f.account_id)
            else {
                continue;
            };
            // On an error the folders marked so far still get their
            // counts and rows refreshed below.
            let (uids, through) = match self.db.mark_folder_read(folder_id) {
                Ok(marked) => marked,
                Err(e) => {
                    self.set_status(&text::detail(&e), true);
                    break;
                }
            };
            if !uids.is_empty()
                && let Some(w) = self.workers.get(&account)
            {
                w.send(Job::MarkAllRead {
                    folder_id,
                    through,
                    uids,
                });
            }
        }
        // The open message, when it was among them, is read now.
        if let Some(open) = self
            .open
            .as_mut()
            .filter(|o| folders.contains(&o.msg.folder_id))
        {
            open.msg.seen = true;
            self.read_wait.cancel();
            self.read_timer.stop();
            self.ui().set_r_unread(false);
        }
        self.rebuild_nav();
        // Asked for: under Unread the list empties now.
        self.reload_rows();
    }

    /// Filter the list as the search text changes. Large scopes are
    /// searched once typing pauses; small ones and a cleared search at once.
    fn search_edited(&mut self, text: &str) {
        self.search = text.to_string();
        let scope = self
            .list_folders()
            .iter()
            .map(|&f| self.db.count_messages(f).unwrap_or(0))
            .sum();
        if !debounces_search(scope, text) {
            self.search_timer.stop();
            self.reload_rows();
            return;
        }
        // Restarting a running timer pushes the search past the next pause.
        self.search_timer
            .start(TimerMode::SingleShot, SEARCH_DEBOUNCE, || {
                with_app(|a| a.reload_rows());
            });
    }

    fn schedule_reload(&self) {
        if !self.reload_timer.running() {
            self.reload_timer
                .start(TimerMode::SingleShot, RELOAD_DEBOUNCE, || {
                    with_app(|a| {
                        a.rebuild_nav();
                        a.reload_rows();
                        a.refresh_message_tabs();
                        a.refresh_popouts();
                    });
                });
        }
    }

    /// Mark the open message's row, or in Drafts the row of the draft the
    /// composer shows.
    fn sync_selection_index(&self) {
        let ui = self.ui();
        let id = match self.view {
            View::Drafts => self
                .drafts
                .active()
                .filter(|_| ui.get_composing())
                .and_then(|d| d.row),
            View::Outbox => self.outbox_open,
            _ => self.open.as_ref().map(|o| o.msg.id),
        };
        let selection = self.rows.selection();
        // Drafts and Outbox rows are picked one at a time, never shown as
        // a selection of messages.
        let count = if self.lists_messages() {
            selection.len()
        } else {
            0
        };
        ui.set_selected_count(count as i32);
        ui.set_selected_id(id.map_or(-1, |id| id as i32));
        // The list follows the open row, or the end of a Shift range.
        let index = id
            .or(selection.cursor())
            .and_then(|id| self.rows.index_of(id))
            .map_or(-1, |i| i as i32);
        ui.set_selected_index(index);
    }

    /// Set a flag on messages in the cache (those without it already),
    /// refresh their rows and the reader, and queue the change for the
    /// server: one job per folder.
    fn set_flags(&mut self, targets: &[db::Brief], flag: Flag, on: bool) {
        let changing: Vec<&db::Brief> = targets.iter().filter(|m| flag.of(m) != on).collect();
        if changing.is_empty() {
            return;
        }
        let ids: Vec<i64> = changing.iter().map(|m| m.id).collect();
        let result = match flag {
            Flag::Seen => self.db.set_seen_many(&ids, on),
            Flag::Flagged => self.db.set_flagged_many(&ids, on),
        };
        if let Err(e) = result {
            return self.set_status(&text::detail(&e), true);
        }
        for ((account, folder_id), group) in by_folder(&changing) {
            if let Some(w) = self.workers.get(&account) {
                w.send(Job::SetFlags {
                    folder_id,
                    uids: group.iter().map(|m| m.uid).collect(),
                    flag: flag.name(),
                    on,
                });
            }
        }
        let ids: HashSet<i64> = ids.into_iter().collect();
        self.rows.changed_many(&ids);
        if let Some(open) = self.open.as_mut().filter(|o| ids.contains(&o.msg.id)) {
            let ui = self.ui.upgrade();
            match flag {
                Flag::Seen => {
                    open.msg.seen = on;
                    ui.inspect(|ui| ui.set_r_unread(!on));
                    // Marked by hand: the delayed mark must not undo it.
                    self.read_wait.cancel();
                    self.read_timer.stop();
                }
                Flag::Flagged => {
                    open.msg.flagged = on;
                    ui.inspect(|ui| ui.set_r_flagged(on));
                }
            }
        }
        if matches!(flag, Flag::Seen) {
            self.rebuild_nav();
            self.refresh_message_tabs();
        }
        self.popouts_flags_changed(&ids);
    }

    /// What an action applies to: the open message, or with none open the
    /// selected rows (several, or one picked with Ctrl or Shift).
    fn targets(&self) -> Vec<db::Brief> {
        if let Some(open) = &self.open {
            return vec![db::Brief::from(&open.msg)];
        }
        let ids = self.rows.selection().ids();
        self.db.briefs(&ids).unwrap_or_else(|e| {
            self.set_status(&text::detail(&e), true);
            Vec::new()
        })
    }

    /// The views whose rows are messages (and so can be multi-selected).
    fn lists_messages(&self) -> bool {
        matches!(self.view, View::AllInboxes | View::Folder(_))
    }

    /// Ctrl+click or Shift+click on row `id`. Drafts and Outbox rows, and
    /// a click while the composer shows, open as a plain click would.
    fn change_selection(&mut self, id: i64, f: impl FnOnce(&mut Selection, &[i64])) {
        if !self.reading() || !self.lists_messages() {
            return self.row_clicked(id);
        }
        self.select_rows(f);
    }

    /// Change the selection (Ctrl/Shift+click, Shift+Up/Down, Ctrl+A). The
    /// open message stays only while it is all that is selected; otherwise
    /// the reader pane shows the selection and its actions.
    fn select_rows(&mut self, f: impl FnOnce(&mut Selection, &[i64])) {
        if !self.reading() || !self.lists_messages() {
            return;
        }
        self.rows.select(f);
        let keep = {
            let selection = self.rows.selection();
            self.open
                .as_ref()
                .is_some_and(|o| selection.len() == 1 && selection.contains(o.msg.id))
        };
        if !keep && self.open.take().is_some() {
            self.reader.close();
            let ui = self.ui();
            ui.set_has_message(false);
            // The page may have had the focus: the list keeps the keys.
            ui.invoke_focus_list();
        }
        if self.open.is_none() {
            self.show_move_targets();
        }
        self.sync_selection_index();
    }

    /// A row was clicked: a message opens in the reader, a saved draft in
    /// the composer.
    fn row_clicked(&mut self, id: i64) {
        match self.view {
            View::Drafts => self.open_saved(id),
            View::Outbox => self.open_outbox(id),
            _ => self.select_message(id),
        }
    }

    /// Up/Down and PageUp/PageDown: open the message `delta` rows away
    /// (with none open, the first).
    fn select_offset(&mut self, delta: i32) {
        self.select_by(|current, last| match current {
            Some(i) => i.saturating_add_signed(delta as isize).min(last),
            None => 0,
        });
    }

    /// Home / End: open the first or the last message.
    fn select_edge(&mut self, end: bool) {
        self.select_by(|_, last| if end { last } else { 0 });
    }

    /// Open the row `pick` chooses, given the current row's index (the
    /// open message, or the selection's cursor) and the last index.
    fn select_by(&mut self, pick: impl FnOnce(Option<usize>, usize) -> usize) {
        if !self.reading() {
            return;
        }
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        let current = self.current_row().and_then(|id| self.rows.index_of(id));
        let next = pick(current, last);
        if Some(next) == current {
            return;
        }
        let Some(id) = self.rows.id_at(next) else {
            return;
        };
        // One pane at a time (below 720 px) with the list showing: the
        // arrows only move the cursor, as opening would hide the list under
        // the keys; Enter or a click opens the row. With a message or an
        // Outbox entry open, they still step to the next one.
        let ui = self.ui();
        if ui.get_one_pane() && self.lists_messages() && self.open.is_none()
            || ui.get_one_pane() && self.view == View::Outbox && self.outbox_open.is_none()
        {
            self.rows.select(|s, _| s.single(id));
            self.sync_selection_index();
            return;
        }
        match self.view {
            // A draft opens in the composer, which takes the keys: the
            // arrows only move to it, and Enter opens it.
            View::Drafts => {
                self.rows.select(|s, _| s.single(id));
                self.sync_selection_index();
            }
            // An Outbox row shows in the reader pane, as a message would.
            View::Outbox => self.open_outbox(id),
            _ => self.select_message(id),
        }
    }

    /// The open message, else the row Shift+Up/Down move from.
    fn current_row(&self) -> Option<i64> {
        match self.view {
            View::Outbox => self.outbox_open.or(self.rows.selection().cursor()),
            _ => self
                .open
                .as_ref()
                .map(|o| o.msg.id)
                .or(self.rows.selection().cursor()),
        }
    }

    /// Enter in the list: open the current row (with none, the first).
    /// True when a message shows, so the page can take the focus.
    fn open_current(&mut self) -> bool {
        if !self.reading() {
            return false;
        }
        if !self.lists_messages() {
            let listed = self
                .current_row()
                .filter(|&id| self.rows.index_of(id).is_some());
            if let Some(id) = listed.or_else(|| self.rows.id_at(0)) {
                self.row_clicked(id);
            }
            // The composer or the entry's buttons have what they need.
            return false;
        }
        let listed = self
            .current_row()
            .filter(|&id| self.rows.index_of(id).is_some());
        if let Some(id) = listed.or_else(|| self.rows.id_at(0))
            && self.open.as_ref().is_none_or(|o| o.msg.id != id)
        {
            self.select_message(id);
        }
        self.open.is_some()
    }

    // ------------------------------------------------------------ reader

    fn select_message(&mut self, id: i64) {
        self.clear_popped();
        let ui = self.ui();
        if ui.get_composing() {
            if self.in_mail_tab() {
                // A draft with something written moves to a tab of its
                // own (saved here and on the server); an empty one goes.
                if self.pane_draft_aside() {
                    self.set_status(&t!(invoke_draft_kept_in_tab), false);
                }
            } else {
                // Parked as Esc and Close park it: saved here and on the
                // server; Drafts brings it back.
                self.park_composer();
            }
            ui.set_composing(false);
        }
        if !self.load_message(id) {
            return;
        }
        if self.filter == Filter::Unread && self.lists_messages() {
            self.kept_unread.insert(id);
        }
        self.rows.select(|s, _| s.single(id));
        self.show_open();
        self.sync_selection_index();
    }

    /// Make message `id` the open one: the reader shows it (its body, or a
    /// download of it), and its read delay starts. The list's selection is
    /// left alone (a tab shows it). False when it is not cached.
    fn load_message(&mut self, id: i64) -> bool {
        let ui = self.ui();
        let mut msg = match self.db.message(id) {
            Ok(Some(m)) => m,
            Ok(None) => return false,
            Err(e) => {
                self.set_status(&text::detail(&e), true);
                return false;
            }
        };
        // The "will always load" bar belongs to the message it was
        // pressed on (also when the next one isn't downloaded yet).
        if self.open.as_ref().is_none_or(|o| o.msg.id != id) {
            ui.set_r_remote_always(SharedString::new());
            ui.set_r_remote_allowed(false);
        }
        match self.read_wait.opened(id, !msg.seen, self.mark_read) {
            Some(Duration::ZERO) => {
                self.set_flags(&[db::Brief::from(&msg)], Flag::Seen, true);
                msg.seen = true;
            }
            Some(delay) => self.read_timer.start(TimerMode::SingleShot, delay, || {
                with_app(|a| a.read_due());
            }),
            None => self.read_timer.stop(),
        }
        let needs_fetch = msg.raw.is_none();
        let account_id = msg.account_id;
        if self.open.as_ref().is_none_or(|o| o.msg.id != id) {
            ui.set_r_content_y(0.0);
            // "View as plain text" lasts for one message, and so does
            // "Change" on an answered invitation.
            ui.set_r_plain(false);
            self.invite_change = false;
        }
        self.open = Some(Open { msg, body: None });
        if needs_fetch {
            self.reader.close();
            if let Some(w) = self.workers.get(&account_id) {
                w.send(Job::FetchBody(id));
            }
        } else {
            self.load_open_body();
        }
        self.move_filter.clear();
        self.show_move_targets();
        true
    }

    /// Show the open message's body: text at once, or HTML through the
    /// render worker (which also parses large messages off this thread).
    fn load_open_body(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(raw) = open.msg.raw.as_deref() else {
            return;
        };
        let id = open.msg.id;
        // Keyed on the From address alone, never the display name.
        let remote = self.db.remote_allowed(&open.msg.from_addr).unwrap_or(false);
        // The caption under an allowed sender's message.
        if let Some(ui) = self.ui.upgrade() {
            ui.set_r_remote_allowed(remote);
        }
        if raw.len() > SYNC_PARSE_MAX {
            let src = reader::Source::Raw(raw.into());
            self.reader.open(id, src, remote);
            return;
        }
        let (body, input) = mime::parse_reader(raw);
        let backfill = backfill_importance(&mut open.msg, &body);
        open.body = Some(body);
        if input.prefers_plain {
            self.reader.close();
        } else {
            let src = reader::Source::Input(Arc::new(input));
            self.reader.open(id, src, remote);
        }
        self.keep_importance(backfill);
    }

    /// A body cached before v13 told the open message's importance
    /// ([`backfill_importance`]): its row keeps it (one row, only this
    /// once) and the list shows it.
    fn keep_importance(&self, backfill: Option<(i64, db::Importance)>) {
        let Some((id, importance)) = backfill else {
            return;
        };
        if let Err(e) = self.db.set_importance(id, importance) {
            self.set_status(&text::detail(&e), true);
        }
        self.rows.changed(id);
    }

    /// Load the open message's remote images: this once, or from now on
    /// for its sender's exact address.
    fn load_remote(&mut self, always: bool) {
        let Some(open) = &self.open else {
            return;
        };
        if always {
            let addr = open.msg.from_addr.clone();
            if let Err(e) = self.db.allow_remote(&addr) {
                return self.set_status(&text::detail(&e), true);
            }
            self.show_remote_senders();
            // Said, with Undo and the setting, until closed or another
            // message opens.
            self.ui().set_r_remote_always(addr.as_str().into());
        }
        self.reader.allow_remote();
    }

    /// Undo on "Images from … will always load": the sender goes back to
    /// blocked from the next message on (what loaded stays on screen).
    fn undo_remote_always(&mut self) {
        let ui = self.ui();
        let addr = ui.get_r_remote_always().to_string();
        ui.set_r_remote_always(SharedString::new());
        if addr.is_empty() {
            return;
        }
        if let Err(e) = self.db.forget_remote(&addr) {
            return self.set_status(&text::detail(&e), true);
        }
        self.show_remote_senders();
        ui.set_r_remote_allowed(false);
    }

    /// Fill the Settings list of senders whose remote images always load.
    fn show_remote_senders(&self) {
        let senders: Vec<SharedString> = match self.db.remote_allow_list() {
            Ok(list) => list.into_iter().map(SharedString::from).collect(),
            Err(e) => return self.set_status(&text::detail(&e), true),
        };
        self.ui()
            .set_remote_senders(ModelRc::new(VecModel::from(senders)));
    }

    fn on_render(&mut self, reply: reader::Reply) {
        if let Some(body) = self.reader.on_reply(reply)
            && let Some(open) = self.open.as_mut()
            && open.body.is_none()
        {
            let backfill = backfill_importance(&mut open.msg, &body);
            open.body = Some(body);
            self.keep_importance(backfill);
            self.show_open();
        }
    }

    fn show_open(&self) {
        let ui = self.ui();
        let Some(open) = &self.open else {
            ui.set_has_message(false);
            return;
        };
        self.fill_reader(&ui, open);
        self.show_draft_reply();
        self.show_invite();
    }

    /// The reader's header, notices and text for `open`, in `ui` (the
    /// main window or a message window).
    fn fill_reader(&self, ui: &AppWindow, open: &Open) {
        let m = &open.msg;
        ui.set_has_message(true);
        ui.set_r_subject(m.subject.as_str().into());
        ui.set_r_from_name(format::sender(&m.from_name, &m.from_addr).into());
        ui.set_r_from_addr(m.from_addr.as_str().into());
        ui.set_r_initials(format::initials(&m.from_name, &m.from_addr).into());
        ui.set_r_avatar(format::avatar_color(&m.from_addr));
        ui.set_r_date(format::reader_date(self.lang, m.date).into());
        ui.set_r_to(format::recipients(&m.to).into());
        ui.set_r_cc(format::recipients(&m.cc).into());
        ui.set_r_flagged(m.flagged);
        ui.set_r_unread(!m.seen);
        ui.set_r_importance(m.importance.code() as i32);
        ui.set_r_server_draft(self.folder_role(m.folder_id) == Role::Drafts);
        ui.set_r_in_junk(self.folder_role(m.folder_id) == Role::Junk);
        ui.set_loading(m.raw.is_none());
        ui.set_r_unreachable(m.raw.is_none() && self.unreachable(m.account_id));
        match &open.body {
            Some(body) => {
                let text = &body.text;
                ui.set_r_body(if text.len() > MAX_BODY_BYTES {
                    let cut = text.floor_char_boundary(MAX_BODY_BYTES);
                    format!("{}\n\n{}", &text[..cut], t!(invoke_body_cut_note)).into()
                } else {
                    text.as_str().into()
                });
                let atts: Vec<AttachmentRow> = body
                    .attachments
                    .iter()
                    .map(|a| AttachmentRow {
                        index: a.index as i32,
                        name: a.name.as_str().into(),
                        size: format::size(self.lang, a.size).into(),
                        kind: format::file_kind(&a.name),
                    })
                    .collect();
                ui.set_r_attachments(ModelRc::new(VecModel::from(atts)));
                let host = self
                    .accounts
                    .iter()
                    .find(|a| a.id == m.account_id)
                    .map_or("", |a| a.imap.host.as_str());
                let reasons = trust::reasons(&body.auth, host, &m.from_name, &m.from_addr);
                ui.set_r_notice(text::trust_notice(&reasons).into());
                let crypt = body
                    .encrypted
                    .map(|kind| t!(invoke_encrypted_notice, kind.label().into()));
                ui.set_r_crypt(crypt.unwrap_or_default().into());
                ui.set_r_unsubscribe(!body.unsubscribe.is_empty());
            }
            None => {
                ui.set_r_body(SharedString::new());
                ui.set_r_attachments(ModelRc::default());
                ui.set_r_notice(SharedString::new());
                ui.set_r_crypt(SharedString::new());
                ui.set_r_unsubscribe(false);
            }
        }
    }

    /// Unsubscribe from the open message's list: a `mailto:` address opens
    /// the composer prefilled, an `https:` one the confirm prompt before
    /// the browser. Nothing is sent or fetched here.
    fn unsubscribe(&mut self) {
        if !self.reading() {
            return;
        }
        let uri = self
            .open
            .as_ref()
            .and_then(|o| o.body.as_ref())
            .map(|b| b.unsubscribe.clone())
            .unwrap_or_default();
        if !uri.is_empty() {
            self.reader.follow(uri);
        }
    }

    /// The open message has been open long enough: it is marked read.
    fn read_due(&mut self) {
        let open = self.open.as_ref().map(|o| o.msg.id);
        if self.read_wait.due(open).is_some()
            && let Some(open) = &self.open
        {
            let brief = db::Brief::from(&open.msg);
            self.set_flags(&[brief], Flag::Seen, true);
        }
    }

    fn close_reader(&mut self) {
        self.open = None;
        self.read_wait.cancel();
        self.read_timer.stop();
        self.rows.select(|s, _| s.clear());
        self.reader.close();
        let ui = self.ui();
        ui.set_has_message(false);
        // The page may have had the focus: the list keeps the keys.
        if self.reading() {
            ui.invoke_focus_list();
        }
        self.sync_selection_index();
    }

    /// All read if any is unread, else all unread (the reader bar's old
    /// toggle; Ctrl+Q and Ctrl+U now mark read and unread).
    fn toggle_read(&mut self) {
        if !self.reading() {
            return;
        }
        let targets = self.targets();
        let seen = targets.iter().any(|m| !m.seen);
        self.set_flags(&targets, Flag::Seen, seen);
    }

    /// All flagged if any isn't, else none.
    fn toggle_flag(&mut self) {
        if !self.reading() {
            return;
        }
        let targets = self.targets();
        let flagged = targets.iter().any(|m| !m.flagged);
        self.set_flags(&targets, Flag::Flagged, flagged);
    }

    /// The selection's Mark read / unread and Flag / Unflag buttons.
    fn mark(&mut self, flag: Flag, on: bool) {
        if self.reading() {
            let targets = self.targets();
            self.set_flags(&targets, flag, on);
        }
    }

    fn archive(&mut self) {
        self.remove_to(Dest::Archive);
    }

    fn delete(&mut self) {
        // A draft or Outbox row of the list (never from a message tab,
        // whose message is no row of the Mail tab's Drafts or Outbox).
        if let Some(row) = self.current_row().filter(|&id| {
            self.in_mail_tab() && !self.lists_messages() && self.rows.index_of(id).is_some()
        }) {
            return self.discard_row(row);
        }
        let targets = self.targets();
        self.delete_targets(targets);
    }

    fn move_to(&mut self, dest: i64) {
        if self.reading() {
            self.note_recent(dest);
        }
        self.remove_to(Dest::Folder(dest));
    }

    /// Move the open message or the selection out of its folders (one job
    /// per folder) and open the message that took its place. The list
    /// changes at once; the server jobs wait out the undo window. Messages
    /// already where they would go stay.
    fn remove_to(&mut self, dest: Dest) {
        let targets = self.targets();
        self.remove_targets(dest, targets);
    }

    /// [`App::remove_to`] on these messages (a row's quick action: that
    /// row only, whatever is open or selected).
    fn remove_targets(&mut self, dest: Dest, targets: Vec<db::Brief>) {
        if !self.reading() {
            return;
        }
        self.move_out(dest, targets);
    }

    /// [`App::remove_targets`] whatever the main window shows (a message
    /// window's own message).
    fn move_out(&mut self, dest: Dest, targets: Vec<db::Brief>) {
        let mut jobs = Vec::new();
        let mut removed = HashSet::new();
        let mut read_first = Vec::new();
        let mut missing = None;
        let mut place = None;
        for ((account, folder_id), group) in by_folder(&targets.iter().collect::<Vec<_>>()) {
            let role_folder = |role| self.db.folder_by_role(account, role).ok().flatten();
            let to = match dest {
                Dest::Archive => match role_folder(Role::Archive) {
                    Some(f) => Some(f.id),
                    None => {
                        missing = Some(t!(invoke_missing_archive));
                        continue;
                    }
                },
                Dest::Trash => match role_folder(Role::Trash) {
                    // From Trash itself: deleted.
                    Some(trash) if trash.id == folder_id => None,
                    Some(trash) => Some(trash.id),
                    None => {
                        missing = Some(t!(invoke_missing_trash));
                        continue;
                    }
                },
                // Never somewhere else instead: without a Junk folder the
                // message stays put.
                Dest::Junk => match role_folder(Role::Junk) {
                    Some(f) => Some(f.id),
                    None => {
                        missing = Some(t!(invoke_missing_junk));
                        continue;
                    }
                },
                Dest::NotJunk => {
                    if self.folder_role(folder_id) != Role::Junk {
                        continue;
                    }
                    match role_folder(Role::Inbox) {
                        Some(f) => {
                            read_first.extend(group.iter().map(|m| **m));
                            Some(f.id)
                        }
                        None => {
                            missing = Some(t!(invoke_missing_inbox));
                            continue;
                        }
                    }
                }
                Dest::Folder(id) => {
                    let same_account = self
                        .folders
                        .iter()
                        .any(|f| f.id == id && f.account_id == account);
                    if !same_account {
                        continue;
                    }
                    Some(id)
                }
            };
            if to == Some(folder_id) {
                continue;
            }
            let uids = group.iter().map(|m| m.uid).collect();
            let job = match to {
                Some(dest) => Job::Move {
                    folder_id,
                    uids,
                    dest,
                },
                None => Job::Delete { folder_id, uids },
            };
            place.get_or_insert_with(|| to.map_or_else(String::new, |id| self.folder_name(id)));
            jobs.push((account, job));
            removed.extend(group.iter().map(|m| m.id));
        }
        // Not junk: marked read at once (not held; nothing is lost by it).
        self.set_flags(&read_first, Flag::Seen, true);
        let Some(place) = place else {
            if let Some(text) = missing {
                self.set_status(&text, true);
            }
            return;
        };
        let mut label = removed_label(&place, removed.len());
        if let Some(text) = missing {
            label = t!(invoke_removed_with_note, label.into(), text.into());
        }
        self.hold(jobs, removed, &label);
    }

    /// Hide moved or deleted messages and offer Undo; their jobs go to the
    /// server when the undo window ends (or sooner: at the next move, or
    /// when the window closes).
    fn hold(&mut self, jobs: Vec<(i64, Job)>, ids: HashSet<i64>, label: &str) {
        self.flush_held();
        let list: Vec<i64> = ids.iter().copied().collect();
        let mut unread = HashMap::new();
        let mut total = HashMap::new();
        for m in self.db.briefs(&list).unwrap_or_default() {
            *total.entry(m.folder_id).or_default() += 1;
            if !m.seen {
                *unread.entry(m.folder_id).or_default() += 1;
            }
        }
        // Done from a message tab: that tab closes (the one to its left
        // shows), as do the other tabs of these messages.
        let from_tab = match self.tabs.active_kind() {
            tabs::TabKind::Message(id) if ids.contains(&id) => Some(id),
            _ => None,
        };
        let closed_tabs = self.close_message_tabs(&ids);
        let closed_windows = self.close_popouts_of(&ids);
        let open = self
            .open
            .as_ref()
            .map(|o| o.msg.id)
            .filter(|id| ids.contains(id));
        // What was open, or the whole selection, went (not a quick action
        // on another row): the next row opens in its place. Otherwise the
        // open message and what is left of the selection stay.
        let hit = self.in_mail_tab() && from_tab.is_none() && open.is_some() || {
            let selected = self.rows.selection().ids();
            self.open.is_none()
                && !selected.is_empty()
                && selected.iter().all(|id| ids.contains(id))
        };
        let index = self.rows.remove_many(&ids);
        if open.is_some() {
            self.open = None;
        }
        self.held_seq += 1;
        let seq = self.held_seq;
        self.held = Some(Held {
            seq,
            ids,
            jobs,
            unread,
            total,
            open,
            tabs: closed_tabs,
            from_tab,
            windows: closed_windows,
        });
        self.rebuild_nav();
        if hit && self.in_mail_tab() {
            match index.and_then(|i| self.rows.neighbour(i)) {
                Some(id) => self.select_message(id),
                None => self.close_reader(),
            }
        } else {
            self.sync_selection_index();
        }
        self.offer_undo(label, UndoOffer::Move(seq));
        self.start_undo_clock(seq);
    }

    /// Send the held jobs to the server now and drop their messages from
    /// the cache. Queued first: that marks the UIDs as pending, so a
    /// running sync can't re-add them after the local delete.
    fn flush_held(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        self.undo_timer.stop();
        self.undo_clock.clear();
        self.clear_undo(UndoOffer::Move(held.seq));
        let ids: Vec<i64> = held.ids.into_iter().collect();
        // A folder reset by a UIDVALIDITY change during the window took its
        // rows (and the meaning of their UIDs) with it: only UIDs of
        // messages still cached are sent.
        let alive: HashSet<(i64, u32)> = self
            .db
            .briefs(&ids)
            .unwrap_or_default()
            .iter()
            .map(|m| (m.folder_id, m.uid))
            .collect();
        for (account, mut job) in held.jobs {
            if let Job::Move {
                folder_id, uids, ..
            }
            | Job::Delete { folder_id, uids } = &mut job
            {
                uids.retain(|u| alive.contains(&(*folder_id, *u)));
                if uids.is_empty() {
                    continue;
                }
            }
            if let Some(w) = self.workers.get(&account) {
                w.send(job);
            }
        }
        self.db.delete_messages(&ids).ok();
    }

    /// Undo: the held messages come back to the list (they never left the
    /// cache or the server), selected as they were.
    fn undo_held(&mut self, seq: u64) {
        let Some(held) = self.held.take_if(|h| h.seq == seq) else {
            return;
        };
        self.undo_timer.stop();
        self.undo_clock.clear();
        self.clear_undo(UndoOffer::Move(seq));
        self.reload_rows();
        self.rebuild_nav();
        if !held.tabs.is_empty() {
            self.reopen_message_tabs(&held.tabs, held.from_tab);
        }
        if held.windows {
            self.set_status(&t!(invoke_undo_window), false);
        }
        if !self.reading() || held.from_tab.is_some() {
            return;
        }
        let listed: Vec<i64> = held
            .ids
            .iter()
            .copied()
            .filter(|&id| self.rows.index_of(id).is_some())
            .collect();
        match held.open.filter(|id| listed.contains(id)) {
            Some(id) => self.select_message(id),
            None if !listed.is_empty() => self.select_rows(|s, _| {
                s.clear();
                for id in listed {
                    s.toggle(id);
                }
            }),
            None => {}
        }
    }

    /// A row was right-clicked: it is selected first unless it already is
    /// (without opening it, so reporting junk never marks it read). False
    /// where rows have no menu (Drafts, Outbox, the composer showing).
    fn message_context(&mut self, id: i64) -> bool {
        if !self.reading() {
            return false;
        }
        // A draft or Outbox row: its own menu (Open, Discard), on it.
        if !self.lists_messages() {
            match self.view {
                View::Outbox => self.open_outbox(id),
                _ => {
                    self.rows.select(|s, _| s.single(id));
                    self.sync_selection_index();
                }
            }
            return true;
        }
        self.select_rows(|s, _| s.right_click(id));
        self.menu_opening();
        true
    }

    /// Delete on a draft or Outbox row (or Discard in its menu): a draft
    /// opens and asks, as the composer's Discard does (one with nothing
    /// written goes at once); an Outbox entry asks first.
    fn discard_row(&mut self, row: i64) {
        match self.view {
            View::Drafts => {
                self.open_saved(row);
                if self.ui().get_composing()
                    && self.drafts.active().is_some_and(|d| d.row == Some(row))
                {
                    self.discard();
                }
            }
            View::Outbox => {
                self.open_outbox(row);
                if self.ui().get_o_can_discard() {
                    self.outbox_discard();
                }
            }
            _ => {}
        }
    }

    /// Reply (0), Reply all (1) or Forward (2) from the row menu: a row
    /// selected without opening it opens first.
    fn menu_respond(&mut self, kind: i32) {
        if !self.open_selected() {
            return;
        }
        match kind {
            0 => self.reply(false),
            1 => self.reply(true),
            _ => self.forward(),
        }
    }

    /// Open the one selected row if no message is open (the row menu acts
    /// on it). False when there is no open message.
    fn open_selected(&mut self) -> bool {
        if self.open.is_none() {
            let ids = self.rows.selection().ids();
            if let [id] = ids[..] {
                self.select_message(id);
            }
        }
        self.open.is_some()
    }

    fn folder_role(&self, id: i64) -> Role {
        self.folders
            .iter()
            .find(|f| f.id == id)
            .map_or(Role::Other, |f| f.role)
    }

    fn folder_name(&self, id: i64) -> String {
        self.folders
            .iter()
            .find(|f| f.id == id)
            .map(|f| text::folder_label(self.lang, f))
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ events

    fn on_event(&mut self, event: Event) {
        self.onboard_event(&event);
        match event {
            Event::Syncing {
                account_id,
                active,
                error,
            } => {
                if active {
                    self.syncing.insert(account_id);
                } else {
                    self.syncing.remove(&account_id);
                    self.end_progress(account_id);
                    self.first_sync.remove(&account_id);
                    if self.syncing.is_empty() && error.is_none() {
                        self.mark_updated();
                    }
                }
                self.ui().set_syncing(!self.syncing.is_empty());
                let state = match &error {
                    _ if active => AccountState::Syncing,
                    None => AccountState::Checked(chrono::Utc::now().timestamp()),
                    Some(p) if p.class == Class::Offline => AccountState::Unreachable,
                    Some(p) => AccountState::Failed(p.clone()),
                };
                self.account_state.insert(account_id, state);
                if self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.msg.raw.is_none() && o.msg.account_id == account_id)
                {
                    let out = self.unreachable(account_id);
                    self.ui().set_r_unreachable(out);
                }
                self.show_account_rows();
                self.rebuild_nav();
                self.show_empty();
                if let Some(p) = error {
                    self.sync_error(account_id, &p);
                } else if !active {
                    // It went through: a problem shows again if it returns,
                    // and the line saying it can't be reached goes.
                    self.shown_problem.remove(&account_id);
                    let standing = self.standing.len();
                    self.standing.retain(|(a, _)| *a != account_id);
                    if self.standing.len() < standing
                        && self.ui().get_status_severity() == Severity::Caution
                    {
                        self.set_status("", false);
                    }
                    if self.warmed.insert(account_id)
                        && let Some(w) = self.workers.get(&account_id)
                    {
                        // The server is reachable and the password works:
                        // sign the interactive worker in now, not on the
                        // first click.
                        w.send(Job::Warm);
                    }
                }
            }
            Event::Progress {
                account_id,
                folder_id,
                done,
                total,
            } => self.show_progress(account_id, folder_id, done, total),
            Event::FolderChanged { folder_id: None } => {
                let old =
                    std::mem::replace(&mut self.folders, self.db.folders().unwrap_or_default());
                self.leave_vanished(&old);
                self.refresh_message_tabs();
                self.rebuild_nav();
                self.land();
                self.settle_view();
                self.schedule_reload();
            }
            Event::FolderChanged { folder_id: Some(f) } => {
                if self.list_folders().contains(&f) {
                    self.schedule_reload();
                } else if !self.reload_timer.running() {
                    self.rebuild_nav();
                }
            }
            Event::BodyReady { message_id } => {
                if self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.msg.id == message_id && o.body.is_none())
                    && let Ok(Some(m)) = self.db.message(message_id)
                    && let Some(open) = self.open.as_mut()
                {
                    open.msg.raw = m.raw;
                    self.load_open_body();
                    self.show_open();
                }
                self.popouts_body_ready(message_id);
                self.raw_arrived(message_id);
                self.schedule_reload();
            }
            Event::BodyFailed { message_id, error } => {
                self.raw_failed(message_id, &error);
                self.popouts_body_failed(
                    message_id,
                    &t!(invoke_could_not_download_body, text::problem(&error).into()),
                );
                if self.open.as_ref().is_some_and(|o| o.msg.id == message_id) {
                    self.reader.close();
                    let ui = self.ui();
                    ui.set_loading(false);
                    ui.set_r_body(
                        t!(invoke_could_not_download_body, text::problem(&error).into()).into(),
                    );
                }
            }
            Event::Sent {
                reply_to, outcome, ..
            } => self.on_sent(reply_to, outcome),
            Event::ActionFailed { account_id, error } => {
                self.sync_error(account_id, &error);
                self.show_account_rows();
                self.rebuild_nav();
            }
            Event::NeedsSync { folder_id } => self.request_folder_sync(folder_id, true),
            // New mail or flag changes seen by IDLE: STATUS first, then the
            // delta or nothing.
            Event::RemoteChanged { folder_id } => self.request_folder_sync(folder_id, false),
            Event::Notice {
                account_id,
                message,
            } => {
                let email = self.account_email(account_id);
                self.set_status(&text::account_line(&email, &text::notice(&message)), false);
            }
            Event::DraftCopyLeft(copy) => self.remove_server_copy(Some(copy)),
        }
    }

    /// Queue a sync of one folder; `full` skips the STATUS gate and delta.
    fn request_folder_sync(&self, folder_id: i64, full: bool) {
        if let Some(f) = self.folders.iter().find(|f| f.id == folder_id)
            && let Some(w) = self.workers.get(&f.account_id)
        {
            w.send(Job::SyncFolder {
                id: folder_id,
                full,
            });
        }
    }

    /// Show an account's problem, once until it clears (a sync that goes
    /// through, or Refresh); a rejected sign-in offers to update the
    /// password.
    fn sync_error(&mut self, account_id: i64, problem: &Problem) {
        let (class, text) = problem.key();
        let key = (class, text.to_string());
        if self.shown_problem.get(&account_id) == Some(&key) {
            return;
        }
        self.shown_problem.insert(account_id, key);
        self.standing.retain(|(a, _)| *a != account_id);
        // A server out of reach stays said until it is back (FLUENT2
        // "Wait UX": attached to what it concerns); it waits for an offer
        // (Undo) to end rather than take it away.
        if matches!(problem.class, Class::Offline | Class::TooManyConnections) {
            self.standing.push((account_id, problem.clone()));
            if self.status_action.get() == StatusAction::None {
                self.set_status("", false);
            }
            return;
        }
        // A changed certificate in one short sentence; its card in Settings
        // has the whole of it.
        let said = match &problem.changed {
            Some(host) => t!(invoke_say_cert_changed_short, text::host(host)),
            None => text::problem(problem),
        };
        let line = text::account_line(&self.account_email(account_id), &said);
        self.set_status(&line, true);
        let action = problem_action(account_id, self.attention(account_id), problem);
        if action != StatusAction::None {
            self.set_status_action(action);
        }
    }

    /// A folder's headers are downloading: Settings and the status line
    /// say how far (the status line only while it shows nothing else).
    fn show_progress(&mut self, account_id: i64, folder_id: i64, done: usize, total: usize) {
        self.progress.insert(account_id, (done, total));
        self.show_account_rows();
        if self.first_sync.contains(&account_id) {
            self.show_empty();
        }
        // Settings shows it on the account's card instead (one indicator).
        let ui = self.ui();
        if !ui.get_show_settings() && (ui.get_status().is_empty() || self.progress_shown.get()) {
            let text = t!(
                invoke_status_downloading,
                self.account_email(account_id).into(),
                format::thousands(self.lang, done).into(),
                format::thousands(self.lang, total).into(),
                self.folder_name(folder_id).into()
            );
            self.set_status(&text, false);
            self.progress_shown.set(true);
        }
    }

    /// An account's sync ended: its download progress goes.
    fn end_progress(&mut self, account_id: i64) {
        if self.progress.remove(&account_id).is_some()
            && self.progress.is_empty()
            && self.progress_shown.get()
        {
            self.set_status("", false);
        }
    }

    /// The folder list changed (`old` before): a reader open on a message
    /// of a folder that is gone closes, and a view of it moves to its
    /// account's inbox.
    fn leave_vanished(&mut self, old: &[Folder]) {
        let gone = |folders: &[Folder], id: i64| folders.iter().all(|f| f.id != id);
        if self.in_mail_tab()
            && self
                .open
                .as_ref()
                .is_some_and(|o| gone(&self.folders, o.msg.folder_id))
        {
            self.close_reader();
        }
        if let View::Folder(id) = self.view
            && gone(&self.folders, id)
        {
            let account = old.iter().find(|f| f.id == id).map(|f| f.account_id);
            let inbox = self
                .folders
                .iter()
                .find(|f| Some(f.account_id) == account && f.role == Role::Inbox);
            self.set_view(inbox.map_or(View::AllInboxes, |f| View::Folder(f.id)));
        }
    }

    /// The wake check: after a wall-clock gap (sleep), every connection is
    /// replaced and, once the network had a moment, every account syncs.
    fn wake_check(&mut self) {
        let now = SystemTime::now();
        let slept = woke(self.last_tick, now);
        self.last_tick = now;
        if !slept {
            return;
        }
        for w in self.workers.values() {
            w.resumed();
        }
        Timer::single_shot(RESUME_SETTLE, || {
            with_app(|a| {
                let full = a
                    .last_full_sync
                    .is_none_or(|t| t.elapsed() >= settings::FULL_SYNC_EVERY);
                a.sync_all(full);
                a.drain_outbox();
            });
        });
    }

    fn account_email(&self, id: i64) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.email.clone())
            .unwrap_or_default()
    }

    /// Put an action button on the status line just set.
    fn set_status_action(&self, action: StatusAction) {
        let ui = self.ui();
        ui.set_status_action(action_label(action).into());
        self.status_action.set(action);
    }

    /// Ask `q` in the in-app prompt; [`App::answer`] acts on the reply.
    fn ask(&mut self, q: Question) {
        let mut certificate: Option<cert::TrustPrompt> = None;
        let (title, text, ok) = match q {
            Question::Resend(_) => (
                t!(invoke_q_resend_title),
                t!(invoke_q_resend_text),
                t!(invoke_q_resend_ok),
            ),
            Question::DiscardOutbox(_) => (
                t!(invoke_q_discard_title),
                t!(invoke_q_discard_text),
                t!(invoke_q_discard_ok),
            ),
            Question::RemoveAccount(id) => {
                // The text, then a sentence of its own if drafts go with it.
                let mut text = t!(invoke_q_remove_text);
                let unsent = self.db.count_unsent(id).unwrap_or(0);
                if unsent > 0 {
                    text.push(' ');
                    text.push_str(&t!(invoke_q_remove_unsent, count(unsent)));
                }
                (
                    t!(invoke_q_remove_title, self.account_email(id).into()),
                    text,
                    t!(invoke_q_remove_ok),
                )
            }
            Question::Rsvp(_, answer) => {
                let (title, text) = self.rsvp_prompt(answer);
                (title, text, t!(invoke_q_rsvp_ok))
            }
            Question::Redownload(id) => (
                t!(invoke_q_redownload_title, self.account_email(id).into()),
                t!(invoke_q_redownload_text),
                t!(invoke_q_redownload_ok),
            ),
            Question::RemoveAllData => {
                let mut text = t!(invoke_q_wipe_text);
                let unsent: i64 = self
                    .accounts
                    .iter()
                    .map(|a| self.db.count_unsent(a.id).unwrap_or(0))
                    .sum();
                if unsent > 0 {
                    text.push(' ');
                    text.push_str(&t!(invoke_q_wipe_unsent, count(unsent)));
                }
                (t!(invoke_q_wipe_title), text, t!(invoke_q_wipe_ok))
            }
            Question::DeleteFrom { trash } => {
                let n = count(self.delete_wait.len() as i64);
                if trash {
                    (
                        t!(invoke_q_delete_trash_title, n),
                        t!(invoke_q_delete_trash_text, n),
                        t!(invoke_q_delete_ok),
                    )
                } else {
                    (
                        t!(invoke_q_delete_junk_title, n),
                        t!(invoke_q_delete_junk_text, n),
                        t!(invoke_q_delete_ok),
                    )
                }
            }
            Question::DeleteSignature(id) => self.signature_question(id),
            Question::TrustCert => match self.trust_prompt() {
                Some(p) => {
                    let said = (p.title.clone(), p.text.clone(), p.ok.clone());
                    certificate = Some(p);
                    said
                }
                None => return,
            },
        };
        self.question = Some(q);
        let ui = self.ui();
        // A certificate: its details, its fingerprint in byte groups (and
        // the one it replaces), and how to check it; Copy is an IconButton
        // beside the fingerprint, not a third button.
        let c = certificate.as_ref();
        let field = |f: fn(&cert::TrustPrompt) -> &str| SharedString::from(c.map_or("", f));
        ui.set_q_detail(field(|p| &p.detail));
        ui.set_q_code(field(|p| &p.code));
        ui.set_q_code_label(field(|p| &p.code_label));
        ui.set_q_old(field(|p| &p.old));
        ui.set_q_old_label(field(|p| &p.old_label));
        ui.set_q_caution(field(|p| &p.caution));
        ui.set_q_note(field(|p| &p.note));
        let alt = match q {
            Question::Rsvp(..) => t!(invoke_q_rsvp_alt),
            _ => String::new(),
        };
        ui.set_q_alt(alt.into());
        ui.set_q_destructive(q.destructive());
        ui.set_q_text(text.into());
        ui.set_q_ok(ok.into());
        ui.set_q_title(title.into());
    }

    /// The prompt was answered (accepted, or cancelled).
    fn answer(&mut self, accepted: bool) {
        self.ui().set_q_title(SharedString::new());
        let Some(q) = self.question.take() else {
            return;
        };
        if q == Question::TrustCert && !accepted {
            self.cancel_certificate();
        }
        if !accepted {
            return;
        }
        match q {
            Question::Resend(row) => self.retry_now(row),
            Question::DiscardOutbox(row) => self.discard_outbox_now(row),
            Question::RemoveAccount(id) => self.remove_account_now(id),
            Question::Rsvp(msg, answer) => self.rsvp(msg, answer, true),
            Question::Redownload(id) => self.redownload_now(id),
            Question::RemoveAllData => self.remove_all_data_now(),
            Question::TrustCert => self.trust_certificate(),
            Question::DeleteFrom { .. } => self.delete_confirmed(),
            Question::DeleteSignature(id) => self.delete_signature_now(id),
        }
    }

    /// The prompt's third answer: an invitation answered without a reply.
    fn answer_alt(&mut self) {
        // A certificate prompt's third button copies the fingerprint and
        // keeps the question open.
        if self.question == Some(Question::TrustCert) {
            if let Some(p) = self.trust_prompt() {
                self.ui().invoke_r_copy_text(p.fingerprint.into());
            }
            return;
        }
        self.ui().set_q_title(SharedString::new());
        if let Some(Question::Rsvp(msg, answer)) = self.question.take() {
            self.rsvp(msg, answer, false);
        }
    }

    /// Show `text` on the status line for 5 s (Informational), or 12 s
    /// for an `error` (Critical). Empty, it clears the line, or shows the
    /// latest account out of reach (Caution) until that changes.
    fn set_status(&self, text: &str, error: bool) {
        // With nothing else to say, the latest account out of reach.
        let latest = self.standing.last().filter(|_| text.is_empty());
        let standing =
            latest.map(|(id, p)| text::account_line(&self.account_email(*id), &text::problem(p)));
        let (text, severity, after) = match &standing {
            Some(standing) => (standing.as_str(), Severity::Caution, None),
            None if error => (text, Severity::Critical, Some(12)),
            None => (text, Severity::Info, Some(5)),
        };
        self.progress_shown.set(false);
        let ui = self.ui();
        ui.set_status_closable(true);
        ui.set_status(text.into());
        ui.set_status_severity(severity);
        ui.set_status_action(SharedString::new());
        self.status_action.set(StatusAction::None);
        // The account out of reach: one attempt now.
        if let Some((id, _)) = latest {
            self.set_status_action(StatusAction::TryNow(*id));
        }
        match after {
            Some(secs) if !text.is_empty() => {
                let after = Duration::from_secs(secs);
                self.status_timer.start(TimerMode::SingleShot, after, || {
                    with_app(|a| a.set_status("", false));
                });
            }
            _ => self.status_timer.stop(),
        }
    }

    /// The status line was closed (its close button, or Esc): an account
    /// out of reach is not said again until it syncs or Refresh.
    fn dismiss_status(&mut self) {
        if self.ui().get_status_severity() == Severity::Caution {
            self.standing.clear();
        }
        if self.status_action.get() == StatusAction::WhatsNew {
            self.whats_new_seen();
        }
        self.set_status("", false);
    }
}

impl App {
    /// Open attachment `index` of the message in the default app for its
    /// type, from a copy marked as downloaded. Types that run code are
    /// refused: they can be saved and checked first.
    fn open_attachment(&mut self, index: i32) {
        let found = self
            .open
            .as_ref()
            .and_then(|o| o.msg.raw.as_deref())
            .zip(usize::try_from(index).ok())
            .and_then(|(raw, i)| mime::attachment(raw, i));
        self.open_attachment_file(found);
    }

    /// Open an attachment (its name and bytes) in the app for its type.
    fn open_attachment_file(&mut self, found: Option<(String, Vec<u8>)>) {
        let Some((name, data)) = found else { return };
        if files::risky(&name) {
            return self.set_status(&t!(invoke_file_can_run, name.as_str().into()), true);
        }
        let dir = files::open_dir(&self.db_path);
        // Writing can take a moment and ShellExecuteW can block (DDE, a
        // slow handler): both stay off the UI thread.
        let open = move || {
            // The sentence is made on the UI thread, which has the catalog.
            let failure = match files::write_open_copy(&dir, &name, &data) {
                Ok(path) if reader::shell_execute(path.as_os_str()) => return,
                Ok(_) => None,
                Err(e) => Some(e.to_string()),
            };
            on_ui(Box::new(move |a| {
                let text = match failure {
                    None => t!(invoke_no_app_opens, name.as_str().into()),
                    Some(e) => t!(invoke_could_not_open_file, name.as_str().into(), e.into()),
                };
                a.set_status(&text, true);
            }));
        };
        background("open-file", Box::new(open)).ok();
    }
}

// ------------------------------------------------------- file dialogs
//
// Native dialogs run a nested message loop, so these handlers take what
// they need from the app state, release it, show the dialog, and then
// re-enter.

fn save_attachment(index: i32) {
    let found = with_app(|a| {
        let raw = a.open.as_ref()?.msg.raw.as_deref()?;
        mime::attachment(raw, index as usize)
    })
    .flatten();
    if let Some((name, data)) = found {
        save_file(&t!(invoke_dlg_save_attachment), &name, data);
    }
}

/// Save `data` where the user picks (offered as `name` in Downloads),
/// marked as downloaded. The file is written off the UI thread.
fn save_file(title: &str, name: &str, data: Vec<u8>) {
    let mut dialog = rfd::FileDialog::new().set_title(title).set_file_name(name);
    if let Some(dir) = dirs::download_dir() {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.save_file() else {
        return;
    };
    let name = name.to_string();
    let write = move || {
        let result = std::fs::write(&path, data);
        if result.is_ok() {
            // Mark of the Web, as a browser download has.
            files::mark_downloaded(&path).ok();
        }
        on_ui(Box::new(move |a| match result {
            Ok(()) => a.set_status(
                &t!(invoke_saved_to, path.display().to_string().into()),
                false,
            ),
            Err(e) => a.set_status(
                &t!(
                    invoke_could_not_save_file,
                    name.as_str().into(),
                    e.to_string().into()
                ),
                true,
            ),
        }));
    };
    if let Err(e) = background("save-file", Box::new(write)) {
        with_app(|a| a.set_status(&t!(invoke_could_not_save, e.to_string().into()), true));
    }
}

// ------------------------------------------------------------ helpers

/// Replace a model's contents, updating rows in place when the row set is
/// unchanged so the list doesn't reset its scroll position or flicker.
fn update_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, new: Vec<T>) {
    if model.row_count() == new.len() {
        for (i, row) in new.into_iter().enumerate() {
            if model.row_data(i).as_ref() != Some(&row) {
                model.set_row_data(i, row);
            }
        }
    } else {
        model.set_vec(new);
    }
}

/// A count for a plural sentence.
fn count(n: i64) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Messages grouped by (account, folder): one server job each.
/// The undo offer after moving `count` messages to the folder `place`
/// (empty: deleted).
fn removed_label(place: &str, count: usize) -> String {
    let n = i32::try_from(count).unwrap_or(i32::MAX);
    match (count, place) {
        (1, "") => t!(invoke_removed_deleted),
        (1, _) => t!(invoke_removed_moved, place.into()),
        (_, "") => t!(invoke_removed_deleted_many, n),
        _ => t!(invoke_removed_moved_many, n, place.into()),
    }
}

fn by_folder<'a>(msgs: &[&'a db::Brief]) -> BTreeMap<(i64, i64), Vec<&'a db::Brief>> {
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for &m in msgs {
        groups
            .entry((m.account_id, m.folder_id))
            .or_default()
            .push(m);
    }
    groups
}

/// The servers an account form names: a preset's, or the ones typed.
/// The preset a form chose (`None`: Other server, or none chosen yet).
fn form_preset(form: &AccountForm) -> Option<usize> {
    usize::try_from(form.provider)
        .ok()
        .filter(|&i| i < providers::PRESETS.len())
}

fn form_servers(form: &AccountForm) -> Result<(Server, Server), text::FormError> {
    if form.provider < 0 {
        return Err(text::FormError::Provider);
    }
    if let Some(p) = form_preset(form).map(|i| &providers::PRESETS[i]) {
        return Ok((p.imap_server(), p.smtp_server()));
    }
    let port = |s: &str, proto| settings::parse_port(s).ok_or(text::FormError::Port(proto));
    let security = |starttls: bool| {
        if starttls {
            Security::StartTls
        } else {
            Security::Tls
        }
    };
    let (imap_host, smtp_host) = (form.imap_host.trim(), form.smtp_host.trim());
    if imap_host.is_empty() || smtp_host.is_empty() {
        return Err(text::FormError::ServerNames);
    }
    let (imap_host, smtp_host) = (
        settings::validate_host(imap_host)?,
        settings::validate_host(smtp_host)?,
    );
    Ok((
        Server {
            host: imap_host,
            port: port(&form.imap_port, text::Proto::Imap)?,
            security: security(form.imap_starttls),
        },
        Server {
            host: smtp_host,
            port: port(&form.smtp_port, text::Proto::Smtp)?,
            security: security(form.smtp_starttls),
        },
    ))
}

/// An account's line in Settings (empty before its first sync).
fn account_status(
    state: Option<&AccountState>,
    attention: Option<Attention>,
    host: &str,
) -> String {
    match attention {
        Some(Attention::SignIn) => return t!(invoke_account_sign_in_rejected),
        Some(Attention::Secure) => return t!(invoke_account_secure_failed),
        None => {}
    }
    match state {
        None => String::new(),
        Some(AccountState::Syncing) => t!(invoke_account_syncing),
        Some(AccountState::Checked(at)) => {
            match chrono::TimeZone::timestamp_opt(&chrono::Local, *at, 0).single() {
                Some(time) => t!(
                    invoke_account_checked,
                    time.format("%H:%M").to_string().into()
                ),
                None => t!(invoke_account_up_to_date),
            }
        }
        Some(AccountState::Unreachable) => t!(invoke_account_unreachable, host.into()),
        Some(AccountState::Failed(p)) => {
            t!(invoke_account_sync_failed, text::problem(p).into())
        }
    }
}

/// What the accounts behind an empty list are doing.
#[derive(Default)]
struct ListState<'a> {
    /// A sync is running.
    syncing: bool,
    /// A shown account's last sync failed.
    failed: Option<&'a AccountState>,
    /// A shown account is in its first download: (done, total), (0, 0)
    /// before the first count.
    fresh: Option<(usize, usize)>,
    /// The folder's account has completed a sync pass.
    synced: bool,
}

/// What an empty list says: no match for the search, no drafts, an empty
/// outbox, nothing under the filter, a new account's first download, still
/// loading, a sync that failed, an empty folder once a sync pass has
/// completed, or nothing (yet).
fn empty_text(
    view: View,
    search: &str,
    filter: Filter,
    state: &ListState,
    lang: komorebi::i18n::Lang,
) -> String {
    match filter {
        _ if !search.trim().is_empty() => t!(invoke_empty_no_matches),
        _ if view == View::Drafts => t!(invoke_empty_no_drafts),
        _ if view == View::Outbox => t!(invoke_empty_outbox),
        Filter::Unread => t!(invoke_empty_no_unread),
        Filter::Flagged => t!(invoke_empty_no_flagged),
        Filter::All => match (state.fresh, state.failed) {
            (Some((_, 0)), _) => t!(invoke_empty_downloading_start),
            (Some((done, total)), _) => t!(
                invoke_empty_downloading,
                format::thousands(lang, done.min(total)).into(),
                format::thousands(lang, total).into()
            ),
            _ if state.syncing => t!(invoke_empty_loading),
            (_, Some(AccountState::Unreachable)) => t!(invoke_empty_unreachable),
            (_, Some(AccountState::Failed(_))) => t!(invoke_empty_failed),
            _ if state.synced => t!(invoke_empty_folder),
            _ => t!(invoke_empty_nothing),
        },
    }
}

/// The label of the button a status action puts on the status line (empty:
/// none).
fn action_label(action: StatusAction) -> String {
    match action {
        StatusAction::None => String::new(),
        StatusAction::EditAccount(_) => t!(invoke_action_update_password),
        StatusAction::RetryAccount(_) => t!(invoke_action_retry),
        StatusAction::ReviewCertificate(_) => t!(invoke_action_review_certificate),
        StatusAction::TryNow(_) => t!(invoke_action_try_now),
        StatusAction::WhatsNew => t!(invoke_action_whats_new),
    }
}

/// The button an account's problem puts on the status line: what the
/// workers wait for decides it (a rejected sign-in: Update password; a
/// changed certificate: Review certificate; another secure-connection
/// failure: Retry), else a server out of reach offers Try now.
fn problem_action(id: i64, attention: Option<Attention>, problem: &Problem) -> StatusAction {
    match attention {
        Some(Attention::SignIn) => StatusAction::EditAccount(id),
        Some(Attention::Secure) if problem.changed.is_some() => StatusAction::ReviewCertificate(id),
        Some(Attention::Secure) => StatusAction::RetryAccount(id),
        None if matches!(problem.class, Class::Offline | Class::TooManyConnections) => {
            StatusAction::TryNow(id)
        }
        None => StatusAction::None,
    }
}

/// Whether a search over `scope` cached messages waits for a typing pause.
/// The importance an opened message's parsed body says its sender set,
/// when its row has none yet (cached before v13): taken into `msg`, and
/// returned (id, importance) for the row to keep. None otherwise; a row
/// that already has one keeps it.
fn backfill_importance(msg: &mut db::Message, body: &mime::Body) -> Option<(i64, db::Importance)> {
    if msg.importance != db::Importance::Normal || body.importance == db::Importance::Normal {
        return None;
    }
    msg.importance = body.importance;
    Some((msg.id, body.importance))
}

fn debounces_search(scope: u32, text: &str) -> bool {
    scope > SEARCH_DEBOUNCE_ABOVE && !text.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each account problem puts its own fix on the status line.
    #[test]
    fn a_problem_maps_to_its_status_action() {
        let changed = Problem::of(&mail::pin::PinMismatch("imap.example.com".into()).into());
        assert!(changed.changed.is_some());
        let untrusted = Problem {
            changed: None,
            ..changed.clone()
        };
        let offline = Problem::of(&anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::TimedOut,
        )));
        let other = Problem::of(&anyhow::anyhow!("opening folder X"));
        let rejected = Problem::of(&anyhow::anyhow!("no"));
        assert_eq!(
            problem_action(7, Some(Attention::Secure), &changed),
            StatusAction::ReviewCertificate(7)
        );
        assert_eq!(
            problem_action(7, Some(Attention::Secure), &untrusted),
            StatusAction::RetryAccount(7)
        );
        assert_eq!(
            problem_action(7, Some(Attention::SignIn), &rejected),
            StatusAction::EditAccount(7)
        );
        assert_eq!(problem_action(7, None, &offline), StatusAction::TryNow(7));
        assert_eq!(problem_action(7, None, &other), StatusAction::None);
    }

    #[test]
    fn start_view_falls_back_to_the_inbox() {
        let server = Server {
            host: "imap.example.invalid".into(),
            port: 993,
            security: Security::Tls,
        };
        let account = |id: i64| Account {
            id,
            email: format!("a{id}@example.invalid"),
            name: String::new(),
            imap: server.clone(),
            smtp: server.clone(),
            save_sent: true,
            signature: String::new(),
            enabled: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            imap_pin: None,
            smtp_pin: None,
        };
        let folder = |id: i64, account_id: i64, role: Role| Folder {
            id,
            account_id,
            name: format!("f{id}"),
            display: format!("f{id}"),
            depth: 0,
            role,
            uidvalidity: 0,
        };
        let one = [account(1)];
        let two = [account(1), account(2)];
        let folders = [
            folder(10, 1, Role::Inbox),
            folder(11, 1, Role::Other),
            folder(20, 2, Role::Inbox),
        ];
        // A folder saved last time and still here opens again.
        assert!(start_view("f11", &one, &folders, 0) == View::Folder(11));
        assert_eq!(View::Folder(11).key(), "f11");
        // Gone (or of an account that is gone): the Inbox, or with more
        // than one account All inboxes.
        assert!(start_view("f99", &one, &folders, 0) == View::Folder(10));
        assert!(start_view("f20", &one, &folders, 0) == View::Folder(10));
        assert!(start_view("f99", &two, &folders, 0) == View::AllInboxes);
        // All inboxes needs two accounts; one opens on its Inbox.
        assert!(start_view("all", &one, &folders, 0) == View::Folder(10));
        assert!(start_view("all", &two, &folders, 0) == View::AllInboxes);
        // Drafts always; the Outbox only while it holds mail.
        assert!(start_view("drafts", &one, &folders, 0) == View::Drafts);
        assert!(start_view("outbox", &one, &folders, 0) == View::Folder(10));
        assert!(start_view("outbox", &one, &folders, 2) == View::Outbox);
        // Nothing saved; no Inbox yet; no account.
        assert!(start_view("", &one, &folders, 0) == View::Folder(10));
        assert!(start_view("", &one, &[], 0) == View::AllInboxes);
        assert!(start_view("drafts", &[], &[], 0) == View::AllInboxes);
    }

    #[test]
    fn account_forms_name_their_servers() {
        let manual = AccountForm {
            provider: providers::PRESETS.len() as i32,
            imap_host: " mail.example.net ".into(),
            imap_port: "143".into(),
            imap_starttls: true,
            smtp_host: "mail.example.net".into(),
            smtp_port: "465".into(),
            ..Default::default()
        };
        let (imap, smtp) = form_servers(&manual).unwrap();
        assert_eq!(
            (imap.host.as_str(), imap.port, imap.security),
            ("mail.example.net", 143, Security::StartTls)
        );
        assert_eq!((smtp.port, smtp.security), (465, Security::Tls));
        let preset = form_servers(&AccountForm::default()).unwrap();
        assert_eq!(preset.0, providers::PRESETS[0].imap_server());
        for (port, field) in [("0", "IMAP"), ("x", "IMAP")] {
            let bad = AccountForm {
                imap_port: port.into(),
                ..manual.clone()
            };
            let proto = if field == "IMAP" {
                text::Proto::Imap
            } else {
                text::Proto::Smtp
            };
            assert_eq!(
                form_servers(&bad).unwrap_err(),
                text::FormError::Port(proto)
            );
        }
        let no_host = AccountForm {
            smtp_host: " ".into(),
            ..manual.clone()
        };
        assert!(form_servers(&no_host).is_err());

        // Host names are checked: the name only, no scheme, port or path.
        for host in [
            "imap://mail.example.net",
            "mail.example.net:993",
            "a/b",
            "me@x",
        ] {
            let bad = AccountForm {
                imap_host: host.into(),
                ..manual.clone()
            };
            assert_eq!(
                form_servers(&bad).unwrap_err(),
                text::FormError::ServerNameOnly
            );
        }
        // Normalised on the way in: case, a trailing dot, an IPv6 literal.
        let odd = AccountForm {
            imap_host: "[::1]".into(),
            smtp_host: "Mail.Example.NET.".into(),
            ..manual.clone()
        };
        let (imap, smtp) = form_servers(&odd).unwrap();
        assert_eq!(
            (imap.host.as_str(), smtp.host.as_str()),
            ("::1", "mail.example.net")
        );
        // Custom ports are fine for either security (Proton Bridge, 2525).
        let bridge = AccountForm {
            imap_port: "1143".into(),
            smtp_port: "2525".into(),
            smtp_starttls: true,
            ..manual
        };
        let (imap, smtp) = form_servers(&bridge).unwrap();
        assert_eq!(
            (imap.port, smtp.port, smtp.security),
            (1143, 2525, Security::StartTls)
        );
    }

    #[test]
    fn a_wall_clock_gap_means_a_wake() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert!(!woke(t, t + RESUME_TICK));
        assert!(!woke(t, t + RESUME_GAP));
        assert!(woke(t, t + RESUME_GAP + Duration::from_secs(1)));
        assert!(woke(t, t + Duration::from_secs(8 * 3600)));
        // A clock set back is no wake.
        assert!(!woke(t, t - Duration::from_secs(3600)));
    }

    #[test]
    fn account_status_lines() {
        let _ui = crate::tr::tests::window();
        let host = "imap.example.org";
        assert_eq!(account_status(None, None, host), "");
        assert_eq!(
            account_status(Some(&AccountState::Syncing), None, host),
            "Syncing\u{a0}…"
        );
        let at = chrono::Utc::now().timestamp();
        let checked = account_status(Some(&AccountState::Checked(at)), None, host);
        assert!(checked.starts_with("Up to date, checked "), "{checked}");
        assert_eq!(checked.len(), "Up to date, checked 12:41".len());
        assert_eq!(
            account_status(Some(&AccountState::Unreachable), None, host),
            "Can't reach imap.example.org"
        );
        // A rejected sign-in wins over the last pass.
        assert_eq!(
            account_status(
                Some(&AccountState::Checked(at)),
                Some(Attention::SignIn),
                host
            ),
            "Sign-in rejected"
        );
        // So does a secure connection that failed (a changed certificate).
        assert_eq!(
            account_status(Some(&AccountState::Syncing), Some(Attention::Secure), host),
            "Secure connection failed"
        );
    }

    #[test]
    fn batches_are_one_job_per_folder() {
        let brief = |id: i64, account_id, folder_id| db::Brief {
            id,
            account_id,
            folder_id,
            uid: id as u32,
            seen: false,
            flagged: false,
        };
        // 200 messages from two inboxes of two accounts, interleaved.
        let msgs: Vec<db::Brief> = (1..=200)
            .map(|id| match id % 4 {
                0 => brief(id, 2, 20),
                _ => brief(id, 1, 10),
            })
            .collect();
        let refs: Vec<&db::Brief> = msgs.iter().collect();
        let groups = by_folder(&refs);
        assert_eq!(
            groups.keys().copied().collect::<Vec<_>>(),
            [(1, 10), (2, 20)]
        );
        assert_eq!(groups[&(1, 10)].len(), 150);
        assert_eq!(groups[&(2, 20)].len(), 50);
        assert!(groups[&(2, 20)].iter().all(|m| m.uid % 4 == 0));
        assert!(by_folder(&[]).is_empty());
    }

    #[test]
    fn undo_offers_name_the_folder() {
        let _ui = crate::tr::tests::window();
        assert_eq!(removed_label("Archive", 1), "Moved to Archive");
        assert_eq!(removed_label("Junk", 3), "3 messages moved to Junk");
        assert_eq!(removed_label("", 1), "Deleted");
        assert_eq!(removed_label("", 200), "200 messages deleted");
    }

    #[test]
    fn only_large_scopes_debounce_search() {
        assert!(!debounces_search(1_200, "inv"));
        assert!(!debounces_search(SEARCH_DEBOUNCE_ABOVE, "inv"));
        assert!(debounces_search(SEARCH_DEBOUNCE_ABOVE + 1, "inv"));
        // Clearing the search always reloads at once.
        assert!(!debounces_search(50_000, ""));
        assert!(!debounces_search(50_000, "  "));
    }

    #[test]
    fn the_read_delay_is_cancelled_by_moving_on() {
        let delay = Duration::from_secs(1);
        let mut wait = ReadWait::default();
        // Arrowing through 1, 2, 3: only 3, still open, is marked.
        assert_eq!(wait.opened(1, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(2, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(3, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.due(Some(3)), Some(3));
        // It fires once only.
        assert_eq!(wait.due(Some(3)), None);
        // A read message never waits, and opening it drops an earlier wait.
        wait.opened(4, true, MarkRead::After(delay));
        assert_eq!(wait.opened(5, false, MarkRead::After(delay)), None);
        assert_eq!(wait.due(Some(4)), None);
        // Closed (or another message open) when the delay ends: nothing.
        wait.opened(6, true, MarkRead::After(delay));
        assert_eq!(wait.due(None), None);
        wait.opened(7, true, MarkRead::After(delay));
        wait.cancel();
        assert_eq!(wait.due(Some(7)), None);
        // At once, or never.
        assert_eq!(wait.opened(8, true, MarkRead::Now), Some(Duration::ZERO));
        assert_eq!(wait.due(Some(8)), None);
        assert_eq!(wait.opened(9, true, MarkRead::Never), None);
    }

    #[test]
    fn empty_list_texts() {
        use komorebi::i18n::Lang;
        let _ui = crate::tr::tests::window();
        let failed = AccountState::Failed(Problem::of(&anyhow::anyhow!("no")));
        let st = |syncing, failed| ListState {
            syncing,
            failed,
            ..Default::default()
        };
        let cases = [
            ("x", Filter::Unread, st(true, None), "No matches"),
            ("", Filter::Unread, st(true, None), "No unread messages"),
            (
                "",
                Filter::Flagged,
                st(false, Some(&failed)),
                "No important messages",
            ),
            ("", Filter::All, st(true, Some(&failed)), "Loading\u{a0}…"),
            ("  ", Filter::All, st(false, None), "Nothing here"),
        ];
        let inbox = View::AllInboxes;
        let text = |view, search, filter, state: &ListState| {
            empty_text(view, search, filter, state, Lang::En)
        };
        for (search, filter, state, want) in cases {
            assert_eq!(text(inbox, search, filter, &state), want);
        }
        let t = text(inbox, "", Filter::All, &st(false, Some(&failed)));
        assert!(t.starts_with("Couldn't check for new mail"), "{t}");
        let unreachable = Some(&AccountState::Unreachable);
        let t = text(inbox, "", Filter::All, &st(false, unreachable));
        assert!(t.starts_with("Can't reach the server"), "{t}");
        // Drafts and the Outbox say what they hold (a search still finds
        // nothing first).
        let drafts = text(View::Drafts, "", Filter::All, &st(true, None));
        assert_eq!(drafts, "No drafts. Select New mail to start one.");
        let outbox = text(View::Outbox, "", Filter::All, &st(false, None));
        assert_eq!(
            outbox,
            "Outbox is empty. Messages wait here while they are sent."
        );
        let search = text(View::Outbox, "x", Filter::All, &st(false, None));
        assert_eq!(search, "No matches");

        // A new account's first download, over Loading (with its count once
        // known), and an empty folder once a sync pass has completed.
        let folder = View::Folder(7);
        let fresh = |f| ListState {
            syncing: true,
            fresh: Some(f),
            ..Default::default()
        };
        assert_eq!(
            text(folder, "", Filter::All, &fresh((0, 0))),
            "Downloading your mail\u{a0}… Newest messages arrive first."
        );
        assert_eq!(
            text(folder, "", Filter::All, &fresh((1234, 5000))),
            "Downloading your mail\u{a0}… 1,234 of 5,000. Newest messages arrive first."
        );
        let synced = ListState {
            synced: true,
            ..Default::default()
        };
        assert_eq!(
            text(folder, "", Filter::All, &synced),
            "This folder is empty"
        );
        // Filters keep their own words.
        assert_eq!(
            text(folder, "", Filter::Unread, &synced),
            "No unread messages"
        );
    }

    /// The icon code points in use, each checked against Segoe MDL2 Assets
    /// (the Windows 10 icon font; Windows 11's Segoe Fluent Icons, used
    /// when present, has them all too). None needed a substitute. A new
    /// icon is added here only once it is checked in that font.
    const MDL2_GLYPHS: [u32; 80] = [
        0xE700, 0xE70B, 0xE70D, 0xE70E, 0xE70F, 0xE710, 0xE711, 0xE712, 0xE713, 0xE715, 0xE71B,
        0xE721, 0xE723, 0xE724, 0xE72A, 0xE72B, 0xE72C, 0xE73E, 0xE74D, 0xE74E, 0xE762, 0xE76C,
        0xE77B, 0xE783, 0xE787, 0xE7B3, 0xE7B8, 0xE7BA, 0xE7C1, 0xE7C3, 0xE894, 0xE898, 0xE8A5,
        0xE8B7, 0xE8B9, 0xE8BB, 0xE8C2, 0xE8C3, 0xE8C8, 0xE8C9, 0xE8DB, 0xE8DD, 0xE8DE, 0xE8FB,
        0xE8FD, 0xE921, 0xE922, 0xE923, 0xE943, 0xE946, 0xE97A, 0xE9B2, 0xE9CE, 0xEB4B, 0xEC59,
        0xF136, 0xF13C, 0xF13D, 0xF13E, 0xF13F, 0xE765, 0xE897, 0xEA18, 0xEA80, 0xED15, 0xE74B,
        // Onboarding: LightningBolt, Globe, Lock.
        0xE945, 0xE774, 0xE72E,
        // The composer's toolbar: Underline, Strikethrough, FontColor,
        // DecreaseIndent, IncreaseIndent, AlignLeft, AlignCenter,
        // AlignRight, Table, Erase (clear formatting), Signature.
        0xE8DC, 0xEDE0, 0xE8D3, 0xE290, 0xE291, 0xE8E4, 0xE8E3, 0xE8E2, 0xE80A, 0xE75C, 0xEE56,
    ];

    #[test]
    fn icons_are_mdl2_glyphs() {
        let sources = [
            include_str!("../ui/theme.slint"),
            include_str!("../ui/widgets.slint"),
            include_str!("../ui/app.slint"),
            include_str!("../ui/onboarding.slint"),
            include_str!("../ui/editor.slint"),
            include_str!("role.rs"),
            include_str!("app.rs"),
        ];
        let mut used = Vec::new();
        for source in sources {
            // Escapes of private-use code points (this pattern has no
            // backslash, so the test doesn't find itself).
            for (i, _) in source
                .match_indices("u{E")
                .chain(source.match_indices("u{F"))
            {
                if !source[..i].ends_with('\\') {
                    continue;
                }
                let hex = &source[i + 2..];
                let hex = &hex[..hex.find('}').unwrap()];
                used.push(u32::from_str_radix(hex, 16).unwrap());
            }
        }
        assert!(used.len() > 40, "{used:?}");
        for cp in used {
            assert!(
                MDL2_GLYPHS.contains(&cp),
                "U+{cp:04X} is not a checked MDL2 glyph"
            );
        }
        // Where the fonts are installed, check the table against them.
        let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts\";
        for name in ["segmdl2.ttf", "SegoeIcons.ttf"] {
            let Ok(data) = std::fs::read(fonts.clone() + name) else {
                continue;
            };
            let font = skrifa::FontRef::new(&data).unwrap();
            let charmap = skrifa::MetadataProvider::charmap(&font);
            for cp in MDL2_GLYPHS {
                assert!(charmap.map(cp).is_some(), "U+{cp:04X} missing from {name}");
            }
        }
    }
}
