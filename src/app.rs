//! UI controller: owns the main-thread state, feeds the Slint models and
//! routes user actions to the cache and the mail workers.

use crate::db::{self, Account, Db, Folder, NewAccount};
use crate::mail::mime::{self, Body};
use crate::mail::smtp::{OutAttachment, Outgoing};
use crate::mail::{self, Event, Job, Notify, Workers};
use crate::providers::{self, Security, Server};
use crate::role::Role;
use crate::rows::Rows;
use crate::{
    AccountForm, AccountRow, AppWindow, AttachmentRow, FolderOption, NavItem, Theme, format,
    secrets,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(150);
/// Longer bodies are cut for display (the text widget lays out all of it).
const MAX_BODY_BYTES: usize = 300_000;
/// The software renderer has no first-frame hook, so network work starts
/// after a short delay that lets the first frame paint.
const START_SYNC_AFTER: Duration = Duration::from_millis(250);
const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

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
}

struct Open {
    msg: db::Message,
    body: Option<Body>,
}

#[derive(Clone, Copy)]
enum Flag {
    Seen,
    Flagged,
}

#[derive(Default)]
struct Compose {
    reply_to: Option<i64>,
    in_reply_to: String,
    references: String,
    attachments: Vec<OutAttachment>,
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
    nav: Rc<VecModel<NavItem>>,
    rows: Rc<Rows>,
    open: Option<Open>,
    syncing: HashSet<i64>,
    compose: Compose,
    started_sync: bool,
    reload_timer: Timer,
    sync_timer: Timer,
    status_timer: Timer,
}

pub fn install(ui: &AppWindow, db_path: PathBuf, mark: &dyn Fn(&str)) -> anyhow::Result<()> {
    let theme = ui.global::<Theme>();
    let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()) + r"\Fonts\";
    if std::path::Path::new(&(fonts.clone() + "SegoeIcons.ttf")).exists() {
        theme.set_icon_font("Segoe Fluent Icons".into());
    }
    if std::path::Path::new(&(fonts + "SegUIVar.ttf")).exists() {
        theme.set_text_font("Segoe UI Variable Text".into());
    }

    let db = Rc::new(Db::open(&db_path)?);
    mark("database open");
    let notify: Notify = Arc::new(|event: Event| {
        slint::invoke_from_event_loop(move || {
            with_app(|app| app.on_event(event));
        })
        .ok();
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
        nav,
        rows,
        open: None,
        syncing: HashSet::new(),
        compose: Compose::default(),
        started_sync: false,
        reload_timer: Timer::default(),
        sync_timer: Timer::default(),
        status_timer: Timer::default(),
    };
    app.load_accounts()?;
    // KOMOREBI_OFFLINE keeps the app off the network and away from the
    // credential store (demos, timing runs).
    if std::env::var_os("KOMOREBI_OFFLINE").is_none() {
        for account in app.accounts.clone() {
            app.spawn_workers(account);
        }
    }
    app.reload_rows();
    mark("model ready");
    wire(ui);
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    Timer::single_shot(START_SYNC_AFTER, start_sync);
    Ok(())
}

/// Start network work once the window is on screen.
fn start_sync() {
    with_app(|app| {
        if app.started_sync {
            return;
        }
        app.started_sync = true;
        app.sync_all();
        app.sync_timer
            .start(TimerMode::Repeated, SYNC_INTERVAL, || {
                with_app(|app| app.sync_all());
            });
    });
}

