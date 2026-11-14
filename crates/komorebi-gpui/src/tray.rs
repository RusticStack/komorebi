//! New-mail notifications, the tray icon and closing to the tray
//! (docs/GPUI-MIGRATION.md phase 11; the Slint app's
//! `src/app/notifications.rs`). The plumbing stays in the library:
//! `komorebi::notify` decides what to say, `komorebi::win_shell` owns the
//! tray icon, its menu and the native Windows notification. Everything is
//! local: notifications come from what the sync workers stored; nothing is
//! sent anywhere and nothing is logged.
//!
//! The main window owns the [`Tray`] (`window.rs`): it puts the icon up
//! after the first frame (never in a snapshot or a test), feeds it the
//! unread count, asks it before closing, and acts on its [`TrayEvent`]s.
//! The settings it reads (`notify`, `notify_detail`, `notify_off`,
//! `close_to_tray`) are read again each time, so the Settings view only
//! writes them.

use gpui::{Context, EventEmitter, Window};
use komorebi::db::Db;
use komorebi::notify::{Content, NewMessage, Notifier, Settings, Toast};
use komorebi_i18n::{tr, trn};
use std::rc::Rc;

/// Messages read at most per check, so a first sync of a big inbox stays
/// one "N new messages" notification.
const READ_AT_MOST: usize = 200;

/// What the tray asks of the main window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// The icon's left click, Open Komorebi, a second launch: the window
    /// comes back from the tray (or from behind other windows).
    Show,
    /// Check mail: a Refresh.
    CheckMail,
    /// Quit: really exit (drafts and tabs saved, the icon removed).
    Quit,
    /// A notification was clicked: the window comes back with this message
    /// in a tab (when it is still cached).
    OpenMessage(i64),
    /// A second launch handed over a `mailto:` link: the window comes back
    /// with a prefilled composer (never sent).
    Mailto(komorebi::mail::render::links::Mailto),
}

/// The notification marks the Slint app keeps (`Notes`).
#[derive(Default)]
pub struct Notes {
    notifier: Notifier,
    /// Only messages stored after this id are new.
    after: i64,
    /// An inbox is still doing its first sync: what it stores is old mail,
    /// so nothing notifies until it is done.
    settling: bool,
    /// The cache's last message id at the last look: while it stays, no
    /// message was stored and the check reads nothing more (it runs on
    /// every status change, `MainWindow::check_new_mail`).
    seen_max: i64,
}

impl Notes {
    /// Remember where the cache ends (older mail never notifies) and
    /// whether an inbox still awaits its first sync.
    pub fn begin(&mut self, db: &Db) {
        self.after = db.max_message_id().unwrap_or(0);
        self.seen_max = self.after;
        self.settling = db.inbox_awaits_first_sync().unwrap_or(false);
    }

    /// The unread inbox mail stored since the last call, as `(account,
    /// message, sender, subject)`, and the mark moved past it. While an
    /// inbox is doing its first sync what is stored is old mail: it is
    /// skipped, and notifying starts with the call after the sync completed.
    pub fn take_new(&mut self, db: &Db) -> Vec<(i64, i64, String, String)> {
        if self.settling {
            self.after = db.max_message_id().unwrap_or(self.after);
            self.seen_max = self.after;
            self.settling = db.inbox_awaits_first_sync().unwrap_or(false);
            return Vec::new();
        }
        // Nothing stored since the last look (MAX of the row id is one
        // index step): the inbox query is skipped.
        let Ok(max) = db.max_message_id() else {
            return Vec::new();
        };
        if max == self.seen_max {
            return Vec::new();
        }
        self.seen_max = max;
        let Ok(found) = db.new_unread_inbox(self.after, READ_AT_MOST) else {
            return Vec::new();
        };
        if let Some(last) = found.last() {
            self.after = last.1;
        }
        found
    }

