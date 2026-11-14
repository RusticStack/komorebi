//! The message list as a lazy model, shared by both apps (moved out of the
//! Slint binary for the GPUI app, docs/GPUI-MIGRATION.md phase 2).
//!
//! The model holds only the ids of the messages in the current view, newest
//! first. A view asks for the rows it is about to show (about a screenful,
//! as both apps' lists virtualise), and each one is read from the SQLite
//! cache on demand ([`Rows::row`]). A reload costs one indexed id query
//! instead of building every row, and the cache stays the single source of
//! truth: after a change, the row is simply read again. The Drafts and
//! Outbox views list saved drafts and outbox rows the same way, by draft id.
//!
//! The rows selected for a batch action (Ctrl+click, Shift+click, Ctrl+A)
//! are a set of ids beside the list, so selecting a whole folder still
//! reads only the rows in view.
//!
//! Nothing here knows a toolkit: each change says which rows it touched
//! ([`Refresh`], [`Removed`], the indices [`Rows::select`] returns), and the
//! app's own model (Slint's `ModelNotify`, a GPUI `cx.notify()`) passes it on.

use crate::db::{Db, Importance, Outbox};
use crate::i18n::Lang;
use crate::{format, mail};
use chrono::{DateTime, Local};
use komorebi_i18n::tr;
use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the ids are and how their rows read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// One row as both apps draw it (the Slint app's `MailRow`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    /// The first line: the sender, or "To: ..." for outgoing mail.
    pub sender: String,
    /// The person the row is about, for its spot's initials: the sender,
    /// or the first recipient of outgoing mail (empty: none).
    pub who: String,
    pub subject: String,
    pub preview: String,
    pub date: String,
    pub unread: bool,
    pub flagged: bool,
    pub attach: bool,
    pub answered: bool,
    /// A local draft replies to it.
    pub draft: bool,
    /// In the multi-selection.
    pub selected: bool,
    /// Its text tag on line 3: its account in All inboxes (with more than
    /// one account), "Account · Folder" in a search of all folders.
    pub tag: String,
    /// An Outbox message that failed or may not have gone.
    pub problem: bool,
    /// Its sender set high importance (not the user's Important).
    pub high_importance: bool,
    /// Its sender set low importance.
    pub low_importance: bool,
}

/// Which rows are selected, by id. A Shift range runs from `anchor` (the
/// last row clicked or Ctrl+clicked) to `cursor` (the row it was extended
/// to, which Shift+Up/Down move). Each change takes the list's ids in order.
#[derive(Default, Debug, Clone)]
pub struct Selection {
    ids: HashSet<i64>,
    anchor: Option<i64>,
    cursor: Option<i64>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
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

/// What a [`Rows::set`] did to the rows shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// The same ids: each of these many rows reads again in place, so the
    /// scroll position stays put.
    InPlace(usize),
    /// Another list: start over.
    Reset,
}

/// What a [`Rows::remove_many`] took out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    /// The smallest former index among them, where the next row now is.
    pub first: usize,
    /// The runs of neighbours removed, `(index, count)`, back to front, so
    /// each index still holds when the one before it is applied.
    pub runs: Vec<(usize, usize)>,
}

/// Cuts a draft's saved text above its quote (the composer's rule; the Slint
/// app passes its own, the GPUI app keeps the whole text until the composer
/// moves into the library).
pub type QuoteCut = fn(&str) -> &str;

