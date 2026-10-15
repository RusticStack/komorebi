//! Read-only message windows ("Open in new window", Ctrl+O; docs/WINUI3.md
//! "Secondary windows"). Each is a second `AppWindow` in its detail-only
//! mode (the reader and its command bar only, under the system caption)
//! with a reader and render worker of its own. Replies and new mail from a
//! window open as composer tabs in the main window, which comes forward.
//! Closing the main window closes them all first; none outlives it.

use super::*;
use crate::mail::mime::Body;
use composer::DraftId;
use std::collections::HashSet;

/// A new window's size in logical px, until one has been resized.
const DEFAULT_SIZE: (u32, u32) = (900, 760);
/// How far a new window sits right and below the one it came from.
const CASCADE: f32 = 32.0;

/// One message window.
struct PopOut {
    id: u32,
    ui: AppWindow,
    open: Option<Open>,
    reader: reader::Reader,
    /// The message was moved or deleted on the server: readable, its
    /// actions off.
    gone: bool,
    /// The Move list's filter.
    move_filter: String,
}

/// The message windows open now.
pub(super) struct PopOuts {
    list: Vec<PopOut>,
    next: u32,
    /// The last window's size in logical px (setting `popout_size`).
    pub(super) size: Option<(u32, u32)>,
    /// The draft the main window's reading pane showed before it went to
    /// a composer window: the pane says so until that window goes.
    popped: Option<DraftId>,
    /// Files dropped on a composer window, attached together (a drop of
    /// several arrives one file at a time).
    dropped: Vec<PathBuf>,
}

impl PopOuts {
    pub(super) fn new(size: Option<(u32, u32)>) -> Self {
        PopOuts {
            list: Vec::new(),
            next: 0,
            size,
            popped: None,
            dropped: Vec::new(),
        }
    }

    fn get(&self, id: u32) -> Option<&PopOut> {
        self.list.iter().find(|p| p.id == id)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut PopOut> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.list.len()
    }

    /// The windows' message ids (tests).
    #[cfg(test)]
    pub(super) fn messages(&self) -> Vec<i64> {
        self.list
            .iter()
            .filter_map(|p| p.open.as_ref().map(|o| o.msg.id))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn window(&self, i: usize) -> Option<AppWindow> {
        self.list.get(i).map(|p| p.ui.clone_strong())
    }
}

impl App {
    /// Open message `id` in a window of its own (one that has it already
    /// comes forward).
    pub(super) fn open_window(&mut self, id: i64) {
        if let Some(p) = self
            .popouts
            .list
            .iter()
            .find(|p| p.open.as_ref().is_some_and(|o| o.msg.id == id))
        {
            crate::bring_to_front(&p.ui);
            return;
        }
        let msg = match self.db.message(id) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => return self.set_status(&text::detail(&e), true),
        };
        let ui = match AppWindow::new() {
            Ok(ui) => ui,
            Err(e) => {
                return self.set_status(&text::detail(&anyhow::anyhow!("{e}")), true);
            }
        };
        self.popouts.next += 1;
        let wid = self.popouts.next;
        self.dress_window(&ui);
        let notify: reader::Notify = Arc::new(move |reply: reader::Reply| {
            on_ui(Box::new(move |a| a.popout_render(wid, reply)));
        });
        let fetch: reader::Fetch = Arc::new(|url: &str| komorebi::fetch::get(url).ok());
        let worker = reader::Worker::new(notify, fetch)
            .with_caches(reader::POPOUT_DOC_CACHE, reader::POPOUT_TILE_CACHE_BYTES);
        let reader = reader::Reader::new(&ui, Box::new(worker), Box::new(reader::SystemShell));
        reader.on_mailto(|m| {
            with_app(|a| a.in_main_tab(|a| a.compose_mailto(m)));
        });
        wire_popout(&ui, wid);
        self.place_window(&ui);
        crate::watch_popout(&ui, move |w, h| {
            with_app(|a| a.popouts.size = Some((w, h)));
        });
        ui.show().ok();
        ui.set_focus_page(true);
        self.popouts.list.push(PopOut {
            id: wid,
            ui,
            open: None,
            reader,
            gone: false,
            move_filter: String::new(),
        });
        // Opened to be read: marked read (unless Settings say never).
        if !msg.seen && self.mark_read != MarkRead::Never {
            self.set_flags(&[db::Brief::from(&msg)], Flag::Seen, true);
        }
        self.popout_load(wid, msg);
    }