    /// The toast for the mail stored since the last call, if the settings
    /// and the window allow one (`focused`: the window is in front).
    pub fn decide(&mut self, db: &Db, focused: bool) -> Option<Toast> {
        let found = self.take_new(db);
        if found.is_empty() {
            return None;
        }
        let events: Vec<NewMessage> = found
            .into_iter()
            .map(|(account_id, message_id, sender, subject)| NewMessage {
                account_id,
                message_id,
                sender,
                subject,
            })
            .collect();
        let s = db.settings().ok()?;
        let accounts = db.accounts().ok()?;
        let settings = Settings {
            paused: s.notify_paused || !s.notify,
            content: if s.notify_detail {
                Content::SenderSubject
            } else {
                Content::Generic
            },
            enabled_accounts: accounts
                .iter()
                .map(|a| a.id)
                .filter(|id| !s.notify_off.contains(id))
                .collect(),
        };
        self.notifier.decide(&events, &settings, focused)
    }
}

/// The words of the tray and its notifications (`ui/tr.slint`).
pub mod text {
    use super::*;

    pub fn open() -> String {
        tr!("tray menu" => "Open Komorebi")
    }

    pub fn check() -> String {
        tr!("tray menu" => "Check mail")
    }

    pub fn pause() -> String {
        tr!("tray menu" => "Pause notifications")
    }

    pub fn quit() -> String {
        tr!("tray menu" => "Quit")
    }

    pub fn tooltip(unread: usize) -> String {
        crate::text::tray_tooltip(unread)
    }

    pub fn new_mail() -> String {
        tr!("New mail")
    }

    pub fn new_mail_body() -> String {
        tr!("Open Komorebi to read it.")
    }

    pub fn burst(n: usize) -> String {
        trn!("{n} new message", "{n} new messages", n)
    }

    pub fn burst_body() -> String {
        tr!("Open Komorebi to read them.")
    }

    /// A toast's title and body.
    pub fn toast(toast: &Toast) -> (String, String) {
        match toast {
            Toast::Message {
                sender, subject, ..
            } => {
                let subject = if subject.trim().is_empty() {
                    crate::views::mail::text::no_subject()
                } else {
                    subject.clone()
                };
                (sender.clone(), subject)
            }
            Toast::NewMail { .. } => (new_mail(), new_mail_body()),
            Toast::Burst { count, .. } => (burst(*count), burst_body()),
        }
    }
}

/// The tray icon and the notification state.
pub struct Tray {
    db: Rc<Db>,
    notes: Notes,
    /// Set by Quit: the next close really exits.
    quitting: bool,
    #[cfg(windows)]
    shell: Option<komorebi::win_shell::Shell>,
    _events: Option<gpui::Task<()>>,
}

impl EventEmitter<TrayEvent> for Tray {}

impl Tray {
    /// The notification marks start at the cache's end; no icon yet.
    pub fn new(db: Rc<Db>) -> Self {
        let mut notes = Notes::default();
        notes.begin(&db);
        Self {
            db,
            notes,
            quitting: false,
            #[cfg(windows)]
            shell: None,
            _events: None,
        }
    }

