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

    // -----------------------------------------------------------------------
    // Internal message notes
    // -----------------------------------------------------------------------

    /// The internal note attached to a message, if any.
    pub fn get_note(&self, message_id: &str) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT body FROM message_notes WHERE message_id = ?1",
            [message_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Create or replace the internal note for a message. An empty body
    /// deletes the note.
    pub fn set_note(&self, message_id: &str, body: &str) -> Result<()> {
        let conn = self.conn();
        if body.trim().is_empty() {
            conn.execute(
                "DELETE FROM message_notes WHERE message_id = ?1",
                [message_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO message_notes (message_id, body, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(message_id) DO UPDATE SET body = excluded.body,
                     updated_at = excluded.updated_at",
                params![message_id, body, chrono::Utc::now().timestamp()],
            )?;
        }
        Ok(())
    }
}
