//! The rules tables (`crate::rules`): an account's rules and their
//! conditions (user data, kept by any future clean-slate migration), the
//! activity log of what automatic runs did (pruned to [`LOG_KEEP`] rows per
//! account), and each Inbox's watermark (`folders.rules_uidvalidity` and
//! `rules_uidnext`): automatic runs act only on UIDs at or above it.

use super::Db;
use crate::rules::{Condition, Field, Op, Rule};
use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::HashSet;

/// v16: local rules. Rules are user data, not cache: a migration that
/// clears the cache must keep `rules` and `rule_conditions`.
pub(super) const RULES: &str = r#"
CREATE TABLE rules (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    match_any INTEGER NOT NULL DEFAULT 0,
    stop INTEGER NOT NULL DEFAULT 0,
    move_to TEXT,
    trash INTEGER NOT NULL DEFAULT 0,
    mark_read INTEGER NOT NULL DEFAULT 0,
    flag INTEGER NOT NULL DEFAULT 0,
    paused INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX rules_account ON rules(account_id, position);
CREATE TABLE rule_conditions (
    rule_id INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    field INTEGER NOT NULL,
    op INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (rule_id, position)
) WITHOUT ROWID;
CREATE TABLE rule_log (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    at INTEGER NOT NULL,
    rule_name TEXT NOT NULL,
    msgid TEXT NOT NULL,
    subject TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    action INTEGER NOT NULL,
    dest TEXT,
    outcome INTEGER NOT NULL
);
CREATE INDEX rule_log_account ON rule_log(account_id, at DESC);
ALTER TABLE folders ADD COLUMN rules_uidvalidity INTEGER;
ALTER TABLE folders ADD COLUMN rules_uidnext INTEGER;
"#;

/// Activity rows kept per account.
pub const LOG_KEEP: i64 = 500;

/// What a logged action was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogAction {
    Move,
    Delete,
    MarkRead,
    Flag,
}

impl LogAction {
    fn code(self) -> i64 {
        match self {
            LogAction::Move => 0,
            LogAction::Delete => 1,
            LogAction::MarkRead => 2,
            LogAction::Flag => 3,
        }
    }

    fn from_code(c: i64) -> Self {
        match c {
            0 => LogAction::Move,
            1 => LogAction::Delete,
            2 => LogAction::MarkRead,
            _ => LogAction::Flag,
        }
    }
}

/// How a logged action ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOutcome {
    Done,
    Failed,
    /// The message was no longer there.
    Gone,
}

impl LogOutcome {
    fn code(self) -> i64 {
        match self {
            LogOutcome::Done => 0,
            LogOutcome::Failed => 1,
            LogOutcome::Gone => 2,
        }
    }

    fn from_code(c: i64) -> Self {
        match c {
            0 => LogOutcome::Done,
            1 => LogOutcome::Failed,
            _ => LogOutcome::Gone,
        }
    }
}

