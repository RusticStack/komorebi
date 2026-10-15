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
        })
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn col_count(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    /// Insert an empty row before row `at` (`at == row_count()` appends).
    /// False at the 10-row limit or past the end.
    pub fn insert_row(&mut self, at: usize) -> bool {
        if self.rows.len() >= MAX_TABLE || at > self.rows.len() {
            return false;
        }
        let row = (0..self.col_count()).map(|_| Cell::text("")).collect();
        self.rows.insert(at, row);
        true
    }

    /// Insert an empty column before column `at`.
    pub fn insert_col(&mut self, at: usize) -> bool {
        if self.col_count() >= MAX_TABLE || at > self.col_count() {
            return false;
        }
        for row in &mut self.rows {
            row.insert(at, Cell::text(""));
        }
        true
    }

    /// Delete row `at`; the last row stays (delete the table instead).
    pub fn delete_row(&mut self, at: usize) -> bool {
        if self.rows.len() <= 1 || at >= self.rows.len() {
            return false;
        }
        self.rows.remove(at);
        true
    }

    pub fn delete_col(&mut self, at: usize) -> bool {
        if self.col_count() <= 1 || at >= self.col_count() {
            return false;
        }
        for row in &mut self.rows {
            row.remove(at);
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
    /// The document as plain text: formatting, tables and logos gone (what
    /// "Switch to plain text" keeps).
    pub fn to_plain(&self) -> Doc {
        Doc::from_plain(&super::emit::plain(self))
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
