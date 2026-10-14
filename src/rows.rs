//! The message list as a lazy Slint model.
//!
//! The model holds only the ids of the messages in the current view, newest
//! first. Slint asks for the rows it is about to show (about a screenful,
//! since `ListView` virtualizes), and each one is read from the SQLite cache
//! on demand. A reload costs one indexed id query instead of building every
//! row, and the cache stays the single source of truth: after a change, the
//! row is simply marked changed and read again. The Drafts and Outbox
//! views list saved drafts and outbox rows the same way, by draft id.
//!
//! The rows selected for a batch action (Ctrl+click, Shift+click, Ctrl+A)
//! are a set of ids beside the list, so selecting a whole folder still
//! reads only the rows in view.

use crate::db::{Db, Outbox};
use crate::tr::t;
use crate::{MailRow, format, mail};
use chrono::{DateTime, Local};
use komorebi::i18n::Lang;
use slint::{Model, ModelNotify, ModelTracker, SharedString};
use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the ids are and how their rows read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Messages, by sender.
    Received,
    /// Messages in Sent or Drafts, by recipient.
    Outgoing,
    /// Messages from several folders (a search of all folders): each
    /// reads as its folder's role says, tagged with the folder's name.
    Mixed,
    /// Local drafts (draft ids), by recipient.
    Drafts,
    /// The outbox (draft ids), by recipient, with where each send is.
    Outbox,
}

