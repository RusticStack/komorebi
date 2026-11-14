//! Settings > Rules without a toolkit: what both apps' rules pages do. The
//! page's state (the accounts in the picker, the shown account's rules and
//! activity, the folders a rule can move to), the editor's form and how it
//! becomes a validated [`Rule`], and the actions behind the buttons (on and
//! off, order, delete, clear the activity, copy as Sieve).
//!
//! Like the rest of the library it produces no sentences: a rule's summary
//! is a list of [`Part`]s, an activity line a [`LogLine`], a failed save a
//! [`Problem`]. The Slint app words them with `t!`, the GPUI app with
//! `tr!` (`views/rules/text.rs`).

use crate::db::{Db, LogAction, LogEntry, LogOutcome};
use crate::role::Role;
use crate::rules::{Condition, Field, Invalid, MAX_CONDITIONS, Op, Rule};
use anyhow::Result;

/// Activity lines shown.
pub const LOG_SHOWN: usize = 50;

/// One piece of a rule's summary: a condition or an action. The summary
/// reads the conditions first, then the actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// "Any of these", said first when a rule with two conditions or more
    /// matches on any.
    Any,
    Cond {
        field: Field,
        op: Op,
        value: String,
    },
    /// Move to the folder with this display name.
    Move(String),
    Delete,
    Read,
    Flag,
    Stop,
}

/// A rule's summary as parts. `display` names a folder by its server path.
pub fn summary(r: &Rule, display: &dyn Fn(&str) -> String) -> Vec<Part> {
    let mut parts = Vec::new();
    if r.match_any && r.conditions.len() > 1 {
        parts.push(Part::Any);
    }
    for c in &r.conditions {
        parts.push(Part::Cond {
            field: c.field,
            op: c.op,
            value: c.value.clone(),
        });
    }
    if let Some(dest) = &r.move_to {
        parts.push(Part::Move(display(dest)));
    }
    if r.trash {
        parts.push(Part::Delete);
    }
    if r.mark_read {
        parts.push(Part::Read);
    }
    if r.flag {
        parts.push(Part::Flag);
    }
    if r.stop {
        parts.push(Part::Stop);
    }
    parts
}

/// A rule as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: i64,
    pub name: String,
    pub parts: Vec<Part>,
    pub enabled: bool,
    /// Its folder is gone: a sync paused it.
    pub paused: bool,
}

/// What an automatic run did, as a line of the activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    /// The action failed; the message stayed in the Inbox.
    Failed,
    /// Moved to the folder with this display name.
    Move(String),
    Delete,
    Read,
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub rule_name: String,
    /// Unix seconds.
    pub at: i64,
    pub subject: String,
    pub from: String,
    pub kind: LogKind,
}

/// A folder a rule can move to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The server path stored in the rule.
    pub path: String,
    pub display: String,
    /// Nesting level, for the indent.
    pub depth: usize,
}

/// The page's state: read again with [`Page::reload`].
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The accounts in the picker (id and address), and the one shown.
    pub accounts: Vec<(i64, String)>,
    pub account: i64,
    pub rules: Vec<Rule>,
    pub cards: Vec<Card>,
    pub activity: Vec<LogLine>,
    /// The Move choices: the account's folders but its Inbox.
    pub choices: Vec<Choice>,
    /// Every folder of the shown account: server path and display name.
    names: Vec<(String, String)>,
}

