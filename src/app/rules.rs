//! Settings > Rules (`ui/rules.slint`, `komorebi::rules`): each account's
//! rules as cards (on and off, order, edit, delete), one rule's page, and
//! the activity of the sync worker's automatic runs. Nothing here runs at
//! start: the page reads the rules when Settings opens.

use super::*;
use crate::{CondRow, LogRow, RuleRow, RulesUi};
use komorebi::db::{LogAction, LogEntry, LogOutcome};
use komorebi::role::Role;
use komorebi::rules::{Condition, Field, Invalid, Op, Rule};
use std::cell::RefCell;

/// Activity rows shown.
const LOG_SHOWN: usize = 50;

/// The page's state (kept here, so the controller needs nothing in `App`).
#[derive(Default)]
struct Page {
    /// The accounts in the picker, and the one shown.
    accounts: Vec<i64>,
    account: i64,
    rules: Vec<Rule>,
    /// The rule being edited (id 0: a new one) and its conditions.
    editing: Option<Rule>,
    conds: Vec<Condition>,
    /// The server paths of the Move choices.
    folders: Vec<String>,
}

thread_local! {
    static PAGE: RefCell<Page> = RefCell::default();
}

fn summary(r: &Rule, display: &dyn Fn(&str) -> String) -> String {
    let mut parts: Vec<String> = Vec::new();
    if r.match_any && r.conditions.len() > 1 {
        parts.push(t!(invoke_rule_any));
    }
    for c in &r.conditions {
        parts.push(t!(
            invoke_rule_cond,
            c.field.code() as i32,
            c.op.code() as i32,
            c.value.as_str().into()
        ));
    }
    if let Some(dest) = &r.move_to {
        parts.push(t!(invoke_rule_move, display(dest).into()));
    }
    if r.trash {
        parts.push(t!(invoke_rule_delete));
    }
    if r.mark_read {
        parts.push(t!(invoke_rule_read));
    }
    if r.flag {
        parts.push(t!(invoke_rule_flag));
    }
    if r.stop {
        parts.push(t!(invoke_rule_stop));
    }
    parts.join(" \u{b7} ")
}

fn cond_row(c: &Condition) -> CondRow {
    CondRow {
        field: c.field.code() as i32,
        op: c.op.code() as i32,
        value: c.value.as_str().into(),
    }
}

impl App {
    /// Read the shown account's rules and activity again.
    pub(super) fn reload_rules(&mut self) {
        let accounts = self.db.accounts().unwrap_or_default();
        let folders = self.db.folders().unwrap_or_default();
        let ui = self.ui();
        let g = ui.global::<RulesUi>();
        PAGE.with_borrow_mut(|p| {
            p.accounts = accounts.iter().map(|a| a.id).collect();
            if !p.accounts.contains(&p.account) {
                p.account = p.accounts.first().copied().unwrap_or(0);
            }
            let names: Vec<SharedString> =
                accounts.iter().map(|a| a.email.as_str().into()).collect();
            g.set_accounts(ModelRc::new(VecModel::from(names)));
            g.set_account(p.accounts.iter().position(|&a| a == p.account).unwrap_or(0) as i32);
            p.rules = self.db.rules_for(p.account).unwrap_or_default();
            let display = |path: &str| {
                folders
                    .iter()
                    .find(|f| f.account_id == p.account && f.name == path)
                    .map_or_else(|| path.to_string(), |f| f.display.clone())
            };
            let rows: Vec<RuleRow> = p
                .rules
                .iter()
                .map(|r| RuleRow {
                    name: r.name.as_str().into(),
                    summary: summary(r, &display).into(),
                    enabled: r.enabled,
                    paused: r.paused,
                })
                .collect();
            g.set_items(ModelRc::new(VecModel::from(rows)));
            let log = self.db.rule_log(p.account, LOG_SHOWN).unwrap_or_default();
            let rows: Vec<LogRow> = log
                .iter()
                .map(|e| {
                    let (subject, from) = (e.subject.as_str().into(), e.from_addr.as_str().into());
                    let text = if e.outcome != LogOutcome::Done {
                        t!(invoke_rule_log_failed, subject, from)
                    } else {
                        match e.action {
                            LogAction::Move => t!(
                                invoke_rule_log_move,
                                subject,
                                from,
                                display(e.dest.as_deref().unwrap_or_default()).into()
                            ),
                            LogAction::Delete => t!(invoke_rule_log_delete, subject, from),
                            LogAction::MarkRead => t!(invoke_rule_log_read, subject, from),
                            LogAction::Flag => t!(invoke_rule_log_flag, subject, from),
                        }
                    };
                    LogRow {
                        title: t!(
                            invoke_rule_log_title,
                            e.rule_name.as_str().into(),
                            format::full_date(self.lang, e.at).into()
                        )
                        .into(),
                        text: text.into(),
                        failed: e.outcome != LogOutcome::Done,
                    }
                })
                .collect();
            g.set_activity(ModelRc::new(VecModel::from(rows)));
            // The Move choices: the account's folders but its Inbox.
            let choices: Vec<_> = folders
                .iter()
                .filter(|f| f.account_id == p.account && f.role != Role::Inbox)
                .collect();
            p.folders = choices.iter().map(|f| f.name.clone()).collect();
            let names: Vec<SharedString> = choices
                .iter()
                .map(|f| {
                    let pad = "   ".repeat(usize::try_from(f.depth).unwrap_or(0));
                    SharedString::from(format!("{pad}{}", f.display))
                })
                .collect();
            g.set_folders(ModelRc::new(VecModel::from(names)));
        });
    }

