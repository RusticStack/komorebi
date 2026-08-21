//! Offline mutation outbox data access.

use super::*;

impl Database {
    // -----------------------------------------------------------------------
    // Outbox - offline mutation queue for server reconciliation
    // -----------------------------------------------------------------------

    /// Enqueue a server-side mutation for later reconciliation. `uid` is the
    /// message's IMAP UID; `kind` is the action ("move", "delete", "flag",
    /// "unflag", "mark_read", "mark_unread"); `target_folder` is the remote
    /// folder name for moves.
    pub fn enqueue_mutation(
        &self,
        account_id: &str,
        message_id: &str,
        uid: u32,
        kind: &str,
        target_folder: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO outbox (id, account_id, message_id, uid, kind, target_folder)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, account_id, message_id, uid, kind, target_folder],
        )?;
        Ok(id)
    }

    /// All mutations not yet applied to the server (oldest first).
    pub fn list_pending_mutations(&self) -> Result<Vec<OutboxEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, message_id, uid, kind, target_folder, created_at
             FROM outbox WHERE applied_at IS NULL ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OutboxEntry {
                id: r.get(0)?,
                account_id: r.get(1)?,
                message_id: r.get(2)?,
                uid: r.get(3)?,
                kind: r.get(4)?,
                target_folder: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Mark a mutation as reconciled with the server.
    pub fn mark_mutation_applied(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE outbox SET applied_at=unixepoch() WHERE id=?1", [id])?;
        Ok(())
    }

    /// Remove all mutations older than `before` that have been applied (housekeeping).
    pub fn purge_applied_mutations(&self, before: i64) -> Result<usize> {
        let conn = self.conn();
        let n = conn.execute(
            "DELETE FROM outbox WHERE applied_at IS NOT NULL AND applied_at < ?1",
            [before],
        )?;
        Ok(n)
    }

    /// Count of pending (unreconciled) mutations, for the sync-state UI.
    pub fn pending_mutation_count(&self) -> Result<i64> {
        let conn = self.conn();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM outbox WHERE applied_at IS NULL",
            [],
            |r| r.get(0),
        )?)
    }
}
/// One entry in the offline mutation outbox (a server-side action awaiting
/// reconciliation).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutboxEntry {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub uid: u32,
    pub kind: String,
    pub target_folder: Option<String>,
    pub created_at: i64,
}
