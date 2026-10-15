//! New-mail notifications, the tray icon and background mode, wired to the
//! plumbing in `komorebi::notify` (what to say) and `komorebi::win_shell`
//! (the tray icon, its native Windows notification, the Run key). Everything
//! is local: notifications come from what the existing sync and IDLE
//! threads stored, nothing is sent anywhere and nothing is logged.

use super::{App, with_app};
use crate::{AppWindow, Tr};
use komorebi::notify::{Content, NewMessage, Notifier, Settings, Toast};
use slint::ComponentHandle;

/// Messages read at most per check, so a first sync of a big inbox stays
/// one "N new messages" notification.
const READ_AT_MOST: usize = 200;

/// The notification state the app keeps.
#[derive(Default)]
pub(super) struct Notes {
    notifier: Notifier,
    /// Notifications on (setting `notify`).
    pub(super) enabled: bool,
    /// Sender and subject shown (setting `notify_detail`).
    pub(super) detail: bool,
    /// Paused from the tray menu (setting `notify_paused`).
    pub(super) paused: bool,
    /// Accounts that never notify (setting `notify_off`).
    pub(super) off: Vec<i64>,
    /// Closing the window keeps the app in the tray (setting `close_to_tray`).
    pub(super) close_to_tray: bool,
    /// Only messages stored after this id are new.
    after: i64,
    /// An inbox is still doing its first sync (after a clean-slate upgrade):
    /// what it stores is old mail, so nothing notifies until it is done.
    settling: bool,
    /// Set by the tray's Quit: the next close really exits.
    pub(super) quitting: bool,
    #[cfg(windows)]
    pub(super) shell: Option<komorebi::win_shell::Shell>,
}

impl Notes {
    pub(super) fn from_settings(s: &crate::db::Settings) -> Self {
        Notes {
            enabled: s.notify,
            detail: s.notify_detail,
            paused: s.notify_paused,
            off: s.notify_off.clone(),
            close_to_tray: s.close_to_tray,
            ..Notes::default()
        }
    }

    /// Whether the tray icon is up (closing to the tray needs it).
    pub(super) fn has_tray(&self) -> bool {
        #[cfg(windows)]
        return self.shell.is_some();
        #[cfg(not(windows))]
        false
    }
}

#[cfg(windows)]
fn labels(ui: &AppWindow) -> komorebi::win_shell::TrayLabels {
    let tr = ui.global::<Tr>();
    komorebi::win_shell::TrayLabels {
        open: tr.invoke_tray_open().into(),
        check_mail: tr.invoke_tray_check().into(),
        pause_notifications: tr.invoke_tray_pause().into(),
        quit: tr.invoke_tray_quit().into(),
    }
}

