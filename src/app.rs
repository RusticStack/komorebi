//! UI controller: owns the main-thread state, feeds the Slint models and
//! routes user actions to the cache and the mail workers.

use crate::db::{self, Account, Db, Filter, Folder, MarkRead, NewAccount};
use crate::mail::mime::{self, Body};
use crate::mail::render::links::Mailto;
use crate::mail::smtp::{OutAttachment, Outgoing};
use crate::mail::{self, Class, Event, Job, Notify, Outcome, Problem, Workers};
use crate::providers::{self, Security, Server};
use crate::role::Role;
use crate::rows::{self, Rows, Selection};
use crate::trust;
use crate::{
    AccountForm, AccountRow, AppWindow, AttachmentRow, BodyEdit, ComposeField, FolderOption,
    FormatAction, NavItem, Suggestion, Theme, format, secrets,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

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
/// How long Archive, Delete, Move and Junk can be undone: their server
/// jobs wait this long.
const UNDO_WINDOW: Duration = Duration::from_secs(5);
/// After the window closes, how long actions already handed to the
/// workers (a held move sent at close) may take to reach the server.
const CLOSE_GRACE: Duration = Duration::from_secs(3);
/// The wake check runs this often; a wall-clock gap between two checks
/// above [`RESUME_GAP`] means the computer slept (or the clock jumped).
const RESUME_TICK: Duration = Duration::from_secs(60);
const RESUME_GAP: Duration = Duration::from_secs(120);
/// After a wake, the network gets this long to come back before the sync.
const RESUME_SETTLE: Duration = Duration::from_secs(3);

mod composer;
mod files;
#[cfg(debug_assertions)]
mod flow;
mod invite;
mod original;
mod outbox;
mod reader;
mod settings;
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

/// The action button on the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StatusAction {
    #[default]
    None,
    /// Open the last saved draft (offered once at start).
    ResumeDraft,
    /// Take back this outbox row during its undo window.
    Undo(i64),
    /// Take back the held move or delete with this sequence number.
    UndoMove(u64),
    /// Open this account's settings to update its password.
    EditAccount(i64),
}

/// An account's last sync, for its line in Settings (this session only).
#[derive(Clone, Debug, PartialEq, Eq)]
enum AccountState {
    Syncing,
    /// Unix seconds of the last sync that went through.
    Checked(i64),
    /// The server could not be reached.
    Unreachable,
    Failed(String),
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
    /// The message that was open, reopened by Undo.
    open: Option<i64>,
}

struct App {
    ui: Weak<AppWindow>,
    db: Rc<Db>,
    db_path: PathBuf,
    notify: Notify,
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    workers: HashMap<i64, Workers>,
    collapsed: HashSet<i64>,
    view: View,
    search: String,
    /// The Unread / Flagged toggle under the search box.
    filter: Filter,
    /// The "All folders" search scope (it applies while searching).
    search_all: bool,
    nav: Rc<VecModel<NavItem>>,
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
    /// Header downloads in progress (done, total), by account.
    progress: HashMap<i64, (usize, usize)>,
    /// The status line shows a download's progress (any other status
    /// replaces it; it never replaces another status).
    progress_shown: Cell<bool>,
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
    /// How long Send waits so it can be undone (setting `undo_send_secs`;
    /// 0 sends at once).
    undo_delay: Duration,
    /// Builds a sent message off the UI thread (tests: by hand).
    builder: Rc<dyn Fn(composer::BuildJob)>,
    /// Deletes an account's saved password (tests: a fake store; under
    /// `KOMOREBI_OFFLINE` nothing, as the store is never touched there).
    forget_password: Rc<dyn Fn(i64) -> anyhow::Result<()>>,
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
    /// Files dropped on the window, attached together once the drop ends.
    dropped: Vec<PathBuf>,
    reader: reader::Reader,
    started_sync: bool,
    /// The app theme (setting `theme`).
    theme: db::ThemeChoice,
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
}

