//! The open message as the server sent it: Save as .eml, Show original,
//! Copy Message-ID and Forward as attachment. A message whose body isn't
//! downloaded yet is fetched on the interactive worker first, and the
//! action runs when it arrives.

use super::*;
use crate::mail::source;

/// An action that needs the open message's raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RawUse {
    SaveEml,
    ShowOriginal,
    /// Forward as attachment.
    Forward,
}

impl App {
    /// Run `action` on the open message (from the row menu, on the one
    /// selected row, which opens first), once its body is here.
    pub(super) fn source_action(&mut self, action: RawUse) {
        if !self.reading() || !self.open_selected() {
            return;
        }
        let Some(open) = &self.open else { return };
        if open.msg.raw.is_some() {
            return self.run_raw(action);
        }
        let (id, account) = (open.msg.id, open.msg.account_id);
        let Some(w) = self.workers.get(&account) else {
            return self.set_status("This message isn't downloaded yet.", true);
        };
        // Asked again: a first download may have failed.
        w.send(Job::FetchBody(id));
        self.raw_wait = Some((id, action));
        self.set_status("Fetching message…", false);
    }

    /// The body of message `id` arrived: run the action waiting for it, if
    /// the message is still the open one.
    pub(super) fn raw_arrived(&mut self, id: i64) {
        let Some((wait, action)) = self.raw_wait else {
            return;
        };
        if wait != id {
            return;
        }
        self.raw_wait = None;
        if self.open.as_ref().is_none_or(|o| o.msg.id != id) {
            return;
        }
        if self.open.as_ref().is_some_and(|o| o.msg.raw.is_none()) {
            let raw = self.db.message(id).ok().flatten().and_then(|m| m.raw);
            if let Some(open) = self.open.as_mut() {
                open.msg.raw = raw;
            }
        }
        self.set_status("", false);
        self.run_raw(action);
    }

    /// The body of message `id` couldn't be downloaded.
    pub(super) fn raw_failed(&mut self, id: i64, error: &str) {
        if self.raw_wait.is_some_and(|(wait, _)| wait == id) {
            self.raw_wait = None;
            self.set_status(&format!("Couldn't download this message: {error}"), true);
        }
    }

    fn run_raw(&mut self, action: RawUse) {
        match action {
            // The save dialog runs its own message loop: never inside
            // `with_app`, so it waits for this call to return.
            RawUse::SaveEml => Timer::single_shot(Duration::ZERO, save_eml),
            RawUse::ShowOriginal => self.show_original(),
            RawUse::Forward => self.forward_attached(),
        }
    }

    /// The open message's file name and exact bytes, for Save as .eml.
    pub(super) fn eml_export(&self) -> Option<(String, Vec<u8>)> {
        let m = &self.open.as_ref()?.msg;
        Some((source::eml_name(&m.subject, m.date), m.raw.clone()?))
    }

    /// The open message's Message-ID with its angle brackets (empty: none).
    fn open_message_id(&self) -> String {
        self.open
            .as_ref()
            .map(|o| o.msg.msgid.trim().trim_matches(['<', '>']))
            .filter(|id| !id.is_empty())
            .map_or_else(String::new, |id| format!("<{id}>"))
    }

    fn show_original(&mut self) {
        let Some(raw) = self.open.as_ref().and_then(|o| o.msg.raw.as_deref()) else {
            return;
        };
        let original = source::original(raw);
        let ui = self.ui();
        ui.set_src_headers(original.headers.into());
        ui.set_src_body(original.body.into());
        ui.set_src_msgid(self.open_message_id().into());
        ui.set_src_show(true);
    }

    /// Copy the open message's Message-ID (`<id@host>`) to the clipboard.
    pub(super) fn copy_message_id(&mut self) {
        if !self.reading() || !self.open_selected() {
            return;
        }
        let id = self.open_message_id();
        if id.is_empty() {
            return self.set_status("This message has no Message-ID.", false);
        }
        self.ui().invoke_r_copy_text(id.as_str().into());
        self.set_status("Message-ID copied.", false);
    }
}

/// Save as .eml: the open message's bytes, exactly as cached.
fn save_eml() {
    if let Some((name, raw)) = with_app(|a| a.eml_export()).flatten() {
        save_file("Save message", &name, raw);
    }
}