    /// Open a rule's page: rule `index` of the list, or a new one.
    pub(super) fn open_rule(&mut self, index: Option<usize>) {
        let ui = self.ui();
        let g = ui.global::<RulesUi>();
        PAGE.with_borrow_mut(|p| {
            let rule = index
                .and_then(|i| p.rules.get(i).cloned())
                .unwrap_or_else(|| Rule {
                    name: t!(invoke_rule_new_name),
                    enabled: true,
                    conditions: vec![Condition {
                        field: Field::From,
                        op: Op::Contains,
                        value: String::new(),
                    }],
                    ..Rule::default()
                });
            p.conds = rule.conditions.clone();
            g.set_name(rule.name.as_str().into());
            g.set_match_any(i32::from(rule.match_any));
            g.set_conds(ModelRc::new(VecModel::from(
                p.conds.iter().map(cond_row).collect::<Vec<_>>(),
            )));
            let at = rule
                .move_to
                .as_ref()
                .and_then(|m| p.folders.iter().position(|f| f == m));
            g.set_act_move(at.is_some());
            g.set_move_to(at.unwrap_or(0) as i32);
            g.set_act_delete(rule.trash);
            g.set_act_read(rule.mark_read);
            g.set_act_flag(rule.flag);
            g.set_stop(rule.stop);
            g.set_error("".into());
            p.editing = Some(rule);
        });
        g.set_editing(true);
    }

    fn rule_conds_changed(&self) {
        let ui = self.ui();
        let g = ui.global::<RulesUi>();
        PAGE.with_borrow(|p| {
            g.set_conds(ModelRc::new(VecModel::from(
                p.conds.iter().map(cond_row).collect::<Vec<_>>(),
            )));
        });
    }

