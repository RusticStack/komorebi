//! Mail rules: conditions evaluated against newly synced messages.

use super::*;

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub enabled: bool,
    /// "sender" or "subject".
    pub field: String,
    /// Case-insensitive substring the field must contain.
    pub value: String,
    /// "mark_read" | "star" | "archive" | "apply_label".
    pub action: String,
    /// Label id for `apply_label`.
    pub action_arg: Option<String>,
}

const RULE_COLUMNS: &str = "id, account_id, name, enabled, field, value, action, action_arg";

fn row_to_rule(r: &rusqlite::Row<'_>) -> rusqlite::Result<Rule> {
    Ok(Rule {
        id: r.get(0)?,
        account_id: r.get(1)?,
        name: r.get(2)?,
        enabled: r.get::<_, i64>(3)? != 0,
        field: r.get(4)?,
        value: r.get(5)?,
        action: r.get(6)?,
        action_arg: r.get(7)?,
    })
}

impl Database {
    pub fn list_rules(&self, account_id: &str) -> Result<Vec<Rule>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RULE_COLUMNS} FROM rules WHERE account_id=?1 ORDER BY rowid"
        ))?;
        let rows = stmt.query_map(params![account_id], row_to_rule)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_rule(
        &self,
        account_id: &str,
        name: &str,
        field: &str,
        value: &str,
        action: &str,
        action_arg: Option<&str>,
    ) -> Result<String> {
        let conn = self.conn();
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO rules (id, account_id, name, enabled, field, value, action, action_arg)
             VALUES (?1,?2,?3,1,?4,?5,?6,?7)",
            params![id, account_id, name, field, value, action, action_arg],
        )?;
        Ok(id)
    }

    pub fn delete_rule(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM rules WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE rules SET enabled=?2 WHERE id=?1",
            params![id, enabled as i64],
        )?;
        Ok(())
    }

    /// Evaluate every enabled rule for `account_id` against newly cached
    /// messages. Returns how many actions fired.
    pub fn apply_rules(&self, account_id: &str, message_ids: &[String]) -> Result<usize> {
        if message_ids.is_empty() {
            return Ok(0);
        }
        let rules: Vec<Rule> = self
            .list_rules(account_id)?
            .into_iter()
            .filter(|r| r.enabled)
            .collect();
        if rules.is_empty() {
            return Ok(0);
        }
        let mut applied = 0;
        let conn = self.conn();
        for id in message_ids {
            let (sender, subject): (Option<String>, Option<String>) = conn.query_row(
                "SELECT sender_email, subject FROM messages WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            for rule in &rules {
                let haystack = match rule.field.as_str() {
                    "subject" => subject.clone().unwrap_or_default().to_lowercase(),
                    _ => sender.clone().unwrap_or_default().to_lowercase(),
                };
                if !haystack.contains(&rule.value.to_lowercase()) {
                    continue;
                }
                match rule.action.as_str() {
                    "mark_read" => {
                        conn.execute("UPDATE messages SET is_read=1 WHERE id=?1", [id])?;
                    }
                    "star" => {
                        conn.execute("UPDATE messages SET is_flagged=1 WHERE id=?1", [id])?;
                    }
                    "archive" => {
                        let archive: Option<String> = conn
                            .query_row(
                                "SELECT f.id FROM folders f WHERE f.account_id=?1
                                 AND f.special_use='archive' LIMIT 1",
                                [account_id],
                                |r| r.get(0),
                            )
                            .optional()?;
                        if let Some(dest) = archive {
                            let _ = self.move_message(id, &dest);
                        }
                    }
                    "apply_label" => {
                        if let Some(label_id) = rule.action_arg.as_deref() {
                            conn.execute(
                                "INSERT OR IGNORE INTO message_labels (message_id, label_id)
                                 VALUES (?1, ?2)",
                                params![id, label_id],
                            )?;
                        }
                    }
                    _ => {}
                }
                applied += 1;
            }
        }
        Ok(applied)
    }
}