impl App {
    /// Start notifications: remember where the cache ends (older mail never
    /// notifies) and put the icon in the tray. Not in snapshots.
    pub(super) fn start_notifications(&mut self) {
        self.notes.after = self.db.max_message_id().unwrap_or(0);
        self.notes.settling = self.db.inbox_awaits_first_sync().unwrap_or(false);
        if komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some() {
            return;
        }
        #[cfg(windows)]
        {
            let ui = self.ui();
            let tooltip = ui.global::<Tr>().invoke_tray_tooltip(0);
            self.notes.shell = komorebi::win_shell::Shell::new(labels(&ui), &tooltip, |event| {
                // Deferred: the handler may run inside a Win32 call made
                // while the app state is borrowed.
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    with_app(|a| a.shell_event(event));
                });
            });
            if let Some(shell) = &self.notes.shell {
                shell.set_paused(self.notes.paused);
            }
        }
        // Started hidden but no tray to come back from: show the window.
        if !self.notes.has_tray() && !self.ui().window().is_visible() {
            self.show_main_window();
        }
        self.ui().set_g_autostart(autostart_enabled());
    }

    #[cfg(windows)]
    fn shell_event(&mut self, event: komorebi::win_shell::ShellEvent) {
        use komorebi::win_shell::ShellEvent;
        match event {
            ShellEvent::Open | ShellEvent::ActivateRequested => self.show_main_window(),
            ShellEvent::CheckMail => self.refresh(),
            ShellEvent::TogglePause => {
                self.notes.paused = !self.notes.paused;
                if let Some(shell) = &self.notes.shell {
                    shell.set_paused(self.notes.paused);
                }
                self.save_setting("notify_paused", if self.notes.paused { "1" } else { "0" });
            }
            ShellEvent::Quit => {
                self.notes.quitting = true;
                self.before_close();
                self.notes.shell = None;
                slint::quit_event_loop().ok();
            }
            ShellEvent::ToastClicked(target) => {
                self.show_main_window();
                if matches!(self.db.message(target.message_id), Ok(Some(_))) {
                    self.open_in_tab(target.message_id, false);
                }
            }
        }
    }

    /// The window back from the tray (or from behind other windows).
    pub(super) fn show_main_window(&mut self) {
        let ui = self.ui();
        ui.show().ok();
        crate::bring_to_front(&ui);
    }

    pub(super) fn save_setting(&mut self, key: &str, value: &str) {
        if let Err(e) = self.db.set_setting(key, value) {
            self.set_status(
                &crate::tr::t!(invoke_not_saved, format!("{e:#}").into()),
                true,
            );
        }
    }

    /// The unread count rose: notify about the inbox messages stored since
    /// the last check, if the user wants that and the window is not in front.
    pub(super) fn notify_new_mail(&mut self) {
        if self.notes.settling {
            // The first sync of an inbox refetches old mail: move the mark
            // along with it, and start notifying once it is complete.
            self.notes.after = self.db.max_message_id().unwrap_or(self.notes.after);
            self.notes.settling = self.db.inbox_awaits_first_sync().unwrap_or(false);
            return;
        }
        let Ok(found) = self.db.new_unread_inbox(self.notes.after, READ_AT_MOST) else {
            return;
        };
        if let Some(last) = found.last() {
            self.notes.after = last.1;
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
        let settings = Settings {
            paused: self.notes.paused || !self.notes.enabled,
            content: if self.notes.detail {
                Content::SenderSubject
            } else {
                Content::Generic
            },
            enabled_accounts: self
                .accounts
                .iter()
                .map(|a| a.id)
                .filter(|id| !self.notes.off.contains(id))
                .collect(),
        };
        let visible = self.ui().window().is_visible();
        let Some(toast) = self
            .notes
            .notifier
            .decide(&events, &settings, self.focused && visible)
        else {
            return;
        };
        #[cfg(windows)]
        if let Some(shell) = &self.notes.shell {
            let ui = self.ui();
            let tr = ui.global::<Tr>();
            let (title, body, target): (String, String, _) = match toast {
                Toast::Message {
                    sender,
                    subject,
                    target,
                } => {
                    let subject = if subject.trim().is_empty() {
                        tr.invoke_no_subject().into()
                    } else {
                        subject
                    };
                    (sender, subject, target)
                }
                Toast::NewMail { target } => (
                    tr.invoke_toast_new_mail().into(),
                    tr.invoke_toast_new_mail_body().into(),
                    target,
                ),
                Toast::Burst { count, target } => (
                    tr.invoke_toast_burst(count.min(i32::MAX as usize) as i32)
                        .into(),
                    tr.invoke_toast_burst_body().into(),
                    target,
                ),
            };
            shell.show_toast(&title, &body, Some(target));
        }
        #[cfg(not(windows))]
        let _ = toast;
    }

    /// The tray tooltip, the menu labels and "Updated" in the language shown.
    pub(super) fn repush_tray(&self, unread: i32) {
        #[cfg(windows)]
        if let Some(shell) = &self.notes.shell {
            let ui = self.ui();
            shell.set_labels(labels(&ui));
            shell.set_tooltip(&ui.global::<Tr>().invoke_tray_tooltip(unread));
        }
        #[cfg(not(windows))]
        let _ = unread;
    }

    /// A sync pass ended: "Updated 14:05" beside Refresh.
    pub(super) fn mark_updated(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.last_updated = Some(now);
        self.show_last_updated();
    }

    pub(super) fn show_last_updated(&self) {
        let ui = self.ui();
        let text = self.last_updated.map_or_else(String::new, |ts| {
            ui.global::<Tr>()
                .invoke_updated_at(crate::format::time(ts).into())
                .into()
        });
        ui.set_last_updated(text.into());
    }

    /// Settings > General notification switches (which 8 to 11).
    pub(super) fn notification_changed(&mut self, which: i32, on: bool) {
        let v = if on { "1" } else { "0" };
        match which {
            8 => {
                self.notes.enabled = on;
                self.save_setting("notify", v);
            }
            9 => {
                self.notes.detail = on;
                self.save_setting("notify_detail", v);
            }
            10 => {
                self.notes.close_to_tray = on;
                self.save_setting("close_to_tray", v);
            }
            11 => {
                set_autostart(on);
                self.ui().set_g_autostart(autostart_enabled());
            }
            _ => {}
        }
    }

    /// The account page's Notifications switch: off adds the account to
    /// `notify_off`, on takes it out.
    pub(super) fn set_account_notify(&mut self, id: i64, on: bool) {
        self.notes.off.retain(|&a| a != id);
        if !on {
            self.notes.off.push(id);
        }
        let v = self
            .notes
            .off
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        self.save_setting("notify_off", &v);
        if let Err(e) = self.load_accounts() {
            self.set_status(&super::text::detail(&e), true);
        }
    }

    pub(super) fn show_notification_settings(&self) {
        let ui = self.ui();
        ui.set_g_notify(self.notes.enabled);
        ui.set_g_notify_detail(self.notes.detail);
        ui.set_g_close_to_tray(self.notes.close_to_tray);
        ui.set_g_autostart(autostart_enabled());
    }
}

fn autostart_enabled() -> bool {
    #[cfg(windows)]
    return komorebi::win_shell::autostart_enabled();
    #[cfg(not(windows))]
    false
}

fn set_autostart(on: bool) {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        komorebi::win_shell::set_autostart(on, &exe);
    }
    #[cfg(not(windows))]
    let _ = on;
}