/// One line of the activity log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub id: i64,
    pub at: i64,
    pub rule_name: String,
    pub msgid: String,
    pub subject: String,
    pub from_addr: String,
    pub action: LogAction,
    /// The destination's server path (a move or delete).
    pub dest: Option<String>,
    pub outcome: LogOutcome,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Db {
    /// An account's rules in order, with their conditions.
    pub fn rules_for(&self, account_id: i64) -> Result<Vec<Rule>> {
        let mut rules: Vec<Rule> = self
            .conn
            .prepare_cached(
                "SELECT id, name, enabled, match_any, stop, move_to, trash, mark_read, flag, paused
                 FROM rules WHERE account_id = ?1 ORDER BY position, id",
            )?
            .query_map([account_id], |r| {
                Ok(Rule {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    enabled: r.get(2)?,
                    match_any: r.get(3)?,
                    stop: r.get(4)?,
                    move_to: r.get(5)?,
                    trash: r.get(6)?,
                    mark_read: r.get(7)?,
                    flag: r.get(8)?,
                    paused: r.get(9)?,
                    conditions: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        if rules.is_empty() {
            return Ok(rules);
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.rule_id, c.field, c.op, c.value FROM rule_conditions c
             JOIN rules r ON r.id = c.rule_id
             WHERE r.account_id = ?1 ORDER BY c.rule_id, c.position",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (rule_id, field, op, value) = row?;
            let (Some(field), Some(op)) = (Field::from_code(field), Op::from_code(op)) else {
                continue;
            };
            if let Some(rule) = rules.iter_mut().find(|r| r.id == rule_id) {
                rule.conditions.push(Condition { field, op, value });
            }
        }
        Ok(rules)
    }

    /// Save a rule (a new one, `id` 0, goes last) with its conditions in
    /// one transaction, within the caps, and return its id. Saving resumes
    /// a paused rule.
    pub fn save_rule(&self, account_id: i64, rule: &Rule) -> Result<i64> {
        let rule = rule.clone().capped();
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let id = if rule.id == 0 {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM rules WHERE account_id = ?1",
                [account_id],
                |r| r.get(0),
            )?;
            if count >= crate::rules::MAX_RULES as i64 {
                anyhow::bail!("an account has at most {} rules", crate::rules::MAX_RULES);
            }
            tx.execute(
                "INSERT INTO rules (account_id, position, name, enabled, match_any, stop,
                     move_to, trash, mark_read, flag, paused)
                 VALUES (?1, (SELECT coalesce(max(position), -1) + 1 FROM rules WHERE account_id = ?1),
                     ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
                params![
                    account_id,
                    rule.name,
                    rule.enabled,
                    rule.match_any,
                    rule.stop,
                    rule.move_to,
                    rule.trash,
                    rule.mark_read,
                    rule.flag
                ],
            )?;
            tx.last_insert_rowid()
        } else {
            tx.execute(
                "UPDATE rules SET name = ?2, enabled = ?3, match_any = ?4, stop = ?5,
                     move_to = ?6, trash = ?7, mark_read = ?8, flag = ?9, paused = 0
                 WHERE id = ?1",
                params![
                    rule.id,
                    rule.name,
                    rule.enabled,
                    rule.match_any,
                    rule.stop,
                    rule.move_to,
                    rule.trash,
                    rule.mark_read,
                    rule.flag
                ],
            )?;
            tx.execute("DELETE FROM rule_conditions WHERE rule_id = ?1", [rule.id])?;
            rule.id
        };
        for (i, c) in rule.conditions.iter().enumerate() {
            tx.execute(
                "INSERT INTO rule_conditions (rule_id, position, field, op, value)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, i as i64, c.field.code(), c.op.code(), c.value],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn delete_rule(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM rules WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_rule_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE rules SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(())
    }

    /// Swap a rule with the one above it (`up`) or below it. Returns
    /// whether it moved.
    pub fn move_rule(&self, id: i64, up: bool) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let Some((account_id, pos)) = tx
            .query_row(
                "SELECT account_id, position FROM rules WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let sql = if up {
            "SELECT id, position FROM rules WHERE account_id = ?1 AND position < ?2
             ORDER BY position DESC LIMIT 1"
        } else {
            "SELECT id, position FROM rules WHERE account_id = ?1 AND position > ?2
             ORDER BY position LIMIT 1"
        };
        let Some((other, other_pos)) = tx
            .query_row(sql, params![account_id, pos], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE rules SET position = ?2 WHERE id = ?1",
            params![id, other_pos],
        )?;
        tx.execute(
            "UPDATE rules SET position = ?2 WHERE id = ?1",
            params![other, pos],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Pause the account's rules whose destination is not among the
    /// folders the server lists (`names`). Returns how many were paused.
    pub fn pause_rules_missing(&self, account_id: i64, names: &HashSet<String>) -> Result<usize> {
        let targets: Vec<(i64, String)> = self
            .conn
            .prepare_cached(
                "SELECT id, move_to FROM rules
                 WHERE account_id = ?1 AND paused = 0 AND move_to IS NOT NULL",
            )?
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut n = 0;
        for (id, dest) in targets {
            if !names.contains(&dest) {
                self.conn
                    .execute("UPDATE rules SET paused = 1 WHERE id = ?1", [id])?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// A folder's rules watermark: (UIDVALIDITY, first UID rules act on).
    pub fn rule_mark(&self, folder_id: i64) -> Result<Option<(u32, u32)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT rules_uidvalidity, rules_uidnext FROM folders WHERE id = ?1",
                [folder_id],
                |r| Ok((r.get::<_, Option<u32>>(0)?, r.get::<_, Option<u32>>(1)?)),
            )
            .optional()?
            .and_then(|(v, n)| v.zip(n)))
    }

    pub fn set_rule_mark(&self, folder_id: i64, uidvalidity: u32, uidnext: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET rules_uidvalidity = ?2, rules_uidnext = ?3 WHERE id = ?1",
            params![folder_id, uidvalidity, uidnext],
        )?;
        Ok(())
    }

    /// Add activity rows, then keep the newest [`LOG_KEEP`] of the account,
    /// in one transaction.
    pub fn log_rule_actions(&self, account_id: i64, entries: &[LogEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let at = now();
        for e in entries {
            tx.execute(
                "INSERT INTO rule_log (account_id, at, rule_name, msgid, subject, from_addr,
                     action, dest, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    account_id,
                    if e.at == 0 { at } else { e.at },
                    e.rule_name,
                    e.msgid,
                    e.subject,
                    e.from_addr,
                    e.action.code(),
                    e.dest,
                    e.outcome.code()
                ],
            )?;
        }
        tx.execute(
            "DELETE FROM rule_log WHERE account_id = ?1 AND id NOT IN
                 (SELECT id FROM rule_log WHERE account_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2)",
            params![account_id, LOG_KEEP],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The account's activity, newest first.
    pub fn rule_log(&self, account_id: i64, limit: usize) -> Result<Vec<LogEntry>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id, at, rule_name, msgid, subject, from_addr, action, dest, outcome
                 FROM rule_log WHERE account_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
            )?
            .query_map(params![account_id, limit as i64], |r| {
                Ok(LogEntry {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    rule_name: r.get(2)?,
                    msgid: r.get(3)?,
                    subject: r.get(4)?,
                    from_addr: r.get(5)?,
                    action: LogAction::from_code(r.get(6)?),
                    dest: r.get(7)?,
                    outcome: LogOutcome::from_code(r.get(8)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn clear_rule_log(&self, account_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM rule_log WHERE account_id = ?1", [account_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewAccount;
    use crate::providers::{Security, Server};

    /// Undo v18 (SMTP without sign-in), the newest.
    const DROP_V18: &str = "ALTER TABLE accounts DROP COLUMN smtp_no_auth;";
    /// Undo v17 (the calendar, `cal::store`), after v18.
    const DROP_V17: &str = "DROP TABLE alarms_done; DROP TABLE cal_queue; DROP TABLE occurrences;
        DROP TABLE events; DROP TABLE calendars;";
    /// Undo v16.
    pub(crate) const DROP_V16: &str = "DROP TABLE rule_log; DROP TABLE rule_conditions;
        DROP TABLE rules; ALTER TABLE folders DROP COLUMN rules_uidvalidity;
        ALTER TABLE folders DROP COLUMN rules_uidnext;";

    fn account(db: &Db, email: &str) -> i64 {
        let host = Server {
            host: "imap.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        db.add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: email.into(),
            name: String::new(),
            imap: host.clone(),
            smtp: host,
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap()
    }

    fn rule(name: &str, value: &str) -> Rule {
        Rule {
            name: name.into(),
            enabled: true,
            move_to: Some("Lists".into()),
            conditions: vec![Condition {
                field: Field::From,
                op: Op::EndsWith,
                value: value.into(),
            }],
            ..Rule::default()
        }
    }

    fn schema(db: &Db) -> Vec<(String, String)> {
        db.conn
            .prepare("SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn migrates_v15_to_v16_like_a_fresh_cache() {
        let fresh = Db::in_memory().unwrap();
        let db = Db::in_memory().unwrap();
        let a = account(&db, "a@example.com");
        let f = db
            .upsert_folder(a, "INBOX", "Inbox", 0, crate::role::Role::Inbox)
            .unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V18} {DROP_V17} {DROP_V16} PRAGMA user_version = 15;"
            ))
            .unwrap();
        let db = Db::init(db.conn).unwrap();
        assert_eq!(schema(&db), schema(&fresh));
        // Nothing was lost, and no folder has a mark yet.
        assert_eq!(db.folders().unwrap().len(), 1);
        assert_eq!(db.rule_mark(f).unwrap(), None);
    }

    #[test]
    fn save_load_order_toggle_and_delete() {
        let db = Db::in_memory().unwrap();
        let a = account(&db, "a@example.com");
        let one = db.save_rule(a, &rule("one", "@x.org")).unwrap();
        let mut two = rule("two", "@y.org");
        two.match_any = true;
        two.conditions.push(Condition {
            field: Field::HasAttachment,
            op: Op::Contains,
            value: String::new(),
        });
        let two = db.save_rule(a, &two).unwrap();
        let got = db.rules_for(a).unwrap();
        assert_eq!(got.iter().map(|r| r.id).collect::<Vec<_>>(), [one, two]);
        assert_eq!(got[1].conditions.len(), 2);
        assert!(got[1].match_any);

        assert!(db.move_rule(two, true).unwrap());
        assert!(!db.move_rule(two, true).unwrap());
        let ids: Vec<i64> = db.rules_for(a).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, [two, one]);

        db.set_rule_enabled(one, false).unwrap();
        let mut edited = db.rules_for(a).unwrap().remove(1);
        assert!(!edited.enabled);
        edited.conditions.truncate(0);
        edited.conditions.push(Condition {
            field: Field::Subject,
            op: Op::Is,
            value: "x".repeat(300),
        });
        db.save_rule(a, &edited).unwrap();
        let back = db.rules_for(a).unwrap().remove(1);
        assert_eq!(back.conditions.len(), 1);
        assert_eq!(back.conditions[0].value.len(), crate::rules::MAX_VALUE);

        db.delete_rule(one).unwrap();
        assert_eq!(db.rules_for(a).unwrap().len(), 1);
    }

    #[test]
    fn missing_folder_pauses_and_saving_resumes() {
        let db = Db::in_memory().unwrap();
        let a = account(&db, "a@example.com");
        let id = db.save_rule(a, &rule("one", "@x.org")).unwrap();
        let names: HashSet<String> = ["INBOX".to_string(), "Lists".into()].into();
        assert_eq!(db.pause_rules_missing(a, &names).unwrap(), 0);
        let names: HashSet<String> = ["INBOX".to_string()].into();
        assert_eq!(db.pause_rules_missing(a, &names).unwrap(), 1);
        let mut r = db.rules_for(a).unwrap().remove(0);
        assert!(r.paused);
        r.move_to = Some("INBOX.Other".into());
        db.save_rule(a, &r).unwrap();
        assert!(!db.rules_for(a).unwrap()[0].paused);
        assert_eq!(db.rules_for(a).unwrap()[0].id, id);
    }

    #[test]
    fn log_prunes_and_account_removal_cascades() {
        let db = Db::in_memory().unwrap();
        let a = account(&db, "a@example.com");
        let b = account(&db, "b@example.com");
        let entry = |i: i64| LogEntry {
            id: 0,
            at: 1_000 + i,
            rule_name: "r".into(),
            msgid: format!("<{i}@x>"),
            subject: "s".into(),
            from_addr: "f@x".into(),
            action: LogAction::Move,
            dest: Some("Lists".into()),
            outcome: LogOutcome::Done,
        };
        let many: Vec<LogEntry> = (0..LOG_KEEP + 20).map(entry).collect();
        db.log_rule_actions(a, &many).unwrap();
        db.log_rule_actions(b, &[entry(1)]).unwrap();
        let log = db.rule_log(a, 1_000).unwrap();
        assert_eq!(log.len() as i64, LOG_KEEP);
        assert_eq!(log[0].msgid, format!("<{}@x>", LOG_KEEP + 19));
        db.save_rule(a, &rule("one", "@x.org")).unwrap();
        db.remove_account(a).unwrap();
        assert!(db.rule_log(a, 10).unwrap().is_empty());
        assert!(db.rules_for(a).unwrap().is_empty());
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM rule_conditions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(db.rule_log(b, 10).unwrap().len(), 1);
        db.clear_rule_log(b).unwrap();
        assert!(db.rule_log(b, 10).unwrap().is_empty());
    }
}