/// Which rows are selected, by id. A Shift range runs from `anchor` (the
/// last row clicked or Ctrl+clicked) to `cursor` (the row it was extended
/// to, which Shift+Up/Down move). Each change takes the list's ids in order.
#[derive(Default, Debug)]
pub struct Selection {
    ids: HashSet<i64>,
    anchor: Option<i64>,
    cursor: Option<i64>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.ids.contains(&id)
    }

    pub fn ids(&self) -> Vec<i64> {
        self.ids.iter().copied().collect()
    }

    /// The row Shift+Up/Down move from.
    pub fn cursor(&self) -> Option<i64> {
        self.cursor
    }

    /// Just `id` (a plain click, or the message opened).
    pub fn single(&mut self, id: i64) {
        self.ids = HashSet::from([id]);
        self.anchor = Some(id);
        self.cursor = Some(id);
    }

    /// Ctrl+click: add or remove `id`; ranges start from it.
    pub fn toggle(&mut self, id: i64) {
        if !self.ids.remove(&id) {
            self.ids.insert(id);
        }
        self.anchor = Some(id);
        self.cursor = Some(id);
    }

    /// Shift+click: the rows from the anchor to `id`, inclusive (just `id`
    /// with no anchor in the list).
    pub fn extend_to(&mut self, rows: &[i64], id: i64) {
        let Some(b) = index(rows, id) else {
            return;
        };
        let (anchor, a) = match self.anchor.and_then(|x| Some((x, index(rows, x)?))) {
            Some(found) => found,
            None => (id, b),
        };
        self.ids = rows[a.min(b)..=a.max(b)].iter().copied().collect();
        self.anchor = Some(anchor);
        self.cursor = Some(id);
    }

    /// Shift+Up/Down: move the cursor `delta` rows (stopping at the ends)
    /// and select from the anchor to it. With no cursor, the first row.
    pub fn extend_by(&mut self, rows: &[i64], delta: isize) {
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let target = match self.cursor.and_then(|c| index(rows, c)) {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.extend_to(rows, rows[target]);
    }

    /// Right-click on `id`: the menu acts on the selection if `id` is in
    /// it, else on `id` alone.
    pub fn right_click(&mut self, id: i64) {
        if !self.ids.contains(&id) {
            self.single(id);
        }
    }

    /// Ctrl+A: every row (ids only; nothing is read).
    pub fn all(&mut self, rows: &[i64]) {
        self.ids = rows.iter().copied().collect();
        let listed = |x: &i64| self.ids.contains(x);
        self.anchor = self.anchor.filter(listed).or(rows.first().copied());
        self.cursor = self.cursor.filter(listed).or(self.anchor);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Keep only rows for which `keep` holds (rows that left the list go).
    pub fn retain(&mut self, keep: impl Fn(&i64) -> bool) {
        self.ids.retain(&keep);
        self.anchor = self.anchor.filter(&keep);
        self.cursor = self.cursor.filter(&keep);
    }
}

fn index(rows: &[i64], id: i64) -> Option<usize> {
    rows.iter().position(|&x| x == id)
}

pub struct Rows {
    db: Rc<Db>,
    ids: RefCell<Vec<i64>>,
    selection: RefCell<Selection>,
    kind: Cell<Kind>,
    /// What [`Kind::Mixed`] rows read by folder id: the folder's tag and
    /// whether its mail is outgoing.
    folders: RefCell<HashMap<i64, (SharedString, bool)>>,
    /// Account names by account id, tagging each row (empty: no tags).
    accounts: RefCell<HashMap<i64, SharedString>>,
    /// Reference time for relative dates, fixed per reload.
    now: Cell<DateTime<Local>>,
    /// The language dates are written in.
    lang: Cell<Lang>,
    notify: ModelNotify,
}

impl Rows {
    pub fn new(db: Rc<Db>) -> Self {
        Self {
            db,
            ids: RefCell::default(),
            selection: RefCell::default(),
            kind: Cell::new(Kind::Received),
            folders: RefCell::default(),
            accounts: RefCell::default(),
            now: Cell::new(Local::now()),
            lang: Cell::new(Lang::default()),
            notify: ModelNotify::default(),
        }
    }

    /// Write dates in `lang` from now on (rows are drawn again by [`Rows::redraw`]).
    pub fn set_lang(&self, lang: Lang) {
        self.lang.set(lang);
    }

    /// Show `ids`. When the list is unchanged (a sync touched flags or
    /// previews), rows refresh in place so the scroll position stays put.
    pub fn set(&self, ids: Vec<i64>, kind: Kind) {
        let same_kind = self.kind.replace(kind) == kind;
        self.now.set(Local::now());
        let same = same_kind && *self.ids.borrow() == ids;
        let len = ids.len();
        if self.selection.borrow().len() > 0 {
            let listed: HashSet<i64> = ids.iter().copied().collect();
            self.selection.borrow_mut().retain(|id| listed.contains(id));
        }
        *self.ids.borrow_mut() = ids;
        if same {
            for i in 0..len {
                self.notify.row_changed(i);
            }
        } else {
            self.notify.reset();
        }
    }

    /// The folder tags and roles of [`Kind::Mixed`] rows (set before
    /// [`Rows::set`]).
    pub fn set_folders(&self, folders: HashMap<i64, (SharedString, bool)>) {
        *self.folders.borrow_mut() = folders;
    }

    /// The account tags of message rows, by account id (set before
    /// [`Rows::set`]; empty: none).
    pub fn set_account_tags(&self, tags: HashMap<i64, SharedString>) {
        *self.accounts.borrow_mut() = tags;
    }

    pub fn len(&self) -> usize {
        self.ids.borrow().len()
    }

    pub fn id_at(&self, index: usize) -> Option<i64> {
        self.ids.borrow().get(index).copied()
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.ids.borrow().iter().position(|&x| x == id)
    }

    /// Re-read every row from the cache (the language changed: dates and
    /// labels read differently).
    pub fn redraw(&self) {
        for i in 0..self.len() {
            self.notify.row_changed(i);
        }
    }

    /// Re-read one row from the cache.
    pub fn changed(&self, id: i64) {
        if let Some(i) = self.index_of(id) {
            self.notify.row_changed(i);
        }
    }

    /// Re-read these rows from the cache.
    pub fn changed_many(&self, ids: &HashSet<i64>) {
        let rows = self.ids.borrow();
        for (i, id) in rows.iter().enumerate() {
            if ids.contains(id) {
                self.notify.row_changed(i);
            }
        }
    }

    /// Drop these rows (and their selection); returns the smallest former
    /// index among them, where the next row now is.
    pub fn remove_many(&self, ids: &HashSet<i64>) -> Option<usize> {
        let mut rows = self.ids.borrow_mut();
        let gone: Vec<usize> = (0..rows.len())
            .filter(|&i| ids.contains(&rows[i]))
            .collect();
        let first = *gone.first()?;
        rows.retain(|id| !ids.contains(id));
        drop(rows);
        self.selection.borrow_mut().retain(|id| !ids.contains(id));
        // Back to front, a run of neighbours at a time, so the indices
        // still hold and the view keeps its scroll position.
        let mut end = gone.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && gone[start - 1] + 1 == gone[start] {
                start -= 1;
            }
            self.notify.row_removed(gone[start], end - start);
            end = start;
        }
        Some(first)
    }

    /// The row to show after rows were removed from `index`: the one that
    /// took its place, or the new last row.
    pub fn neighbour(&self, index: usize) -> Option<i64> {
        self.id_at(index)
            .or_else(|| self.id_at(self.len().checked_sub(1)?))
    }

    pub fn selection(&self) -> Ref<'_, Selection> {
        self.selection.borrow()
    }

    /// Change the selection (given the list's ids) and redraw the rows
    /// whose state changed.
    pub fn select(&self, f: impl FnOnce(&mut Selection, &[i64])) {
        let rows = self.ids.borrow();
        let before = self.selection.borrow().ids.clone();
        f(&mut self.selection.borrow_mut(), &rows);
        let sel = self.selection.borrow();
        if before == sel.ids {
            return;
        }
        for (i, id) in rows.iter().enumerate() {
            if before.contains(id) != sel.ids.contains(id) {
                self.notify.row_changed(i);
            }
        }
    }
}

