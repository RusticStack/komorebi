//! Account data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider_kind: String,
    pub auth_mode: String,
    pub username: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub signature_html: Option<String>,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct AccountDraft {
    pub email: String,
    pub display_name: Option<String>,
    pub provider_kind: String,
    pub auth_mode: String,
    pub username: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
}

impl Database {
    pub fn insert_account(&self, draft: &AccountDraft) -> Result<Account> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let account = Account {
            id: id.clone(),
            email: draft.email.clone(),
            display_name: draft.display_name.clone(),
            provider_kind: draft.provider_kind.clone(),
            auth_mode: draft.auth_mode.clone(),
            username: draft.username.clone(),
            imap_host: draft.imap_host.clone(),
            imap_port: draft.imap_port,
            imap_security: draft.imap_security.clone(),
            smtp_host: draft.smtp_host.clone(),
            smtp_port: draft.smtp_port,
            smtp_security: draft.smtp_security.clone(),
            signature_html: None,
            created_at: now,
            updated_at: None,
        };
        let conn = self.conn();
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, provider_kind, auth_mode, username,
                imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                account.id,
                account.email,
                account.display_name,
                account.provider_kind,
                account.auth_mode,
                account.username,
                account.imap_host,
                account.imap_port,
                account.imap_security,
                account.smtp_host,
                account.smtp_port,
                account.smtp_security,
                account.created_at,
            ],
        )?;
        Ok(account)
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, email, display_name, provider_kind, auth_mode, username,
                    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security,
                    signature_html, created_at, updated_at
             FROM accounts ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_account)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, email, display_name, provider_kind, auth_mode, username,
                    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security,
                    signature_html, created_at, updated_at
             FROM accounts WHERE id = ?1",
            [id],
            row_to_account,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Persist the per-account signature (stored as plain text in
    /// `signature_html`; it is inserted verbatim into composed bodies).
    pub fn update_account_signature(&self, id: &str, signature: &str) -> Result<()> {
        let conn = self.conn();
        let sig: Option<String> = if signature.trim().is_empty() {
            None
        } else {
            Some(signature.to_string())
        };
        conn.execute(
            "UPDATE accounts SET signature_html = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, sig, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Delete an account and cascade its folders/messages/drafts/attachments.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        email: row.get(1)?,
        display_name: row.get(2)?,
        provider_kind: row.get(3)?,
        auth_mode: row.get(4)?,
        username: row.get(5)?,
        imap_host: row.get(6)?,
        imap_port: row.get(7)?,
        imap_security: row.get(8)?,
        smtp_host: row.get(9)?,
        smtp_port: row.get(10)?,
        smtp_security: row.get(11)?,
        signature_html: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}
