//! Reminder (snooze/follow-up) data access (split out of db/mod.rs).

use super::messages::row_to_message;
use super::*;

// Reminders (snooze + follow-up)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reminder {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub kind: String,
    pub label: Option<String>,
    pub due_at: i64,
    pub created_at: i64,
}

impl Database {
    /// Create or replace the single reminder for a message (one snooze or
    /// follow-up at a time; scheduling a new one overwrites the previous).
    pub fn upsert_reminder(
        &self,
        account_id: &str,
        message_id: &str,
        kind: &str,
        label: Option<&str>,
        due_at: i64,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO reminders (id, account_id, message_id, kind, label, due_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(message_id) DO UPDATE SET
                kind=excluded.kind,
                label=excluded.label,
                due_at=excluded.due_at,
                created_at=unixepoch()",
            params![
                Uuid::new_v4().to_string(),
                account_id,
                message_id,
                kind,
                label,
                due_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_reminder(&self, message_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM reminders WHERE message_id=?1", [message_id])?;
        Ok(())
    }

    pub fn get_reminder(&self, message_id: &str) -> Result<Option<Reminder>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, account_id, message_id, kind, label, due_at, created_at
             FROM reminders WHERE message_id=?1",
            [message_id],
            |r| {
                Ok(Reminder {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    message_id: r.get(2)?,
                    kind: r.get(3)?,
                    label: r.get(4)?,
                    due_at: r.get(5)?,
                    created_at: r.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// All reminders that are still pending (due in the future).
    pub fn list_active_reminders(&self, now: i64) -> Result<Vec<Reminder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, message_id, kind, label, due_at, created_at
             FROM reminders WHERE due_at > ?1 ORDER BY due_at ASC",
        )?;
        let rows = stmt.query_map([now], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                message_id: r.get(2)?,
                kind: r.get(3)?,
                label: r.get(4)?,
                due_at: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Messages currently snoozed (hidden from the inbox until due).
    pub fn list_snoozed_messages(&self, now: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.folder_id, m.subject, m.sender_name, m.sender_email, m.date_received,
                    m.is_read, m.is_flagged, m.snippet, m.ai_category
             FROM reminders r JOIN messages m ON m.id = r.message_id
             WHERE r.kind='snooze' AND r.due_at > ?1
             ORDER BY r.due_at ASC",
        )?;
        let rows = stmt.query_map([now], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Lazily release snoozes whose due time has passed (the message returns
    /// to the inbox on the next reload). Returns the number removed.
    pub fn delete_expired_snoozes(&self, now: i64) -> Result<usize> {
        let conn = self.conn();
        let n = conn.execute(
            "DELETE FROM reminders WHERE kind='snooze' AND due_at <= ?1",
            [now],
        )?;
        Ok(n)
    }

    // -----------------------------------------------------------------------
    // Labels (user labels, separate from the AI `ai_category` triage field)
    // -----------------------------------------------------------------------
}