impl Page {
    /// Read the shown account's rules and activity again (the account stays
    /// when it still exists, else the first one shows).
    pub fn reload(&mut self, db: &Db) {
        let accounts = db.accounts().unwrap_or_default();
        let folders = db.folders().unwrap_or_default();
        self.accounts = accounts.iter().map(|a| (a.id, a.email.clone())).collect();
        if !self.accounts.iter().any(|(id, _)| *id == self.account) {
            self.account = self.accounts.first().map_or(0, |(id, _)| *id);
        }
        self.rules = db.rules_for(self.account).unwrap_or_default();
        self.names = folders
            .iter()
            .filter(|f| f.account_id == self.account)
            .map(|f| (f.name.clone(), f.display.clone()))
            .collect();
        self.cards = self
            .rules
            .iter()
            .map(|r| Card {
                id: r.id,
                name: r.name.clone(),
                parts: summary(r, &|p| self.display(p)),
                enabled: r.enabled,
                paused: r.paused,
            })
            .collect();
        self.activity = db
            .rule_log(self.account, LOG_SHOWN)
            .unwrap_or_default()
            .into_iter()
            .map(|e| {
                let kind = if e.outcome != LogOutcome::Done {
                    LogKind::Failed
                } else {
                    match e.action {
                        LogAction::Move => {
                            LogKind::Move(self.display(e.dest.as_deref().unwrap_or_default()))
                        }
                        LogAction::Delete => LogKind::Delete,
                        LogAction::MarkRead => LogKind::Read,
                        LogAction::Flag => LogKind::Flag,
                    }
                };
                LogLine {
                    rule_name: e.rule_name,
                    at: e.at,
                    subject: e.subject,
                    from: e.from_addr,
                    kind,
                }
            })
            .collect();
        self.choices = folders
            .iter()
            .filter(|f| f.account_id == self.account && f.role != Role::Inbox)
            .map(|f| Choice {
                path: f.name.clone(),
                display: f.display.clone(),
                depth: usize::try_from(f.depth).unwrap_or(0),
            })
            .collect();
    }

    /// A folder's display name by its server path (the path itself when
    /// the folder is gone).
    pub fn display(&self, path: &str) -> String {
        self.names
            .iter()
            .find(|(name, _)| name == path)
            .map_or_else(|| path.to_string(), |(_, display)| display.clone())
    }

    /// Show another account (by position in the picker).
    pub fn pick_account(&mut self, db: &Db, index: usize) {
        if let Some((id, _)) = self.accounts.get(index) {
            self.account = *id;
        }
        self.reload(db);
    }

    /// The position of the shown account in the picker.
    pub fn account_index(&self) -> usize {
        self.accounts
            .iter()
            .position(|(id, _)| *id == self.account)
            .unwrap_or(0)
    }

    /// The id of the rule at list position `index`.
    pub fn id_at(&self, index: usize) -> Option<i64> {
        self.rules.get(index).map(|r| r.id)
    }

    /// The name of rule `id`.
    pub fn name_of(&self, id: i64) -> String {
        self.rules
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.clone())
            .unwrap_or_default()
    }

    /// Turn the rule at `index` on or off.
    pub fn set_enabled(&mut self, db: &Db, index: usize, on: bool) {
        if let Some(id) = self.id_at(index) {
            db.set_rule_enabled(id, on).ok();
        }
        self.reload(db);
    }

    /// Move the rule at `index` one place up or down.
    pub fn move_rule(&mut self, db: &Db, index: usize, up: bool) {
        if let Some(id) = self.id_at(index) {
            db.move_rule(id, up).ok();
        }
        self.reload(db);
    }

    /// Move rule `id` to place `to` (a drag in the list; past the end: the
    /// last place), the others keeping their order. By id, so a list read
    /// again during the drag cannot move another rule; a rule gone since
    /// changes nothing. One transaction: a failure leaves the order as it
    /// was, and the error is the cache's.
    pub fn move_rule_to(&mut self, db: &Db, id: i64, to: usize) -> Result<()> {
        let result = db.move_rule_to(id, to).map(|_| ());
        self.reload(db);
        result
    }

    /// The list position of rule `id`.
    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.rules.iter().position(|r| r.id == id)
    }

    /// Delete rule `id`; the error is the cache's.
    pub fn delete(&mut self, db: &Db, id: i64) -> Result<()> {
        let result = db.delete_rule(id);
        self.reload(db);
        result
    }

    pub fn clear_activity(&mut self, db: &Db) {
        db.clear_rule_log(self.account).ok();
        self.reload(db);
    }

    /// The shown account's rules as a Sieve script, for the clipboard.
    pub fn sieve(&self, db: &Db) -> String {
        let rules = db.rules_for(self.account).unwrap_or_default();
        let trash = db
            .folder_by_role(self.account, Role::Trash)
            .ok()
            .flatten()
            .map(|f| f.name);
        crate::rules::to_sieve(&rules, trash.as_deref())
    }

    /// Save `rule` for the shown account and read the page again.
    pub fn save(&mut self, db: &Db, rule: &Rule) -> Result<()> {
        db.save_rule(self.account, rule)?;
        self.reload(db);
        Ok(())
    }
}