fn wire(ui: &AppWindow) {
    ui.on_nav_clicked(|item| {
        with_app(|a| a.nav_clicked(item));
    });
    ui.on_message_selected(|id| {
        with_app(|a| a.select_message(id as i64));
    });
    ui.on_select_offset(|d| {
        with_app(|a| a.select_offset(d));
    });
    ui.on_refresh(|| {
        with_app(|a| a.sync_all());
    });
    ui.on_search_edited(|text| {
        with_app(|a| {
            a.search = text.to_string();
            a.reload_rows();
        });
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
        with_app(|a| a.move_open_to(id as i64));
    });
    ui.on_save_attachment(save_attachment);
    ui.on_compose(|| {
        with_app(|a| a.compose_new());
    });
    ui.on_send(|| {
        with_app(|a| a.send());
    });
    ui.on_discard(|| {
        with_app(|a| a.discard());
    });
    ui.on_attach(attach_files);
    ui.on_remove_attachment(|i| {
        with_app(|a| {
            let i = i as usize;
            if i < a.compose.attachments.len() {
                a.compose.attachments.remove(i);
                a.show_compose_attachments();
            }
        });
    });
    ui.on_add_account(|form| {
        with_app(|a| a.add_account(form));
    });
    ui.on_remove_account(|id| {
        with_app(|a| a.remove_account(id as i64));
    });
    ui.on_settings_closed(|| {
        with_app(|a| {
            if let Some(ui) = a.ui.upgrade() {
                ui.set_a_error(SharedString::new());
            }
        });
    });
    ui.on_dismiss_status(|| {
        with_app(|a| a.set_status("", false));
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
        let rows: Vec<AccountRow> = self
            .accounts
            .iter()
            .map(|a| AccountRow {
                id: a.id as i32,
                email: a.email.as_str().into(),
                detail: format!("{} · {}", a.imap.host, a.smtp.host).into(),
            })
            .collect();
        ui.set_accounts(ModelRc::new(VecModel::from(rows)));
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
        self.rebuild_nav();
        Ok(())
    }

    fn spawn_workers(&mut self, account: Account) {
        let id = account.id;
        let workers = Workers::spawn(account, self.db_path.clone(), self.notify.clone());
        self.workers.insert(id, workers);
    }

    fn sync_all(&mut self) {
        for w in self.workers.values() {
            w.send(Job::SyncAll);
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
            let (imap, smtp) = match providers::PRESETS.get(form.provider as usize) {
                Some(p) => (p.imap_server(), p.smtp_server()),
                None => {
                    let port = |s: &str, what: &str| {
                        s.trim()
                            .parse::<u16>()
                            .map_err(|_| format!("Enter a valid {what} port."))
                    };
                    let imap_host = form.imap_host.trim();
                    let smtp_host = form.smtp_host.trim();
                    if imap_host.is_empty() || smtp_host.is_empty() {
                        return Err("Enter the IMAP and SMTP server names.".into());
                    }
                    (
                        Server {
                            host: imap_host.into(),
                            port: port(&form.imap_port, "IMAP")?,
                            security: Security::Tls,
                        },
                        Server {
                            host: smtp_host.into(),
                            port: port(&form.smtp_port, "SMTP")?,
                            security: if form.smtp_starttls {
                                Security::StartTls
                            } else {
                                Security::Tls
                            },
                        },
                    )
                }
            };
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
        ui.set_a_busy(true);
        ui.set_a_error(SharedString::new());
        std::thread::spawn(move || {
            let result = mail::verify(&new.imap, &new.email, &password);
            slint::invoke_from_event_loop(move || {
                with_app(|app| app.finish_add_account(new, password, result));
            })
            .ok();
        });
    }

    fn finish_add_account(
        &mut self,
        new: NewAccount,
        password: String,
        result: anyhow::Result<()>,
    ) {
        let ui = self.ui();
        ui.set_a_busy(false);
        if let Err(e) = result {
            ui.set_a_error(format!("Couldn't sign in: {e:#}").into());
            return;
        }
        let saved = self.db.add_account(&new).and_then(|id| {
            secrets::set_password(id, &password).inspect_err(|_| {
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
                    self.workers[&id].send(Job::SyncAll);
                }
                ui.set_show_settings(false);
                self.set_status(&format!("Added {}. Downloading mail…", new.email), false);
            }
            Err(e) => ui.set_a_error(format!("{e:#}").into()),
        }
    }

    fn remove_account(&mut self, id: i64) {
        // Dropping the workers closes their channels; the threads log out.
        self.workers.remove(&id);
        self.syncing.remove(&id);
        if let Err(e) = self.db.remove_account(id) {
            self.set_status(&format!("{e:#}"), true);
            return;
        }
        secrets::delete_password(id).ok();
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

    // -------------------------------------------------------- navigation

    fn rebuild_nav(&mut self) {
        let unread = self.db.unread_counts().unwrap_or_default();
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
            depth: 0,
            expanded: true,
        }];
        for a in &self.accounts {
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
                depth: 0,
                expanded,
            });
            if expanded {
                items.extend(folders.iter().map(|f| NavItem {
                    kind: 2,
                    id: f.id as i32,
                    label: f.display.as_str().into(),
                    icon: f.role.icon().into(),
                    unread: count(f),
                    depth: f.depth.min(4) as i32,
                    expanded: true,
                }));
            }
        }
        update_model(&self.nav, items);
    }

    fn nav_clicked(&mut self, item: NavItem) {
        match item.kind {
            0 => self.set_view(View::AllInboxes),
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
        let ui = self.ui();
        ui.set_search_text(SharedString::new());
        ui.set_selected_folder(match view {
            View::AllInboxes => 0,
            View::Folder(id) => id as i32,
        });
        self.close_reader();
        self.reload_rows();
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
        }
    }

    fn view_role(&self) -> Role {
        match self.view {
            View::AllInboxes => Role::Inbox,
            View::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map_or(Role::Other, |f| f.role),
        }
    }

    // ------------------------------------------------------ message list

    fn reload_rows(&mut self) {
        let ids = match self.db.message_ids(&self.view_folders(), &self.search) {
            Ok(ids) => ids,
            Err(e) => {
                self.set_status(&format!("{e:#}"), true);
                Vec::new()
            }
        };
        self.rows.set(ids, self.view_role().is_outgoing());

        let ui = self.ui();
        ui.set_list_title(match self.view {
            View::AllInboxes => "All inboxes".into(),
            View::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.display.as_str().into())
                .unwrap_or_default(),
        });
        self.sync_selection_index();
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

    fn sync_selection_index(&self) {
        let ui = self.ui();
        let id = self.open.as_ref().map(|o| o.msg.id);
        ui.set_selected_id(id.map_or(-1, |id| id as i32));
        let index = id
            .and_then(|id| self.rows.index_of(id))
            .map_or(-1, |i| i as i32);
        ui.set_selected_index(index);
    }

    /// Set a flag in the cache, refresh its list row, and queue the change
    /// for the server.
    fn set_flag(&self, m: &db::Message, flag: Flag, on: bool) {
        let (result, name) = match flag {
            Flag::Seen => (self.db.set_seen(m.id, on), "\\Seen"),
            Flag::Flagged => (self.db.set_flagged(m.id, on), "\\Flagged"),
        };
        if let Err(e) = result {
            return self.set_status(&format!("{e:#}"), true);
        }
        if let Some(w) = self.workers.get(&m.account_id) {
            w.send(Job::SetFlag {
                folder_id: m.folder_id,
                uid: m.uid,
                flag: name,
                on,
            });
        }
        self.rows.changed(m.id);
    }

    fn select_offset(&mut self, delta: i32) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let current = self
            .open
            .as_ref()
            .and_then(|o| self.rows.index_of(o.msg.id));
        let next = match current {
            Some(i) => i.saturating_add_signed(delta as isize).min(len - 1),
            None => 0,
        };
        if Some(next) != current
            && let Some(id) = self.rows.id_at(next)
        {
            self.select_message(id);
        }
    }

    // ------------------------------------------------------------ reader

    fn select_message(&mut self, id: i64) {
        let ui = self.ui();
        if ui.get_composing() {
            // The draft stays in the composer; "New mail" brings it back.
            ui.set_composing(false);
        }
        let mut msg = match self.db.message(id) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        };
        if !msg.seen {
            self.set_flag(&msg, Flag::Seen, true);
            msg.seen = true;
            self.rebuild_nav();
        }
        let needs_fetch = msg.raw.is_none();
        let account_id = msg.account_id;
        let folder_id = msg.folder_id;
        self.open = Some(Open { msg, body: None });
        if needs_fetch {
            if let Some(w) = self.workers.get(&account_id) {
                w.send(Job::FetchBody(id));
            }
        } else {
            self.parse_open_body();
        }
        let targets: Vec<FolderOption> = self
            .folders
            .iter()
            .filter(|f| f.account_id == account_id && f.id != folder_id)
            .map(|f| FolderOption {
                id: f.id as i32,
                label: f.display.as_str().into(),
            })
            .collect();
        ui.set_move_targets(ModelRc::new(VecModel::from(targets)));
        self.show_open();
        self.sync_selection_index();
    }

    fn parse_open_body(&mut self) {
        if let Some(open) = self.open.as_mut()
            && let Some(raw) = open.msg.raw.as_deref()
        {
            open.body = Some(mime::parse_body(raw));
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
        ui.set_r_to(m.to.as_str().into());
        ui.set_r_cc(m.cc.as_str().into());
        ui.set_r_flagged(m.flagged);
        ui.set_r_unread(!m.seen);
        ui.set_loading(open.body.is_none());
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
            }
            None => {
                ui.set_r_body(SharedString::new());
                ui.set_r_attachments(ModelRc::default());
            }
        }
    }

    fn close_reader(&mut self) {
        self.open = None;
        self.ui().set_has_message(false);
        self.sync_selection_index();
    }

    fn toggle_read(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.msg.seen = !open.msg.seen;
        let seen = open.msg.seen;
        if let Some(open) = &self.open {
            self.set_flag(&open.msg, Flag::Seen, seen);
        }
        self.ui().set_r_unread(!seen);
        self.rebuild_nav();
    }

    fn toggle_flag(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.msg.flagged = !open.msg.flagged;
        let flagged = open.msg.flagged;
        if let Some(open) = &self.open {
            self.set_flag(&open.msg, Flag::Flagged, flagged);
        }
        self.ui().set_r_flagged(flagged);
    }

    fn archive(&mut self) {
        let Some(open) = &self.open else { return };
        match self.db.folder_by_role(open.msg.account_id, Role::Archive) {
            Ok(Some(f)) if f.id != open.msg.folder_id => self.move_open_to(f.id),
            Ok(Some(_)) => {}
            _ => self.set_status("This account has no Archive folder.", true),
        }
    }

    fn delete(&mut self) {
        let Some(open) = &self.open else { return };
        let (account, folder_id, uid) = (open.msg.account_id, open.msg.folder_id, open.msg.uid);
        match self.db.folder_by_role(account, Role::Trash) {
            Ok(Some(trash)) if trash.id == folder_id => {
                // Queue first: that marks the UID as pending, so a running
                // sync can't re-add it after the local delete.
                if let Some(w) = self.workers.get(&account) {
                    w.send(Job::Delete { folder_id, uid });
                }
                self.remove_open_locally();
            }
            Ok(Some(trash)) => self.move_open_to(trash.id),
            _ => self.set_status("This account has no Trash folder.", true),
        }
    }

    fn move_open_to(&mut self, dest: i64) {
        let Some(open) = &self.open else { return };
        let (account, folder_id, uid) = (open.msg.account_id, open.msg.folder_id, open.msg.uid);
        if let Some(w) = self.workers.get(&account) {
            w.send(Job::Move {
                folder_id,
                uid,
                dest,
            });
        }
        self.remove_open_locally();
    }

    /// Drop the open message from the cache and list, then select the
    /// message that took its place.
    fn remove_open_locally(&mut self) {
        let Some(open) = self.open.take() else { return };
        self.db.delete_message(open.msg.id).ok();
        let index = self.rows.remove(open.msg.id);
        self.rebuild_nav();
        let next = index.and_then(|i| {
            self.rows
                .id_at(i)
                .or_else(|| self.rows.id_at(self.rows.len().checked_sub(1)?))
        });
        match next {
            Some(id) => self.select_message(id),
            None => self.close_reader(),
        }
    }

    // ---------------------------------------------------------- composer

    fn compose_new(&mut self) {
        let ui = self.ui();
        if self.accounts.is_empty() {
            return;
        }
        let has_draft = !ui.get_c_to().is_empty()
            || !ui.get_c_subject().is_empty()
            || !ui.get_c_body().is_empty()
            || !self.compose.attachments.is_empty();
        if !has_draft {
            let account = match self.view {
                View::Folder(f) => self
                    .folders
                    .iter()
                    .find(|x| x.id == f)
                    .map(|x| x.account_id),
                View::AllInboxes => self.open.as_ref().map(|o| o.msg.account_id),
            };
            self.start_compose(
                "New message",
                account,
                "",
                "",
                "",
                String::new(),
                Compose::default(),
            );
        } else {
            ui.set_composing(true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_compose(
        &mut self,
        title: &str,
        account: Option<i64>,
        to: &str,
        cc: &str,
        subject: &str,
        body: String,
        compose: Compose,
    ) {
        let ui = self.ui();
        let index = account
            .and_then(|id| self.accounts.iter().position(|a| a.id == id))
            .unwrap_or(0);
        ui.set_c_title(title.into());
        ui.set_c_from_index(index as i32);
        ui.set_c_to(to.into());
        ui.set_c_cc(cc.into());
        ui.set_c_bcc(SharedString::new());
        ui.set_c_show_cc(!cc.is_empty());
        ui.set_c_subject(subject.into());
        ui.set_c_body(body.into());
        self.compose = compose;
        self.show_compose_attachments();
        ui.set_composing(true);
    }

    fn reply(&mut self, all: bool) {
        let Some(open) = &self.open else { return };
        let Some(body) = &open.body else {
            return self.set_status("The message is still downloading.", false);
        };
        let m = &open.msg;
        let me = self
            .accounts
            .iter()
            .find(|a| a.id == m.account_id)
            .map(|a| a.email.to_lowercase())
            .unwrap_or_default();
        let primary = if body.reply_to.is_empty() {
            m.from_addr.clone()
        } else {
            body.reply_to.clone()
        };
        let not_me = |addr: &String| !addr.to_lowercase().contains(&me) || me.is_empty();
        let mut to = vec![primary.clone()];
        let mut cc = Vec::new();
        if all {
            to.extend(
                mail::smtp::split_addresses(&m.to)
                    .into_iter()
                    .filter(|a| not_me(a) && !a.contains(&primary)),
            );
            cc.extend(
                mail::smtp::split_addresses(&m.cc)
                    .into_iter()
                    .filter(not_me),
            );
        }
        let quoted = format!(
            "\n\nOn {}, {} wrote:\n{}",
            format::full_date(m.date),
            format::sender(&m.from_name, &m.from_addr),
            quote(&body.text)
        );
        let compose = Compose {
            reply_to: Some(m.id),
            in_reply_to: m.msgid.clone(),
            references: body.references.clone(),
            attachments: Vec::new(),
        };
        let subject = prefixed("Re:", &m.subject);
        let account = Some(m.account_id);
        self.start_compose(
            if all { "Reply all" } else { "Reply" },
            account,
            &to.join(", "),
            &cc.join(", "),
            &subject,
            quoted,
            compose,
        );
    }

    fn forward(&mut self) {
        let Some(open) = &self.open else { return };
        let (Some(body), Some(raw)) = (&open.body, open.msg.raw.as_deref()) else {
            return self.set_status("The message is still downloading.", false);
        };
        let m = &open.msg;
        let text = format!(
            "\n\n---------- Forwarded message ----------\nFrom: {} <{}>\nDate: {}\nSubject: {}\nTo: {}\n\n{}",
            m.from_name,
            m.from_addr,
            format::full_date(m.date),
            m.subject,
            m.to,
            body.text
        );
        let attachments = mime::attachments(raw, |_| true)
            .into_iter()
            .map(|(filename, data)| OutAttachment { filename, data })
            .collect();
        let compose = Compose {
            attachments,
            ..Default::default()
        };
        let subject = prefixed("Fwd:", &m.subject);
        let account = Some(m.account_id);
        self.start_compose("Forward", account, "", "", &subject, text, compose);
    }

    fn show_compose_attachments(&self) {
        let rows: Vec<AttachmentRow> = self
            .compose
            .attachments
            .iter()
            .enumerate()
            .map(|(i, a)| AttachmentRow {
                index: i as i32,
                name: a.filename.as_str().into(),
                size: format::size(a.data.len()).into(),
            })
            .collect();
        self.ui()
            .set_c_attachments(ModelRc::new(VecModel::from(rows)));
    }

    fn discard(&mut self) {
        let ui = self.ui();
        self.compose = Compose::default();
        ui.set_c_to(SharedString::new());
        ui.set_c_cc(SharedString::new());
        ui.set_c_bcc(SharedString::new());
        ui.set_c_subject(SharedString::new());
        ui.set_c_body(SharedString::new());
        self.show_compose_attachments();
        ui.set_composing(false);
    }

    fn send(&mut self) {
        let ui = self.ui();
        let Some(account) = self.accounts.get(ui.get_c_from_index().max(0) as usize) else {
            return;
        };
        let outgoing = Outgoing {
            from_name: account.name.clone(),
            from_email: account.email.clone(),
            to: ui.get_c_to().into(),
            cc: ui.get_c_cc().into(),
            bcc: ui.get_c_bcc().into(),
            subject: ui.get_c_subject().into(),
            body: ui.get_c_body().into(),
            attachments: self.compose.attachments.clone(),
            in_reply_to: self.compose.in_reply_to.clone(),
            references: self.compose.references.clone(),
        };
        // Catch address mistakes before handing off to the worker.
        if let Err(e) = mail::smtp::check_recipients(&outgoing) {
            return self.set_status(&format!("{e:#}"), true);
        }
        if let Some(w) = self.workers.get(&account.id) {
            ui.set_c_sending(true);
            w.send(Job::Send {
                outgoing: Box::new(outgoing),
                reply_to: self.compose.reply_to,
            });
        }
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
                }
                self.ui().set_syncing(!self.syncing.is_empty());
                if let Some(e) = error {
                    let email = self.account_email(account_id);
                    self.set_status(&format!("{email}: {e}"), true);
                }
            }
            Event::FolderChanged { folder_id: None } => {
                self.folders = self.db.folders().unwrap_or_default();
                self.rebuild_nav();
                self.schedule_reload();
            }
            Event::FolderChanged { folder_id: Some(f) } => {
                if self.view_folders().contains(&f) {
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
                    self.parse_open_body();
                    self.show_open();
                }
                self.schedule_reload();
            }
            Event::BodyFailed { message_id, error } => {
                if self.open.as_ref().is_some_and(|o| o.msg.id == message_id) {
                    let ui = self.ui();
                    ui.set_loading(false);
                    ui.set_r_body(format!("Couldn't download this message.\n\n{error}").into());
                }
            }
            Event::Sent {
                result,
                sent_folder,
            } => {
                let ui = self.ui();
                ui.set_c_sending(false);
                match result {
                    Ok(()) => {
                        if let Some(id) = self.compose.reply_to {
                            self.rows.changed(id);
                        }
                        self.discard();
                        self.set_status("Message sent.", false);
                        if let Some(f) = sent_folder {
                            self.request_folder_sync(f);
                        }
                    }
                    Err(e) => self.set_status(&e, true),
                }
            }
            Event::ActionFailed { account_id, error } => {
                let email = self.account_email(account_id);
                self.set_status(&format!("{email}: {error}"), true);
            }
            Event::NeedsSync { folder_id } => self.request_folder_sync(folder_id),
        }
    }

    fn request_folder_sync(&self, folder_id: i64) {
        if let Some(f) = self.folders.iter().find(|f| f.id == folder_id)
            && let Some(w) = self.workers.get(&f.account_id)
        {
            w.send(Job::SyncFolder(folder_id));
        }
    }

    fn account_email(&self, id: i64) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.email.clone())
            .unwrap_or_default()
    }

    fn set_status(&self, text: &str, error: bool) {
        let ui = self.ui();
        ui.set_status(text.into());
        ui.set_status_error(error);
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

// ------------------------------------------------------- file dialogs
//
// Native dialogs run a nested message loop, so these handlers take what
// they need from the app state, release it, show the dialog, and then
// re-enter.

fn attach_files() {
    let Some(paths) = rfd::FileDialog::new()
        .set_title("Attach files")
        .pick_files()
    else {
        return;
    };
    let mut added = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".into());
        match std::fs::read(&path) {
            Ok(data) => added.push(OutAttachment {
                filename: name,
                data,
            }),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    with_app(|a| {
        a.compose.attachments.extend(added);
        let total: usize = a.compose.attachments.iter().map(|x| x.data.len()).sum();
        a.show_compose_attachments();
        if !errors.is_empty() {
            a.set_status(&errors.join("; "), true);
        } else if total > MAX_ATTACHMENTS_BYTES {
            a.set_status(
                "Attachments are over 25 MB; many servers will reject this message.",
                true,
            );
        }
    });
}

fn save_attachment(index: i32) {
    let found = with_app(|a| {
        let raw = a.open.as_ref()?.msg.raw.as_deref()?;
        mime::attachment(raw, index as usize)
    })
    .flatten();
    let Some((name, data)) = found else { return };
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save attachment")
        .set_file_name(&name);
    if let Some(dir) = dirs::download_dir() {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.save_file() else {
        return;
    };
    let result = std::fs::write(&path, data);
    with_app(|a| match result {
        Ok(()) => a.set_status(&format!("Saved {}", path.display()), false),
        Err(e) => a.set_status(&format!("Couldn't save {name}: {e}"), true),
    });
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

fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    if s.get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    {
        s.to_string()
    } else {
        format!("{prefix} {s}")
    }
}

fn quote(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.starts_with('>') {
                format!(">{l}")
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_prefixes() {
        assert_eq!(prefixed("Re:", "Hello"), "Re: Hello");
        assert_eq!(prefixed("Re:", "RE: Hello"), "RE: Hello");
        assert_eq!(prefixed("Fwd:", "  "), "Fwd: ");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("a\n> b"), "> a\n>> b");
    }
}