    /// Save the open rule; it stays open with the reason when it can't be.
    pub(super) fn save_rule(&mut self) {
        let ui = self.ui();
        let g = ui.global::<RulesUi>();
        let result = PAGE.with_borrow(|p| {
            let mut rule = p.editing.clone()?;
            rule.name = g.get_name().trim().to_string();
            rule.match_any = g.get_match_any() == 1;
            rule.conditions = p.conds.clone();
            rule.move_to = if g.get_act_move() {
                usize::try_from(g.get_move_to())
                    .ok()
                    .and_then(|i| p.folders.get(i).cloned())
            } else {
                None
            };
            rule.trash = g.get_act_delete() && rule.move_to.is_none();
            rule.mark_read = g.get_act_read();
            rule.flag = g.get_act_flag();
            rule.stop = g.get_stop();
            let error = if rule.name.is_empty() {
                Some(t!(invoke_rule_err_name))
            } else {
                match rule.check() {
                    Ok(()) => None,
                    Err(Invalid::NoConditions | Invalid::TooMany) => {
                        Some(t!(invoke_rule_err_conds))
                    }
                    Err(Invalid::EmptyValue) => Some(t!(invoke_rule_err_value)),
                    Err(Invalid::NoActions | Invalid::MoveAndDelete) => {
                        Some(t!(invoke_rule_err_action))
                    }
                }
            };
            Some((p.account, rule, error))
        });
        let Some((account, rule, error)) = result else {
            return;
        };
        if let Some(error) = error {
            g.set_error(error.into());
            return;
        }
        match self.db.save_rule(account, &rule) {
            Ok(_) => {
                g.set_editing(false);
                PAGE.with_borrow_mut(|p| p.editing = None);
                self.reload_rules();
            }
            Err(e) => g.set_error(text::detail(&e).into()),
        }
    }

    pub(super) fn rule_question(&self, id: i64) -> (String, String, String) {
        let name = PAGE.with_borrow(|p| {
            p.rules
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.name.clone())
                .unwrap_or_default()
        });
        (
            t!(invoke_q_rule_delete_title, name.into()),
            t!(invoke_q_rule_delete_text),
            t!(invoke_rule_delete),
        )
    }

    pub(super) fn delete_rule_now(&mut self, id: i64) {
        if let Err(e) = self.db.delete_rule(id) {
            self.set_status(&text::detail(&e), true);
        }
        self.reload_rules();
    }

    /// Settings > Rules with made-up rules (snapshots, offline only: they
    /// are written to the throwaway cache), and the first one's page.
    pub(super) fn sample_rules(&mut self, edit: bool) {
        let account = self
            .db
            .accounts()
            .ok()
            .and_then(|a| a.first().map(|a| a.id));
        if let Some(account) = account
            && komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
            && self.db.rules_for(account).is_ok_and(|r| r.is_empty())
        {
            let folder = self
                .db
                .folders()
                .unwrap_or_default()
                .into_iter()
                .find(|f| f.account_id == account && f.role == Role::Archive)
                .map_or_else(|| "Archive".to_string(), |f| f.name);
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
                self.db.save_rule(account, r).ok();
            }
            // A rule whose folder is gone, paused as a sync would.
            let gone = Rule {
                name: "Old project".into(),
                enabled: true,
                move_to: Some("Projects/2019".into()),
                conditions: vec![cond(Field::Subject, Op::StartsWith, "[proj]")],
                ..Rule::default()
            };
            self.db.save_rule(account, &gone).ok();
            let names: std::collections::HashSet<String> = self
                .db
                .folders()
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.account_id == account)
                .map(|f| f.name)
                .collect();
            self.db.pause_rules_missing(account, &names).ok();
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
            self.db
                .log_rule_actions(
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
        let ui = self.ui();
        ui.set_settings_section(2);
        ui.set_show_settings(true);
        self.reload_rules();
        if edit {
            self.open_rule(Some(1));
        }
    }
}