pub struct Rows {
    db: Rc<Db>,
    ids: RefCell<Vec<i64>>,
    selection: RefCell<Selection>,
    kind: Cell<Kind>,
    /// What [`Kind::Mixed`] rows read by folder id: the folder's tag and
    /// whether its mail is outgoing.
    folders: RefCell<HashMap<i64, (String, bool)>>,
    /// Account names by account id, tagging each row (empty: no tags).
    accounts: RefCell<HashMap<i64, String>>,
    /// Reference time for relative dates, fixed per reload.
    now: Cell<DateTime<Local>>,
    /// The language dates are written in.
    lang: Cell<Lang>,
    above_quote: QuoteCut,
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
            above_quote: |text| text,
        }
    }

    /// Draft rows preview only the text above the quote, cut by `cut`.
    pub fn with_above_quote(mut self, cut: QuoteCut) -> Self {
        self.above_quote = cut;
        self
    }

    pub fn db(&self) -> &Rc<Db> {
        &self.db
    }

    /// Write dates in `lang` from now on (each row reads again when shown).
    pub fn set_lang(&self, lang: Lang) {
        self.lang.set(lang);
    }

    pub fn kind(&self) -> Kind {
        self.kind.get()
    }

    /// Show `ids`. When the list is unchanged (a sync touched flags or
    /// previews), rows refresh in place so the scroll position stays put.
    pub fn set(&self, ids: Vec<i64>, kind: Kind) -> Refresh {
        let same_kind = self.kind.replace(kind) == kind;
        self.now.set(Local::now());
        let same = same_kind && *self.ids.borrow() == ids;
        let len = ids.len();
        if !self.selection.borrow().is_empty() {
            let listed: HashSet<i64> = ids.iter().copied().collect();
            self.selection.borrow_mut().retain(|id| listed.contains(id));
        }
        *self.ids.borrow_mut() = ids;
        if same {
            Refresh::InPlace(len)
        } else {
            Refresh::Reset
        }
    }

    /// The folder tags and roles of [`Kind::Mixed`] rows (set before
    /// [`Rows::set`]).
    pub fn set_folders(&self, folders: HashMap<i64, (String, bool)>) {
        *self.folders.borrow_mut() = folders;
    }

    /// The account tags of message rows, by account id (set before
    /// [`Rows::set`]; empty: none).
    pub fn set_account_tags(&self, tags: HashMap<i64, String>) {
        *self.accounts.borrow_mut() = tags;
    }

    pub fn len(&self) -> usize {
        self.ids.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.borrow().is_empty()
    }

    pub fn id_at(&self, index: usize) -> Option<i64> {
        self.ids.borrow().get(index).copied()
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.ids.borrow().iter().position(|&x| x == id)
    }

    /// The ids in order.
    pub fn ids(&self) -> Ref<'_, Vec<i64>> {
        self.ids.borrow()
    }

    /// The indices of these rows.
    pub fn indices_of(&self, ids: &HashSet<i64>) -> Vec<usize> {
        let rows = self.ids.borrow();
        (0..rows.len())
            .filter(|&i| ids.contains(&rows[i]))
            .collect()
    }

    /// Drop these rows (and their selection).
    pub fn remove_many(&self, ids: &HashSet<i64>) -> Option<Removed> {
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
        let mut runs = Vec::new();
        let mut end = gone.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && gone[start - 1] + 1 == gone[start] {
                start -= 1;
            }
            runs.push((gone[start], end - start));
            end = start;
        }
        Some(Removed { first, runs })
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

    /// Change the selection (given the list's ids); the indices of the rows
    /// whose state changed come back, to draw again.
    pub fn select(&self, f: impl FnOnce(&mut Selection, &[i64])) -> Vec<usize> {
        let rows = self.ids.borrow();
        let before = self.selection.borrow().ids.clone();
        f(&mut self.selection.borrow_mut(), &rows);
        let sel = self.selection.borrow();
        if before == sel.ids {
            return Vec::new();
        }
        rows.iter()
            .enumerate()
            .filter(|(_, id)| before.contains(id) != sel.ids.contains(id))
            .map(|(i, _)| i)
            .collect()
    }

    /// Row `index`, read from the cache now (`None`: past the end, or the
    /// message is gone from the cache).
    pub fn row(&self, index: usize) -> Option<Row> {
        let id = self.id_at(index)?;
        let now = self.now.get();
        let lang = self.lang.get();
        if self.kind.get() == Kind::Drafts {
            let d = self.db.draft_header(id).ok()??;
            let to = format::recipients(&d.to);
            return Some(Row {
                id,
                sender: if d.to.trim().is_empty() {
                    tr!("(no recipient)")
                } else {
                    tr!("To: {0}", to)
                },
                who: to,
                subject: d.subject,
                // The text above the quote, without the marks the HTML
                // part converts.
                preview: mail::mime::preview(&mail::compose_html::plain((self.above_quote)(
                    &d.preview,
                ))),
                date: format::list_date(lang, d.updated, now),
                attach: d.has_attach,
                ..Default::default()
            });
        }
        if self.kind.get() == Kind::Outbox {
            let e = self.db.outbox_entry(id).ok()??;
            let to = format::recipients(&e.to);
            return Some(Row {
                id,
                sender: tr!("To: {0}", to),
                who: to,
                subject: e.subject.clone(),
                preview: format::outbox_status(&e, now.timestamp()),
                date: format::list_date(lang, e.updated, now),
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
            kind => (String::new(), kind == Kind::Outgoing),
        };
        let (sender, who) = if outgoing {
            let to = mail::smtp::split_addresses(&h.to);
            let first = to.first().map_or("", String::as_str);
            let (name, addr) = mail::smtp::mailbox_parts(first);
            (tr!("To: {0}", first), format::sender(&name, addr))
        } else {
            let from = format::sender(&h.from_name, &h.from_addr);
            (from.clone(), from)
        };
        if tag.is_empty()
            && let Some(name) = self.accounts.borrow().get(&h.account_id)
        {
            tag = name.clone();
        }
        Some(Row {
            id,
            sender,
            who,
            subject: h.subject,
            preview: h.preview,
            date: format::list_date(lang, h.date, now),
            // Sent mail and server drafts are the user's own: never bold.
            unread: !h.seen && !outgoing,
            flagged: h.flagged,
            attach: h.has_attach,
            answered: h.answered,
            draft: h.has_draft,
            selected: self.selection.borrow().contains(id),
            tag,
            problem: false,
            high_importance: h.importance == Importance::High,
            low_importance: h.importance == Importance::Low,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [i64; 6] = [10, 11, 12, 13, 14, 15];

    /// A list over an empty in-memory cache (no row is read).
    fn rows() -> Rows {
        Rows::new(Rc::new(Db::in_memory().unwrap()))
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
        let removed = rows.remove_many(&HashSet::from([11, 12, 14])).unwrap();
        assert_eq!(removed.first, 1);
        // Back to front: 14 alone, then the run 11 and 12.
        assert_eq!(removed.runs, [(4, 1), (1, 2)]);
        let left: Vec<_> = (0..rows.len()).filter_map(|i| rows.id_at(i)).collect();
        assert_eq!(left, [10, 13, 15]);
        assert_eq!(rows.selection().len(), 0);
        // The row after the first removed one takes its place...
        assert_eq!(rows.neighbour(1), Some(13));
        // ...or, past the end, the new last row.
        assert_eq!(
            rows.remove_many(&HashSet::from([13, 15])).map(|r| r.first),
            Some(1)
        );
        assert_eq!(rows.neighbour(1), Some(10));
        assert_eq!(rows.remove_many(&HashSet::from([99])), None);
        assert_eq!(
            rows.remove_many(&HashSet::from([10])).map(|r| r.first),
            Some(0)
        );
        assert_eq!(rows.neighbour(0), None);
    }

    #[test]
    fn a_reload_keeps_only_listed_selected_rows() {
        let rows = rows();
        assert_eq!(rows.set(ROWS.to_vec(), Kind::Received), Refresh::Reset);
        assert_eq!(
            rows.set(ROWS.to_vec(), Kind::Received),
            Refresh::InPlace(6),
            "the same list refreshes in place"
        );
        rows.select(|s, ids| s.all(ids));
        rows.set(vec![10, 12, 16], Kind::Received);
        let mut kept = rows.selection().ids();
        kept.sort_unstable();
        assert_eq!(kept, [10, 12]);
    }

    #[test]
    fn a_selection_change_names_the_rows_it_touched() {
        let rows = rows();
        rows.set(ROWS.to_vec(), Kind::Received);
        assert_eq!(rows.select(|s, _| s.single(12)), [2]);
        assert_eq!(rows.select(|s, _| s.single(12)), Vec::<usize>::new());
        assert_eq!(rows.select(|s, ids| s.extend_to(ids, 14)), [3, 4]);
        assert_eq!(rows.indices_of(&HashSet::from([15, 10])), [0, 5]);
    }

    #[test]
    fn rows_read_from_the_cache_by_kind() {
        let mut db = Db::in_memory().unwrap();
        let seeded = crate::seed::seed(&mut db, 4).unwrap();
        let db = Rc::new(db);
        let rows = Rows::new(db.clone());
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        rows.set(ids.clone(), Kind::Received);
        let row = rows.row(0).expect("a row");
        assert_eq!(row.id, ids[0]);
        assert!(!row.sender.is_empty() && row.sender == row.who);
        assert!(!row.sender.starts_with("To: "));
        assert!(rows.row(ids.len()).is_none(), "past the end");
        // Outgoing: the first recipient, after "To:".
        rows.set(ids, Kind::Outgoing);
        let row = komorebi_i18n::with_lang(Lang::En, || rows.row(0)).expect("a row");
        assert!(row.sender.starts_with("To: "), "{}", row.sender);
        assert!(!row.unread, "the user's own mail is never bold");
    }
}
