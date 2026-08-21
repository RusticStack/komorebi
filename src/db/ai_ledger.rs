//! AI action ledger data access (split out of db/mod.rs).

use super::*;

// ---------------------------------------------------------------------------
// AI actions ledger
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiAction {
    pub id: String,
    pub account_id: Option<String>,
    pub message_id: Option<String>,
    pub kind: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub output: Option<String>,
    pub created_at: i64,
}

impl Database {
    pub fn insert_ai_action(
        &self,
        account_id: Option<&str>,
        message_id: Option<&str>,
        kind: &str,
        provider: Option<&str>,
        model: Option<&str>,
        output: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO ai_actions (id, account_id, message_id, kind, provider, model, output)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, account_id, message_id, kind, provider, model, output],
        )?;
        Ok(id)
    }

    pub fn list_ai_actions(&self, message_id: &str, limit: i64) -> Result<Vec<AiAction>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, message_id, kind, provider, model, output, created_at
             FROM ai_actions WHERE message_id=?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![message_id, limit], |r| {
            Ok(AiAction {
                id: r.get(0)?,
                account_id: r.get(1)?,
                message_id: r.get(2)?,
                kind: r.get(3)?,
                provider: r.get(4)?,
                model: r.get(5)?,
                output: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

// ---------------------------------------------------------------------------
