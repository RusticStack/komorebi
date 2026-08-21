//! User label data access.

use super::messages::row_to_message;
use super::*;

impl Database {
    pub fn insert_label(&self, account_id: &str, name: &str, color: Option<&str>) -> Result<Label> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO labels (id, account_id, name, color) VALUES (?1, ?2, ?3, ?4)",
            params![id, account_id, name, color],
        )?;
        Ok(Label {
            id,
            account_id: account_id.to_string(),
            name: name.to_string(),
            color: color.map(String::from),
            created_at: chrono::Utc::now().timestamp(),
        })
    }

    pub fn list_labels(&self, account_id: &str) -> Result<Vec<Label>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, color, created_at
             FROM labels WHERE account_id=?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(Label {
                id: r.get(0)?,
                account_id: r.get(1)?,
                name: r.get(2)?,
                color: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_label(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM labels WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn rename_label(&self, id: &str, new_name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE labels SET name=?2 WHERE id=?1",
            params![id, new_name],
        )?;
        Ok(())
    }

    /// Apply a label to a message (idempotent - safe to call repeatedly).
    pub fn apply_label(&self, message_id: &str, label_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            [message_id, label_id],
        )?;
        Ok(())
    }

    /// Remove a label from a message (idempotent).
    pub fn remove_label(&self, message_id: &str, label_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM message_labels WHERE message_id=?1 AND label_id=?2",
            [message_id, label_id],
        )?;
        Ok(())
    }

    /// List the labels applied to a message.
    pub fn list_message_labels(&self, message_id: &str) -> Result<Vec<Label>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT l.id, l.account_id, l.name, l.color, l.created_at
             FROM labels l
             JOIN message_labels ml ON ml.label_id = l.id
             WHERE ml.message_id=?1 ORDER BY l.name",
        )?;
        let rows = stmt.query_map([message_id], |r| {
            Ok(Label {
                id: r.get(0)?,
                account_id: r.get(1)?,
                name: r.get(2)?,
                color: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// List messages that carry a given label.
    pub fn list_messages_by_label(&self, label_id: &str, limit: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.folder_id, m.subject, m.sender_name, m.sender_email, m.date_received,
                    m.is_read, m.is_flagged, m.snippet, m.ai_category
             FROM messages m
             JOIN message_labels ml ON ml.message_id = m.id
             WHERE ml.label_id=?1
             ORDER BY m.date_received DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![label_id, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
/// A user-created label (distinct from the AI triage `ai_category` field).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Label {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub color: Option<String>,
    pub created_at: i64,
}