/// Set up the controller on `ui` over the cache at `db_path`. Returns where
/// the window was when it last closed, if that was saved.
pub fn install(
    ui: &AppWindow,
    db_path: PathBuf,
    mark: &dyn Fn(&str),
) -> anyhow::Result<Option<db::Geometry>> {
    let theme = ui.global::<Theme>();
    theme.on_readable(settings::readable);
    let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts\";
    if std::path::Path::new(&(fonts.clone() + "SegoeIcons.ttf")).exists() {
        theme.set_icon_font("Segoe Fluent Icons".into());
    }
    if std::path::Path::new(&(fonts + "SegUIVar.ttf")).exists() {
        theme.set_text_font("Segoe UI Variable Text".into());
    }

    let open_dir = files::open_dir(&db_path);
    let started = std::time::SystemTime::now();
    let (db, damaged) = Db::open_ui(&db_path)?;
    let db = Rc::new(db);
    let settings = db.settings()?;
    if settings.diagnostics_log {
        komorebi::diag::set(Some(komorebi::diag::log_path(&db_path)));
    }
    // Before the window first shows (System leaves the style following
    // Windows).
    settings::apply_theme(ui, settings.theme, None);
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
    let rows = Rc::new(Rows::new(db.clone()));
    ui.set_nav_items(ModelRc::from(nav.clone()));
    ui.set_rows(ModelRc::from(rows.clone()));

    let mut app = App {
        ui: ui.as_weak(),
        db,
        db_path,
        notify,
        accounts: Vec::new(),
        folders: Vec::new(),
        workers: HashMap::new(),
        collapsed: HashSet::new(),
        view: View::AllInboxes,
        search: String::new(),
        filter: Filter::All,
        search_all: false,
        nav,
        rows,
        open: None,
        mark_read: settings.mark_read,
        read_wait: ReadWait::default(),
        read_timer: Timer::default(),
        syncing: HashSet::new(),
        warmed: HashSet::new(),
        account_state: HashMap::new(),
        shown_problem: HashMap::new(),
        progress: HashMap::new(),
        progress_shown: Cell::new(false),
        last_tick: SystemTime::now(),
        resume_timer: Timer::default(),
        drafts: composer::Drafts::default(),
        draft_count: 0,
        autosave_timer: Timer::default(),
        last_edit: Instant::now(),
        server_saves: HashMap::new(),
        pending_sends: HashMap::new(),
        send_seq: 0,
        undo_delay: Duration::from_secs(settings.undo_send_secs),
        builder: Rc::new(composer::spawn_build),
        // KOMOREBI_OFFLINE never touches the credential store.
        forget_password: if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            Rc::new(|_| Ok(()))
        } else {
            Rc::new(secrets::delete_password)
        },
        remove_data: false,
        outbox_count: 0,
        outbox_open: None,
        status_action: Cell::default(),
        question: None,
        invite_change: false,
        raw_wait: None,
        held: None,
        held_seq: 0,
        dropped: Vec::new(),
        reader,
        started_sync: false,
        theme: settings.theme,
        check_interval: Duration::from_secs(settings.check_interval_secs),
        last_full_sync: None,
        flash: settings.flash_taskbar,
        focused: true,
        inbox_unread: None,
        normal_geometry: None,
        reload_timer: Timer::default(),
        search_timer: Timer::default(),
        sync_timer: Timer::default(),
        status_timer: Timer::default(),
    };
    app.load_accounts()?;
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
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    if let Some(aside) = damaged {
        let name = aside.file_name().unwrap_or_default().to_string_lossy();
        let text = format!(
            "The mail cache was damaged, so Komorebi started a new one; the old file is kept as {name}. \
             Add your accounts again in Settings."
        );
        // After the launch notices, so this one stays up.
        Timer::single_shot(Duration::ZERO, move || notice(&text));
    }
    #[cfg(debug_assertions)]
    if let Some(dir) = komorebi::dev_var("KOMOREBI_FLOW") {
        flow::start(dir.into());
    }

    Timer::single_shot(START_SYNC_AFTER, start_sync);
    Timer::single_shot(START_SYNC_AFTER, || {
        with_app(|a| a.offer_drafts());
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
    let Some(path) = close_for_removal(ui) else {
        return false;
    };
    let left = wipe::remove_files(&path);
    if !left.is_empty() {
        let list: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Komorebi")
            .set_description(format!(
                "The passwords were deleted, but these files are still in use and were not \
                 removed. Delete them by hand:\n\n{}",
                list.join("\n")
            ))
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
    with_app(|a| a.db.optimize());
}

/// Run `f` on a short-lived thread named `name`. Boxed, as is `on_ui`'s
/// argument, so callers share one copy of the spawn code.
fn background(name: &str, f: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
}

/// Run `f` against the app state on the UI thread, from another thread.
fn on_ui(f: Box<dyn FnOnce(&mut App) + Send>) {
    slint::invoke_from_event_loop(move || {
        with_app(f);
    })
    .ok();
}

/// Show a status line that stays until the user dismisses it (or another
/// status replaces it).
pub fn notice(text: &str) {
    with_app(|a| {
        a.set_status(text, false);
        a.status_timer.stop();
    });
}

/// Snapshot QA: made-up attachments in the composer showing.
pub fn sample_attachments() {
    with_app(|a| a.sample_attachments());
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

/// The window gained or lost the keyboard focus.
pub fn window_focused(focused: bool) {
    with_app(|a| {
        a.focused = focused;
        if focused {
            a.refresh_title_bar();
        }
    });
}

/// The window exists: set its title bar's theme.
pub fn refresh_title_bar() {
    with_app(|a| a.refresh_title_bar());
}

/// Windows switched its app theme to dark (or light).
pub fn system_theme(dark: bool) {
    with_app(|a| a.system_theme(dark));
}

/// The window moved or was resized while neither maximized nor minimized.
pub fn window_placed(g: db::Geometry) {
    with_app(|a| a.normal_geometry = Some(g));
}

fn wire(ui: &AppWindow) {
    ui.on_nav_clicked(|item| {
        with_app(|a| a.nav_clicked(item));
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
    ui.on_c_close(|| {
        with_app(|a| a.close_composer());
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
    ui.on_edit_account(|id| {
        with_app(|a| a.edit_account(id as i64));
    });
    ui.on_remove_account(|id| {
        with_app(|a| a.remove_account(id as i64));
    });
    ui.on_settings_closed(|| {
        with_app(|a| {
            if let Some(ui) = a.ui.upgrade() {
                ui.set_a_error(SharedString::new());
                ui.set_a_edit_id(0);
                ui.set_a_edit(AccountForm::default());
            }
        });
    });
    ui.on_r_load_remote(|always| {
        with_app(|a| a.load_remote(always));
    });
    ui.on_settings_opened(|| {
        with_app(|a| {
            a.show_remote_senders();
            a.show_general();
        });
    });
    ui.on_general_changed(|which, value| {
        with_app(|a| a.general_changed(which, value));
    });
    ui.on_account_enabled(|id, on| {
        with_app(|a| a.set_account_enabled(id as i64, on));
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
    ui.on_open_repository(|| {
        use reader::Shell;
        reader::SystemShell.open_url(env!("CARGO_PKG_REPOSITORY"));
    });
    ui.on_forget_remote(|address| {
        with_app(|a| {
            if let Err(e) = a.db.forget_remote(&address) {
                a.set_status(&format!("{e:#}"), true);
            }
            a.show_remote_senders();
        });
    });
    ui.on_forget_all_remote(|| {
        with_app(|a| {
            if let Err(e) = a.db.forget_all_remote() {
                a.set_status(&format!("{e:#}"), true);
            }
            a.show_remote_senders();
        });
    });
    ui.on_dismiss_status(|| {
        with_app(|a| a.set_status("", false));
    });
    ui.on_status_action_clicked(|| {
        with_app(|a| match a.status_action.get() {
            StatusAction::None => {}
            StatusAction::ResumeDraft => a.resume_last_saved(),
            StatusAction::Undo(row) => a.undo_send(row),
            StatusAction::UndoMove(seq) => a.undo_held(seq),
            StatusAction::EditAccount(id) => {
                a.set_status("", false);
                a.edit_account(id);
            }
        });
    });
    // Drafts are saved and held moves sent before the window goes, and
    // where the window is, for the next start.
    ui.window().on_close_requested(|| {
        with_app(|a| {
            a.flush_held();
            a.save_all_drafts();
            a.save_geometry();
        });
        slint::CloseRequestResponse::HideWindow
    });
}

impl App {
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
        ui.set_c_from_options(ModelRc::new(VecModel::from(from)));
        // The accounts may have moved: the open draft's From follows.
        if let Some(d) = self.drafts.active_id().and_then(|id| self.drafts.get(id))
            && let Some(i) = self.accounts.iter().position(|a| a.id == d.account_id)
        {
            ui.set_c_from_index(i as i32);
        }
        self.rebuild_nav();
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
                let warn = self.sign_in_rejected(a.id);
                let status = if !a.enabled {
                    "Off: no connections; its downloaded mail stays".into()
                } else if let Some(&(done, total)) = self.progress.get(&a.id) {
                    format!(
                        "Downloading {} of {}…",
                        format::thousands(done),
                        format::thousands(total)
                    )
                } else {
                    account_status(self.account_state.get(&a.id), warn, &a.imap.host)
                };
                AccountRow {
                    id: a.id as i32,
                    email: a.email.as_str().into(),
                    detail: format!("{} · {}", a.imap.host, a.smtp.host).into(),
                    status: status.into(),
                    warn,
                    enabled: a.enabled,
                    dot: settings::account_color(i),
                    first: i == 0,
                    last: i == last,
                }
            })
            .collect();
        self.ui().set_accounts(ModelRc::new(VecModel::from(rows)));
    }

    /// The account's sign-in was rejected (or it has no saved password):
    /// nothing signs in until the password changes or Refresh.
    fn sign_in_rejected(&self, id: i64) -> bool {
        self.workers.get(&id).is_some_and(Workers::sign_in_rejected)
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
        let email = form.email.trim().to_string();
        let password = form.password.to_string();
        let parsed = (|| -> Result<NewAccount, String> {
            if !email.contains('@') {
                return Err("Enter your full email address.".into());
            }
            let (imap, smtp) = form_servers(&form)?;
            Ok(NewAccount {
                email: email.clone(),
                name: form.name.trim().to_string(),
                imap,
                smtp,
                save_sent: form.save_sent,
            })
        })();
        let new = match parsed {
            Ok(n) => n,
            Err(e) => {
                ui.set_a_error(e.into());
                return;
            }
        };
        if self
            .accounts
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&new.email))
        {
            ui.set_a_error("This account is already added.".into());
            return;
        }
        let signature = form.signature.to_string();
        ui.set_a_busy(true);
        ui.set_a_error(SharedString::new());
        let check = move || {
            let result = mail::verify_account(&new.imap, &new.smtp, &new.email, &password);
            on_ui(Box::new(move |app| {
                app.finish_add_account(new, signature, password, result)
            }));
        };
        background("check-account", Box::new(check)).expect("spawning account check");
    }

    fn finish_add_account(
        &mut self,
        new: NewAccount,
        signature: String,
        password: String,
        result: anyhow::Result<()>,
    ) {
        let ui = self.ui();
        ui.set_a_busy(false);
        if let Err(e) = result {
            ui.set_a_error(format!("Couldn't sign in. {e:#}").into());
            return;
        }
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
            };
            secrets::set_password(id, &password)
                .and_then(|()| self.db.update_account(&with_signature).map(drop))
                .inspect_err(|_| {
                    secrets::delete_password(id).ok();
                    self.db.remove_account(id).ok();
                })?;
            Ok(id)
        });
        match saved {
            Ok(id) => {
                if let Err(e) = self.load_accounts() {
                    self.set_status(&format!("{e:#}"), true);
                }
                if let Some(account) = self.accounts.iter().find(|a| a.id == id).cloned() {
                    self.spawn_workers(account);
                    self.workers[&id].send(Job::SyncAll { full: true });
                }
                ui.set_show_settings(false);
                self.set_status(&format!("Added {}. Downloading mail…", new.email), false);
            }
            Err(e) => ui.set_a_error(format!("{e:#}").into()),
        }
    }

    /// Edit (or "Update password"): the settings form, filled with the
    /// account's settings. The password is never shown.
    fn edit_account(&mut self, id: i64) {
        let Some(a) = self.accounts.iter().find(|a| a.id == id) else {
            return;
        };
        let ui = self.ui();
        ui.set_a_edit(AccountForm {
            id: a.id as i32,
            provider: providers::PRESETS.len() as i32,
            name: a.name.as_str().into(),
            email: a.email.as_str().into(),
            password: SharedString::new(),
            imap_host: a.imap.host.as_str().into(),
            imap_port: a.imap.port.to_string().into(),
            imap_starttls: a.imap.security == Security::StartTls,
            smtp_host: a.smtp.host.as_str().into(),
            smtp_port: a.smtp.port.to_string().into(),
            smtp_starttls: a.smtp.security == Security::StartTls,
            save_sent: a.save_sent,
            signature: a.signature.as_str().into(),
        });
        ui.set_a_edit_id(id as i32);
        ui.set_a_error(SharedString::new());
        ui.set_show_settings(true);
    }

    /// Save an edited account. A new password or other servers are
    /// checked first (IMAP, then SMTP) on a thread; the name, signature and
    /// Sent choice save at once.
    fn update_account(&mut self, form: AccountForm) {
        let ui = self.ui();
        let id = form.id as i64;
        let Some(old) = self.accounts.iter().find(|a| a.id == id).cloned() else {
            ui.set_a_error("This account was removed.".into());
            return;
        };
        let (imap, smtp) = match form_servers(&form) {
            Ok(servers) => servers,
            Err(e) => {
                ui.set_a_error(e.into());
                return;
            }
        };
        let edited = Account {
            name: form.name.trim().to_string(),
            imap,
            smtp,
            save_sent: form.save_sent,
            signature: form.signature.to_string(),
            ..old.clone()
        };
        let password = Some(form.password.to_string()).filter(|p| !p.is_empty());
        if password.is_none() && edited.imap == old.imap && edited.smtp == old.smtp {
            return self.finish_update_account(old, edited, None, Ok(()));
        }
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            ui.set_a_error("Offline: a password or server change can't be checked.".into());
            return;
        }
        ui.set_a_busy(true);
        ui.set_a_error(SharedString::new());
        let check = move || {
            let result = match &password {
                Some(p) => Ok(p.clone()),
                None => secrets::get_password(id),
            }
            .and_then(|p| mail::verify_account(&edited.imap, &edited.smtp, &edited.email, &p));
            on_ui(Box::new(move |app| {
                app.finish_update_account(old, edited, password, result)
            }));
        };
        background("check-account", Box::new(check)).expect("spawning account check");
    }

    fn finish_update_account(
        &mut self,
        old: Account,
        edited: Account,
        password: Option<String>,
        result: anyhow::Result<()>,
    ) {
        let ui = self.ui();
        ui.set_a_busy(false);
        let id = edited.id;
        if let Err(e) = result {
            ui.set_a_error(format!("Not saved. {e:#}").into());
            return;
        }
        // Removed while the check ran: never save a password for it.
        if !self.accounts.iter().any(|a| a.id == id) {
            ui.set_a_error("This account was removed.".into());
            return;
        }
        if let Some(p) = &password
            && let Err(e) = secrets::set_password(id, p)
        {
            ui.set_a_error(format!("Not saved. {e:#}").into());
            return;
        }
        if let Err(e) = self.db.update_account(&edited) {
            ui.set_a_error(format!("Not saved. {e:#}").into());
            return;
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&format!("{e:#}"), true);
        }
        // The workers hold the account as it was: servers and the Sent
        // choice need new ones; a new password only lifts the latch.
        let settings_changed =
            edited.imap != old.imap || edited.smtp != old.smtp || edited.save_sent != old.save_sent;
        if settings_changed && self.workers.contains_key(&id) {
            self.respawn_workers(id);
        } else if password.is_some()
            && let Some(w) = self.workers.get(&id)
        {
            w.credentials_changed();
            w.send(Job::SyncAll { full: true });
        }
        self.show_account_rows();
        self.rebuild_nav();
        ui.set_a_edit_id(0);
        ui.set_show_settings(false);
        self.set_status(&format!("Saved {}.", edited.email), false);
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
            self.set_status(&format!("Not removed. {e:#}"), true);
            return;
        }
        // A held move goes now (one of this account's is dropped with it).
        self.flush_held();
        // Dropping the workers closes their channels; the threads log out.
        self.workers.remove(&id);
        self.syncing.remove(&id);
        self.warmed.remove(&id);
        self.account_state.remove(&id);
        if let Err(e) = self.db.remove_account(id) {
            self.set_status(&format!("{e:#}"), true);
            return;
        }
        if self.ui().get_a_edit_id() == id as i32 {
            self.ui().set_a_edit_id(0);
        }
        // Its drafts and outbox went with it.
        if self.drafts.drop_account(id) && self.ui().get_composing() {
            self.ui().set_composing(false);
        }
        self.pending_sends.retain(|_, p| p.account != id);
        self.refresh_draft_count();
        self.outbox_changed();
        if let View::Folder(f) = self.view
            && self.folders.iter().any(|x| x.id == f && x.account_id == id)
        {
            self.view = View::AllInboxes;
        }
        if self.open.as_ref().is_some_and(|o| o.msg.account_id == id) {
            self.close_reader();
        }
        self.load_accounts().ok();
        self.reload_rows();
        self.ui().set_syncing(!self.syncing.is_empty());
    }

    /// Remove all data, confirmed: every account's password first (if one
    /// can't be deleted, nothing else is touched), then nothing more is
    /// sent or synced and the window closes; the files go after that
    /// ([`remove_data_if_asked`]).
    fn remove_all_data_now(&mut self) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        if let Err(e) = wipe::forget_passwords(&ids, &*self.forget_password) {
            self.set_status(&format!("Stopped, no files removed. {e:#}"), true);
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
        slint::quit_event_loop().ok();
    }

    // -------------------------------------------------------- navigation

    fn rebuild_nav(&mut self) {
        let mut unread = self.db.unread_counts().unwrap_or_default();
        // Held messages are out of the list, so out of the counts.
        for (folder, n) in self.held.iter().flat_map(|h| &h.unread) {
            if let Some(count) = unread.get_mut(folder) {
                *count = (*count - n).max(0);
            }
        }
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
        if self.accounts.is_empty() {
            update_model(&self.nav, Vec::new());
            return;
        }
        let mut items = vec![NavItem {
            kind: 0,
            id: 0,
            label: "All inboxes".into(),
            icon: "\u{E8F1}".into(),
            unread: inbox_unread,
            count: 0,
            depth: 0,
            expanded: true,
            warn: false,
            ..Default::default()
        }];
        // Shown while drafts are saved (and while it is the open view).
        if self.draft_count > 0 || self.view == View::Drafts {
            items.push(NavItem {
                kind: 3,
                id: 0,
                label: "Drafts".into(),
                icon: Role::Drafts.icon().into(),
                unread: 0,
                count: self.draft_count as i32,
                depth: 0,
                expanded: true,
                warn: false,
                ..Default::default()
            });
        }
        // Shown while messages wait in the outbox.
        if self.outbox_count > 0 || self.view == View::Outbox {
            items.push(NavItem {
                kind: 4,
                id: 0,
                label: "Outbox".into(),
                // Upload: unlike Sent's paper plane, also in the compact rail.
                icon: "\u{E898}".into(),
                unread: 0,
                count: self.outbox_count as i32,
                depth: 0,
                expanded: true,
                warn: false,
                ..Default::default()
            });
        }
        let dots = self.accounts.len() > 1;
        for (i, a) in self.accounts.iter().enumerate() {
            let expanded = !self.collapsed.contains(&a.id);
            let folders: Vec<&Folder> = self
                .folders
                .iter()
                .filter(|f| f.account_id == a.id)
                .collect();
            items.push(NavItem {
                kind: 1,
                id: a.id as i32,
                label: a.email.as_str().into(),
                icon: SharedString::new(),
                unread: folders
                    .iter()
                    .filter(|f| f.role == Role::Inbox)
                    .map(|f| count(f))
                    .sum(),
                count: 0,
                depth: 0,
                expanded,
                warn: self.sign_in_rejected(a.id),
                off: !a.enabled,
                dot: if dots {
                    settings::account_color(i)
                } else {
                    slint::Color::default()
                },
            });
            if expanded {
                items.extend(folders.iter().map(|f| NavItem {
                    kind: 2,
                    id: f.id as i32,
                    label: f.display.as_str().into(),
                    icon: f.role.icon().into(),
                    unread: count(f),
                    count: 0,
                    depth: f.depth.min(4) as i32,
                    expanded: true,
                    warn: false,
                    off: !a.enabled,
                    ..Default::default()
                }));
            }
        }
        update_model(&self.nav, items);
    }

    fn nav_clicked(&mut self, item: NavItem) {
        match item.kind {
            0 => self.set_view(View::AllInboxes),
            3 => self.set_view(View::Drafts),
            4 => self.set_view(View::Outbox),
            1 => {
                let id = item.id as i64;
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
                self.rebuild_nav();
            }
            _ => self.set_view(View::Folder(item.id as i64)),
        }
    }

    fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.search.clear();
        self.search_timer.stop();
        self.filter = Filter::All;
        self.search_all = false;
        let ui = self.ui();
        ui.set_search_text(SharedString::new());
        ui.set_list_filter(0);
        ui.set_search_all(false);
        ui.set_selected_folder(match view {
            View::AllInboxes => 0,
            View::Folder(id) => id as i32,
            View::Drafts => -1,
            View::Outbox => -2,
        });
        self.close_reader();
        self.outbox_open = None;
        ui.set_o_show(false);
        self.reload_rows();
        // The Drafts entry stays only while drafts are saved.
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
                self.db
                    .message_ids_filtered(&self.list_folders(), &self.search, self.filter),
                if self.searching_all() {
                    self.rows.set_folders(
                        self.folders
                            .iter()
                            .map(|f| (f.id, (f.display.as_str().into(), f.role.is_outgoing())))
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
            self.set_status(&format!("{e:#}"), true);
            Vec::new()
        });
        if let Some(held) = self.held.as_ref().filter(|_| self.lists_messages()) {
            ids.retain(|id| !held.ids.contains(id));
        }
        // With more than one account, All inboxes marks each row with its
        // account's dot.
        let dots = if self.view == View::AllInboxes && self.accounts.len() > 1 {
            self.accounts
                .iter()
                .enumerate()
                .map(|(i, a)| (a.id, settings::account_color(i)))
                .collect()
        } else {
            HashMap::new()
        };
        self.rows.set_dots(dots);
        self.rows.set(ids, kind);

        let ui = self.ui();
        ui.set_list_title(match self.view {
            _ if self.searching_all() => "All folders".into(),
            View::AllInboxes => "All inboxes".into(),
            View::Drafts => "Drafts".into(),
            View::Outbox => "Outbox".into(),
            View::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.display.as_str().into())
                .unwrap_or_default(),
        });
        self.show_empty();
        self.sync_selection_index();
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
        let text = empty_text(&self.search, self.filter, !self.syncing.is_empty(), failed);
        self.ui().set_list_empty(text.into());
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

    /// Esc in the list: clears the search, else the selection and the
    /// message open, else the status line.
    fn escape(&mut self) {
        if !self.reading() {
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
            return self.set_status("", false);
        }
        self.close_reader();
    }

    /// Mark every message of the view read: in the cache at once, then one
    /// job per folder (one STORE, up to the highest UID cached, so mail
    /// that arrived since the last sync stays unread).
    fn mark_all_read(&mut self) {
        if !self.reading() || !self.lists_messages() {
            return;
        }
        for folder_id in self.view_folders() {
            let Some(account) = self
                .folders
                .iter()
                .find(|f| f.id == folder_id)
                .map(|f| f.account_id)
            else {
                continue;
            };
            let (uids, through) = match self.db.mark_folder_read(folder_id) {
                Ok(marked) => marked,
                Err(e) => return self.set_status(&format!("{e:#}"), true),
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
        self.read_wait.cancel();
        self.read_timer.stop();
        if let Some(open) = self.open.as_mut() {
            open.msg.seen = true;
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
        ui.set_selected_count(selection.len() as i32);
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
            return self.set_status(&format!("{e:#}"), true);
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
        }
    }

    /// What an action applies to: the open message, or with none open the
    /// selected rows (several, or one picked with Ctrl or Shift).
    fn targets(&self) -> Vec<db::Brief> {
        if let Some(open) = &self.open {
            return vec![db::Brief::from(&open.msg)];
        }
        let ids = self.rows.selection().ids();
        self.db.briefs(&ids).unwrap_or_else(|e| {
            self.set_status(&format!("{e:#}"), true);
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
            // The selection moves within one account: the view's, if it
            // has only one (in a search of all folders, the selection's).
            let folders = if self.searching_all() {
                let ids = self.rows.selection().ids();
                let mut folders: Vec<i64> = self
                    .db
                    .briefs(&ids)
                    .unwrap_or_default()
                    .iter()
                    .map(|m| m.folder_id)
                    .collect();
                folders.sort_unstable();
                folders.dedup();
                folders
            } else {
                self.view_folders()
            };
            let mut accounts = self
                .folders
                .iter()
                .filter(|f| folders.contains(&f.id))
                .map(|f| f.account_id);
            let account = accounts.next().filter(|&a| accounts.all(|b| b == a));
            let targets = account.map_or_else(Vec::new, |a| self.move_options(a, &folders));
            self.ui()
                .set_move_targets(ModelRc::new(VecModel::from(targets)));
        }
        self.sync_selection_index();
    }

    /// The folders of `account` a message can move to, but `except`.
    fn move_options(&self, account: i64, except: &[i64]) -> Vec<FolderOption> {
        self.folders
            .iter()
            .filter(|f| f.account_id == account && !except.contains(&f.id))
            .map(|f| FolderOption {
                id: f.id as i32,
                label: f.display.as_str().into(),
            })
            .collect()
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
        // Drafts and Outbox rows are not messages; they open with a click.
        if !self.reading() || !self.lists_messages() {
            return;
        }
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        let current = self.current_row().and_then(|id| self.rows.index_of(id));
        let next = pick(current, last);
        if Some(next) != current
            && let Some(id) = self.rows.id_at(next)
        {
            self.select_message(id);
        }
    }

    /// The open message, else the row Shift+Up/Down move from.
    fn current_row(&self) -> Option<i64> {
        self.open
            .as_ref()
            .map(|o| o.msg.id)
            .or(self.rows.selection().cursor())
    }

    /// Enter in the list: open the current row (with none, the first).
    /// True when a message shows, so the page can take the focus.
    fn open_current(&mut self) -> bool {
        if !self.reading() || !self.lists_messages() {
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
        let ui = self.ui();
        if ui.get_composing() {
            // The draft is saved and kept; "New mail" brings it back.
            if let Some(id) = self.drafts.active_id() {
                self.save_draft(id);
            }
            ui.set_composing(false);
        }
        let mut msg = match self.db.message(id) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        };
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
        let folder_id = msg.folder_id;
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
        let targets = self.move_options(account_id, &[folder_id]);
        ui.set_move_targets(ModelRc::new(VecModel::from(targets)));
        self.rows.select(|s, _| s.single(id));
        self.show_open();
        self.sync_selection_index();
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
        if raw.len() > SYNC_PARSE_MAX {
            let src = reader::Source::Raw(raw.into());
            self.reader.open(id, src, remote);
            return;
        }
        let (body, input) = mime::parse_reader(raw);
        open.body = Some(body);
        if input.prefers_plain {
            self.reader.close();
        } else {
            let src = reader::Source::Input(Arc::new(input));
            self.reader.open(id, src, remote);
        }
    }

    /// Load the open message's remote images: this once, or from now on
    /// for its sender's exact address.
    fn load_remote(&mut self, always: bool) {
        let Some(open) = &self.open else {
            return;
        };
        if always {
            if let Err(e) = self.db.allow_remote(&open.msg.from_addr) {
                return self.set_status(&format!("{e:#}"), true);
            }
            self.show_remote_senders();
        }
        self.reader.allow_remote();
    }

    /// Fill the Settings list of senders whose remote images always load.
    fn show_remote_senders(&self) {
        let senders: Vec<SharedString> = match self.db.remote_allow_list() {
            Ok(list) => list.into_iter().map(SharedString::from).collect(),
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        };
        self.ui()
            .set_remote_senders(ModelRc::new(VecModel::from(senders)));
    }

    fn on_render(&mut self, reply: reader::Reply) {
        if let Some(body) = self.reader.on_reply(reply)
            && let Some(open) = self.open.as_mut()
            && open.body.is_none()
        {
            open.body = Some(body);
            self.show_open();
        }
    }

    fn show_open(&self) {
        let ui = self.ui();
        let Some(open) = &self.open else {
            ui.set_has_message(false);
            return;
        };
        let m = &open.msg;
        ui.set_has_message(true);
        ui.set_r_subject(m.subject.as_str().into());
        ui.set_r_from_name(format::sender(&m.from_name, &m.from_addr).into());
        ui.set_r_from_addr(m.from_addr.as_str().into());
        ui.set_r_initials(format::initials(&m.from_name, &m.from_addr).into());
        ui.set_r_avatar(format::avatar_color(&m.from_addr));
        ui.set_r_date(format::full_date(m.date).into());
        ui.set_r_to(format::recipients(&m.to).into());
        ui.set_r_cc(format::recipients(&m.cc).into());
        ui.set_r_flagged(m.flagged);
        ui.set_r_unread(!m.seen);
        ui.set_loading(m.raw.is_none());
        match &open.body {
            Some(body) => {
                let text = &body.text;
                ui.set_r_body(if text.len() > MAX_BODY_BYTES {
                    let cut = text.floor_char_boundary(MAX_BODY_BYTES);
                    format!("{}\n\n[Message shortened for display]", &text[..cut]).into()
                } else {
                    text.as_str().into()
                });
                let atts: Vec<AttachmentRow> = body
                    .attachments
                    .iter()
                    .map(|a| AttachmentRow {
                        index: a.index as i32,
                        name: a.name.as_str().into(),
                        size: format::size(a.size).into(),
                    })
                    .collect();
                ui.set_r_attachments(ModelRc::new(VecModel::from(atts)));
                let host = self
                    .accounts
                    .iter()
                    .find(|a| a.id == m.account_id)
                    .map_or("", |a| a.imap.host.as_str());
                let mut notice = trust::notice(&body.auth, host, &m.from_name, &m.from_addr);
                ui.set_r_notice_warn(!notice.is_empty());
                if let Some(kind) = body.encrypted {
                    if !notice.is_empty() {
                        notice.push('\n');
                    }
                    notice.push_str(&format!(
                        "This message is encrypted ({}). Komorebi cannot decrypt it; save the \
                         encrypted file to open it in a program that has the key.",
                        kind.label()
                    ));
                }
                ui.set_r_notice(notice.into());
                ui.set_r_unsubscribe(!body.unsubscribe.is_empty());
            }
            None => {
                ui.set_r_body(SharedString::new());
                ui.set_r_attachments(ModelRc::default());
                ui.set_r_notice(SharedString::new());
                ui.set_r_unsubscribe(false);
            }
        }
        self.show_invite();
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

    /// Ctrl+Q: all read if any is unread, else all unread.
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
        self.remove_to(Dest::Trash);
    }

    fn move_to(&mut self, dest: i64) {
        self.remove_to(Dest::Folder(dest));
    }

    /// Move the open message or the selection out of its folders (one job
    /// per folder) and open the message that took its place. The list
    /// changes at once; the server jobs wait out the undo window. Messages
    /// already where they would go stay.
    fn remove_to(&mut self, dest: Dest) {
        if !self.reading() {
            return;
        }
        let targets = self.targets();
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
                        missing = Some("This account has no Archive folder.");
                        continue;
                    }
                },
                Dest::Trash => match role_folder(Role::Trash) {
                    // From Trash itself: deleted.
                    Some(trash) if trash.id == folder_id => None,
                    Some(trash) => Some(trash.id),
                    None => {
                        missing = Some("This account has no Trash folder.");
                        continue;
                    }
                },
                // Never somewhere else instead: without a Junk folder the
                // message stays put.
                Dest::Junk => match role_folder(Role::Junk) {
                    Some(f) => Some(f.id),
                    None => {
                        missing = Some("This account has no Junk folder, so nothing was moved.");
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
                            missing = Some("This account has no Inbox folder.");
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
                self.set_status(text, true);
            }
            return;
        };
        let mut label = removed_label(&place, removed.len());
        if let Some(text) = missing {
            label = format!("{label}. {text}");
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
        for m in self.db.briefs(&list).unwrap_or_default() {
            if !m.seen {
                *unread.entry(m.folder_id).or_default() += 1;
            }
        }
        let open = self
            .open
            .as_ref()
            .map(|o| o.msg.id)
            .filter(|id| ids.contains(id));
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
            open,
        });
        self.rebuild_nav();
        match index.and_then(|i| self.rows.neighbour(i)) {
            Some(id) => self.select_message(id),
            None => self.close_reader(),
        }
        self.set_status(label, false);
        self.set_status_action("Undo", StatusAction::UndoMove(seq));
        Timer::single_shot(UNDO_WINDOW, move || {
            with_app(|a| {
                if a.held.as_ref().is_some_and(|h| h.seq == seq) {
                    a.flush_held();
                }
            });
        });
    }

    /// Send the held jobs to the server now and drop their messages from
    /// the cache. Queued first: that marks the UIDs as pending, so a
    /// running sync can't re-add them after the local delete.
    fn flush_held(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        if self.status_action.get() == StatusAction::UndoMove(held.seq) {
            self.set_status("", false);
        }
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
        self.set_status("", false);
        self.reload_rows();
        self.rebuild_nav();
        if !self.reading() {
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
        if !self.reading() || !self.lists_messages() {
            return false;
        }
        self.select_rows(|s, _| s.right_click(id));
        self.menu_opening();
        true
    }

    /// The row menu or the reader's More menu opens: its labels follow
    /// what it acts on.
    fn menu_opening(&self) {
        let targets = self.targets();
        let ui = self.ui();
        ui.set_menu_unread(targets.iter().any(|m| !m.seen));
        ui.set_menu_unflagged(targets.iter().any(|m| !m.flagged));
        ui.set_menu_junk(
            !targets.is_empty()
                && targets
                    .iter()
                    .all(|m| self.folder_role(m.folder_id) == Role::Junk),
        );
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
            .map(|f| f.display.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ events

    fn on_event(&mut self, event: Event) {
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
                }
                self.ui().set_syncing(!self.syncing.is_empty());
                let state = match &error {
                    _ if active => AccountState::Syncing,
                    None => AccountState::Checked(chrono::Utc::now().timestamp()),
                    Some(p) if p.class == Class::Offline => AccountState::Unreachable,
                    Some(p) => AccountState::Failed(p.text().to_string()),
                };
                self.account_state.insert(account_id, state);
                self.show_account_rows();
                self.rebuild_nav();
                self.show_empty();
                if let Some(p) = error {
                    self.sync_error(account_id, &p);
                } else if !active {
                    // It went through: a problem shows again if it returns.
                    self.shown_problem.remove(&account_id);
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
                self.rebuild_nav();
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
                self.raw_arrived(message_id);
                self.schedule_reload();
            }
            Event::BodyFailed { message_id, error } => {
                self.raw_failed(message_id, &error);
                if self.open.as_ref().is_some_and(|o| o.msg.id == message_id) {
                    self.reader.close();
                    let ui = self.ui();
                    ui.set_loading(false);
                    ui.set_r_body(format!("Couldn't download this message.\n\n{error}").into());
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
                self.set_status(&format!("{email}: {message}"), false);
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
        let email = self.account_email(account_id);
        self.set_status(&format!("{email}: {}", problem.text()), true);
        if self.sign_in_rejected(account_id) {
            self.set_status_action("Update password", StatusAction::EditAccount(account_id));
        }
    }

    /// A folder's headers are downloading: Settings and the status line
    /// say how far (the status line only while it shows nothing else).
    fn show_progress(&mut self, account_id: i64, folder_id: i64, done: usize, total: usize) {
        self.progress.insert(account_id, (done, total));
        self.show_account_rows();
        if self.ui().get_status().is_empty() || self.progress_shown.get() {
            let text = format!(
                "{}: downloading {} of {} in {}",
                self.account_email(account_id),
                format::thousands(done),
                format::thousands(total),
                self.folder_name(folder_id)
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
        if self
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
    fn set_status_action(&self, label: &str, action: StatusAction) {
        self.ui().set_status_action(label.into());
        self.status_action.set(action);
    }

    /// Ask `q` in the in-app prompt; [`App::answer`] acts on the reply.
    fn ask(&mut self, q: Question) {
        let (title, text, ok) = match q {
            Question::Resend(_) => (
                "Send this message again?".to_string(),
                "It may already have been sent. Check your Sent folder first, \
                 or the recipients may get it twice."
                    .to_string(),
                "Send again",
            ),
            Question::DiscardOutbox(_) => (
                "Discard this message?".into(),
                "It is not sent, and it is deleted from this computer with its attachments.".into(),
                "Discard",
            ),
            Question::RemoveAccount(id) => {
                let mut text = "Removes this account and its downloaded mail from this \
                                computer. Mail on the server is not affected."
                    .to_string();
                match self.db.count_unsent(id).unwrap_or(0) {
                    0 => {}
                    1 => text.push_str(" Its 1 unsent draft or Outbox message will be lost."),
                    n => text.push_str(&format!(
                        " Its {n} unsent drafts and Outbox messages will be lost."
                    )),
                }
                (
                    format!("Remove {}?", self.account_email(id)),
                    text,
                    "Remove",
                )
            }
            Question::Rsvp(_, answer) => {
                let (title, text) = self.rsvp_prompt(answer);
                (title, text, "Send reply")
            }
            Question::Redownload(id) => (
                format!("Download the mail of {} again?", self.account_email(id)),
                "Komorebi forgets the mail it downloaded for this account and downloads \
                 it again. Mail on the server is not affected; drafts, the Outbox and \
                 settings stay."
                    .into(),
                "Download again",
            ),
            Question::RemoveAllData => {
                let mut text = "Deletes the saved passwords of all accounts, then the downloaded \
                                mail, drafts, Outbox, settings and logs on this computer, and \
                                closes Komorebi. Mail on the server is not affected."
                    .to_string();
                let unsent: i64 = self
                    .accounts
                    .iter()
                    .map(|a| self.db.count_unsent(a.id).unwrap_or(0))
                    .sum();
                match unsent {
                    0 => {}
                    1 => text.push_str(" 1 unsent draft or Outbox message will be lost."),
                    n => text.push_str(&format!(
                        " {n} unsent drafts and Outbox messages will be lost."
                    )),
                }
                (
                    "Remove all of Komorebi's data?".into(),
                    text,
                    "Remove all data",
                )
            }
        };
        self.question = Some(q);
        let ui = self.ui();
        let alt = matches!(q, Question::Rsvp(..)).then_some("Don't send a reply");
        ui.set_q_alt(alt.unwrap_or_default().into());
        ui.set_q_text(text.into());
        ui.set_q_ok(ok.into());
        ui.set_q_title(title.into());
    }

    /// The prompt was answered (accepted, or cancelled).
    fn answer(&mut self, accepted: bool) {
        self.ui().set_q_title(SharedString::new());
        let Some(q) = self.question.take().filter(|_| accepted) else {
            return;
        };
        match q {
            Question::Resend(row) => self.retry_now(row),
            Question::DiscardOutbox(row) => self.discard_outbox_now(row),
            Question::RemoveAccount(id) => self.remove_account_now(id),
            Question::Rsvp(msg, answer) => self.rsvp(msg, answer, true),
            Question::Redownload(id) => self.redownload_now(id),
            Question::RemoveAllData => self.remove_all_data_now(),
        }
    }

    /// The prompt's third answer: an invitation answered without a reply.
    fn answer_alt(&mut self) {
        self.ui().set_q_title(SharedString::new());
        if let Some(Question::Rsvp(msg, answer)) = self.question.take() {
            self.rsvp(msg, answer, false);
        }
    }

    fn set_status(&self, text: &str, error: bool) {
        self.progress_shown.set(false);
        let ui = self.ui();
        ui.set_status(text.into());
        ui.set_status_error(error);
        ui.set_status_action(SharedString::new());
        self.status_action.set(StatusAction::None);
        if text.is_empty() {
            self.status_timer.stop();
        } else {
            let after = if error {
                Duration::from_secs(12)
            } else {
                Duration::from_secs(5)
            };
            self.status_timer.start(TimerMode::SingleShot, after, || {
                with_app(|a| a.set_status("", false));
            });
        }
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
        let Some((name, data)) = found else { return };
        if files::risky(&name) {
            return self.set_status(
                &format!("{name} can run programs, so Komorebi doesn't open it. Save it to check it first."),
                true,
            );
        }
        let dir = files::open_dir(&self.db_path);
        // Writing can take a moment and ShellExecuteW can block (DDE, a
        // slow handler): both stay off the UI thread.
        let open = move || {
            let error = match files::write_open_copy(&dir, &name, &data) {
                Ok(path) if reader::shell_execute(path.as_os_str()) => return,
                Ok(_) => format!("No app on this computer opens {name}."),
                Err(e) => format!("Couldn't open {name}: {e}"),
            };
            on_ui(Box::new(move |a| a.set_status(&error, true)));
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
        save_file("Save attachment", &name, data);
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
            Ok(()) => a.set_status(&format!("Saved {}", path.display()), false),
            Err(e) => a.set_status(&format!("Couldn't save {name}: {e}"), true),
        }));
    };
    if let Err(e) = background("save-file", Box::new(write)) {
        with_app(|a| a.set_status(&format!("Couldn't save: {e}"), true));
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

/// Messages grouped by (account, folder): one server job each.
/// The undo offer after moving `count` messages to the folder `place`
/// (empty: deleted).
fn removed_label(place: &str, count: usize) -> String {
    match (count, place) {
        (1, "") => "Deleted".into(),
        (1, _) => format!("Moved to {place}"),
        (_, "") => format!("{count} messages deleted"),
        _ => format!("{count} messages moved to {place}"),
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
fn form_servers(form: &AccountForm) -> Result<(Server, Server), String> {
    if let Some(p) = providers::PRESETS.get(form.provider as usize) {
        return Ok((p.imap_server(), p.smtp_server()));
    }
    let port = |s: &str, what: &str| {
        s.trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| format!("Enter a valid {what} port."))
    };
    let security = |starttls: bool| {
        if starttls {
            Security::StartTls
        } else {
            Security::Tls
        }
    };
    let (imap_host, smtp_host) = (form.imap_host.trim(), form.smtp_host.trim());
    if imap_host.is_empty() || smtp_host.is_empty() {
        return Err("Enter the IMAP and SMTP server names.".into());
    }
    Ok((
        Server {
            host: imap_host.into(),
            port: port(&form.imap_port, "IMAP")?,
            security: security(form.imap_starttls),
        },
        Server {
            host: smtp_host.into(),
            port: port(&form.smtp_port, "SMTP")?,
            security: security(form.smtp_starttls),
        },
    ))
}

/// An account's line in Settings (empty before its first sync).
fn account_status(state: Option<&AccountState>, rejected: bool, host: &str) -> String {
    if rejected {
        return "Sign-in rejected".into();
    }
    match state {
        None => String::new(),
        Some(AccountState::Syncing) => "Syncing…".into(),
        Some(AccountState::Checked(at)) => {
            match chrono::TimeZone::timestamp_opt(&chrono::Local, *at, 0).single() {
                Some(t) => format!("Up to date, checked {}", t.format("%H:%M")),
                None => "Up to date".into(),
            }
        }
        Some(AccountState::Unreachable) => format!("Can't reach {host}"),
        Some(AccountState::Failed(e)) => format!("Last sync failed: {e}"),
    }
}

/// What an empty list says: no match for the search, nothing under the
/// filter, still loading, a sync that failed, or truly nothing.
fn empty_text(
    search: &str,
    filter: Filter,
    syncing: bool,
    failed: Option<&AccountState>,
) -> &'static str {
    match filter {
        _ if !search.trim().is_empty() => "No matches",
        Filter::Unread => "No unread messages",
        Filter::Flagged => "No flagged messages",
        Filter::All if syncing => "Loading…",
        Filter::All => match failed {
            Some(AccountState::Unreachable) => {
                "Can't reach the server, so this list may be incomplete"
            }
            Some(AccountState::Failed(_)) => {
                "Couldn't check for new mail, so this list may be incomplete"
            }
            _ => "Nothing here",
        },
    }
}

/// Whether a search over `scope` cached messages waits for a typing pause.
fn debounces_search(scope: u32, text: &str) -> bool {
    scope > SEARCH_DEBOUNCE_ABOVE && !text.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(
                form_servers(&bad).unwrap_err(),
                format!("Enter a valid {field} port.")
            );
        }
        let no_host = AccountForm {
            smtp_host: " ".into(),
            ..manual
        };
        assert!(form_servers(&no_host).is_err());
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
        let host = "imap.example.org";
        assert_eq!(account_status(None, false, host), "");
        assert_eq!(
            account_status(Some(&AccountState::Syncing), false, host),
            "Syncing…"
        );
        let at = chrono::Utc::now().timestamp();
        let checked = account_status(Some(&AccountState::Checked(at)), false, host);
        assert!(checked.starts_with("Up to date, checked "), "{checked}");
        assert_eq!(checked.len(), "Up to date, checked 12:41".len());
        assert_eq!(
            account_status(Some(&AccountState::Unreachable), false, host),
            "Can't reach imap.example.org"
        );
        // A rejected sign-in wins over the last pass.
        assert_eq!(
            account_status(Some(&AccountState::Checked(at)), true, host),
            "Sign-in rejected"
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
        let failed = AccountState::Failed("no".into());
        let cases = [
            ("x", Filter::Unread, true, None, "No matches"),
            ("", Filter::Unread, true, None, "No unread messages"),
            (
                "",
                Filter::Flagged,
                false,
                Some(&failed),
                "No flagged messages",
            ),
            ("", Filter::All, true, Some(&failed), "Loading…"),
            ("  ", Filter::All, false, None, "Nothing here"),
        ];
        for (search, filter, syncing, state, want) in cases {
            assert_eq!(empty_text(search, filter, syncing, state), want);
        }
        let text = empty_text("", Filter::All, false, Some(&failed));
        assert!(text.starts_with("Couldn't check for new mail"), "{text}");
        let text = empty_text("", Filter::All, false, Some(&AccountState::Unreachable));
        assert!(text.starts_with("Can't reach the server"), "{text}");
    }

    /// The icon code points in use, each checked against Segoe MDL2 Assets
    /// (the Windows 10 icon font; Windows 11's Segoe Fluent Icons, used
    /// when present, has them all too). None needed a substitute. A new
    /// icon is added here only once it is checked in that font.
    const MDL2_GLYPHS: [u32; 42] = [
        0xE70B, 0xE70D, 0xE70E, 0xE70F, 0xE710, 0xE711, 0xE712, 0xE713, 0xE715, 0xE71B, 0xE721,
        0xE723, 0xE724, 0xE72A, 0xE72B, 0xE72C, 0xE74D, 0xE74E, 0xE762, 0xE76C, 0xE77B, 0xE783,
        0xE790, 0xE7B8, 0xE7C1, 0xE898, 0xE8A5, 0xE8B7, 0xE8B9, 0xE8C2, 0xE8C3, 0xE8C9, 0xE8DB,
        0xE8DD, 0xE8DE, 0xE8F1, 0xE8FB, 0xE8FD, 0xE946, 0xE97A, 0xE9CE, 0xEB4B,
    ];

    #[test]
    fn icons_are_mdl2_glyphs() {
        let sources = [
            include_str!("../ui/theme.slint"),
            include_str!("../ui/widgets.slint"),
            include_str!("../ui/app.slint"),
            include_str!("role.rs"),
            include_str!("app.rs"),
        ];
        let mut used = Vec::new();
        for source in sources {
            // Escapes of private-use code points (this pattern has no
            // backslash, so the test doesn't find itself).
            for (i, _) in source.match_indices("u{E") {
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