    /// A new window's globals (each window has its own Theme, Tr and
    /// Palette) and detail-only mode.
    fn dress_window(&self, ui: &AppWindow) {
        let main = self.ui();
        let theme = ui.global::<Theme>();
        theme.on_readable(settings::readable);
        let from = main.global::<Theme>();
        theme.set_icon_font(from.get_icon_font());
        theme.set_text_font(from.get_text_font());
        theme.set_display_font(from.get_display_font());
        settings::apply_popout_theme(ui, &self.system, self.theme);
        ui.set_detail_only(true);
        ui.set_has_accounts(true);
        ui.set_tab_kind(1);
        ui.set_app_icon(main.get_app_icon());
    }

    /// A new window's size (the last one's, else 900 x 760) and place: 32
    /// px right and below the main window, kept on its monitor.
    fn place_window(&self, ui: &AppWindow) {
        let (w, h) = self.popouts.size.unwrap_or(DEFAULT_SIZE);
        ui.window()
            .set_size(slint::LogicalSize::new(w as f32, h as f32));
        let main = self.ui();
        let scale = main.window().scale_factor();
        let at = main.window().position();
        let step = (CASCADE * (self.popouts.list.len() as f32 + 1.0) * scale).round() as i32;
        ui.window()
            .set_position(slint::PhysicalPosition::new(at.x + step, at.y + step));
    }

    /// Show `msg` in window `wid`: its text at once, or its page through
    /// the window's render worker; a body not downloaded yet is fetched by
    /// its account's worker.
    fn popout_load(&mut self, wid: u32, msg: db::Message) {
        let remote = self.db.remote_allowed(&msg.from_addr).unwrap_or(false);
        let Some(p) = self.popouts.get_mut(wid) else {
            return;
        };
        let id = msg.id;
        let mut body = None;
        match msg.raw.as_deref() {
            None => {
                p.reader.close();
                if let Some(w) = self.workers.get(&msg.account_id) {
                    w.send(Job::FetchBody(id));
                }
            }
            Some(raw) if raw.len() > SYNC_PARSE_MAX => {
                p.reader.open(id, reader::Source::Raw(raw.into()), remote);
            }
            Some(raw) => {
                let (parsed, input) = mime::parse_reader(raw);
                body = Some(parsed);
                if input.prefers_plain {
                    p.reader.close();
                } else {
                    p.reader
                        .open(id, reader::Source::Input(Arc::new(input)), remote);
                }
            }
        }
        p.ui.set_r_remote_allowed(remote);
        p.open = Some(Open { msg, body });
        self.popout_fill(wid);
    }

    /// Window `wid`'s header, text and commands, in the language shown.
    fn popout_fill(&self, wid: u32) {
        let Some(p) = self.popouts.get(wid) else {
            return;
        };
        let Some(open) = &p.open else {
            return;
        };
        self.fill_reader(&p.ui, open);
        // No reply bar, invitation card or tabs here.
        p.ui.set_r_draft_reply(false);
        p.ui.set_i_title(SharedString::new());
        p.ui.set_r_gone(p.gone);
        p.ui.set_r_unreachable(open.msg.raw.is_none() && self.unreachable(open.msg.account_id));
        let subject = if open.msg.subject.trim().is_empty() {
            t!(invoke_no_subject)
        } else {
            open.msg.subject.clone()
        };
        p.ui.set_window_title(t!(invoke_popout_title, subject.into()).into());
        let choices = self.move_options(open.msg.account_id, &[open.msg.folder_id]);
        let entries = triage::move_entries(&choices, &self.move_recent, &p.move_filter);
        p.ui.set_move_possible(!choices.is_empty() && !p.gone);
        p.ui.set_move_filter(p.move_filter.as_str().into());
        p.ui.set_move_targets(ModelRc::new(VecModel::from(entries)));
    }

    /// A reply from window `wid`'s render worker.
    pub(super) fn popout_render(&mut self, wid: u32, reply: reader::Reply) {
        let Some(p) = self.popouts.get_mut(wid) else {
            // Its window closed: dropped.
            return;
        };
        if let Some(body) = p.reader.on_reply(reply)
            && let Some(open) = p.open.as_mut()
            && open.body.is_none()
        {
            open.body = Some(body);
            self.popout_fill(wid);
        }
    }

