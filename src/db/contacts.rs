//! Contacts harvested from synced mail (senders + recipients).

use super::*;

#[derive(Debug, Clone)]
pub struct Contact {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub name: Option<String>,
    pub count: i64,
}

impl Database {
    /// Record an address seen on a message (bumps frequency, refreshes name).
    pub fn touch_contact(&self, account_id: &str, email: &str, name: Option<&str>) {
        let email = email.trim();
        if email.is_empty() || !email.contains('@') {
            return;
        }
        let conn = self.conn();
        let _ = conn.execute(
            "INSERT INTO contacts (id, account_id, email, name, count, last_seen)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(account_id, email) DO UPDATE SET
                count = count + 1,
                last_seen = excluded.last_seen,
                name = COALESCE(excluded.name, contacts.name)",
            params![
                Uuid::new_v4().to_string(),
                account_id,
                email.to_lowercase(),
                name,
                chrono::Utc::now().timestamp()
            ],
        );
    }

    /// Most-frequent contacts whose email/name start with `query`.
    pub fn search_contacts(&self, account_id: &str, query: &str) -> Result<Vec<Contact>> {
        let like = format!("{}%", query.to_lowercase());
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, email, COALESCE(name,''), count
             FROM contacts
             WHERE account_id=?1 AND (email LIKE ?2 OR LOWER(COALESCE(name,'')) LIKE ?2)
             ORDER BY count DESC LIMIT 5",
        )?;
        let rows = stmt.query_map(params![account_id, like], |r| {
            Ok(Contact {
                id: r.get(0)?,
                account_id: r.get(1)?,
                email: r.get(2)?,
                name: Some(r.get::<_, String>(3)?).filter(|s| !s.is_empty()),
                count: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