/// What is wrong with a rule that cannot be saved yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// "Give the rule a name."
    Name,
    /// "Add a condition." (none, or too many)
    Conditions,
    /// "Each condition needs a value."
    Value,
    /// "Choose at least one action."
    Action,
}

/// One rule's page: the form's fields, filled from a rule or blank.
#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    /// The rule being edited (id 0: a new one); what the form does not
    /// show (its id, position, paused flag) stays.
    base: Rule,
    pub name: String,
    pub match_any: bool,
    pub conds: Vec<Condition>,
    /// Move to a folder, and which of the page's choices.
    pub move_on: bool,
    pub move_to: usize,
    pub delete: bool,
    pub read: bool,
    pub flag: bool,
    pub stop: bool,
}

/// A condition with nothing typed.
pub fn blank_condition() -> Condition {
    Condition {
        field: Field::From,
        op: Op::Contains,
        value: String::new(),
    }
}

impl Editor {
    /// A new rule: on, with one empty "From contains" condition.
    pub fn blank(name: String) -> Self {
        Self::open(
            &Rule {
                name,
                enabled: true,
                conditions: vec![blank_condition()],
                ..Rule::default()
            },
            &[],
        )
    }

    /// The form for `rule`; `choices` are the page's Move choices.
    pub fn open(rule: &Rule, choices: &[Choice]) -> Self {
        let at = rule
            .move_to
            .as_ref()
            .and_then(|m| choices.iter().position(|c| &c.path == m));
        Self {
            base: rule.clone(),
            name: rule.name.clone(),
            match_any: rule.match_any,
            conds: rule.conditions.clone(),
            move_on: at.is_some(),
            move_to: at.unwrap_or(0),
            delete: rule.trash,
            read: rule.mark_read,
            flag: rule.flag,
            stop: rule.stop,
        }
    }

    /// Whether this edits a saved rule (false: a new one).
    pub fn is_saved(&self) -> bool {
        self.base.id != 0
    }

    /// Another condition, up to the most a rule holds.
    pub fn add_cond(&mut self) {
        if self.conds.len() < MAX_CONDITIONS {
            self.conds.push(blank_condition());
        }
    }

    pub fn remove_cond(&mut self, index: usize) {
        if index < self.conds.len() {
            self.conds.remove(index);
        }
    }

    /// Set condition `index` to a field, an operator and a value. True when
    /// the field or operator changed (what the row shows changes; a typed
    /// value stays the text box's own).
    pub fn set_cond(&mut self, index: usize, field: Field, op: Op, value: &str) -> bool {
        let Some(c) = self.conds.get_mut(index) else {
            return false;
        };
        let shape = c.field != field || c.op != op;
        c.field = field;
        c.op = op;
        c.value = value.to_string();
        shape
    }

    /// Move to a folder and Delete exclude each other: choosing one
    /// switches the other off.
    pub fn choose_move(&mut self, on: bool) {
        self.move_on = on;
        if on {
            self.delete = false;
        }
    }

    pub fn choose_delete(&mut self, on: bool) {
        self.delete = on;
        if on {
            self.move_on = false;
        }
    }

    /// The form as a rule that can be saved, or what is wrong with it.
    pub fn build(&self, choices: &[Choice]) -> Result<Rule, Problem> {
        let mut rule = self.base.clone();
        rule.name = self.name.trim().to_string();
        rule.match_any = self.match_any;
        rule.conditions = self.conds.clone();
        rule.move_to = if self.move_on {
            choices.get(self.move_to).map(|c| c.path.clone())
        } else {
            None
        };
        rule.trash = self.delete && rule.move_to.is_none();
        rule.mark_read = self.read;
        rule.flag = self.flag;
        rule.stop = self.stop;
        if rule.name.is_empty() {
            return Err(Problem::Name);
        }
        match rule.check() {
            Ok(()) => Ok(rule),
            Err(Invalid::NoConditions | Invalid::TooMany) => Err(Problem::Conditions),
            Err(Invalid::EmptyValue) => Err(Problem::Value),
            Err(Invalid::NoActions | Invalid::MoveAndDelete) => Err(Problem::Action),
        }
    }
}