    /// A body arrived from the server: the windows waiting for it show it.
    pub(super) fn popouts_body_ready(&mut self, message_id: i64) {
        let waiting: Vec<u32> = self
            .popouts
            .list
            .iter()
            .filter(|p| {
                p.open
                    .as_ref()
                    .is_some_and(|o| o.msg.id == message_id && o.msg.raw.is_none())
            })
            .map(|p| p.id)
            .collect();
        for wid in waiting {
            if let Ok(Some(m)) = self.db.message(message_id) {
                self.popout_load(wid, m);
            }
        }
    }

    /// A body could not be downloaded: the windows waiting for it say so.
    pub(super) fn popouts_body_failed(&self, message_id: i64, text: &str) {
        for p in &self.popouts.list {
            if p.open.as_ref().is_some_and(|o| o.msg.id == message_id) {
                p.reader.close();
                p.ui.set_loading(false);
                p.ui.set_r_body(text.into());
            }
        }
    }

    /// Window `wid`'s message as the actions see it (none while it is gone
    /// from the server).
    fn popout_brief(&self, wid: u32) -> Option<db::Brief> {
        let p = self.popouts.get(wid)?;
        if p.gone {
            return None;
        }
        p.open.as_ref().map(|o| db::Brief::from(&o.msg))
    }

    /// Run `f` so that a draft it starts opens as a tab of the main window,
    /// which then comes forward (a reply from a message window).
    fn in_main_tab(&mut self, f: impl FnOnce(&mut App)) {
        self.draft_tab = true;
        f(self);
        self.draft_tab = false;
        crate::bring_to_front(&self.ui());
    }

    /// Reply (0), Reply all (1), Forward (2) or Forward as attachment (3)
    /// from window `wid`: a composer tab in the main window.
    fn popout_respond(&mut self, wid: u32, kind: i32) {
        if self.popout_brief(wid).is_none() {
            return;
        }
        let Some(open) = self.popouts.get(wid).and_then(|p| p.open.as_ref()) else {
            return;
        };
        let (msg, body): (db::Message, Option<Body>) = (open.msg.clone(), open.body.clone());
        self.in_main_tab(|a| match kind {
            0 | 1 => a.reply_to(&msg, body.as_ref(), kind == 1),
            2 => a.forward_of(&msg, body.as_ref()),
            _ => {
                if let Some(raw) = msg.raw.clone() {
                    a.start_draft(composer::Draft::forward_attached(&msg, raw));
                }
            }
        });
    }

    /// Archive, Delete, Move or Junk from window `wid`: the window closes
    /// with its message (Undo brings the message back, not the window).
    fn popout_remove(&mut self, wid: u32, dest: Dest) {
        let Some(brief) = self.popout_brief(wid) else {
            return;
        };
        if let Dest::Folder(f) = dest {
            self.note_recent(f);
        }
        self.move_out(dest, vec![brief]);
    }

    /// Delete from window `wid`: from Trash or Junk the main window asks
    /// first (it comes forward), as the reader's Delete does.
    fn popout_delete(&mut self, wid: u32) {
        let Some(brief) = self.popout_brief(wid) else {
            return;
        };
        let role = self.folder_role(brief.folder_id);
        if role == Role::Trash || role == Role::Junk {
            self.delete_wait = vec![brief.id];
            self.ask(Question::DeleteFrom {
                trash: role == Role::Trash,
            });
            crate::bring_to_front(&self.ui());
            return;
        }
        self.move_out(Dest::Trash, vec![brief]);
    }

    /// Read / unread, important or not, from window `wid` (`on`: None
    /// toggles).
    fn popout_flag(&mut self, wid: u32, flag: Flag, on: Option<bool>) {
        let Some(brief) = self.popout_brief(wid) else {
            return;
        };
        let on = on.unwrap_or(!flag.of(&brief));
        self.set_flags(&[brief], flag, on);
    }

    /// Flags changed on these messages: the windows showing one follow.
    pub(super) fn popouts_flags_changed(&mut self, ids: &HashSet<i64>) {
        let shown: Vec<i64> = self
            .popouts
            .list
            .iter()
            .filter_map(|p| p.open.as_ref().map(|o| o.msg.id))
            .filter(|id| ids.contains(id))
            .collect();
        if shown.is_empty() {
            return;
        }
        let briefs = self.db.briefs(&shown).unwrap_or_default();
        for p in &mut self.popouts.list {
            let Some(open) = p.open.as_mut() else {
                continue;
            };
            if let Some(b) = briefs.iter().find(|b| b.id == open.msg.id) {
                open.msg.seen = b.seen;
                open.msg.flagged = b.flagged;
                p.ui.set_r_unread(!b.seen);
                p.ui.set_r_flagged(b.flagged);
            }
        }
    }