impl Model for Rows {
    type Data = MailRow;

    fn row_count(&self) -> usize {
        self.len()
    }

    fn row_data(&self, row: usize) -> Option<MailRow> {
        let id = self.id_at(row)?;
        if self.kind.get() == Kind::Drafts {
            let d = self.db.draft_header(id).ok()??;
            return Some(MailRow {
                id: id as i32,
                sender: if d.to.trim().is_empty() {
                    t!(invoke_row_no_recipient).into()
                } else {
                    t!(invoke_row_to, format::recipients(&d.to).into()).into()
                },
                subject: d.subject.into(),
                // The text above the quote, without the marks the HTML
                // part converts.
                preview: mail::mime::preview(&mail::compose_html::plain(crate::app::above_quote(
                    &d.preview,
                )))
                .into(),
                date: format::list_date(self.lang.get(), d.updated, self.now.get()).into(),
                attach: d.has_attach,
                ..Default::default()
            });
        }
        if self.kind.get() == Kind::Outbox {
            let e = self.db.outbox_entry(id).ok()??;
            let to = format::recipients(&e.to);
            return Some(MailRow {
                id: id as i32,
                sender: t!(invoke_row_to, to.into()).into(),
                subject: e.subject.as_str().into(),
                preview: format::outbox_status(&e, self.now.get().timestamp()).into(),
                date: format::list_date(self.lang.get(), e.updated, self.now.get()).into(),
                attach: e.has_attach,
                problem: matches!(e.state, Outbox::Failed | Outbox::Unconfirmed),
                ..Default::default()
            });
        }
        let h = self.db.header(id).ok()??;
        let (mut tag, outgoing) = match self.kind.get() {
            Kind::Mixed => self
                .folders
                .borrow()
                .get(&h.folder_id)
                .cloned()
                .unwrap_or_default(),
            kind => (SharedString::new(), kind == Kind::Outgoing),
        };
        let sender = if outgoing {
            let to = mail::smtp::split_addresses(&h.to);
            t!(invoke_row_to, to.first().map_or("", String::as_str).into()).into()
        } else {
            format::sender(&h.from_name, &h.from_addr).into()
        };
        Some(MailRow {
            id: id as i32,
            sender,
            subject: h.subject.into(),
            preview: h.preview.into(),
            date: format::list_date(self.lang.get(), h.date, self.now.get()).into(),
            // Sent mail and server drafts are the user's own: never bold.
            unread: !h.seen && !outgoing,
            flagged: h.flagged,
            attach: h.has_attach,
            answered: h.answered,
            draft: h.has_draft,
            selected: self.selection.borrow().contains(id),
            tag: {
                if tag.is_empty()
                    && let Some(name) = self.accounts.borrow().get(&h.account_id)
                {
                    tag = name.clone();
                }
                tag
            },
            problem: false,
            high_importance: h.importance == crate::db::Importance::High,
            low_importance: h.importance == crate::db::Importance::Low,
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [i64; 6] = [10, 11, 12, 13, 14, 15];

    /// A list over an empty in-memory cache (no row is read).
    fn rows() -> Rows {
        let path = format!(
            "file:komorebi-rows-{:?}?mode=memory",
            std::thread::current().id()
        );
        Rows::new(Rc::new(Db::open(std::path::Path::new(&path)).unwrap()))
    }

    fn sorted(s: &Selection) -> Vec<i64> {
        let mut ids = s.ids();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn right_click_keeps_a_selection_holding_the_row() {
        let mut s = Selection::default();
        s.all(&ROWS[..3]);
        s.right_click(11);
        assert_eq!(sorted(&s), [10, 11, 12]);
        // Outside the selection: just that row.
        s.right_click(14);
        assert_eq!(sorted(&s), [14]);
        assert_eq!(s.cursor(), Some(14));
        s.clear();
        s.right_click(13);
        assert_eq!(sorted(&s), [13]);
    }

    #[test]
    fn toggle_range_all_and_clear() {
        let mut s = Selection::default();
        s.single(11);
        s.toggle(13);
        assert_eq!(sorted(&s), [11, 13]);
        s.toggle(11);
        assert_eq!(sorted(&s), [13]);
        // A range runs from the last row (Ctrl+)clicked, even one just
        // unselected (as in Explorer), either way.
        s.extend_to(&ROWS, 13);
        assert_eq!(sorted(&s), [11, 12, 13]);
        s.extend_to(&ROWS, 10);
        assert_eq!(sorted(&s), [10, 11]);
        // With no anchor, Shift+click selects just that row.
        s.clear();
        assert_eq!(s.len(), 0);
        s.extend_to(&ROWS, 12);
        assert_eq!(sorted(&s), [12]);
        // An id that isn't listed changes nothing.
        s.extend_to(&ROWS, 99);
        assert_eq!(sorted(&s), [12]);
        s.all(&ROWS);
        assert_eq!(sorted(&s), ROWS);
        assert_eq!(s.cursor(), Some(12));
        s.clear();
        s.all(&ROWS);
        assert_eq!(s.cursor(), Some(10));
        s.all(&[]);
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn shift_arrows_extend_from_the_anchor() {
        let mut s = Selection::default();
        s.extend_by(&ROWS, 1);
        assert_eq!(sorted(&s), [10]);
        s.single(12);
        s.extend_by(&ROWS, 1);
        s.extend_by(&ROWS, 1);
        assert_eq!(sorted(&s), [12, 13, 14]);
        // Back past the anchor flips the range.
        for _ in 0..3 {
            s.extend_by(&ROWS, -1);
        }
        assert_eq!(sorted(&s), [11, 12]);
        // The ends hold.
        s.extend_by(&ROWS, -5);
        assert_eq!(sorted(&s), [10, 11, 12]);
        s.extend_by(&ROWS, 50);
        assert_eq!(sorted(&s), [12, 13, 14, 15]);
        assert_eq!(s.cursor(), Some(15));
        s.extend_by(&[], 1);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn rows_that_leave_the_list_leave_the_selection() {
        let mut s = Selection::default();
        s.single(11);
        s.extend_to(&ROWS, 14);
        let listed = HashSet::from([10, 12, 13]);
        s.retain(|id| listed.contains(id));
        assert_eq!(sorted(&s), [12, 13]);
        assert_eq!(s.cursor(), None);
        // A range then starts at the row clicked.
        s.extend_to(&ROWS, 15);
        assert_eq!(sorted(&s), [15]);
    }

    #[test]
    fn removal_selects_the_neighbour() {
        let rows = rows();
        rows.set(ROWS.to_vec(), Kind::Received);
        rows.select(|s, ids| {
            s.single(11);
            s.extend_to(ids, 12);
            s.toggle(14);
        });
        assert_eq!(rows.selection().len(), 3);
        assert_eq!(rows.remove_many(&HashSet::from([11, 12, 14])), Some(1));
        let left: Vec<_> = (0..rows.len()).filter_map(|i| rows.id_at(i)).collect();
        assert_eq!(left, [10, 13, 15]);
        assert_eq!(rows.selection().len(), 0);
        // The row after the first removed one takes its place...
        assert_eq!(rows.neighbour(1), Some(13));
        // ...or, past the end, the new last row.
        assert_eq!(rows.remove_many(&HashSet::from([13, 15])), Some(1));
        assert_eq!(rows.neighbour(1), Some(10));
        assert_eq!(rows.remove_many(&HashSet::from([99])), None);
        assert_eq!(rows.remove_many(&HashSet::from([10])), Some(0));
        assert_eq!(rows.neighbour(0), None);
    }

    #[test]
    fn a_reload_keeps_only_listed_selected_rows() {
        let rows = rows();
        rows.set(ROWS.to_vec(), Kind::Received);
        rows.select(|s, ids| s.all(ids));
        rows.set(vec![10, 12, 16], Kind::Received);
        let mut kept = rows.selection().ids();
        kept.sort_unstable();
        assert_eq!(kept, [10, 12]);
    }
}
