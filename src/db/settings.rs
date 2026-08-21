//! Settings key/value data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// Settings (key/value, JSON values)
// ---------------------------------------------------------------------------

impl Database {
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn list_settings(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
