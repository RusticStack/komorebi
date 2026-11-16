//! What the composer does with its drafts (the Slint app's
//! `src/app/composer.rs` `impl App`, over the same library model,
//! `komorebi::compose`): start, show and park drafts, save them two
//! seconds after the last edit and on every way out, copy them to the
//! server at most every 30 seconds, discard them, and send them after
//! their undo window.
//!
//! The formatted body's document lives in the editor while its draft
//! shows; [`ComposerView::lend`] puts it back into the draft for whatever
//! reads the whole draft (saving, sending, the send guards).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use gpui::{AppContext as _, Context, Focusable as _, PathPromptOptions, Window};
use komorebi::compose::{
    self, AUTOSAVE_AFTER, Attached, Draft, DraftId, Field, PendingSend, SendBlock, SendCheck,
    Stored,
};
use komorebi::db;
use komorebi::mail::mime::Body;
use komorebi::mail::render::links::Mailto;
use komorebi::mail::smtp::Outgoing;

use super::{ComposerEvent, ComposerView, Status, sends, text};
use crate::widgets::dialog::{self, Confirm};

impl ComposerView {
    // ---- starting and showing -------------------------------------------

    /// New mail from `account` (none: the first account).
    pub fn compose_new(
        &mut self,
        account: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(account) = account.or_else(|| self.accounts.first().map(|a| a.id)) else {
            return self.say(Status::NoAccountWrite, cx);
        };
        self.start(Draft::new(account), window, cx);
    }

    /// A `mailto:` link: a new message with its fields.
    pub fn compose_mailto(
        &mut self,
        m: Mailto,
        account: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(account) = account.or_else(|| self.accounts.first().map(|a| a.id)) else {
            return self.say(Status::NoAccountWrite, cx);
        };
        self.start(
            Draft {
                show_cc: !m.cc.is_empty() || !m.bcc.is_empty(),
                to: m.to,
                cc: m.cc,
                bcc: m.bcc,
                subject: m.subject,
                body: m.body,
                ..Draft::new(account)
            },
            window,
            cx,
        );
    }

    /// Reply (or Reply all) to `msg`, whose parsed body is `body` (none:
    /// still downloading). A message replied to already brings its draft
    /// back.
    pub fn reply(
        &mut self,
        msg: &db::Message,
        body: Option<&Body>,
        all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(body) = body else {
            return self.say(Status::StillDownloading, cx);
        };
        if let Some(id) = self.drafts.find_reply(msg.id) {
            return self.resume(id, window, cx);
        }
        match self.db.draft_for_message(msg.id) {
            Ok(Some(row)) => return self.open_saved(row, window, cx),
            Ok(None) => {}
            Err(e) => return self.say(Status::Error(format!("{e:#}")), cx),
        }
        let me = self
            .accounts
            .iter()
            .find(|a| a.id == msg.account_id)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let heads = text::heads(komorebi_i18n::current());
        let mut draft = Draft::reply(msg, body, &me, all, &heads);
        if let Some(html) = compose::quotable(msg.raw.as_deref()) {
            draft.quote_html(msg, &html, &heads);
        }
        draft.quote_text(msg, compose::strip_signature(&body.text), &heads);
        self.start(draft, window, cx);
    }

