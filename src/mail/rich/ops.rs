//! Editing operations on the model that need no selection: table shape and
//! clearing formatting. (The editor, when it exists, builds on these.)

use super::model::*;

impl Table {
    /// An empty `rows` x `cols` table, each 1 to 10.
    pub fn new(rows: usize, cols: usize) -> Option<Table> {
        if !(1..=MAX_TABLE).contains(&rows) || !(1..=MAX_TABLE).contains(&cols) {
            return None;
        }
        Some(Table {
            rows: (0..rows)
                .map(|_| (0..cols).map(|_| Cell::text("")).collect())
                .collect(),
            ..Table::default()
        })
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn col_count(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    /// Insert an empty row before row `at` (`at == row_count()` appends).
    /// False at the 10-row limit or past the end. A table with merged
    /// cells keeps its rows and columns (false for each of these four):
    /// only the whole table can be deleted.
    pub fn insert_row(&mut self, at: usize) -> bool {
        if self.is_merged() || self.rows.len() >= MAX_TABLE || at > self.rows.len() {
            return false;
        }
        let row = (0..self.col_count()).map(|_| Cell::text("")).collect();
        self.rows.insert(at, row);
        true
    }

    /// Insert an empty column before column `at`.
    pub fn insert_col(&mut self, at: usize) -> bool {
        if self.is_merged() || self.col_count() >= MAX_TABLE || at > self.col_count() {
            return false;
        }
        for row in &mut self.rows {
            row.insert(at, Cell::text(""));
        }
        if !self.widths.is_empty() {
            self.widths.insert(at.min(self.widths.len()), 0);
        }
        true
    }

    /// Delete row `at`; the last row stays (delete the table instead).
    pub fn delete_row(&mut self, at: usize) -> bool {
        if self.is_merged() || self.rows.len() <= 1 || at >= self.rows.len() {
            return false;
        }
        self.rows.remove(at);
        true
    }

    pub fn delete_col(&mut self, at: usize) -> bool {
        if self.is_merged() || self.col_count() <= 1 || at >= self.col_count() {
            return false;
        }
        for row in &mut self.rows {
            row.remove(at);
        }
        if at < self.widths.len() {
            self.widths.remove(at);
        }
        true
    }
}

impl Para {
    /// Remove character formatting (keeps text, links and the paragraph's
    /// own alignment, list and quote).
    pub fn clear_formatting(&mut self) {
        for r in &mut self.runs {
            let link = r.style.link.take();
            r.style = Style::default();
            r.style.link = link;
        }
    }
}

impl Doc {
    /// The document as plain text: formatting and tables gone, pictures as their text (what
    /// "Switch to plain text" keeps).
    pub fn to_plain(&self) -> Doc {
        Doc::from_plain(&super::emit::plain(self))
    }

    /// Take out the pictures `gone` names (their bytes are missing).
    pub fn drop_images(&mut self, gone: &dyn Fn(&str) -> bool) {
        fn para(p: &mut Para, gone: &dyn Fn(&str) -> bool) {
            p.runs
                .retain(|r| r.image.as_ref().is_none_or(|i| !gone(&i.cid)));
        }
        fn go(blocks: &mut [Block], gone: &dyn Fn(&str) -> bool) {
            for b in blocks {
                match b {
                    Block::Para(p) => para(p, gone),
                    Block::Table(t) => {
                        for p in t.rows.iter_mut().flatten().flat_map(|c| c.paras.iter_mut()) {
                            para(p, gone);
                        }
                    }
                    Block::Signature(s) => go(&mut s.blocks, gone),
                    Block::Quote(_) => {}
                }
            }
        }
        go(&mut self.blocks, gone);
        self.normalize();
    }

    /// Clear character formatting everywhere in the message body.
    pub fn clear_formatting(&mut self) {
        fn go(blocks: &mut [Block]) {
            for b in blocks {
                match b {
                    Block::Para(p) => p.clear_formatting(),
                    Block::Table(t) => {
                        for p in t.rows.iter_mut().flatten().flat_map(|c| c.paras.iter_mut()) {
                            p.clear_formatting();
                        }
                    }
                    Block::Signature(_) | Block::Quote(_) => {}
                }
            }
        }
        go(&mut self.blocks);
        self.normalize();
    }
}
