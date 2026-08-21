//! Folder data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub remote_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub special_use: Option<String>,
    pub unread_count: i64,
    pub total_count: i64,
    pub last_sync_at: Option<i64>,
}

impl Database {
    pub fn upsert_folder(
        &self,
        account_id: &str,
        name: &str,
        display_name: Option<&str>,
        special_use: Option<&str>,
        remote_id: Option<&str>,
    ) -> Result<Folder> {
        let conn = self.conn();
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM folders WHERE account_id=?1 AND name=?2",
                params![account_id, name],
                |r| r.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE folders SET display_name=?3, special_use=?4, remote_id=?5
                     WHERE id=?1 AND account_id=?2",
                    params![id, account_id, display_name, special_use, remote_id],
                )?;
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO folders (id, account_id, name, display_name, special_use, remote_id)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![id, account_id, name, display_name, special_use, remote_id],
                )?;
                id
            }
        };
        let row = conn.query_row(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE id=?1",
            [&id],
            |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    remote_id: r.get(2)?,
                    name: r.get(3)?,
                    display_name: r.get(4)?,
                    special_use: r.get(5)?,
                    unread_count: r.get(6)?,
                    total_count: r.get(7)?,
                    last_sync_at: r.get(8)?,
                })
            },
        )?;
        Ok(row)
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE account_id=?1 ORDER BY
                CASE special_use WHEN 'inbox' THEN 0 WHEN 'sent' THEN 1 WHEN 'drafts' THEN 2
                                 WHEN 'trash' THEN 3 WHEN 'archive' THEN 4 WHEN 'junk' THEN 5
                                 ELSE 6 END, name",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                remote_id: r.get(2)?,
                name: r.get(3)?,
                display_name: r.get(4)?,
                special_use: r.get(5)?,
                unread_count: r.get(6)?,
                total_count: r.get(7)?,
                last_sync_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_folder(&self, id: &str) -> Result<Option<Folder>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE id=?1",
            [id],
            |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    remote_id: r.get(2)?,
                    name: r.get(3)?,
                    display_name: r.get(4)?,
                    special_use: r.get(5)?,
                    unread_count: r.get(6)?,
                    total_count: r.get(7)?,
                    last_sync_at: r.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_folder_counts(&self, id: &str, unread: i64, total: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE folders SET unread_count=?2, total_count=?3, last_sync_at=unixepoch()
             WHERE id=?1",
            params![id, unread, total],
        )?;
        Ok(())
    }

    /// Delete a folder, cascading to its messages and attachments.
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM folders WHERE id=?1", [id])?;
        Ok(())
    }

    /// Highest cached IMAP UID for a folder, or 0.
    pub fn folder_max_uid(&self, folder_id: &str) -> Result<u32> {
        let conn = self.conn();
        let uid: Option<i64> = conn
            .query_row(
                "SELECT MAX(imap_uid) FROM messages WHERE folder_id=?1",
                [folder_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(uid.unwrap_or(0).max(0) as u32)
    }
}