pub(super) fn wire(ui: &AppWindow) {
    let g = ui.global::<RulesUi>();
    g.on_pick_account(|i| {
        with_app(|a| {
            PAGE.with_borrow_mut(|p| {
                if let Some(id) = usize::try_from(i).ok().and_then(|i| p.accounts.get(i)) {
                    p.account = *id;
                }
            });
            a.reload_rules();
        });
    });
    g.on_add(|| {
        with_app(|a| a.open_rule(None));
    });
    g.on_edit(|i| {
        with_app(|a| a.open_rule(usize::try_from(i).ok()));
    });
    g.on_delete(|i| {
        with_app(|a| {
            let id = PAGE.with_borrow(|p| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| p.rules.get(i))
                    .map(|r| r.id)
            });
            if let Some(id) = id {
                a.ask(Question::DeleteRule(id));
            }
        });
    });
    g.on_toggle(|i, on| {
        with_app(|a| {
            let id = PAGE.with_borrow(|p| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| p.rules.get(i))
                    .map(|r| r.id)
            });
            if let Some(id) = id {
                a.db.set_rule_enabled(id, on).ok();
            }
            a.reload_rules();
        });
    });
    g.on_move(|i, up| {
        with_app(|a| {
            let id = PAGE.with_borrow(|p| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| p.rules.get(i))
                    .map(|r| r.id)
            });
            if let Some(id) = id {
                a.db.move_rule(id, up).ok();
            }
            a.reload_rules();
        });
    });
    g.on_copy_sieve(|| {
        with_app(|a| {
            let account = PAGE.with_borrow(|p| p.account);
            let rules = a.db.rules_for(account).unwrap_or_default();
            let trash =
                a.db.folder_by_role(account, Role::Trash)
                    .ok()
                    .flatten()
                    .map(|f| f.name);
            let script = komorebi::rules::to_sieve(&rules, trash.as_deref());
            a.ui().invoke_r_copy_text(script.into());
            a.set_status(&t!(invoke_rule_sieve_copied), false);
        });
    });
    g.on_clear_activity(|| {
        with_app(|a| {
            let account = PAGE.with_borrow(|p| p.account);
            a.db.clear_rule_log(account).ok();
            a.reload_rules();
        });
    });
    g.on_add_cond(|| {
        with_app(|a| {
            PAGE.with_borrow_mut(|p| {
                if p.conds.len() < komorebi::rules::MAX_CONDITIONS {
                    p.conds.push(Condition {
                        field: Field::From,
                        op: Op::Contains,
                        value: String::new(),
                    });
                }
            });
            a.rule_conds_changed();
        });
    });
    g.on_remove_cond(|i| {
        with_app(|a| {
            PAGE.with_borrow_mut(|p| {
                if let Ok(i) = usize::try_from(i)
                    && i < p.conds.len()
                {
                    p.conds.remove(i);
                }
            });
            a.rule_conds_changed();
        });
    });
    g.on_set_cond(|i, field, op, value| {
        with_app(|a| {
            let reshown = PAGE.with_borrow_mut(|p| {
                let c = usize::try_from(i).ok().and_then(|i| p.conds.get_mut(i))?;
                let field = Field::from_code(i64::from(field)).unwrap_or(Field::From);
                let op = Op::from_code(i64::from(op)).unwrap_or(Op::Contains);
                let shape = c.field != field || c.op != op;
                c.field = field;
                c.op = op;
                c.value = value.to_string();
                Some(shape)
            });
            // Only a new field or operator changes what the row shows; the
            // value stays the text box's own while it is typed.
            if reshown == Some(true) {
                a.rule_conds_changed();
            }
        });
    });
    g.on_save(|| {
        with_app(|a| a.save_rule());
    });
    g.on_cancel(|| {
        with_app(|a| {
            PAGE.with_borrow_mut(|p| p.editing = None);
            a.ui().global::<RulesUi>().set_editing(false);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reads_conditions_then_actions() {
        let r = Rule {
            name: "r".into(),
            match_any: true,
            move_to: Some("INBOX/Lists".into()),
            mark_read: true,
            conditions: vec![
                Condition {
                    field: Field::From,
                    op: Op::EndsWith,
                    value: "@github.com".into(),
                },
                Condition {
                    field: Field::HasAttachment,
                    op: Op::Contains,
                    value: String::new(),
                },
            ],
            ..Rule::default()
        };
        // Without a window the sentences are empty; the joins still show.
        let s = summary(&r, &|p| p.rsplit('/').next().unwrap_or(p).to_string());
        assert_eq!(s.matches('\u{b7}').count(), 4);
    }
}
