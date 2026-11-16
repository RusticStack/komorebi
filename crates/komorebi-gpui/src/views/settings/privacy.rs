//! Settings > Privacy (the Slint app's remote-image senders, diagnostics
//! switch, logs folder and `wipe` question): who may always load remote
//! images, the diagnostics log, and Remove all data.

use gpui::{Context, IntoElement, ParentElement, Styled, Window, div};
use komorebi_i18n::tr;

use super::card::{Card, group_head, page_head};
use super::{Message, Section, Setting, SettingsEvent, SettingsView, account, text};
use crate::icons::Icon;
use crate::widgets::button::{Button, Variant};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::toggle::Toggle;

impl SettingsView {
    /// A sender's remote images load no more (their next message asks again).
    pub fn forget_sender(&mut self, address: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.db.forget_remote(address) {
            self.message = Some(Message::NotSaved(format!("{e:#}")));
        }
        self.remote_senders = self.db.remote_allow_list().unwrap_or_default();
        cx.notify();
    }

    /// Every sender is forgotten.
    pub fn forget_all_senders(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.db.forget_all_remote() {
            self.message = Some(Message::NotSaved(format!("{e:#}")));
        }
        self.remote_senders = self.db.remote_allow_list().unwrap_or_default();
        cx.notify();
    }

    /// The diagnostics log switch: the library starts or stops writing it.
    pub fn set_diagnostics(&mut self, on: bool, cx: &mut Context<Self>) {
        let path = self.db.file().map(|p| komorebi::diag::log_path(&p));
        komorebi::diag::set(if on { path } else { None });
        self.set_setting(Setting::DiagnosticsLog(on), cx);
    }

    /// Account icons from the mail domains on or off. Off: nothing is asked
    /// again and the fetched icons go (chosen images stay).
    pub fn set_account_icons(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Err(e) = komorebi::favicon::set_enabled(&self.db, on) {
            self.message = Some(Message::NotSaved(format!("{e:#}")));
        }
        cx.emit(SettingsEvent::AccountsChanged);
        cx.notify();
    }

    /// Open the folder of the log files in File Explorer.
    pub fn open_logs_folder(&self, cx: &mut Context<Self>) {
        if let Some(dir) = self
            .db
            .file()
            .map(|p| komorebi::diag::log_path(&p))
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        {
            cx.open_with_system(&dir);
        }
    }

    /// Remove all data, after the question: the saved password of every
    /// account goes first, while the app still runs (if one can't be
    /// deleted, nothing else is touched); then the shell closes the cache
    /// and the window and removes the files ([`super::wipe::remove_files`]).
    pub fn wipe_now(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        let forget = super::services::get(cx).forget_password;
        match super::wipe::forget_passwords(&ids, &forget) {
            Ok(()) => cx.emit(SettingsEvent::WipeAndQuit),
            Err(e) => {
                self.message = Some(Message::NotReset(format!("{e:#}")));
                cx.notify();
            }
        }
    }

    /// The mail cache's encryption: (encrypted now, wanted at next start).
    pub fn cache_lock(&self) -> (bool, bool) {
        match self.db.file() {
            Some(path) => {
                let now = komorebi::cachecrypt::is_encrypted(&path);
                (now, komorebi::cachecrypt::intent(&path).unwrap_or(now))
            }
            None => (false, false),
        }
    }

    /// Record the cache's encryption for the next start (the Slint app's
    /// `CacheLock.set`); switching it on is asked first.
    pub fn set_cache_lock(&mut self, encrypt: bool, cx: &mut Context<Self>) {
        if let Some(path) = self.db.file()
            && let Err(e) = komorebi::cachecrypt::set_intent(&path, encrypt)
        {
            self.message = Some(Message::NotSaved(format!("{e:#}")));
        }
        cx.notify();
    }

