//! The message list as a lazy Slint model.
//!
//! The model holds only the ids of the messages in the current view, newest
//! first. Slint asks for the rows it is about to show (about a screenful,
//! since `ListView` virtualizes), and each one is read from the SQLite cache
//! on demand. A reload costs one indexed id query instead of building every
//! row, and the cache stays the single source of truth: after a change, the
//! row is simply marked changed and read again.

use crate::db::Db;
use crate::{MailRow, format, mail};
use chrono::{DateTime, Local};
use slint::{Model, ModelNotify, ModelTracker};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Rows {
    db: Rc<Db>,
    ids: RefCell<Vec<i64>>,
    /// Sent/Drafts: show the recipient instead of the sender.
    outgoing: Cell<bool>,
    /// Reference time for relative dates, fixed per reload.
    now: Cell<DateTime<Local>>,
    notify: ModelNotify,
}

impl Rows {
    pub fn new(db: Rc<Db>) -> Self {
        Self {
            db,
            ids: RefCell::default(),
            outgoing: Cell::new(false),
            now: Cell::new(Local::now()),
            notify: ModelNotify::default(),
        }
    }

    /// Show `ids`. When the list is unchanged (a sync touched flags or
    /// previews), rows refresh in place so the scroll position stays put.
    pub fn set(&self, ids: Vec<i64>, outgoing: bool) {
        self.outgoing.set(outgoing);
        self.now.set(Local::now());
        let same = *self.ids.borrow() == ids;
        let len = ids.len();
        *self.ids.borrow_mut() = ids;
        if same {
            for i in 0..len {
                self.notify.row_changed(i);
            }
        } else {
            self.notify.reset();
        }
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

    /// Re-read one row from the cache.
    pub fn changed(&self, id: i64) {
        if let Some(i) = self.index_of(id) {
            self.notify.row_changed(i);
        }
    }

    /// Drop one row; returns its former index.
    pub fn remove(&self, id: i64) -> Option<usize> {
        let i = self.index_of(id)?;
        self.ids.borrow_mut().remove(i);
        self.notify.row_removed(i, 1);
        Some(i)
    }
}

impl Model for Rows {
    type Data = MailRow;

    fn row_count(&self) -> usize {
        self.len()
    }

    fn row_data(&self, row: usize) -> Option<MailRow> {
        let id = self.id_at(row)?;
        let h = self.db.header(id).ok()??;
        let sender = if self.outgoing.get() {
            let to = mail::smtp::split_addresses(&h.to);
            format!("To: {}", to.first().map_or("", String::as_str)).into()
        } else {
            format::sender(&h.from_name, &h.from_addr).into()
        };
        Some(MailRow {
            id: id as i32,
            sender,
            subject: h.subject.into(),
            preview: h.preview.into(),
            date: format::list_date(h.date, self.now.get()).into(),
            unread: !h.seen,
            flagged: h.flagged,
            attach: h.has_attach,
            answered: h.answered,
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
