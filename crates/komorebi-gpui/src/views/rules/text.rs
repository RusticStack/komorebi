//! Settings > Rules in words (docs/GPUI-I18N.md): each the Slint app's
//! catalog entry (`ui/rules.slint`, `ui/tr.slint`), so the template only
//! gains file names. The page keeps typed values (`rules_page::Part`,
//! `LogKind`, `Problem`) and words them here in `render`.

use komorebi::rules::{Field, Op};
use komorebi::rules_page::{LogKind, LogLine, Part, Problem};
use komorebi_i18n::{tr, trn};

pub fn title() -> String {
    tr!("Rules")
}

/// Above the cards: how many, and that they run top to bottom.
pub fn in_order(n: usize) -> String {
    trn!("{n} rule, in this order", "{n} rules, in this order", n)
}

pub fn intro() -> String {
    tr!(
        "Rules run while Komorebi is open. Mail that arrives while it is closed is sorted the next time it starts."
    )
}

pub fn none_yet() -> String {
    tr!("No rules yet. A rule can file, delete or mark new mail in the Inbox.")
}

pub fn paused() -> String {
    tr!("Folder not found. This rule is paused.")
}

pub fn on() -> String {
    tr!("On")
}

pub fn off() -> String {
    tr!("Off")
}

pub fn edit() -> String {
    tr!("Edit")
}

/// A card's open button, named by its rule (as Settings > Accounts' "Edit
/// {0}").
pub fn edit_named(name: &str) -> String {
    tr!("Edit {0}", name)
}

/// A card's "..." button, named by its rule.
pub fn more_actions_named(name: &str) -> String {
    tr!("More actions for {0}", name)
}

/// A card's drag handle (its name and tooltip), named by its rule.
pub fn drag_to_reorder(name: &str) -> String {
    tr!("Drag to reorder {0}", name)
}

pub fn move_up() -> String {
    tr!("Move up")
}

pub fn move_down() -> String {
    tr!("Move down")
}

pub fn delete_named(name: &str) -> String {
    tr!("Delete {0}", name)
}

pub fn new_rule() -> String {
    tr!("New rule")
}

pub fn copy_sieve() -> String {
    tr!("Copy as Sieve")
}

pub fn copy_sieve_tip() -> String {
    tr!("Copy the rules as a script for your provider\u{2019}s mail filters")
}

pub fn sieve_copied() -> String {
    tr!("Copied as a Sieve script. Paste it into your provider\u{2019}s mail filters.")
}

pub fn activity() -> String {
    tr!("Activity")
}

pub fn clear_activity() -> String {
    tr!("Clear activity")
}

pub fn account() -> String {
    tr!("Account")
}

// ---- One rule's page

pub fn rule_name() -> String {
    tr!("Rule name")
}

pub fn conditions() -> String {
    tr!("Conditions")
}

pub fn all_must_match() -> String {
    tr!("All must match")
}

pub fn any_can_match() -> String {
    tr!("Any can match")
}

pub fn add_condition() -> String {
    tr!("Add condition")
}

pub fn value() -> String {
    tr!("Value")
}

/// Condition `n`'s controls, named by their row so Narrator's list of
/// fields and buttons tells them apart (1 is the first row).
pub fn condition_n(n: usize) -> String {
    tr!("Condition {0}", n)
}

pub fn condition_op_n(n: usize) -> String {
    tr!("Condition {0} comparison", n)
}

pub fn condition_value_n(n: usize) -> String {
    tr!("Condition {0} value", n)
}

pub fn remove_condition_n(n: usize) -> String {
    tr!("Remove condition {0}", n)
}

pub fn do_this() -> String {
    tr!("Do this")
}

pub fn move_to_folder() -> String {
    tr!("Move to folder")
}

pub fn delete() -> String {
    tr!("Delete")
}

pub fn mark_read() -> String {
    tr!("Mark as read")
}

pub fn mark_important() -> String {
    tr!("Mark as important")
}

pub fn stop() -> String {
    tr!("Stop processing more rules")
}

pub fn save() -> String {
    tr!("Save")
}

pub fn cancel() -> String {
    tr!("Cancel")
}

