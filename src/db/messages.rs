//! Message data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub folder_id: String,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub snippet: Option<String>,
    pub ai_category: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageDetail {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub imap_uid: Option<i64>,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub snippet: Option<String>,
    pub ai_category: Option<String>,
    pub ai_summary: Option<String>,
    pub has_attachments: bool,
    pub rfc_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MessageDraft {
    pub account_id: String,
    pub folder_id: String,
    pub imap_uid: Option<u32>,
    pub uid_validity: Option<u32>,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub size_bytes: Option<i64>,
    pub has_attachments: bool,
    pub rfc_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_message(&self, m: &MessageDraft) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        // Resolve threading lazily so callers only supply the raw
        // Message-ID / In-Reply-To / References headers.
        let thread_id = match &m.thread_id {
            Some(tid) => Some(tid.clone()),
            None => self.resolve_thread_id(
                m.rfc_message_id.as_deref(),
                m.in_reply_to.as_deref(),
                m.references_header.as_deref(),
            )?,
        };
        // Store Message-IDs normalized (angle brackets stripped) so thread
        // resolution can match them against In-Reply-To/References values.
        let rfc_message_id = m.rfc_message_id.as_deref().map(normalize_message_id);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO messages (id, account_id, folder_id, imap_uid, uid_validity, subject,
                sender_name, sender_email, to_addresses, cc_addresses, bcc_addresses,
                date_received, is_read, is_flagged, snippet, body_text, body_html, size_bytes,
                has_attachments, rfc_message_id, in_reply_to, references_header, thread_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
             ON CONFLICT(id) DO NOTHING",
            params![
                id,
                m.account_id,
                m.folder_id,
                m.imap_uid.map(|v| v as i64),
                m.uid_validity.map(|v| v as i64),
                m.subject,
                m.sender_name,
                m.sender_email,
                m.to_addresses,
                m.cc_addresses,
                m.bcc_addresses,
                m.date_received,
                m.is_read as i64,
                m.is_flagged as i64,
                m.snippet,
                m.body_text,
                m.body_html,
                m.size_bytes,
                m.has_attachments as i64,
                rfc_message_id,
                m.in_reply_to,
                m.references_header,
                thread_id,
            ],
        )?;
        Ok(id)
    }

    /// Find a message by folder + IMAP UID (dedupe across syncs).
    pub fn find_message_by_uid(&self, folder_id: &str, imap_uid: u32) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id FROM messages WHERE folder_id=?1 AND imap_uid=?2 LIMIT 1",
            params![folder_id, imap_uid as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_messages(
        &self,
        folder_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE folder_id=?1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![folder_id, limit, offset], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn search_messages(&self, query: &str, limit: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.folder_id, m.subject, m.sender_name, m.sender_email, m.date_received,
                    m.is_read, m.is_flagged, m.snippet, m.ai_category
             FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY m.date_received DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All starred (flagged) messages across folders, newest first.
    pub fn list_flagged_messages(&self, limit: i64, offset: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE is_flagged=1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All messages across an account's folders, newest first (for smart views).
    pub fn list_account_messages(&self, account_id: &str, limit: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE account_id=?1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Set of message ids that have at least one cached attachment.
    pub fn list_message_ids_with_attachments(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT message_id FROM attachments")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?)
    }

    pub fn get_message_detail(&self, id: &str) -> Result<Option<MessageDetail>> {
        let conn = self.conn();
        let sql = format!("SELECT {DETAIL_COLUMNS} FROM messages WHERE id=?1");
        conn.query_row(&sql, [id], row_to_detail)
            .optional()
            .map_err(Into::into)
    }

    /// All messages in the same conversation, oldest first.
    pub fn list_thread_messages(&self, thread_id: &str) -> Result<Vec<MessageDetail>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {DETAIL_COLUMNS} FROM messages WHERE thread_id=?1
             ORDER BY date_received ASC, created_at ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([thread_id], row_to_detail)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Resolve the conversation id for an incoming message by matching its
    /// In-Reply-To/References against already-cached Message-IDs. Returns None
    /// when the message has no usable identifiers (it stands alone).
    pub fn resolve_thread_id(
        &self,
        message_id: Option<&str>,
        in_reply_to: Option<&str>,
        references: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self.conn();
        let mut candidates: Vec<String> = Vec::new();
        if let Some(irt) = in_reply_to {
            candidates.push(normalize_message_id(irt));
        }
        if let Some(refs) = references {
            candidates.extend(parse_references(refs));
        }
        for c in candidates {
            if c.is_empty() {
                continue;
            }
            let found: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT id, thread_id FROM messages WHERE rfc_message_id=?1 LIMIT 1",
                    [&c],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((_id, tid)) = found {
                return Ok(Some(tid.unwrap_or(c)));
            }
        }
        Ok(message_id
            .map(normalize_message_id)
            .filter(|s| !s.is_empty()))
    }

    pub fn mark_read(&self, id: &str, read: bool) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_read=?2 WHERE id=?1",
            params![id, read as i64],
        )?;
        Ok(())
    }

    pub fn mark_flagged(&self, id: &str, flagged: bool) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_flagged=?2 WHERE id=?1",
            params![id, flagged as i64],
        )?;
        Ok(())
    }

    /// Move a message into another folder (local cache move; a later sync
    /// reconciles the server-side flags/location).
    pub fn move_message(&self, id: &str, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET folder_id=?2 WHERE id=?1",
            params![id, folder_id],
        )?;
        Ok(())
    }

    /// Mark every message in a folder as read.
    pub fn mark_folder_read(&self, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_read=1 WHERE folder_id=?1",
            [folder_id],
        )?;
        Ok(())
    }

    /// Move every message from one folder into another.
    pub fn move_all_messages(&self, from_folder_id: &str, to_folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET folder_id=?2 WHERE folder_id=?1",
            params![from_folder_id, to_folder_id],
        )?;
        Ok(())
    }

    /// Permanently delete every message in a folder (used to empty trash).
    pub fn delete_all_messages(&self, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM messages WHERE folder_id=?1", [folder_id])?;
        Ok(())
    }

    pub fn set_ai_fields(
        &self,
        id: &str,
        category: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET ai_category=?2, ai_summary=?3,
                ai_triaged_at = CASE WHEN ?2 IS NOT NULL THEN unixepoch() ELSE ai_triaged_at END
             WHERE id=?1",
            params![id, category, summary],
        )?;
        Ok(())
    }
}