    /// The icon goes up in the notification area (Windows may refuse: the
    /// app then runs without one, and closing the window quits).
    pub fn start(&mut self, unread: usize, cx: &mut Context<Self>) {
        #[cfg(windows)]
        {
            use futures::StreamExt as _;
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            // The handler runs inside the tray window's procedure: the
            // event waits for the next pass of the UI loop.
            self.shell =
                komorebi::win_shell::Shell::new(labels(), &text::tooltip(unread), move |e| {
                    tx.unbounded_send(e).ok();
                });
            if let Some(shell) = &self.shell {
                let paused = self.db.settings().is_ok_and(|s| s.notify_paused);
                shell.set_paused(paused);
                shell.set_unread(unread > 0);
            }
            self._events = Some(cx.spawn(async move |this, cx| {
                while let Some(event) = rx.next().await {
                    if this.update(cx, |t, cx| t.on_shell(event, cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        #[cfg(not(windows))]
        let _ = (unread, cx);
    }

    /// Whether the icon is up (closing to the tray needs it).
    pub fn has_icon(&self) -> bool {
        #[cfg(windows)]
        return self.shell.is_some();
        #[cfg(not(windows))]
        false
    }

    /// Closing the window keeps the app in the tray (the setting, an icon
    /// to come back from, and not after Quit).
    pub fn keeps_running(&self) -> bool {
        !self.quitting && self.has_icon() && self.db.settings().is_ok_and(|s| s.close_to_tray)
    }

    /// Quit was chosen: the icon goes, the next close exits.
    pub fn quit(&mut self) {
        self.quitting = true;
        #[cfg(windows)]
        {
            self.shell = None;
        }
    }

    /// The unread count changed: the tooltip and the icon's dot follow, and
    /// the inbox mail stored since the last check notifies.
    pub fn set_unread(&mut self, unread: usize, focused: bool) {
        #[cfg(windows)]
        if let Some(shell) = &self.shell {
            shell.set_tooltip(&text::tooltip(unread));
            shell.set_unread(unread > 0);
        }
        self.notify_new_mail(focused);
    }

    /// The cache changed (a sync pass, a recount): new mail is found by
    /// message id, not by the unread total, so a message read in the same
    /// refresh never hides a new one (the Slint app's `show_unread`).
    /// Returns the toast decided, if any.
    pub fn check_new(&mut self, focused: bool) -> Option<Toast> {
        self.notify_new_mail(focused)
    }

    /// The menu and the tooltip in the language shown.
    pub fn relabel(&self, unread: usize) {
        #[cfg(windows)]
        if let Some(shell) = &self.shell {
            shell.set_labels(labels());
            shell.set_tooltip(&text::tooltip(unread));
        }
        #[cfg(not(windows))]
        let _ = unread;
    }

    fn notify_new_mail(&mut self, focused: bool) -> Option<Toast> {
        let toast = self.notes.decide(&self.db, focused)?;
        #[cfg(windows)]
        if let Some(shell) = &self.shell {
            let target = match &toast {
                Toast::Message { target, .. }
                | Toast::NewMail { target }
                | Toast::Burst { target, .. } => *target,
            };
            let (title, body) = text::toast(&toast);
            shell.show_toast(&title, &body, Some(target));
        }
        Some(toast)
    }

    #[cfg(windows)]
    fn on_shell(&mut self, event: komorebi::win_shell::ShellEvent, cx: &mut Context<Self>) {
        use komorebi::win_shell::ShellEvent;
        match event {
            ShellEvent::Open | ShellEvent::ActivateRequested => cx.emit(TrayEvent::Show),
            ShellEvent::Mailto(m) => cx.emit(TrayEvent::Mailto(m)),
            ShellEvent::CheckMail => cx.emit(TrayEvent::CheckMail),
            ShellEvent::TogglePause => {
                let paused = !self.db.settings().is_ok_and(|s| s.notify_paused);
                if let Some(shell) = &self.shell {
                    shell.set_paused(paused);
                }
                if let Err(e) = self
                    .db
                    .set_setting("notify_paused", if paused { "1" } else { "0" })
                {
                    eprintln!("komorebi-gpui: saving notify_paused: {e:#}");
                }
            }
            ShellEvent::Quit => cx.emit(TrayEvent::Quit),
            ShellEvent::ToastClicked(target) => cx.emit(TrayEvent::OpenMessage(target.message_id)),
        }
    }
}

#[cfg(windows)]
fn labels() -> komorebi::win_shell::TrayLabels {
    komorebi::win_shell::TrayLabels {
        open: text::open(),
        check_mail: text::check(),
        pause_notifications: text::pause(),
        quit: text::quit(),
    }
}

/// Whether the unread count going from `before` to `now` flashes the
/// taskbar button: the setting is on, the window is in the background and
/// the count rose (the Slint app's `show_unread`).
pub fn should_flash(setting: bool, active: bool, before: i64, now: i64) -> bool {
    setting && !active && now > before
}

/// Flash the taskbar button until the window comes to the front.
pub fn flash_taskbar(window: &Window) {
    #[cfg(windows)]
    if let Some(hwnd) = crate::snapshot::window_handle(window) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
        };
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: HWND(hwnd as *mut core::ffi::c_void),
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        // SAFETY: a sized struct naming GPUI's live top-level window.
        unsafe {
            let _ = FlashWindowEx(&info);
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// Hide the window to the tray, or show it again in front.
pub fn set_shown(window: &Window, shown: bool) {
    #[cfg(windows)]
    if let Some(hwnd) = crate::snapshot::window_handle(window) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, ShowWindow};
        let how = if shown { SW_SHOW } else { SW_HIDE };
        // SAFETY: the handle is GPUI's live top-level window.
        unsafe {
            let _ = ShowWindow(HWND(hwnd as *mut core::ffi::c_void), how);
        }
    }
    if shown {
        window.activate_window();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::db::{Fp, NewAccount, NewHeader};
    use komorebi::i18n::Lang;
    use komorebi::providers::{Security, Server};
    use komorebi::role::Role;
    use komorebi_i18n::with_lang;

    fn account(db: &Db) -> (i64, i64) {
        let server = |host: &str| Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        let id = db
            .add_account(&NewAccount {
                email: "me@example.invalid".into(),
                name: String::new(),
                imap: server("imap.example.invalid"),
                smtp: server("smtp.example.invalid"),
                save_sent: true,
                imap_user: String::new(),
                smtp_user: String::new(),
                smtp_no_auth: false,
            })
            .unwrap();
        let inbox = db
            .upsert_folder(id, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        (id, inbox)
    }

    fn mail(db: &mut Db, account: i64, folder: i64, uid: u32) {
        db.insert_headers(
            account,
            folder,
            &[NewHeader {
                uid,
                msgid: format!("<{uid}@example.invalid>"),
                subject: format!("Subject {uid}"),
                from_addr: "ana@example.invalid".into(),
                date: 1_700_000_000 + i64::from(uid),
                ..NewHeader::default()
            }],
        )
        .unwrap();
    }

    /// Port of `a_new_accounts_first_sync_is_silent_and_later_mail_notifies`
    /// (`src/app/notifications.rs`).
    #[test]
    fn a_new_accounts_first_sync_is_silent_and_later_mail_notifies() {
        let cache = crate::test_support::TempCache::new("notes-first");
        let mut db = cache.open();
        let (acc, inbox) = account(&db);
        assert!(db.inbox_awaits_first_sync().unwrap());
        let mut notes = Notes::default();
        notes.begin(&db);
        assert!(notes.settling);
        mail(&mut db, acc, inbox, 1);
        mail(&mut db, acc, inbox, 2);
        assert!(notes.take_new(&db).is_empty());
        mail(&mut db, acc, inbox, 3);
        assert!(notes.take_new(&db).is_empty());
        assert!(notes.settling);
        assert_eq!(notes.after, db.max_message_id().unwrap());
        mail(&mut db, acc, inbox, 4);
        db.finish_folder(
            inbox,
            Fp {
                uidnext: 5,
                messages: 4,
                hms: 1,
            },
        )
        .unwrap();
        assert!(!db.inbox_awaits_first_sync().unwrap());
        assert!(notes.take_new(&db).is_empty());
        assert!(!notes.settling);
        mail(&mut db, acc, inbox, 5);
        let found = notes.take_new(&db);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, acc);
        assert_eq!(found[0].2, "ana@example.invalid");
        assert_eq!(found[0].3, "Subject 5");
        assert!(notes.take_new(&db).is_empty());
    }

    /// The taskbar flashes for a rise while the window is behind, with the
    /// setting on.
    #[test]
    fn the_taskbar_flashes_for_a_rise_in_the_background() {
        assert!(should_flash(true, false, 2, 3));
        assert!(!should_flash(false, false, 2, 3), "the setting is off");
        assert!(!should_flash(true, true, 2, 3), "the window is in front");
        assert!(!should_flash(true, false, 3, 3), "no rise");
        assert!(!should_flash(true, false, 4, 3), "a fall");
    }

    /// A message read and one arriving in the same pass leave the unread
    /// total as it was, and the new one still notifies (found by id).
    #[test]
    fn a_read_and_a_new_message_in_one_pass_still_notify() {
        let cache = crate::test_support::TempCache::new("tray-same-total");
        let mut db = cache.open();
        let (acc, inbox) = account(&db);
        db.finish_folder(
            inbox,
            Fp {
                uidnext: 1,
                messages: 0,
                hms: 1,
            },
        )
        .unwrap();
        db.set_setting("notify", "1").unwrap();
        mail(&mut db, acc, inbox, 1);
        let mut tray = Tray::new(Rc::new(db));
        let mut db = cache.open();
        // One read, one new: the total stays at one.
        let first = db.message_ids(&[inbox], "").unwrap();
        db.set_seen_many(&first, true).unwrap();
        mail(&mut db, acc, inbox, 2);
        assert!(tray.check_new(false).is_some(), "the new one notifies");
        assert!(tray.check_new(false).is_none(), "once");
    }

    /// Port of `a_synced_inbox_never_settles_and_read_mail_does_not_notify`.
    #[test]
    fn a_synced_inbox_never_settles_and_read_mail_does_not_notify() {
        let cache = crate::test_support::TempCache::new("notes-synced");
        let mut db = cache.open();
        let (acc, inbox) = account(&db);
        mail(&mut db, acc, inbox, 1);
        db.finish_folder(
            inbox,
            Fp {
                uidnext: 2,
                messages: 1,
                hms: 1,
            },
        )
        .unwrap();
        let mut notes = Notes::default();
        notes.begin(&db);
        assert!(!notes.settling);
        assert!(notes.take_new(&db).is_empty());
        db.insert_headers(
            acc,
            inbox,
            &[NewHeader {
                uid: 2,
                msgid: "<2@example.invalid>".into(),
                seen: true,
                ..NewHeader::default()
            }],
        )
        .unwrap();
        assert!(notes.take_new(&db).is_empty());
        mail(&mut db, acc, inbox, 3);
        assert_eq!(notes.take_new(&db).len(), 1);
    }

    /// The toast follows the settings: off, paused, the window in front, an
    /// account turned off say nothing; sender and subject only with
    /// `notify_detail`.
    #[test]
    fn the_toast_follows_the_settings() {
        let cache = crate::test_support::TempCache::new("notes-settings");
        let mut db = cache.open();
        let (acc, inbox) = account(&db);
        db.finish_folder(
            inbox,
            Fp {
                uidnext: 1,
                messages: 0,
                hms: 1,
            },
        )
        .unwrap();
        db.set_setting("notify", "1").unwrap();
        db.set_setting("notify_detail", "1").unwrap();
        let mut notes = Notes::default();
        notes.begin(&db);
        mail(&mut db, acc, inbox, 1);
        assert!(notes.decide(&db, true).is_none(), "the window is in front");
        mail(&mut db, acc, inbox, 2);
        assert!(matches!(
            notes.decide(&db, false),
            Some(Toast::Message { ref subject, .. }) if subject == "Subject 2"
        ));
        db.set_setting("notify_detail", "0").unwrap();
        mail(&mut db, acc, inbox, 3);
        mail(&mut db, acc, inbox, 4);
        assert!(matches!(
            notes.decide(&db, false),
            Some(Toast::Burst { count: 2, .. })
        ));
        db.set_setting("notify_off", &acc.to_string()).unwrap();
        mail(&mut db, acc, inbox, 5);
        assert!(notes.decide(&db, false).is_none(), "the account is off");
        with_lang(Lang::En, || {
            let (title, body) = text::toast(&Toast::Burst {
                count: 3,
                target: komorebi::notify::Target {
                    account_id: acc,
                    message_id: 1,
                },
            });
            assert_eq!(
                (title.as_str(), body.as_str()),
                ("3 new messages", "Open Komorebi to read them.")
            );
        });
        with_lang(Lang::PtPt, || {
            assert_ne!(text::check(), "Check mail");
            assert_ne!(text::new_mail_body(), "Open Komorebi to read it.");
        });
    }
}