    /// A sync changed the cache: a window whose message is gone says so,
    /// and its actions go off.
    pub(super) fn refresh_popouts(&mut self) {
        let ids: Vec<i64> = self
            .popouts
            .list
            .iter()
            .filter_map(|p| p.open.as_ref().map(|o| o.msg.id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let Ok(found) = self.db.briefs(&ids) else {
            return;
        };
        let gone: Vec<u32> = self
            .popouts
            .list
            .iter()
            .filter(|p| {
                !p.gone
                    && p.open
                        .as_ref()
                        .is_some_and(|o| found.iter().all(|b| b.id != o.msg.id))
            })
            .map(|p| p.id)
            .collect();
        for wid in gone {
            if let Some(p) = self.popouts.get_mut(wid) {
                p.gone = true;
            }
            self.popout_fill(wid);
        }
    }

    /// Messages left their folders (moved, deleted, junk): their windows
    /// close. True if any did.
    pub(super) fn close_popouts_of(&mut self, ids: &HashSet<i64>) -> bool {
        let hit: Vec<u32> = self
            .popouts
            .list
            .iter()
            .filter(|p| p.open.as_ref().is_some_and(|o| ids.contains(&o.msg.id)))
            .map(|p| p.id)
            .collect();
        for &wid in &hit {
            self.close_popout(wid);
        }
        !hit.is_empty()
    }

    /// Close window `wid`: it goes from the screen now; its reader and
    /// render worker go once the event that closed it is over.
    pub(super) fn close_popout(&mut self, wid: u32) {
        let Some(i) = self.popouts.list.iter().position(|p| p.id == wid) else {
            return;
        };
        let p = self.popouts.list.remove(i);
        p.ui.hide().ok();
        Timer::single_shot(Duration::ZERO, move || drop(p));
    }

    /// The main window closes: every message window first.
    pub(super) fn close_all_popouts(&mut self) {
        for p in std::mem::take(&mut self.popouts.list) {
            p.ui.hide().ok();
        }
    }

    /// Keep the last message window's size for the next one.
    pub(super) fn save_popout_size(&self) {
        if let Some((w, h)) = self.popouts.size {
            self.db.set_setting("popout_size", &format!("{w} {h}")).ok();
        }
    }

    /// Windows' appearance or the theme choice changed: every window
    /// follows.
    pub(super) fn theme_popouts(&self) {
        for p in &self.popouts.list {
            settings::apply_popout_theme(&p.ui, &self.system, self.theme);
        }
    }

    /// The language changed: what Rust put in the windows says it in it.
    pub(super) fn repush_popouts(&self) {
        for p in &self.popouts.list {
            p.reader.language_changed();
            self.popout_fill(p.id);
        }
    }

    /// Window `wid`'s More menu: the commands that act on its message here
    /// (Show original, Save as .eml and Copy message ID are in the main
    /// window's).
    fn popout_menu(&self, wid: u32) {
        let Some(p) = self.popouts.get(wid) else {
            return;
        };
        let ui = &p.ui;
        let junk = p
            .open
            .as_ref()
            .is_some_and(|o| self.folder_role(o.msg.folder_id) == Role::Junk);
        let (html, plain) = (ui.get_r_html(), ui.get_r_plain());
        let original = ui.get_r_original();
        let menu = triage::popout_menu(
            junk,
            html,
            plain,
            original,
            ui.get_r_dimmed(),
            ui.get_r_remote_blocked() > 0,
            !p.gone,
        );
        let model = |v: Vec<MenuCommand>| ModelRc::new(VecModel::from(v));
        let [a, b, c] = menu;
        ui.set_more_menu_1(model(a));
        ui.set_more_menu_2(model(b));
        ui.set_more_menu_3(model(c));
        ui.set_more_menu_4(ModelRc::default());
        ui.set_more_menu_5(ModelRc::default());
    }

    /// The Move list's filter in window `wid`.
    fn popout_move_filter(&mut self, wid: u32, text: &str) {
        if let Some(p) = self.popouts.get_mut(wid) {
            p.move_filter = text.to_string();
        }
        self.popout_fill(wid);
    }

    /// Load window `wid`'s remote images, this once or always for its
    /// sender.
    fn popout_load_remote(&mut self, wid: u32, always: bool) {
        let Some(p) = self.popouts.get(wid) else {
            return;
        };
        let Some(addr) = p.open.as_ref().map(|o| o.msg.from_addr.clone()) else {
            return;
        };
        if always {
            if let Err(e) = self.db.allow_remote(&addr) {
                return self.set_status(&text::detail(&e), true);
            }
            self.show_remote_senders();
            p.ui.set_r_remote_always(addr.as_str().into());
        }
        p.reader.allow_remote();
    }

    /// Undo on the window's "will always load" bar.
    fn popout_remote_undo(&mut self, wid: u32) {
        let Some(p) = self.popouts.get(wid) else {
            return;
        };
        let addr = p.ui.get_r_remote_always().to_string();
        p.ui.set_r_remote_always(SharedString::new());
        if addr.is_empty() {
            return;
        }
        if let Err(e) = self.db.forget_remote(&addr) {
            return self.set_status(&text::detail(&e), true);
        }
        self.show_remote_senders();
        p.ui.set_r_remote_allowed(false);
    }

    /// An attachment of window `wid`'s message (its name and bytes).
    fn popout_attachment(&self, wid: u32, index: i32) -> Option<(String, Vec<u8>)> {
        let raw = self.popouts.get(wid)?.open.as_ref()?.msg.raw.as_deref()?;
        mime::attachment(raw, usize::try_from(index).ok()?)
    }

    /// Unsubscribe from window `wid`'s list (the composer tab or the link
    /// prompt, as in the main window).
    fn popout_unsubscribe(&mut self, wid: u32) {
        let Some(p) = self.popouts.get(wid) else {
            return;
        };
        let uri = p
            .open
            .as_ref()
            .and_then(|o| o.body.as_ref())
            .map(|b| b.unsubscribe.clone())
            .unwrap_or_default();
        if !uri.is_empty() {
            p.reader.follow(uri);
        }
    }

    /// Retry on a body that could not be downloaded.
    fn popout_retry(&mut self, wid: u32) {
        let Some(open) = self.popouts.get(wid).and_then(|p| p.open.as_ref()) else {
            return;
        };
        if let Some(w) = self.workers.get(&open.msg.account_id) {
            w.send(Job::FetchBody(open.msg.id));
        }
    }
}

/// Window `wid`'s callbacks, each on the app state (never held across a
/// native dialog: Save attachment takes what it needs first).
fn wire_popout(ui: &AppWindow, wid: u32) {
    ui.on_reply(move || {
        with_app(|a| a.popout_respond(wid, 0));
    });
    ui.on_reply_all(move || {
        with_app(|a| a.popout_respond(wid, 1));
    });
    ui.on_forward(move || {
        with_app(|a| a.popout_respond(wid, 2));
    });
    ui.on_menu_respond(move |kind| {
        with_app(|a| a.popout_respond(wid, kind));
    });
    ui.on_forward_attached(move || {
        with_app(|a| a.popout_respond(wid, 3));
    });
    ui.on_archive(move || {
        with_app(|a| a.popout_remove(wid, Dest::Archive));
    });
    ui.on_delete(move || {
        with_app(|a| a.popout_delete(wid));
    });
    ui.on_junk(move |report| {
        with_app(|a| a.popout_remove(wid, if report { Dest::Junk } else { Dest::NotJunk }));
    });
    ui.on_move_to(move |id| {
        with_app(|a| a.popout_remove(wid, Dest::Folder(i64::from(id))));
    });
    ui.on_move_filter_edited(move |text| {
        with_app(|a| a.popout_move_filter(wid, &text));
    });
    ui.on_mark_read(move |on| {
        with_app(|a| a.popout_flag(wid, Flag::Seen, Some(on)));
    });
    ui.on_toggle_read(move || {
        with_app(|a| a.popout_flag(wid, Flag::Seen, None));
    });
    ui.on_mark_flagged(move |on| {
        with_app(|a| a.popout_flag(wid, Flag::Flagged, Some(on)));
    });
    ui.on_toggle_flag(move || {
        with_app(|a| a.popout_flag(wid, Flag::Flagged, None));
    });
    ui.on_menu_opening(move || {
        with_app(|a| a.popout_menu(wid));
    });
    ui.on_r_load_remote(move |always| {
        with_app(|a| a.popout_load_remote(wid, always));
    });
    ui.on_r_remote_undo(move || {
        with_app(|a| a.popout_remote_undo(wid));
    });
    ui.on_forget_remote(|address| {
        with_app(|a| {
            if let Err(e) = a.db.forget_remote(&address) {
                a.set_status(&text::detail(&e), true);
            }
            a.show_remote_senders();
        });
    });
    ui.on_unsubscribe(move || {
        with_app(|a| a.popout_unsubscribe(wid));
    });
    ui.on_r_retry_body(move || {
        with_app(|a| a.popout_retry(wid));
    });
    ui.on_open_attachment(move |index| {
        with_app(|a| {
            let found = a.popout_attachment(wid, index);
            a.open_attachment_file(found);
        });
    });
    ui.on_save_attachment(move |index| {
        let found = with_app(|a| a.popout_attachment(wid, index)).flatten();
        if let Some((name, data)) = found {
            save_file(&t!(invoke_dlg_save_attachment), &name, data);
        }
    });
    // New mail (Ctrl+N): a composer tab in the main window.
    ui.on_compose(|| {
        with_app(|a| a.in_main_tab(App::compose_new));
    });
    // Esc, Ctrl+W, and the caption's close.
    ui.on_close_window(move || {
        with_app(|a| a.close_popout(wid));
    });
    ui.on_tab_key(move |key| {
        if key == 5 {
            with_app(|a| a.close_popout(wid));
        }
    });
    ui.window().on_close_requested(move || {
        // After this event: the window's own callback is running.
        Timer::single_shot(Duration::ZERO, move || {
            with_app(|a| a.close_popout(wid));
        });
        slint::CloseRequestResponse::HideWindow
    });
}

/// A composer window: one draft, with its own view state (the draft shown,
/// its suggestions, body pane and preview sequence), swapped in while an
/// event of this window runs ([`App::in_composer`]), so typing here never
/// moves the main window's draft.
pub(super) struct ComposerWin {
    pub(super) draft: DraftId,
    pub(super) ui: AppWindow,
    view: DraftView,
}

/// What a composer window holds while it is not in view.
#[derive(Default)]
struct DraftView {
    active: Option<DraftId>,
    suggest: Option<composer::Suggest>,
    pane: composer::rich::Pane,
    preview_seq: u64,
}

impl App {
    /// The composer window holding draft `id`.
    pub(super) fn composer_window(&self, id: DraftId) -> Option<AppWindow> {
        self.cwins
            .iter()
            .find(|w| w.draft == id)
            .map(|w| w.ui.clone_strong())
    }

    /// The drafts held by composer windows.
    pub(super) fn windowed_drafts(&self) -> Vec<DraftId> {
        self.cwins.iter().map(|w| w.draft).collect()
    }

    fn swap_draft_view(&mut self, view: &mut DraftView) {
        self.drafts.swap_view(&mut view.active, &mut view.suggest);
        self.swap_body_pane(&mut view.pane);
        std::mem::swap(&mut self.preview_seq, &mut view.preview_seq);
    }

    /// Run `f` with composer window `id`'s view in place of the main
    /// window's: the active draft is `id`, and the composer view, the body
    /// editor and the preview draw into that window. None when no window
    /// holds `id` or the draft is gone (sent: a second Send does nothing).
    pub(super) fn in_composer<R>(
        &mut self,
        id: DraftId,
        f: impl FnOnce(&mut App) -> R,
    ) -> Option<R> {
        if !self.drafts.contains(id) {
            return None;
        }
        if self.scoped == Some(id) {
            return Some(f(self));
        }
        let i = self.cwins.iter().position(|w| w.draft == id)?;
        let mut view = std::mem::take(&mut self.cwins[i].view);
        view.active = Some(id);
        let outer = self.scoped.replace(id);
        self.swap_draft_view(&mut view);
        let r = f(self);
        self.swap_draft_view(&mut view);
        self.scoped = outer;
        if let Some(w) = self.cwins.iter_mut().find(|w| w.draft == id) {
            w.view = view;
        }
        Some(r)
    }

    /// `f` in draft `id`'s composer window when one holds it, otherwise on
    /// the main window's view.
    pub(super) fn in_draft_view<R: Default>(
        &mut self,
        id: Option<DraftId>,
        f: impl FnOnce(&mut App) -> R,
    ) -> R {
        match id {
            Some(id) if self.scoped != Some(id) && self.cwins.iter().any(|w| w.draft == id) => {
                self.in_composer(id, f).unwrap_or_default()
            }
            _ => f(self),
        }
    }

    /// Draft `id` in a composer window of its own ("Open in new window",
    /// Ctrl+O on a composer tab or the reading pane's draft). It leaves the
    /// main window: its tab is dropped, or the reading pane's composer
    /// closes. A window that has it already comes forward.
    pub(super) fn open_draft_window(&mut self, id: DraftId) {
        if let Some(ui) = self.composer_window(id) {
            crate::bring_to_front(&ui);
            return;
        }
        if !self.drafts.contains(id) {
            return;
        }
        let ui = match AppWindow::new() {
            Ok(ui) => ui,
            Err(e) => {
                return self.set_status(&text::detail(&anyhow::anyhow!("{e}")), true);
            }
        };
        // Saved as it stands before it moves.
        self.save_draft(id);
        if let Some(i) = self.tabs.find(tabs::TabKind::Draft(id)) {
            if i == self.tabs.active() {
                self.drop_active_tab();
            } else {
                self.tabs.remove(i);
                self.show_tabs();
            }
        } else if self.drafts.active_id() == Some(id) {
            let main = self.ui();
            main.set_composing(false);
            main.set_c_suggestions(ModelRc::default());
            // The pane says where the draft went.
            self.popouts.popped = Some(id);
            main.set_draft_popped(true);
        }
        self.drafts.set_tabbed(id, false);
        if self.drafts.active_id() == Some(id) {
            self.drafts.hide();
        }
        self.dress_window(&ui);
        ui.set_tab_kind(2);
        ui.set_c_from_options(self.ui().get_c_from_options());
        wire_composer(&ui, id);
        composer::rich::wire_for(&ui, Some(id));
        self.place_window(&ui);
        crate::watch_composer_window(
            &ui,
            move |w, h| {
                with_app(|a| a.popouts.size = Some((w, h)));
            },
            move |drag| composer_file(id, drag),
        );
        self.cwins.push(ComposerWin {
            draft: id,
            ui: ui.clone_strong(),
            view: DraftView::default(),
        });
        self.in_composer(id, |a| {
            a.ed_reset(composer::rich::BODY);
            a.show_draft();
        });
        ui.show().ok();
    }

    /// Saved draft `row` in a composer window (Undo send from one).
    pub(super) fn open_saved_in_window(&mut self, row: i64) {
        if let Some(id) = self.drafts.find_row(row) {
            return self.open_draft_window(id);
        }
        match composer::load(&self.db, row) {
            Ok(Some((mut draft, _))) => {
                self.prepare_draft(&mut draft);
                let id = self.drafts.add(draft);
                self.open_draft_window(id);
            }
            Ok(None) => self.set_status(&t!(invoke_draft_gone), true),
            Err(e) => self.set_status(
                &t!(invoke_could_not_open_draft, format!("{e:#}").into()),
                true,
            ),
        }
    }

    /// Composer window of draft `id` goes (closed, sent, discarded): from
    /// the screen now, its window once the event that closed it is over.
    pub(super) fn drop_composer_window(&mut self, id: DraftId) {
        let Some(i) = self.cwins.iter().position(|w| w.draft == id) else {
            return;
        };
        let w = self.cwins.remove(i);
        w.ui.hide().ok();
        if self.popouts.popped == Some(id) {
            self.clear_popped();
        }
        Timer::single_shot(Duration::ZERO, move || drop(w));
    }

    /// The main window's pane no longer stands for a popped-out draft.
    pub(super) fn clear_popped(&mut self) {
        if self.popouts.popped.take().is_some() {
            self.ui().set_draft_popped(false);
        }
    }

    /// "Bring to front" in the main window's pane: the composer window of
    /// the draft that left it.
    pub(super) fn bring_popped(&mut self) {
        match self.popouts.popped {
            Some(id) if self.cwins.iter().any(|w| w.draft == id) => self.open_draft_window(id),
            _ => self.clear_popped(),
        }
    }

    /// Every composer window goes (the main window closes or the app
    /// quits): each draft is saved first, as a close of its window saves.
    pub(super) fn close_composer_windows(&mut self) {
        for id in self.windowed_drafts() {
            self.in_composer(id, |a| a.save_draft(id));
        }
        for w in std::mem::take(&mut self.cwins) {
            w.ui.hide().ok();
        }
        self.clear_popped();
    }

    /// Close (Esc, Ctrl+W, the caption's close) on composer window `id`:
    /// the draft is saved as Esc saves it.
    pub(super) fn close_composer_window(&mut self, id: DraftId) {
        if self.in_composer(id, App::close_composer).is_none() {
            // Its draft went already: only the window is left.
            self.drop_composer_window(id);
        }
    }
}

/// A file dragged over composer window `id`, or dropped on it: it goes in
/// that window's draft.
fn composer_file(id: DraftId, drag: crate::FileDrag) {
    with_app(|a| {
        let hover = move |a: &App, on: bool| {
            if let Some(ui) = a.composer_window(id) {
                ui.set_file_hover(on);
            }
        };
        match drag {
            crate::FileDrag::Hover => hover(a, true),
            crate::FileDrag::Left => hover(a, false),
            crate::FileDrag::Dropped(path) => {
                if a.popouts.dropped.is_empty() {
                    Timer::single_shot(Duration::ZERO, move || {
                        with_app(|a| {
                            let paths = std::mem::take(&mut a.popouts.dropped);
                            if a.in_composer(id, |a| a.attach_dropped(paths)).is_none() {
                                // Its draft went meanwhile (sent).
                                hover(a, false);
                            }
                        });
                    });
                }
                a.popouts.dropped.push(path);
            }
        }
    });
}

/// Composer window `id`'s callbacks: each runs with its draft in view.
fn wire_composer(ui: &AppWindow, id: DraftId) {
    composer::recipients::wire(ui);
    fn run<R: Default>(id: DraftId, f: impl FnOnce(&mut App) -> R) -> R {
        with_app(|a| a.in_composer(id, f).unwrap_or_default()).unwrap_or_default()
    }
    ui.on_send(move || run(id, App::send));
    ui.on_c_send_anyway(move || run(id, App::send_anyway));
    ui.on_discard(move || run(id, App::discard));
    ui.on_discard_confirmed(move || run(id, App::discard_confirmed));
    ui.on_attach(move || composer::attach_files_to(id));
    ui.on_c_paste_files(move || run(id, App::paste_files));
    ui.on_remove_attachment(move |i| run(id, |a| a.remove_attachment(i)));
    ui.on_c_edited(move |field, text| run(id, |a| a.draft_edited(field, &text)));
    ui.on_c_left(move |field| run(id, |a| a.draft_left(field)));
    ui.on_c_suggest_accept(move |index| run(id, |a| a.accept_suggestion(index)));
    ui.on_c_suggest_close(move || run(id, App::close_suggestions));
    ui.on_c_from_changed(move |index| run(id, |a| a.draft_account(index)));
    ui.on_c_reveal(move |field| run(id, |a| a.reveal_cc(field)));
    ui.on_c_set_importance(move |code| run(id, |a| a.draft_importance(code)));
    ui.on_c_close(move || {
        with_app(|a| a.close_composer_window(id));
    });
    ui.on_c_unfold_quote(move || run(id, App::unfold_quote));
    ui.on_c_preview_render(move |width, dark, first| {
        run(id, |a| a.render_preview(width, dark, first));
    });
    ui.on_c_format(move |action, anchor, cursor| {
        run(id, |a| a.format_body(action, anchor, cursor))
    });
    ui.on_c_link(move |url, anchor, cursor| run(id, |a| a.link_body(&url, anchor, cursor)));
    ui.on_c_unformat(move || run(id, App::unformat_body));
    let rich = ui.global::<crate::Rich>();
    rich.on_pick_signature(move |i| run(id, |a| a.pick_signature(i)));
    rich.on_set_rich(move |on| run(id, |a| a.set_rich(on)));
    // New mail (Ctrl+N): a composer tab in the main window.
    ui.on_compose(|| {
        with_app(|a| a.in_main_tab(App::compose_new));
    });
    ui.on_close_window(move || {
        with_app(|a| a.close_composer_window(id));
    });
    ui.on_tab_key(move |key| {
        if key == 5 {
            with_app(|a| a.close_composer_window(id));
        }
    });
    ui.window().on_close_requested(move || {
        Timer::single_shot(Duration::ZERO, move || {
            with_app(|a| a.close_composer_window(id));
        });
        slint::CloseRequestResponse::HideWindow
    });
}