pub(crate) fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        subject: row.get(2)?,
        sender_name: row.get(3)?,
        sender_email: row.get(4)?,
        date_received: row.get(5)?,
        is_read: row.get(6)?,
        is_flagged: row.get(7)?,
        snippet: row.get(8)?,
        ai_category: row.get(9)?,
    })
}

/// Columns selected for a full [`MessageDetail`], in `row_to_detail` order.
const DETAIL_COLUMNS: &str =
    "id, account_id, folder_id, imap_uid, subject, sender_name, sender_email,
    to_addresses, cc_addresses, date_received, is_read, is_flagged, body_text,
    body_html, snippet, ai_category, ai_summary, has_attachments, rfc_message_id,
    in_reply_to, references_header, thread_id";

fn row_to_detail(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageDetail> {
    Ok(MessageDetail {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        imap_uid: row.get(3)?,
        subject: row.get(4)?,
        sender_name: row.get(5)?,
        sender_email: row.get(6)?,
        to_addresses: row.get(7)?,
        cc_addresses: row.get(8)?,
        date_received: row.get(9)?,
        is_read: row.get(10)?,
        is_flagged: row.get(11)?,
        body_text: row.get(12)?,
        body_html: row.get(13)?,
        snippet: row.get(14)?,
        ai_category: row.get(15)?,
        ai_summary: row.get(16)?,
        has_attachments: row.get(17)?,
        rfc_message_id: row.get(18)?,
        in_reply_to: row.get(19)?,
        references_header: row.get(20)?,
        thread_id: row.get(21)?,
    })
}

/// Strip angle brackets and surrounding whitespace from a Message-ID.
fn normalize_message_id(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// Split a References header into normalized Message-IDs.
fn parse_references(references: &str) -> Vec<String> {
    references
        .split_whitespace()
        .map(normalize_message_id)
        .filter(|s| !s.is_empty())
        .collect()
}

impl Database {
    /// The cached folder a message currently lives in (by folder id).
    pub fn folder_name_of_message(&self, message_id: &str) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT f.name FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
            [message_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }
}