/// Made-up rules, a paused one and some activity for snapshots. Offline
/// only (`KOMOREBI_OFFLINE`), because they are written to the cache; a
/// cache that has rules is left alone.
pub fn sample(db: &Db) {
    let Some(account) = db.accounts().ok().and_then(|a| a.first().map(|a| a.id)) else {
        return;
    };
    if crate::dev_var("KOMOREBI_OFFLINE").is_none()
        || !db.rules_for(account).is_ok_and(|r| r.is_empty())
    {
        return;
    }
    let folders = db.folders().unwrap_or_default();
    let folder = folders
        .iter()
        .find(|f| f.account_id == account && f.role == Role::Archive)
        .map_or_else(|| "Archive".to_string(), |f| f.name.clone());
    let cond = |field, op, value: &str| Condition {
        field,
        op,
        value: value.into(),
    };
    let rules = [
        Rule {
            name: "GitHub notifications".into(),
            enabled: true,
            move_to: Some(folder),
            conditions: vec![cond(Field::From, Op::EndsWith, "@github.com")],
            ..Rule::default()
        },
        Rule {
            name: "Invoices".into(),
            enabled: true,
            match_any: true,
            flag: true,
            stop: true,
            conditions: vec![
                cond(Field::Subject, Op::Contains, "invoice"),
                cond(Field::Subject, Op::Contains, "fatura"),
                cond(Field::HasAttachment, Op::Contains, ""),
            ],
            ..Rule::default()
        },
        Rule {
            name: "Newsletters".into(),
            enabled: false,
            mark_read: true,
            conditions: vec![cond(Field::ToCc, Op::Is, "news@lists.example.org")],
            ..Rule::default()
        },
    ];
    for r in &rules {
        db.save_rule(account, r).ok();
    }
    // A rule whose folder is gone, paused as a sync would.
    let gone = Rule {
        name: "Old project".into(),
        enabled: true,
        move_to: Some("Projects/2019".into()),
        conditions: vec![cond(Field::Subject, Op::StartsWith, "[proj]")],
        ..Rule::default()
    };
    db.save_rule(account, &gone).ok();
    let names: std::collections::HashSet<String> = folders
        .into_iter()
        .filter(|f| f.account_id == account)
        .map(|f| f.name)
        .collect();
    db.pause_rules_missing(account, &names).ok();
    let entry = |action, dest: Option<&str>, outcome, subject: &str| LogEntry {
        id: 0,
        at: 1_760_000_000,
        rule_name: "GitHub notifications".into(),
        msgid: "<sample@example.com>".into(),
        subject: subject.into(),
        from_addr: "notifications@github.com".into(),
        action,
        dest: dest.map(str::to_string),
        outcome,
    };
    let archive = rules[0].move_to.clone().unwrap_or_default();
    db.log_rule_actions(
        account,
        &[
            entry(
                LogAction::Move,
                Some(&archive),
                LogOutcome::Done,
                "[repo] New pull request",
            ),
            entry(
                LogAction::Move,
                Some(&archive),
                LogOutcome::Failed,
                "[repo] Build failed",
            ),
            entry(LogAction::MarkRead, None, LogOutcome::Done, "Weekly digest"),
        ],
    )
    .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(field: Field, op: Op, value: &str) -> Condition {
        Condition {
            field,
            op,
            value: value.into(),
        }
    }

    fn choices() -> Vec<Choice> {
        ["Archive", "Lists/Rust"]
            .iter()
            .map(|p| Choice {
                path: (*p).into(),
                display: (*p).rsplit('/').next().unwrap_or(p).into(),
                depth: 0,
            })
            .collect()
    }

    /// A drag moves a rule to its new place; the others keep their order.
    #[test]
    fn a_rule_moves_to_the_place_it_is_dropped_on() {
        use crate::db::NewAccount;
        use crate::providers::{Security, Server};
        let db = Db::in_memory().unwrap();
        let host = Server {
            host: "imap.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        let account = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "a@example.com".into(),
                name: String::new(),
                imap: host.clone(),
                smtp: host,
                save_sent: true,
                smtp_no_auth: false,
            })
            .unwrap();
        for name in ["A", "B", "C", "D"] {
            let r = Rule {
                name: name.into(),
                enabled: true,
                mark_read: true,
                conditions: vec![cond(Field::From, Op::Contains, "x")],
                ..Rule::default()
            };
            db.save_rule(account, &r).unwrap();
        }
        let mut page = Page::default();
        page.reload(&db);
        let names = |p: &Page| p.rules.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        let id = |p: &Page, i: usize| p.id_at(i).unwrap();
        let a = id(&page, 0);
        page.move_rule_to(&db, a, 2).unwrap();
        assert_eq!(names(&page), ["B", "C", "A", "D"]);
        page.move_rule_to(&db, id(&page, 3), 0).unwrap();
        assert_eq!(names(&page), ["D", "B", "C", "A"]);
        // Past the end is the last place; onto itself changes nothing.
        page.move_rule_to(&db, id(&page, 1), 9).unwrap();
        assert_eq!(names(&page), ["D", "C", "A", "B"]);
        page.move_rule_to(&db, id(&page, 2), 2).unwrap();
        assert_eq!(names(&page), ["D", "C", "A", "B"]);
        assert_eq!(page.index_of(a), Some(2));
        // A rule deleted during the drag moves nothing.
        db.delete_rule(a).unwrap();
        page.move_rule_to(&db, a, 0).unwrap();
        assert_eq!(names(&page), ["D", "C", "B"]);
        // With a gap in the positions (the deleted rule's), a move still
        // lands in place.
        page.move_rule_to(&db, id(&page, 2), 0).unwrap();
        assert_eq!(names(&page), ["B", "D", "C"]);
    }

    /// Ported with the Slint app's `summary_reads_conditions_then_actions`
    /// (`src/app/rules.rs`): the parts are the conditions, then the actions.
    #[test]
    fn summary_reads_conditions_then_actions() {
        let r = Rule {
            name: "r".into(),
            match_any: true,
            move_to: Some("INBOX/Lists".into()),
            mark_read: true,
            conditions: vec![
                cond(Field::From, Op::EndsWith, "@github.com"),
                cond(Field::HasAttachment, Op::Contains, ""),
            ],
            ..Rule::default()
        };
        let parts = summary(&r, &|p| p.rsplit('/').next().unwrap_or(p).to_string());
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], Part::Any);
        assert_eq!(
            parts[1],
            Part::Cond {
                field: Field::From,
                op: Op::EndsWith,
                value: "@github.com".into()
            }
        );
        assert_eq!(parts[3], Part::Move("Lists".into()));
        assert_eq!(parts[4], Part::Read);
    }

    #[test]
    fn any_is_said_only_for_two_conditions_or_more() {
        let r = Rule {
            match_any: true,
            flag: true,
            conditions: vec![cond(Field::Subject, Op::Is, "x")],
            ..Rule::default()
        };
        let parts = summary(&r, &|p| p.to_string());
        assert_eq!(parts.len(), 2);
        assert!(!parts.contains(&Part::Any));
    }

    #[test]
    fn a_form_needs_a_name_a_value_and_an_action() {
        let ch = choices();
        let mut e = Editor::blank("New rule".into());
        e.name = "  ".into();
        assert_eq!(e.build(&ch), Err(Problem::Name));
        e.name = "GitHub".into();
        assert_eq!(e.build(&ch), Err(Problem::Value));
        assert!(!e.set_cond(0, Field::From, Op::Contains, "@github.com"));
        assert_eq!(e.build(&ch), Err(Problem::Action));
        e.read = true;
        let rule = e.build(&ch).expect("valid");
        assert_eq!(rule.name, "GitHub");
        assert!(rule.mark_read && !rule.flag && rule.move_to.is_none());
        e.conds.clear();
        assert_eq!(e.build(&ch), Err(Problem::Conditions));
    }

    #[test]
    fn conditions_are_added_removed_and_reshaped() {
        let mut e = Editor::blank("n".into());
        e.add_cond();
        assert_eq!(e.conds.len(), 2);
        assert!(e.set_cond(1, Field::Subject, Op::Contains, ""));
        // Typing in the value alone does not change the row's shape.
        assert!(!e.set_cond(1, Field::Subject, Op::Contains, "inv"));
        e.remove_cond(5);
        assert_eq!(e.conds.len(), 2);
        e.remove_cond(1);
        assert_eq!(e.conds.len(), 1);
        for _ in 0..MAX_CONDITIONS * 2 {
            e.add_cond();
        }
        assert_eq!(e.conds.len(), MAX_CONDITIONS);
    }

    #[test]
    fn move_and_delete_exclude_each_other() {
        let ch = choices();
        let mut e = Editor::blank("n".into());
        e.set_cond(0, Field::From, Op::Is, "a@b.c");
        e.choose_delete(true);
        e.choose_move(true);
        assert!(e.move_on && !e.delete);
        e.move_to = 1;
        let rule = e.build(&ch).expect("move");
        assert_eq!(rule.move_to.as_deref(), Some("Lists/Rust"));
        assert!(!rule.trash);
        e.choose_delete(true);
        assert!(!e.move_on);
        assert!(e.build(&ch).expect("delete").trash);
    }

    #[test]
    fn an_open_rule_keeps_its_folder_and_identity() {
        let ch = choices();
        let rule = Rule {
            id: 7,
            name: "Lists".into(),
            move_to: Some("Lists/Rust".into()),
            conditions: vec![cond(Field::ToCc, Op::Contains, "rust")],
            ..Rule::default()
        };
        let e = Editor::open(&rule, &ch);
        assert!(e.is_saved() && e.move_on);
        assert_eq!(e.move_to, 1);
        let back = e.build(&ch).expect("valid");
        assert_eq!(back.id, 7);
        assert_eq!(back.move_to, rule.move_to);
        // A folder that is gone shows no choice.
        let gone = Rule {
            move_to: Some("Gone".into()),
            ..rule
        };
        assert!(!Editor::open(&gone, &ch).move_on);
    }

    #[test]
    fn the_page_saves_toggles_reorders_and_deletes() {
        let mut db = Db::in_memory().expect("cache");
        let seeded = crate::seed::seed(&mut db, 3).expect("seed");
        let mut page = Page::default();
        page.reload(&db);
        assert_eq!(page.account, seeded.account);
        assert!(page.cards.is_empty());
        let ch = page.choices.clone();
        assert!(ch.iter().all(|c| c.path != "INBOX"));

        for name in ["GitHub", "Invoices"] {
            let mut e = Editor::blank(name.into());
            e.set_cond(0, Field::From, Op::Contains, name);
            e.read = true;
            page.save(&db, &e.build(&ch).expect("valid"))
                .expect("saved");
        }
        let names = |p: &Page| p.cards.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&page), ["GitHub", "Invoices"]);
        assert_eq!(
            page.cards[0].parts,
            [
                Part::Cond {
                    field: Field::From,
                    op: Op::Contains,
                    value: "GitHub".into()
                },
                Part::Read
            ]
        );

        page.set_enabled(&db, 0, false);
        assert!(!page.cards[0].enabled);
        page.move_rule(&db, 0, false);
        assert_eq!(names(&page), ["Invoices", "GitHub"]);
        // GitHub is off now: only the enabled rule is in the script.
        let sieve = page.sieve(&db);
        assert!(sieve.contains("# Invoices") && !sieve.contains("# GitHub"));

        let id = page.id_at(1).expect("second");
        assert_eq!(page.name_of(id), "GitHub");
        page.delete(&db, id).expect("deleted");
        assert_eq!(names(&page), ["Invoices"]);
    }
}