    /// Forward `msg` with its attachments.
    pub fn forward(
        &mut self,
        msg: &db::Message,
        body: Option<&Body>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(body), Some(raw)) = (body, msg.raw.as_deref()) else {
            return self.say(Status::StillDownloading, cx);
        };
        let attachments = komorebi::mail::mime::attachments(raw, |_| true)
            .into_iter()
            .map(|(filename, data)| komorebi::mail::smtp::OutAttachment { filename, data }.into())
            .collect();
        let heads = text::heads(komorebi_i18n::current());
        let mut draft = Draft::forward(msg, body, attachments, &heads);
        if let Some(html) = compose::quotable(Some(raw)) {
            draft.quote_html(msg, &html, &heads);
        }
        draft.quote_text(msg, &body.text, &heads);
        self.start(draft, window, cx);
    }

    /// Forward `msg` as an attachment (its bytes as received).
    pub fn forward_attached(
        &mut self,
        msg: &db::Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(raw) = msg.raw.clone() else {
            return self.say(Status::StillDownloading, cx);
        };
        self.start(Draft::forward_attached(msg, raw), window, cx);
    }

    /// Open saved draft `row` (the Drafts list, or a reply to its message).
    pub fn open_saved(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.drafts.find_row(row) {
            return self.resume(id, window, cx);
        }
        // Open in a draft tab or a composer window already: that one comes
        // forward, and the draft is never open twice.
        if let Some(other) = self.sends.holder(row, cx.entity_id(), cx) {
            let _ = other.update(cx, |_, cx| cx.emit(ComposerEvent::Raise));
            return;
        }
        // The row here, the copies of its attachments and pictures read on
        // the background executor (a large file never holds the window);
        // a draft with none opens at once.
        match compose::load_job(&self.db, row) {
            Ok(Some(job)) if !job.has_files() => {
                let (draft, missing) = compose::finish_load(&self.db, job.run());
                self.start(draft, window, cx);
                if !missing.is_empty() {
                    self.say(Status::LeftOut(missing.join(", ")), cx);
                }
            }
            Ok(Some(job)) => {
                let read = cx.background_spawn(async move { job.run() });
                let window = window.window_handle();
                cx.spawn(async move |this, cx| {
                    let loaded = read.await;
                    let _ = window.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            // Opened meanwhile (a second click): once only.
                            if let Some(id) = this.drafts.find_row(row) {
                                return this.resume(id, window, cx);
                            }
                            let (draft, missing) = compose::finish_load(&this.db, loaded);
                            this.start(draft, window, cx);
                            if !missing.is_empty() {
                                this.say(Status::LeftOut(missing.join(", ")), cx);
                            }
                        });
                    });
                })
                .detach();
            }
            Ok(None) => {
                self.say(Status::DraftGone, cx);
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to: None,
                    count: true,
                });
            }
            Err(e) => self.say(Status::Error(format!("{e:#}")), cx),
        }
    }

    /// Edit message `message`, a draft in the server's Drafts folder (saved
    /// on another device or in webmail; the reader's Edit): its text and
    /// attachments become a saved draft of this PC linked to the server's
    /// message, so saving it replaces that message and the folder keeps one
    /// copy. Opening it again finds the same draft. The Slint app's
    /// `edit_server_draft` (src/app/composer.rs).
    pub fn edit_server_draft(&mut self, message: i64, window: &mut Window, cx: &mut Context<Self>) {
        let msg = match self.db.message(message) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => return self.say(Status::Error(format!("{e:#}")), cx),
        };
        let folder = match self.db.folder(msg.folder_id) {
            Ok(Some(f)) if f.role == komorebi::role::Role::Drafts => f,
            Ok(_) => return,
            Err(e) => return self.say(Status::Error(format!("{e:#}")), cx),
        };
        let Some(raw) = msg.raw.as_deref() else {
            return self.say(Status::StillDownloading, cx);
        };
        let body = komorebi::mail::mime::parse_body(raw);
        let copy = db::ServerCopy {
            account_id: msg.account_id,
            folder_id: msg.folder_id,
            uidvalidity: folder.uidvalidity,
            uid: msg.uid,
        };
        let fields = komorebi::mail::mime::draft_fields(raw);
        let row = db::DraftRow {
            account_id: msg.account_id,
            to: msg.to.clone(),
            cc: msg.cc.clone(),
            bcc: fields.bcc,
            subject: msg.subject.clone(),
            body: body.text.clone(),
            in_reply_to: fields.in_reply_to,
            refs: fields.references,
            message_id: msg.msgid.clone(),
            importance: msg.importance,
            ..Default::default()
        };
        let files: Vec<(String, String, Vec<u8>)> =
            komorebi::mail::mime::attachments(raw, |_| true)
                .into_iter()
                .map(|(name, data)| (name, String::new(), data))
                .collect();
        match self.db.adopt_server_draft(&row, &copy, &files) {
            Ok(id) => {
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to: None,
                    count: true,
                });
                self.open_saved(id, window, cx);
            }
            Err(e) => self.say(Status::Error(format!("{e:#}")), cx),
        }
    }

    /// Whether message `message` has a reply in progress, in this composer
    /// or saved in Drafts: the reader's "You started a reply to this
    /// message" bar, whose Resume opens it again (a reply to the message
    /// brings its draft back). The Slint app's `show_draft_reply`.
    pub fn reply_started(&self, message: i64) -> bool {
        self.drafts.find_reply(message).is_some()
            || self.db.draft_for_message(message).ok().flatten().is_some()
    }

    /// Make `draft` the active one and show it; a draft in progress is
    /// saved and kept.
    pub fn start(&mut self, mut draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        compose::prepare(&self.db, &mut draft, &self.defaults);
        self.stow(cx);
        let parked = self.park(cx);
        self.drafts.open(draft);
        self.show_draft(window, cx);
        if parked {
            self.say(Status::EarlierDraftSaved, cx);
        }
    }

    /// Show draft `id`, in memory already, saving the one it replaces.
    fn resume(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        self.stow(cx);
        if self.drafts.active_id() != Some(id) {
            let parked = self.park(cx);
            self.drafts.switch_to(id);
            if parked {
                self.say(Status::EarlierDraftSaved, cx);
            }
        }
        self.show_draft(window, cx);
    }

    /// Push the active draft into the fields and the body, and focus To for
    /// a message without recipients and the top of the body otherwise.
    pub(crate) fn show_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        // The account may have been removed since; fall back to the first.
        if !accounts.contains(&d.account_id) {
            d.account_id = accounts.first().copied().unwrap_or_default();
        }
        let ed = d.rich.take();
        let fields = [
            (Field::To, d.to.clone()),
            (Field::Cc, d.cc.clone()),
            (Field::Bcc, d.bcc.clone()),
        ];
        let (subject, body, empty_to) = (d.subject.clone(), d.body.clone(), d.to.trim().is_empty());
        self.filling = true;
        for (field, value) in fields {
            if let Some(input) = self.input(field).map(|i| i.input.clone()) {
                let tail = compose::recipients::tail(&value).to_string();
                input.update(cx, |s, cx| s.set_value(tail, window, cx));
            }
        }
        self.subject
            .update(cx, |s, cx| s.set_value(subject, window, cx));
        self.plain.update(cx, |s, cx| s.set_value(body, window, cx));
        self.body.update(cx, |b, cx| b.set_doc(ed, 0, cx));
        self.filling = false;
        self.guard = None;
        self.drafts.suggest = None;
        self.link_open = false;
        self.composing = true;
        if empty_to {
            self.focus_field(Field::To, window, cx);
        } else {
            self.focus_body(window, cx);
        }
        cx.notify();
    }

    /// The formatted document goes back from the editor into the draft
    /// showing, before another draft takes the composer or it hides.
    fn stow(&mut self, cx: &mut Context<Self>) {
        let ed = self.body.update(cx, |b, _| b.take());
        if let Some(ed) = ed
            && self.composing
            && let Some(d) = self.drafts.active_mut()
        {
            d.rich = Some(ed);
        }
    }

    /// Runs `f` on the active draft with its whole document in it (the
    /// formatted body's editor gives it back afterwards).
    pub(crate) fn lend<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Draft) -> R,
    ) -> Option<R> {
        let id = self.drafts.active_id()?;
        self.lend_id(id, cx, f)
    }

    fn lend_id<R>(
        &mut self,
        id: DraftId,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Draft) -> R,
    ) -> Option<R> {
        if !self.drafts.contains(id) {
            return None;
        }
        let shown = self.drafts.active_id() == Some(id) && self.composing;
        let ed = if shown {
            self.body.update(cx, |b, _| b.take())
        } else {
            None
        };
        let d = self.drafts.get_mut(id)?;
        let lent = ed.is_some();
        if lent {
            d.rich = ed;
        }
        let r = f(d);
        if lent || (shown && d.rich.is_some()) {
            let ed = d.rich.take();
            self.body.update(cx, |b, _| b.put(ed));
        }
        Some(r)
    }

    // ---- editing --------------------------------------------------------

    /// Something in the draft changed: the guard bar asked about the draft
    /// as it was, and it is saved two seconds after the last edit.
    pub(crate) fn edited(&mut self, cx: &mut Context<Self>) {
        self.guard = None;
        if matches!(self.status, Some(Status::SendBlocked(_))) {
            self.status = None;
        }
        self.schedule_autosave(cx);
        cx.notify();
    }

    pub(crate) fn plain_edited(&mut self, cx: &mut Context<Self>) {
        if self.filling {
            return;
        }
        let value = self.plain.read(cx).value().to_string();
        if let Some(d) = self.drafts.active_mut()
            && d.rich.is_none()
            && d.body != value
        {
            d.edit(Field::Body, &value);
            self.edited(cx);
        }
    }

    pub(crate) fn subject_edited(&mut self, cx: &mut Context<Self>) {
        if self.filling {
            return;
        }
        let value = self.subject.read(cx).value().to_string();
        if let Some(d) = self.drafts.active_mut()
            && d.subject != value
        {
            d.edit(Field::Subject, &value);
            self.edited(cx);
        }
    }

    pub(crate) fn body_event(&mut self, e: &super::editor::EditorEvent, cx: &mut Context<Self>) {
        use super::editor::EditorEvent;
        match e {
            EditorEvent::Changed => {
                if let Some(d) = self.drafts.active_mut() {
                    d.touched = true;
                    d.dirty = true;
                }
                self.edited(cx);
            }
            EditorEvent::Moved => cx.notify(),
            EditorEvent::PasteFiles(paths) => self.attach_paths(paths.clone(), cx),
            // The composer's "Drop to attach" steps aside for the text.
            EditorEvent::DropOver(_) => cx.notify(),
            EditorEvent::Resized => {}
        }
    }

    /// Another From account: its signature replaces the old one while that
    /// is untouched, and the draft is saved at once unless it is blank.
    pub(crate) fn pick_account(
        &mut self,
        account: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.drafts.active().map(|d| d.kind) else {
            return;
        };
        let sig = compose::signature_for(&self.db, account, kind);
        let changed = self.lend(cx, |d| d.switch_account(account, sig.as_ref()));
        let Some(Some(changed)) = changed else {
            return;
        };
        if changed {
            self.refresh_body(window, cx);
        }
        if let Some(id) = self.drafts.active_id() {
            self.save_draft(id, cx);
        }
        cx.notify();
    }

    /// The format bar's importance: saved at once unless the message is
    /// blank.
    pub(crate) fn set_importance(&mut self, importance: db::Importance, cx: &mut Context<Self>) {
        let changed = self.lend(cx, |d| d.set_importance(importance));
        if changed == Some(true)
            && let Some(id) = self.drafts.active_id()
        {
            self.save_draft(id, cx);
        }
        cx.notify();
    }

    /// The message's format: plain text, or formatted again.
    pub(crate) fn set_rich(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = self.defaults.doc();
        let changed = self.lend(cx, |d| d.set_rich(on, defaults));
        if changed != Some(true) {
            return;
        }
        // The document is in the editor now (`lend` gave it there), or the
        // text goes into the field: either way the body starts again.
        let body = self
            .drafts
            .active()
            .map(|d| d.body.clone())
            .unwrap_or_default();
        self.filling = true;
        self.plain.update(cx, |s, cx| s.set_value(body, window, cx));
        self.filling = false;
        self.body.update(cx, |b, cx| {
            let ed = b.take();
            b.set_doc(ed, 0, cx);
        });
        self.focus_body(window, cx);
        self.edited(cx);
    }

    /// The body changed from outside the editor or the field (a signature
    /// swapped, a quote unfolded): show it again.
    pub(crate) fn refresh_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body.read(cx).is_formatted() {
            self.body.update(cx, |b, cx| b.changed_outside(cx));
            return;
        }
        let body = self
            .drafts
            .active()
            .map(|d| d.body.clone())
            .unwrap_or_default();
        self.filling = true;
        self.plain.update(cx, |s, cx| s.set_value(body, window, cx));
        self.filling = false;
    }

    /// "Show quoted text": the folded quote back in the body, unchanged
    /// (nothing to save).
    pub(crate) fn unfold_quote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body.read(cx).is_formatted() {
            self.body.update(cx, |b, cx| {
                b.act(&komorebi::compose::rich::Action::ShowQuote, cx)
            });
            return;
        }
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if d.unfold_quote() {
            self.refresh_body(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn reveal_cc(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self.drafts.active_mut() {
            d.show_cc = true;
        }
        cx.notify();
        self.focus_field(field, window, cx);
    }

    pub(crate) fn focus_field(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = match field {
            Field::Subject => Some(self.subject.read(cx).focus_handle(cx)),
            Field::Body => None,
            f => self
                .input(f)
                .map(|i| i.input.clone())
                .map(|i| i.read(cx).focus_handle(cx)),
        };
        match handle {
            Some(h) => window.focus(&h, cx),
            None => self.focus_body(window, cx),
        }
    }

    /// The body typed so far (bytes), for the A/B harness (`bench.rs`).
    pub(crate) fn bench_body_len(&self, cx: &gpui::App) -> usize {
        if self.body.read(cx).is_formatted() {
            self.body.read(cx).text().len()
        } else {
            self.plain.read(cx).value().len()
        }
    }

    pub(crate) fn focus_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = if self.body.read(cx).is_formatted() {
            self.body.read(cx).focus_handle(cx)
        } else {
            self.plain.read(cx).focus_handle(cx)
        };
        window.focus(&handle, cx);
    }

    pub(crate) fn say(&mut self, status: Status, cx: &mut Context<Self>) {
        self.status = Some(status.clone());
        cx.emit(ComposerEvent::Status(status));
        cx.notify();
    }

    // ---- saving ---------------------------------------------------------

    /// Save draft `id` now. False (with the error said) if that failed;
    /// the draft stays in memory either way.
    pub(crate) fn save_draft(&mut self, id: DraftId, cx: &mut Context<Self>) -> bool {
        self.store_draft(id, false, cx)
    }

    /// Save draft `id`: its row at once, the copies of its attachments and
    /// pictures written on the background executor (a large file never
    /// holds the window), or here with `wait` (it is being sent, or the
    /// window is closing). False (with the error said) if that failed.
    fn store_draft(&mut self, id: DraftId, wait: bool, cx: &mut Context<Self>) -> bool {
        let Some(account) = self.drafts.get(id).map(|d| d.account_id) else {
            return true;
        };
        let email = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let db = self.db.clone();
        let Some((stored, reply_to)) = self.lend_id(id, cx, |d| {
            let stored = if wait {
                compose::store(&db, d, &email).map(|s| (s, Vec::new()))
            } else {
                compose::store_rows(&db, d, &email)
            };
            (stored, d.reply_to)
        }) else {
            return true;
        };
        let stored = stored.map(|(s, jobs)| {
            if !jobs.is_empty() {
                self.write_copies(id, jobs, cx);
            }
            s
        });
        match stored {
            Ok(Stored::Nothing) => true,
            Ok(Stored::Updated) => {
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to: None,
                    count: false,
                });
                cx.notify();
                true
            }
            Ok(Stored::Inserted) => {
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to,
                    count: true,
                });
                cx.notify();
                true
            }
            Ok(Stored::Deleted(copy)) => {
                if let Some(copy) = copy {
                    cx.emit(ComposerEvent::RemoveServerCopy(copy));
                }
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to,
                    count: true,
                });
                cx.notify();
                true
            }
            Err(e) => {
                self.say(Status::CouldNotSave(format!("{e:#}")), cx);
                false
            }
        }
    }

    /// A save's copies written on the background executor. One that fails
    /// loses its row (the next save writes it again) and is said; a server
    /// copy asked for meanwhile goes once they are all written.
    fn write_copies(&mut self, id: DraftId, jobs: Vec<compose::FileJob>, cx: &mut Context<Self>) {
        let Some(row) = jobs.first().map(|j| j.draft) else {
            return;
        };
        self.writing.entry(row).or_insert((id, 0, None, false)).1 += 1;
        let task = cx.background_spawn(async move {
            jobs.into_iter()
                .filter_map(|j| j.run().err().map(|e| (j, format!("{e:#}"))))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let failed = task.await;
            let _ = this.update(cx, |this, cx| {
                let db = this.db.clone();
                for (j, _) in &failed {
                    this.lend_id(id, cx, |d| compose::copy_failed(&db, d, j));
                }
                if let Some((_, e)) = failed.first() {
                    this.say(Status::CouldNotSave(e.clone()), cx);
                }
                let done = this.writing.get_mut(&row).and_then(|w| {
                    w.1 = w.1.saturating_sub(1);
                    (w.1 == 0).then_some((w.2, w.3))
                });
                if let Some((wanted, send)) = done {
                    this.writing.remove(&row);
                    if send {
                        // Send was pressed meanwhile: it goes now (a copy
                        // that failed is written again by its save).
                        this.send_saved(id, cx);
                    } else if let Some(now) = wanted {
                        this.save_to_server(id, now, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Whether draft row `row`'s copies are still being written.
    pub fn writing(&self, row: i64) -> bool {
        self.writing.contains_key(&row)
    }

    /// Have the sync worker copy draft `id`'s saved row to the server's
    /// Drafts; unless `now`, only if the last copy was asked for 30 s ago.
    /// While its copies are being written, it goes once they are.
    pub(crate) fn save_to_server(&mut self, id: DraftId, now: bool, cx: &mut Context<Self>) {
        let Some((row, account)) = self
            .drafts
            .get(id)
            .and_then(|d| Some((d.row?, d.account_id)))
        else {
            return;
        };
        if let Some(w) = self.writing.get_mut(&row) {
            w.2 = Some(w.2.unwrap_or(false) || now);
            return;
        }
        let at = cx.background_executor().now();
        if !now && !compose::server_save_due(self.server_saves.get(&row).copied(), at) {
            return;
        }
        self.server_saves.insert(row, at);
        cx.emit(ComposerEvent::SaveToServer { row, account });
    }

    /// Save the active draft before another one takes its place. True if
    /// it was saved and stays.
    fn park(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.drafts.active_id() else {
            return false;
        };
        let kept = self.save_draft(id, cx) && self.lend_id(id, cx, |d| d.keep()).unwrap_or(false);
        if kept {
            self.save_to_server(id, true, cx);
        }
        kept
    }

    /// An edit: save the active draft two seconds after the last one (a
    /// design delay, the Slint app's `AUTOSAVE_AFTER`).
    pub(crate) fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.last_edit = cx.background_executor().now();
        if self.autosave.is_some() {
            return;
        }
        self.autosave = Some(cx.spawn(async move |this, cx| {
            let mut wait = AUTOSAVE_AFTER;
            loop {
                cx.background_executor().timer(wait).await; // timer-exempt: the autosave delay the design asks for
                let rest = this.update(cx, |this, cx| {
                    let now = cx.background_executor().now();
                    let rest = compose::autosave_wait(this.last_edit, now);
                    if rest.is_none() {
                        this.autosave = None;
                        this.autosave_now(cx);
                    }
                    rest
                });
                match rest {
                    Ok(Some(rest)) => wait = rest,
                    _ => break,
                }
            }
        }));
    }

    fn autosave_now(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        if self.save_draft(id, cx) {
            self.save_to_server(id, false, cx);
        }
    }

    /// Save every changed draft (the window is closing). False if any save
    /// failed (the error is said).
    pub fn save_all(&mut self, cx: &mut Context<Self>) -> bool {
        let mut all = true;
        for id in self.drafts.ids() {
            all &= self.store_draft(id, true, cx);
        }
        all
    }

    // ---- leaving --------------------------------------------------------

    /// Esc or Close: a draft worth keeping is saved here and copied to the
    /// server's Drafts at once ("Draft saved"); an empty one goes.
    pub fn close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.park_composer(cx) {
            self.hide(cx);
        }
    }

    /// Every way out of the composer that keeps the draft parks it the
    /// same way. False when a draft worth keeping could not be saved: it
    /// stays shown ("Could not save the draft"), so its text is never out
    /// of sight with no saved copy.
    pub fn park_composer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.drafts.active_id() else {
            return true;
        };
        let saved = self.save_draft(id, cx);
        let keep = self.lend_id(id, cx, |d| d.keep()).unwrap_or(false);
        if !keep {
            self.drafts.remove(id);
        } else if saved {
            self.save_to_server(id, true, cx);
            self.say(Status::DraftSaved, cx);
        } else if self.composing {
            return false;
        }
        true
    }

    /// Save the draft shown now, without waiting for the autosave delay
    /// (its tab or window is left).
    pub fn flush_autosave(&mut self, cx: &mut Context<Self>) {
        if !self.composing {
            return;
        }
        self.autosave = None;
        self.autosave_now(cx);
    }

    /// The reading pane's draft makes room (another draft starts in the
    /// pane, another message is selected): one with something written is
    /// saved, here and on the server, and leaves this composer; its saved
    /// row is returned for the shell to open in a background tab of its own
    /// ("Draft kept in a tab", [`Status::KeptInTab`]). An empty one goes.
    /// One whose save failed stays shown here with the error said, as it
    /// has no saved row for a tab to open: the composer stays open, which
    /// the caller sees with [`ComposerView::is_open`].
    /// The Slint app's `pane_draft_aside` (src/app/tabs.rs).
    pub fn set_aside(&mut self, cx: &mut Context<Self>) -> Option<i64> {
        let row = self.hand_over(cx)?;
        self.say(Status::KeptInTab, cx);
        Some(row)
    }

    /// The draft shown leaves for a composer window of its own (Ctrl+O,
    /// "Open in new window"): as [`ComposerView::hand_over`], but a reply
    /// or forward not yet changed is kept too (the user asked for it in a
    /// window). `None` for an empty new message, which the window starts
    /// afresh, or (still open) for one that could not be saved.
    pub fn pop_out(&mut self, cx: &mut Context<Self>) -> Option<i64> {
        if !self.composing {
            return None;
        }
        self.lend(cx, |d| {
            if !d.is_empty() {
                d.touched = true;
                d.dirty = true;
            }
        });
        self.hand_over(cx)
    }

    /// The draft shown leaves this composer for another (a tab or a window
    /// of its own) by its saved row, as [`ComposerView::set_aside`] without
    /// the status line: saved here and on the server, then hidden. An empty
    /// one goes (`None`); one that could not be saved stays (`None`, and
    /// [`ComposerView::is_open`] stays true).
    pub fn hand_over(&mut self, cx: &mut Context<Self>) -> Option<i64> {
        if !self.composing {
            return None;
        }
        let id = self.drafts.active_id()?;
        // Written here: the other composer reads the copies at once.
        let saved = self.store_draft(id, true, cx);
        let keep = self.lend_id(id, cx, |d| d.keep()).unwrap_or(false);
        let row = self.drafts.get(id).and_then(|d| d.row);
        if !keep {
            self.drafts.remove(id);
            self.hide(cx);
            return None;
        }
        let Some(row) = row.filter(|_| saved) else {
            // Not saved: kept in sight, with "Could not save the draft".
            return None;
        };
        self.save_to_server(id, true, cx);
        self.stow(cx);
        // The tab's composer opens the saved row; this one lets it go.
        self.drafts.remove(id);
        self.hide(cx);
        Some(row)
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        self.stow(cx);
        self.end_preview(cx);
        self.autosave = None;
        self.composing = false;
        self.guard = None;
        self.drafts.suggest = None;
        self.drafts.hide();
        self.body.update(cx, |b, cx| b.set_doc(None, 0, cx));
        cx.emit(ComposerEvent::Closed);
        cx.notify();
    }

    /// Discard: a draft with something the user wrote asks first.
    pub fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.lend(cx, |d| d.keep()).unwrap_or(false);
        if !keep {
            return self.discard_confirmed(cx);
        }
        let (title, message, confirm, cancel) = text::discard_question();
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: title.into(),
                message: message.into(),
                confirm: confirm.into(),
                cancel: cancel.into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = view.update(cx, |this, cx| this.discard_confirmed(cx));
            },
            window,
            cx,
        );
    }

    /// Drop the active draft with its saved row and attachment copies.
    pub fn discard_confirmed(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.drafts.active_id()
            && let Some(d) = self.drafts.remove(id)
            && let Some(row) = d.row
        {
            match self.db.delete_draft(row) {
                Ok(Some(copy)) => cx.emit(ComposerEvent::RemoveServerCopy(copy)),
                Ok(None) => {}
                Err(e) => self.say(Status::Error(format!("{e:#}")), cx),
            }
            cx.emit(ComposerEvent::DraftsChanged {
                reply_to: d.reply_to,
                count: true,
            });
        }
        self.hide(cx);
    }

    // ---- sending --------------------------------------------------------

    /// Send (Ctrl+Enter, Alt+S, the Send pill); `anyway`: "Send anyway" on
    /// the guard bar. The draft is saved as it is, moves to the outbox and
    /// the composer closes; its message is built while the undo window
    /// runs.
    pub fn send(&mut self, anyway: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(check) = self.lend(cx, |d| d.check_send(anyway)) else {
            return;
        };
        match check {
            // Send is always on: pressed too early, the field at fault says
            // why (inline, announced) and takes the focus.
            SendCheck::Blocked(SendBlock::FixAddress(field)) => {
                cx.notify();
                return self.focus_field(field, window, cx);
            }
            SendCheck::Blocked(SendBlock::NoRecipient) => {
                cx.notify();
                return self.focus_field(Field::To, window, cx);
            }
            SendCheck::Ask(guard) => {
                self.guard = Some(guard);
                return cx.notify();
            }
            SendCheck::Ready => {}
        }
        self.guard = None;
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        // Saved as it stands, even a reply sent just as it was filled in.
        if let Some(d) = self.drafts.get_mut(id) {
            d.touched = true;
            d.dirty = true;
        }
        // Copies of its files still being written (an attachment added a
        // moment ago): it goes once they are, as a server copy does.
        if let Some(row) = self.drafts.get(id).and_then(|d| d.row)
            && let Some(w) = self.writing.get_mut(&row)
        {
            w.3 = true;
            return;
        }
        self.send_saved(id, cx);
    }

    /// Send draft `id`, its checks passed and no copy of its files still
    /// being written: saved as it stands, moved to the outbox, the
    /// composer closed.
    fn send_saved(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(account) = self
            .drafts
            .get(id)
            .and_then(|d| self.accounts.iter().find(|a| a.id == d.account_id))
            .cloned()
        else {
            return self.say(Status::NoAccount, cx);
        };
        if let Some(d) = self.drafts.get_mut(id) {
            d.touched = true;
            d.dirty = true;
        }
        // Written here: the outbox reads the copies.
        if !self.store_draft(id, true, cx) {
            return;
        }
        let Some(row) = self.drafts.get(id).and_then(|d| d.row) else {
            return;
        };
        match self.db.outbox_send(row) {
            Ok(true) => {}
            Ok(false) => return self.say(Status::DraftGone, cx),
            Err(e) => return self.say(Status::Error(format!("{e:#}")), cx),
        }
        // The body is this draft's only while it is the one shown (Send may
        // have waited for its copies while another was opened).
        let shown = self.drafts.active_id() == Some(id);
        let Some(mut d) = self.drafts.remove(id) else {
            return;
        };
        if shown && let Some(ed) = self.body.update(cx, |b, _| b.take()) {
            d.rich = Some(ed);
        }
        let reply_to = d.reply_to;
        let outgoing = compose::outgoing(&self.db, &account, d);
        // Told before Closed: a draft tab (and the subscription to its
        // composer) goes with Closed. The send itself is the app's
        // (`sends.rs`), so it goes out whatever becomes of this composer.
        cx.emit(ComposerEvent::DraftsChanged {
            reply_to,
            count: true,
        });
        self.queue_send(row, account.id, outgoing, self.undo_delay, cx);
        if shown {
            self.hide(cx);
        }
    }

    /// Build outbox row `row`'s message off the UI thread and queue it
    /// once `delay` (the undo window) has passed and the build is done
    /// (the app's sends, [`Sends`]).
    pub fn queue_send(
        &mut self,
        row: i64,
        account: i64,
        outgoing: Outgoing,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        let Some(db_path) = self.db.file() else {
            return self.say(Status::Error("the cache has no file".into()), cx);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let subject = compose::short_subject(&outgoing.subject, &text::no_subject());
        let entry = sends::Entry {
            send: PendingSend {
                seq: 0,
                account,
                cancel: cancel.clone(),
                built: false,
                due: delay.is_zero(),
                subject: subject.clone(),
            },
            db: self.db.clone(),
            origin: cx.entity_id(),
        };
        let job = compose::BuildJob {
            row,
            seq: 0,
            db_path,
            outgoing,
            cancel,
        };
        self.sends.start(row, entry, job, delay, cx);
        if !delay.is_zero() {
            cx.emit(ComposerEvent::Sending {
                row,
                subject,
                secs: delay.as_secs(),
            });
        }
    }

    /// What the app's sends have to say, told through this composer (the
    /// home one, `sends.rs`).
    pub(crate) fn relay(&mut self, event: ComposerEvent, cx: &mut Context<Self>) {
        if let ComposerEvent::Status(status) = &event {
            self.status = Some(status.clone());
            cx.notify();
        }
        cx.emit(event);
    }

    /// Undo while it waits: the message goes back to Drafts, unchanged,
    /// and opens in this composer. Too late once a send has it. Any
    /// composer undoes any send (the one that sent it may be gone).
    pub fn undo_send(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        // The flag first: a build finishing now can't store its result.
        self.sends.cancel(row);
        match self.db.outbox_unsend(row) {
            Ok(true) => {
                self.sends.remove(row);
                cx.emit(ComposerEvent::Unsent { row });
                self.open_saved(row, window, cx);
                let reply_to = self.drafts.active().and_then(|d| d.reply_to);
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to,
                    count: true,
                });
            }
            Ok(false) => {
                self.sends.remove(row);
                cx.emit(ComposerEvent::Unsent { row });
            }
            Err(e) => self.say(Status::Error(format!("{e:#}")), cx),
        }
    }

    /// Undo of a send another composer started (a draft tab or a composer
    /// window): the message goes back to Drafts, unchanged, and its saved
    /// row is returned for the shell to open in a tab or a window. This
    /// composer's own draft is not touched: opening the row here first
    /// would park it, and one whose save fails would be left out of sight
    /// when the row moved on, with no saved copy and no tab. `None` when it
    /// is too late (a send has it) or the cache failed (the error is said).
    pub fn unsend_for_elsewhere(&mut self, row: i64, cx: &mut Context<Self>) -> Option<i64> {
        // The flag first: a build finishing now can't store its result.
        self.sends.cancel(row);
        match self.db.outbox_unsend(row) {
            Ok(true) => {
                self.sends.remove(row);
                cx.emit(ComposerEvent::Unsent { row });
                let reply_to = self
                    .db
                    .draft(row)
                    .ok()
                    .flatten()
                    .and_then(|d| d.reply_to_msg);
                cx.emit(ComposerEvent::DraftsChanged {
                    reply_to,
                    count: true,
                });
                Some(row)
            }
            Ok(false) => {
                self.sends.remove(row);
                cx.emit(ComposerEvent::Unsent { row });
                None
            }
            Err(e) => {
                self.say(Status::Error(format!("{e:#}")), cx);
                None
            }
        }
    }

    /// Sends still in their undo window: every composer's, as the Slint
    /// app's one list.
    pub fn pending_sends(&self) -> Vec<i64> {
        self.sends.rows()
    }

    /// The composer that started the send of `row`, while it waits. A
    /// draft tab's is gone once it sent: the shell can compare it with the
    /// pane composer's and reopen such a send in a tab on Undo, as the
    /// Slint app does.
    pub fn sent_by(&self, row: i64) -> Option<gpui::EntityId> {
        self.sends.origin(row)
    }

    // ---- attachments ----------------------------------------------------

    /// Attach (Ctrl+Shift+A): the system's file dialog.
    pub fn attach(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.composing {
            return;
        }
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = answer.await {
                let _ = this.update(cx, |this, cx| this.attach_paths(paths, cx));
            }
        })
        .detach();
    }

    /// Read `paths` off the UI thread and attach them to the draft showing
    /// now (a drop, a paste, the dialog).
    pub fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        let read = cx.background_spawn(async move { compose::read_files(&paths) });
        cx.spawn(async move |this, cx| {
            let (added, errors) = read.await;
            let _ = this.update(cx, |this, cx| this.add_attachments(id, added, errors, cx));
        })
        .detach();
    }

    /// Files read for draft `id`: added, whatever their size (Komorebi
    /// sets no limit; the server's refusal is the only one), and saved once
    /// (the copies are made now, not by the autosave).
    pub(crate) fn add_attachments(
        &mut self,
        id: DraftId,
        added: Vec<Attached>,
        errors: Vec<compose::AttachFail>,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.drafts.get_mut(id) else {
            return;
        };
        d.add_attachments(added);
        self.guard = None;
        cx.notify();
        if !self.save_draft(id, cx) {
            return;
        }
        if !errors.is_empty() {
            let lines = errors.iter().map(text::attach_fail).collect();
            self.say(Status::AttachFailed(lines), cx);
        }
    }

    /// Remove attachment `index`, and its saved copy at once.
    pub(crate) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        let Some(d) = self.drafts.get_mut(id) else {
            return;
        };
        match d.remove_attachment(&db, index) {
            Ok(true) => {
                self.save_draft(id, cx);
            }
            Ok(false) => {}
            Err(e) => self.say(Status::Error(format!("{e:#}")), cx),
        }
        cx.notify();
    }
}