pub fn back() -> String {
    tr!("Back")
}

/// A condition's field, as its select says it.
pub fn field(f: Field) -> String {
    match f {
        Field::From => tr!("rule field" => "From"),
        Field::ToCc => tr!("To or Cc"),
        Field::Subject => tr!("rule field" => "Subject"),
        Field::HasAttachment => tr!("Has attachment"),
    }
}

/// A condition's operator, as its select says it.
pub fn op(o: Op) -> String {
    match o {
        Op::Contains => tr!("contains"),
        Op::NotContains => tr!("does not contain"),
        Op::Is => tr!("rule operator" => "is"),
        Op::StartsWith => tr!("starts with"),
        Op::EndsWith => tr!("ends with"),
    }
}

/// What is wrong with a rule that cannot be saved.
pub fn problem(p: Problem) -> String {
    match p {
        Problem::Name => tr!("Give the rule a name."),
        Problem::Conditions => tr!("Add a condition."),
        Problem::Value => tr!("Each condition needs a value."),
        Problem::Action => tr!("Choose at least one action."),
    }
}

/// A cache failure behind its lead-in sentence (the Slint app's
/// `text::detail`).
pub fn detail(e: &str) -> String {
    tr!("Something went wrong. {0}", e)
}

pub fn delete_question(name: &str) -> String {
    tr!("Delete rule \u{201c}{0}\u{201d}?", name)
}

pub fn delete_question_text() -> String {
    tr!("Mail it already moved stays where it is.")
}

/// One condition as a sentence ("From ends with @github.com").
fn cond(field: Field, op: Op, v: &str) -> String {
    match (field, op) {
        (Field::HasAttachment, _) => tr!("Has an attachment"),
        (Field::From, Op::Contains) => tr!("From contains {0}", v),
        (Field::From, Op::NotContains) => tr!("From does not contain {0}", v),
        (Field::From, Op::Is) => tr!("From is {0}", v),
        (Field::From, Op::StartsWith) => tr!("From starts with {0}", v),
        (Field::From, Op::EndsWith) => tr!("From ends with {0}", v),
        (Field::ToCc, Op::Contains) => tr!("To or Cc contains {0}", v),
        (Field::ToCc, Op::NotContains) => tr!("To or Cc does not contain {0}", v),
        (Field::ToCc, Op::Is) => tr!("To or Cc is {0}", v),
        (Field::ToCc, Op::StartsWith) => tr!("To or Cc starts with {0}", v),
        (Field::ToCc, Op::EndsWith) => tr!("To or Cc ends with {0}", v),
        (Field::Subject, Op::Contains) => tr!("Subject contains {0}", v),
        (Field::Subject, Op::NotContains) => tr!("Subject does not contain {0}", v),
        (Field::Subject, Op::Is) => tr!("Subject is {0}", v),
        (Field::Subject, Op::StartsWith) => tr!("Subject starts with {0}", v),
        (Field::Subject, Op::EndsWith) => tr!("Subject ends with {0}", v),
    }
}

/// A rule's summary: its parts as sentences, joined by middle dots (the
/// Slint app's `words`).
pub fn summary(parts: &[Part]) -> String {
    let sentences: Vec<String> = parts
        .iter()
        .map(|p| match p {
            Part::Any => tr!("Any of these"),
            Part::Cond { field, op, value } => cond(*field, *op, value),
            Part::Move(dest) => tr!("Move to {0}", dest),
            Part::Delete => tr!("Delete"),
            Part::Read => tr!("Mark as read"),
            Part::Flag => tr!("Mark as important"),
            Part::Stop => tr!("Stop processing more rules"),
        })
        .collect();
    sentences.join(" \u{b7} ")
}

/// An activity line's title: the rule and when.
pub fn log_title(line: &LogLine) -> String {
    let when = komorebi::format::full_date(komorebi_i18n::current(), line.at);
    tr!("{0}, {1}", line.rule_name, when)
}