    fn ask_cache_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: tr!("Encrypt the mail cache?").into(),
                message: tr!("The key is kept for your Windows user on this computer. If it is lost (another Windows user, a reset Windows password, a copied data folder), the cache cannot be opened: mail can be downloaded again, but drafts and the Outbox exist only in the cache and are lost.").into(),
                confirm: tr!("Encrypt at next start").into(),
                cancel: tr!("Cancel").into(),
                destructive: false,
            },
            move |_, cx| {
                let _ = this.update(cx, |this, cx| this.set_cache_lock(true, cx));
            },
            window,
            cx,
        );
    }

    fn ask_wipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unsent = account::unsent(&self.db, None);
        let message = if unsent > 0 {
            format!("{} {}", text::q_wipe_text(), text::q_wipe_unsent(unsent))
        } else {
            text::q_wipe_text()
        };
        let this = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::q_wipe_title().into(),
                message: message.into(),
                confirm: text::q_wipe_ok().into(),
                cancel: tr!("Cancel").into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = this.update(cx, |this, cx| this.wipe_now(cx));
            },
            window,
            cx,
        );
    }

    pub(crate) fn render_privacy(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let senders = self.remote_senders.clone();
        let mut images = div().flex().flex_col();
        images = images.child(
            Card::new(
                "privacy-images",
                tr!("Senders whose images always load"),
            )
            .note(if senders.is_empty() {
                tr!("None yet. Everyone else's images stay blocked until you load them, and tracking pixels never load.")
            } else {
                tr!("Everyone else's images stay blocked until you load them, and tracking pixels never load.")
            })
            .icon(Icon::Image)
            .when_some((!senders.is_empty()).then_some(()), |card, ()| {
                card.control(
                    Button::new("privacy-forget-all")
                        .label(tr!("Forget all"))
                        .variant(Variant::OnInset)
                        .on_click(cx.listener(|this, _, _, cx| this.forget_all_senders(cx))),
                )
            }),
        );
        for (ix, address) in senders.into_iter().enumerate() {
            let who = address.clone();
            images = images.child(
                Card::new(format!("privacy-sender-{ix}"), address.clone()).control(
                    Button::new(format!("privacy-sender-remove-{ix}"))
                        .label(tr!("Remove"))
                        .name(tr!("Remove {0}", address))
                        .variant(Variant::OnInset)
                        .on_click(cx.listener(move |this, _, _, cx| this.forget_sender(&who, cx))),
                ),
            );
        }
        let icons_on = komorebi::favicon::enabled(&self.db);
        let icons = Card::new("privacy-icons", tr!("Icons from mail domains"))
            .note(tr!("An account without an image of its own shows its mail domain's icon. Komorebi asks the domain's own website for it when the account is added and about once a month, so that website sees your IP address; no other service is asked. Off: nothing is asked and the icons fetched are removed."))
            .icon(Icon::Image)
            .control(
                Toggle::new("privacy-icons-toggle", tr!("Icons from mail domains"), icons_on)
                    .state_label(tr!("On"), tr!("Off"))
                    .on_change({
                        let this = this.clone();
                        move |on, _, cx| {
                            let _ = this.update(cx, |this, cx| this.set_account_icons(on, cx));
                        }
                    }),
            );
        let diag_on = komorebi::diag::enabled();
        let diag = Card::new("privacy-diag", tr!("Diagnostics log"))
            .note(tr!("Records only server commands, response codes and timings on this PC, never mail, addresses or passwords; nothing is sent."))
            .control(
                Toggle::new("privacy-diag-toggle", tr!("Keep a diagnostics log"), diag_on)
                    .state_label(tr!("On"), tr!("Off"))
                    .on_change(move |on, _, cx| {
                        let _ = this.update(cx, |this, cx| this.set_diagnostics(on, cx));
                    }),
            );
        let logs = Card::new("privacy-logs", tr!("Logs folder"))
            .note(tr!("The log files, in the data folder."))
            .icon(Icon::Folder)
            .control(
                Button::new("privacy-logs-open")
                    .label(tr!("Open logs folder"))
                    .name(tr!("Open the logs folder in File Explorer"))
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(|this, _, _, cx| this.open_logs_folder(cx))),
            );
        let wipe = Card::new("privacy-wipe", tr!("Remove all data"))
            .note(tr!("Deletes the saved passwords of all accounts from Windows Credential Manager, then everything Komorebi keeps in the data folder (downloaded mail, drafts, the Outbox, settings and logs), and closes Komorebi. Mail on the server is not affected."))
            .icon(Icon::Delete)
            .control(
                Button::new("privacy-wipe-button")
                    .label(tr!("Remove all data"))
                    .variant(Variant::Destructive)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_wipe(window, cx))),
            );
        let (now, wanted) = self.cache_lock();
        let lock = Card::new("privacy-cache", tr!("Encrypt the mail cache on this computer"))
            .note(tr!("Downloaded mail, drafts and the Outbox are encrypted with a key only your Windows user can unlock. If that key is lost, drafts and the Outbox, which exist only in the cache, are lost too. Attachment copies are not encrypted."))
            .control(
                Toggle::new(
                    "privacy-cache-toggle",
                    tr!("Encrypt the mail cache on this computer"),
                    wanted,
                )
                .state_label(tr!("On"), tr!("Off"))
                .on_change({
                    let this = cx.entity().downgrade();
                    move |on, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            if on {
                                this.ask_cache_lock(window, cx);
                            } else {
                                this.set_cache_lock(false, cx);
                            }
                        });
                    }
                }),
            );
        let pending = (wanted != now).then(|| {
            Card::new(
                "privacy-cache-pending",
                if wanted {
                    tr!("The cache is encrypted when Komorebi next starts.")
                } else {
                    tr!("The cache is decrypted when Komorebi next starts.")
                },
            )
            .note(tr!("This can take a minute for a large cache."))
            .control(
                Button::new("privacy-restart")
                    .label(tr!("Restart now"))
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::RestartNow))),
            )
        });
        div()
            .flex()
            .flex_col()
            .child(page_head(
                Section::Privacy.name(),
                Some(tr!("What Komorebi keeps on this PC.").into()),
                cx,
            ))
            .child(group_head(tr!("Remote images"), cx))
            .child(images)
            .child(group_head(tr!("Account icons"), cx))
            .child(icons)
            .child(group_head(tr!("Diagnostics"), cx))
            .child(diag)
            .child(logs)
            .child(group_head(tr!("Mail cache"), cx))
            .child(lock)
            .children(pending)
            .child(group_head(tr!("settings group" => "Reset"), cx))
            .child(wipe)
    }
}

use gpui::prelude::*;
