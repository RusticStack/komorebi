//! Draft autosave: debounced flush, restore, discard.

use super::*;

impl App {
    pub(crate) fn resume_latest_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(drafts) = self.db.list_drafts() else {
            return;
        };
        let Some(draft) = drafts.into_iter().max_by_key(|d| d.updated_at) else {
            return;
        };
        if draft.to_addresses.is_none() && draft.subject.is_none() && draft.body.is_none() {
            return;
        }
        self.draft_id = Some(draft.id);
        if let Some(to) = draft.to_addresses {
            self.to_input
                .update(cx, |s, cx| s.set_value(to, window, cx));
        }
        if let Some(subject) = draft.subject {
            self.subject_input
                .update(cx, |s, cx| s.set_value(subject, window, cx));
        }
        if let Some(body) = draft.body {
            self.body_input
                .update(cx, |s, cx| s.set_value(body, window, cx));
        }
    }

    /// Schedule a debounced autosave after a compose-field edit.
    pub(crate) fn queue_draft_autosave(&mut self, cx: &mut Context<Self>) {
        if self.panel != Panel::Compose {
            return;
        }
        self.draft_dirty = true;
        let entity = cx.entity();
        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = entity.update(cx, |this, cx| this.flush_draft(cx));
        })
        .detach();
    }

    /// Write the current compose fields to the drafts table (no-op when
    /// nothing changed or everything is empty).
    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if !self.draft_dirty || self.panel != Panel::Compose {
            return;
        }
        let to = self.to_input.read(cx).value().trim().to_string();
        let subject = self.subject_input.read(cx).value().trim().to_string();
        let body = self.body_input.read(cx).value().to_string();
        if to.is_empty() && subject.is_empty() && body.trim().is_empty() {
            return;
        }
        let account_id = self
            .accounts
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let result = match self.draft_id.clone() {
            Some(id) => self
                .db
                .update_draft(&id, Some(&subject), Some(&to), Some(&body))
                .map(|_| id),
            None => self
                .db
                .save_draft(&account_id, Some(&subject), Some(&to), Some(&body)),
        };
        match result {
            Ok(id) => self.draft_id = Some(id),
            Err(e) => log::error!("draft autosave failed: {e}"),
        }
        self.draft_dirty = false;
        cx.notify();
    }

    /// Discard the draft currently being edited.
    pub(crate) fn discard_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.draft_id.take() {
            let _ = self.db.delete_draft(&id);
        }
        self.draft_dirty = false;
        cx.notify();
    }
}