/// What an automatic run did.
pub fn log_text(line: &LogLine) -> String {
    let (s, f) = (line.subject.as_str(), line.from.as_str());
    match &line.kind {
        LogKind::Failed => tr!(
            "Couldn\u{2019}t act on \u{201c}{0}\u{201d} from {1}. It stayed in the Inbox.",
            s,
            f
        ),
        LogKind::Move(dest) => tr!("Moved \u{201c}{0}\u{201d} from {1} to {2}.", s, f, dest),
        LogKind::Delete => tr!("Deleted \u{201c}{0}\u{201d} from {1}.", s, f),
        LogKind::Read => tr!("Marked \u{201c}{0}\u{201d} from {1} as read.", s, f),
        LogKind::Flag => tr!("Marked \u{201c}{0}\u{201d} from {1} as important.", s, f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    fn parts() -> Vec<Part> {
        vec![
            Part::Cond {
                field: Field::From,
                op: Op::EndsWith,
                value: "@github.com".into(),
            },
            Part::Read,
        ]
    }

    /// The Slint view test's summary (`rules_are_saved_toggled_reordered_and_deleted_in_settings`):
    /// "From ends with @github.com · Mark as read".
    #[test]
    fn a_summary_reads_as_the_slint_app_says_it() {
        with_lang(Lang::En, || {
            assert_eq!(
                summary(&parts()),
                "From ends with @github.com \u{b7} Mark as read"
            );
            assert_eq!(problem(Problem::Value), "Each condition needs a value.");
        });
        with_lang(Lang::PtPt, || {
            let s = summary(&parts());
            assert!(s.contains("@github.com") && s.contains('\u{b7}'), "{s}");
            assert_ne!(s, "From ends with @github.com \u{b7} Mark as read");
            assert_ne!(problem(Problem::Value), "Each condition needs a value.");
        });
    }

    /// Repeated controls carry their row or their rule in the name
    /// (stage D: Narrator's list of fields and buttons tells them apart).
    #[test]
    fn repeated_controls_are_named_by_their_row_or_rule() {
        with_lang(Lang::En, || {
            assert_eq!(condition_n(2), "Condition 2");
            assert_eq!(condition_op_n(2), "Condition 2 comparison");
            assert_eq!(condition_value_n(2), "Condition 2 value");
            assert_eq!(remove_condition_n(2), "Remove condition 2");
            assert_eq!(edit_named("GitHub"), "Edit GitHub");
            assert_eq!(more_actions_named("GitHub"), "More actions for GitHub");
            assert_eq!(drag_to_reorder("GitHub"), "Drag to reorder GitHub");
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(condition_n(2), "Condi\u{e7}\u{e3}o 2");
            assert_eq!(
                condition_op_n(2),
                "Compara\u{e7}\u{e3}o da condi\u{e7}\u{e3}o 2"
            );
            assert_eq!(condition_value_n(2), "Valor da condi\u{e7}\u{e3}o 2");
            assert_eq!(remove_condition_n(2), "Remover condi\u{e7}\u{e3}o 2");
            assert_eq!(edit_named("GitHub"), "Editar GitHub");
            assert_eq!(
                more_actions_named("GitHub"),
                "Mais a\u{e7}\u{f5}es para GitHub"
            );
            assert_eq!(drag_to_reorder("GitHub"), "Arraste para reordenar GitHub");
        });
    }

    #[test]
    fn the_count_says_the_order_matters() {
        with_lang(Lang::En, || {
            assert_eq!(in_order(1), "1 rule, in this order");
            assert_eq!(in_order(4), "4 rules, in this order");
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(in_order(1), "1 regra, por esta ordem");
            assert_eq!(in_order(4), "4 regras, por esta ordem");
        });
    }

    #[test]
    fn a_log_line_names_the_message_and_where_it_went() {
        let line = LogLine {
            rule_name: "GitHub".into(),
            at: 1_760_000_000,
            subject: "New pull request".into(),
            from: "notifications@github.com".into(),
            kind: LogKind::Move("Archive".into()),
        };
        with_lang(Lang::En, || {
            assert_eq!(
                log_text(&line),
                "Moved \u{201c}New pull request\u{201d} from notifications@github.com to Archive."
            );
            assert!(log_title(&line).starts_with("GitHub, "));
        });
    }
}
