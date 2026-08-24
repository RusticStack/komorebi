//! Draft and attachment data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// Drafts, attachments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub body: Option<String>,
    pub send_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub display_name: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub created_at: i64,
}

impl Database {
    pub fn save_draft(
        &self,
        account_id: &str,
        subject: Option<&str>,
        to_addresses: Option<&str>,
        body: Option<&str>,
    ) -> Result<String> {
        let conn = self.conn();
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO drafts (id, account_id, subject, to_addresses, body)
             VALUES (?1,?2,?3,?4,?5)",
            params![id, account_id, subject, to_addresses, body],
        )?;
        Ok(id)
    }

    pub fn list_drafts(&self) -> Result<Vec<Draft>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, subject, to_addresses, cc_addresses, bcc_addresses,
                    body, send_at, updated_at
             FROM drafts WHERE send_at IS NULL ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Draft {
                id: r.get(0)?,
                account_id: r.get(1)?,
                subject: r.get(2)?,
                to_addresses: r.get(3)?,
                cc_addresses: r.get(4)?,
                bcc_addresses: r.get(5)?,
                body: r.get(6)?,
                send_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Update an existing draft's content and bump its timestamp.
    pub fn update_draft(
        &self,
        id: &str,
        subject: Option<&str>,
        to_addresses: Option<&str>,
        body: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE drafts SET subject=?2, to_addresses=?3, body=?4, updated_at=?5 WHERE id=?1",
            params![
                id,
                subject,
                to_addresses,
                body,
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub fn delete_draft(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM drafts WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn insert_attachment(
        &self,
        message_id: &str,
        display_name: &str,
        mime: Option<&str>,
        data: &[u8],
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO attachments (id, message_id, display_name, mime, size_bytes, data)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, message_id, display_name, mime, data.len() as i64, data],
        )?;
        Ok(id)
    }

    pub fn list_attachments(&self, message_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, display_name, mime, size_bytes, created_at
             FROM attachments WHERE message_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([message_id], |r| {
            Ok(Attachment {
                id: r.get(0)?,
                message_id: r.get(1)?,
                display_name: r.get(2)?,
                mime: r.get(3)?,
                size_bytes: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_attachment_data(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn();
        conn.query_row("SELECT data FROM attachments WHERE id=?1", [id], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_attachment(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM attachments WHERE id=?1", [id])?;
        Ok(())
    }
    // -----------------------------------------------------------------------
    // Scheduled send
    // -----------------------------------------------------------------------

    /// Queue a message for sending at `send_at` (stored as a draft row).
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_draft(
        &self,
        account_id: &str,
        subject: Option<&str>,
        to_addresses: Option<&str>,
        cc_addresses: Option<&str>,
        bcc_addresses: Option<&str>,
        body: Option<&str>,
        send_at: i64,
    ) -> Result<String> {
        let conn = self.conn();
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO drafts (id, account_id, subject, to_addresses, cc_addresses,
                 bcc_addresses, body, send_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                account_id,
                subject,
                to_addresses,
                cc_addresses,
                bcc_addresses,
                body,
                send_at
            ],
        )?;
        Ok(id)
    }

    /// Drafts whose scheduled time has passed (oldest first), excluding
    /// normal drafts.
    pub fn list_due_schedules(&self, now: i64) -> Result<Vec<Draft>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, subject, to_addresses, cc_addresses, bcc_addresses,
                    body, send_at, updated_at
             FROM drafts WHERE send_at IS NOT NULL AND send_at <= ?1
             ORDER BY send_at ASC",
        )?;
        let rows = stmt.query_map(params![now], |r| {
            Ok(Draft {
                id: r.get(0)?,
                account_id: r.get(1)?,
                subject: r.get(2)?,
                to_addresses: r.get(3)?,
                cc_addresses: r.get(4)?,
                bcc_addresses: r.get(5)?,
                body: r.get(6)?,
                send_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
