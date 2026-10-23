//! Local mail rules: an account's ordered list of "if these conditions,
//! do these actions" for new mail in its Inbox (run by the sync worker,
//! see `mail::rules`) and for a folder the user picks (Run now).
//!
//! Conditions look only at header fields the cache also keeps (sender,
//! recipients, subject, the attachment heuristic), so a rule decides the
//! same at arrival and on cached rows. Matching ignores case and accents
//! (`crate::fold`). No action sends mail: move, delete (to Trash), mark as
//! read and mark as important are all there is.
//!
//! Rule values are typed by the user and header text comes from senders,
//! so this module denies the panicking lints (as `mail::ical` does) and
//! enforces the caps below.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use crate::fold::fold;
use crate::mail::smtp::{mailbox_parts, split_addresses};

/// Rules kept per account.
pub const MAX_RULES: usize = 100;
/// Conditions kept per rule.
pub const MAX_CONDITIONS: usize = 20;
/// Characters kept of a condition's value.
pub const MAX_VALUE: usize = 200;
/// Characters kept of a rule's name.
pub const MAX_NAME: usize = 200;
/// Addresses read of a To or Cc field.
const MAX_ADDRESSES: usize = 200;

/// What a condition looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The sender's name or address.
    From,
    /// Each To and Cc address and name.
    ToCc,
    Subject,
    /// The message has an attachment (no value).
    HasAttachment,
}

impl Field {
    pub const ALL: [Field; 4] = [
        Field::From,
        Field::ToCc,
        Field::Subject,
        Field::HasAttachment,
    ];

    pub fn code(self) -> i64 {
        match self {
            Field::From => 0,
            Field::ToCc => 1,
            Field::Subject => 2,
            Field::HasAttachment => 3,
        }
    }

    pub fn from_code(code: i64) -> Option<Self> {
        Field::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// How a condition compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Contains,
    NotContains,
    Is,
    StartsWith,
    EndsWith,
}

impl Op {
    pub const ALL: [Op; 5] = [
        Op::Contains,
        Op::NotContains,
        Op::Is,
        Op::StartsWith,
        Op::EndsWith,
    ];

    pub fn code(self) -> i64 {
        match self {
            Op::Contains => 0,
            Op::NotContains => 1,
            Op::Is => 2,
            Op::StartsWith => 3,
            Op::EndsWith => 4,
        }
    }

    pub fn from_code(code: i64) -> Option<Self> {
        Op::ALL.into_iter().find(|o| o.code() == code)
    }

    /// One folded candidate against a folded needle (the positive form for
    /// "does not contain").
    fn hit(self, candidate: &str, needle: &str) -> bool {
        match self {
            Op::Contains | Op::NotContains => candidate.contains(needle),
            Op::Is => candidate == needle,
            Op::StartsWith => candidate.starts_with(needle),
            Op::EndsWith => candidate.ends_with(needle),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

/// A rule as stored and edited.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rule {
    /// 0 for a rule not saved yet.
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    /// Any condition is enough (else all must match).
    pub match_any: bool,
    /// No later rule runs on a message this one matched.
    pub stop: bool,
    /// The destination's server path.
    pub move_to: Option<String>,
    /// Delete: move to the account's Trash.
    pub trash: bool,
    pub mark_read: bool,
    pub flag: bool,
    /// Its destination is gone from the server: the rule waits until the
    /// user picks another folder.
    pub paused: bool,
    pub conditions: Vec<Condition>,
}

/// Why a rule can't be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    NoConditions,
    NoActions,
    /// A condition other than Has attachment has an empty value.
    EmptyValue,
    /// Move and Delete together.
    MoveAndDelete,
    TooMany,
}

impl Rule {
    /// Whether the rule can be saved as it is.
    pub fn check(&self) -> Result<(), Invalid> {
        if self.conditions.is_empty() {
            return Err(Invalid::NoConditions);
        }
        if self.conditions.len() > MAX_CONDITIONS {
            return Err(Invalid::TooMany);
        }
        if self
            .conditions
            .iter()
            .any(|c| c.field != Field::HasAttachment && c.value.trim().is_empty())
        {
            return Err(Invalid::EmptyValue);
        }
        if self.move_to.is_some() && self.trash {
            return Err(Invalid::MoveAndDelete);
        }
        if self.move_to.is_none() && !self.trash && !self.mark_read && !self.flag {
            return Err(Invalid::NoActions);
        }
        Ok(())
    }

    /// The rule with its name and values cut to the caps.
    pub fn capped(mut self) -> Self {
        self.name = cut(&self.name, MAX_NAME);
        self.conditions.truncate(MAX_CONDITIONS);
        for c in &mut self.conditions {
            c.value = cut(c.value.trim(), MAX_VALUE);
        }
        self
    }
}

/// `s` cut to `max` characters.
pub fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Where a message goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dest {
    Folder(String),
    Trash,
}

/// What the rules decided for one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub mark_read: bool,
    pub flag: bool,
    pub dest: Option<Dest>,
    /// The rules that matched, as indexes into [`Compiled`] (see
    /// [`Compiled::name`]).
    pub matched: Vec<usize>,
}

impl Outcome {
    pub fn any(&self) -> bool {
        !self.matched.is_empty()
    }
}

/// A message's fields as rules see them (from a header just fetched or a
/// cached row).
#[derive(Debug, Clone, Copy, Default)]
pub struct Fields<'a> {
    pub from_name: &'a str,
    pub from_addr: &'a str,
    /// Address fields as cached: `Name <a@x>, b@y`.
    pub to: &'a str,
    pub cc: &'a str,
    pub subject: &'a str,
    pub has_attach: bool,
}

struct CCondition {
    field: Field,
    op: Op,
    needle: String,
}

struct CRule {
    id: i64,
    name: String,
    match_any: bool,
    stop: bool,
    dest: Option<Dest>,
    mark_read: bool,
    flag: bool,
    conditions: Vec<CCondition>,
}

/// An account's enabled rules with their values folded once per pass.
#[derive(Default)]
pub struct Compiled {
    rules: Vec<CRule>,
}

/// The enabled, unpaused and valid rules of `rules`, in order, within the
/// caps. `skip_dest` leaves out a rule that moves to that folder (Run now
/// on the folder itself).
pub fn compile(rules: &[Rule], skip_dest: Option<&str>) -> Compiled {
    let rules = rules
        .iter()
        .take(MAX_RULES)
        .filter(|r| r.enabled && !r.paused && r.check().is_ok())
        .filter(|r| skip_dest.is_none() || r.move_to.as_deref() != skip_dest)
        .map(|r| CRule {
            id: r.id,
            name: r.name.clone(),
            match_any: r.match_any,
            stop: r.stop,
            dest: if r.trash {
                Some(Dest::Trash)
            } else {
                r.move_to.clone().map(Dest::Folder)
            },
            mark_read: r.mark_read,
            flag: r.flag,
            conditions: r
                .conditions
                .iter()
                .take(MAX_CONDITIONS)
                .map(|c| CCondition {
                    field: c.field,
                    op: c.op,
                    needle: fold(&cut(c.value.trim(), MAX_VALUE)).into_owned(),
                })
                .collect(),
        })
        .collect();
    Compiled { rules }
}

impl Compiled {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// The name of matched rule `i` (see [`Outcome::matched`]).
    pub fn name(&self, i: usize) -> &str {
        self.rules.get(i).map_or("", |r| r.name.as_str())
    }

    /// The id of matched rule `i`.
    pub fn id(&self, i: usize) -> i64 {
        self.rules.get(i).map_or(0, |r| r.id)
    }

    /// Whether any rule could move a message (so a Trash is needed).
    pub fn moves(&self) -> bool {
        self.rules.iter().any(|r| r.dest.is_some())
    }
}

/// The folded texts a field offers for one message.
struct Seen {
    from: [String; 2],
    to_cc: Vec<String>,
    subject: String,
}

impl Seen {
    fn new(m: &Fields) -> Self {
        let mut to_cc = Vec::new();
        for list in [m.to, m.cc] {
            for entry in split_addresses(list).iter().take(MAX_ADDRESSES) {
                let (name, addr) = mailbox_parts(entry);
                if !addr.is_empty() {
                    to_cc.push(fold(addr).into_owned());
                }
                if !name.is_empty() {
                    to_cc.push(fold(&name).into_owned());
                }
            }
        }
        Seen {
            from: [
                fold(m.from_name.trim()).into_owned(),
                fold(m.from_addr.trim()).into_owned(),
            ],
            to_cc,
            subject: fold(m.subject.trim()).into_owned(),
        }
    }

    fn matches(&self, c: &CCondition, has_attach: bool) -> bool {
        let candidates: &[String] = match c.field {
            Field::HasAttachment => return has_attach,
            Field::From => &self.from,
            Field::ToCc => &self.to_cc,
            Field::Subject => std::slice::from_ref(&self.subject),
        };
        let hit = candidates
            .iter()
            .filter(|s| !s.is_empty())
            .any(|s| c.op.hit(s, &c.needle));
        // "does not contain": no name or address contains it.
        if c.op == Op::NotContains { !hit } else { hit }
    }
}

/// Run the rules on one message, top to bottom: read and important add up,
/// the first move or delete decides where it goes and ends the run, as
/// does a matching rule with "stop".
pub fn evaluate(compiled: &Compiled, m: &Fields) -> Outcome {
    let mut out = Outcome::default();
    if compiled.rules.is_empty() {
        return out;
    }
    let seen = Seen::new(m);
    for (i, r) in compiled.rules.iter().enumerate() {
        let mut each = r.conditions.iter().map(|c| seen.matches(c, m.has_attach));
        let hit = if r.match_any {
            each.any(|b| b)
        } else {
            each.all(|b| b)
        };
        if !hit || r.conditions.is_empty() {
            continue;
        }
        out.matched.push(i);
        out.mark_read |= r.mark_read;
        out.flag |= r.flag;
        if let Some(dest) = &r.dest {
            out.dest = Some(dest.clone());
            break;
        }
        if r.stop {
            break;
        }
    }
    out
}

/// A Sieve quoted string (RFC 5228) of `s`, `"` and `\` escaped, control
/// characters dropped; with `wild` a `:matches` pattern: `*` and `?` in
/// `s` are escaped and `lead` and `tail` are put around it unescaped.
fn sieve_str(s: &str, wild: bool, lead: &str, tail: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(4));
    out.push('"');
    out.push_str(lead);
    for c in s.chars().filter(|c| !c.is_control()) {
        match c {
            '"' | '\\' => out.push('\\'),
            '*' | '?' if wild => out.push_str("\\\\"),
            _ => {}
        }
        out.push(c);
    }
    out.push_str(tail);
    out.push('"');
    out
}

/// The rules as a Sieve script (RFC 5228 with `fileinto` and the
/// `imap4flags` extension), for the user to paste into a provider's filter
/// box ("Copy as Sieve"); nothing is sent anywhere. Disabled and paused
/// rules are left out. `trash` is the account's Trash path. Sieve compares
/// ignoring ASCII case only, not accents.
pub fn to_sieve(rules: &[Rule], trash: Option<&str>) -> String {
    let rules: Vec<&Rule> = rules
        .iter()
        .take(MAX_RULES)
        .filter(|r| r.enabled && !r.paused && r.check().is_ok())
        .collect();
    let files = rules.iter().any(|r| r.move_to.is_some() || r.trash);
    let flags = rules.iter().any(|r| r.mark_read || r.flag);
    let mut needs = Vec::new();
    if files {
        needs.push("\"fileinto\"");
    }
    if flags {
        needs.push("\"imap4flags\"");
    }
    let mut out = String::new();
    if !needs.is_empty() {
        out.push_str(&format!("require [{}];\n", needs.join(", ")));
    }
    for r in rules {
        let name: String = r.name.chars().filter(|c| !c.is_control()).collect();
        out.push_str(&format!("\n# {name}\n"));
        let tests: Vec<String> = r
            .conditions
            .iter()
            .take(MAX_CONDITIONS)
            .map(|c| {
                let headers = match c.field {
                    Field::From => "\"from\"",
                    Field::ToCc => "[\"to\", \"cc\"]",
                    Field::Subject => "\"subject\"",
                    Field::HasAttachment => {
                        return "header :contains \"content-type\" \"multipart/mixed\"".to_string();
                    }
                };
                let v = cut(c.value.trim(), MAX_VALUE);
                let plain = sieve_str(&v, false, "", "");
                match c.op {
                    Op::Contains => format!("header :contains {headers} {plain}"),
                    Op::NotContains => format!("not header :contains {headers} {plain}"),
                    Op::Is if c.field == Field::Subject => format!("header :is {headers} {plain}"),
                    Op::Is => format!("address :is {headers} {plain}"),
                    Op::StartsWith => {
                        format!("header :matches {headers} {}", sieve_str(&v, true, "", "*"))
                    }
                    Op::EndsWith => {
                        format!("header :matches {headers} {}", sieve_str(&v, true, "*", ""))
                    }
                }
            })
            .collect();
        let join = if r.match_any { "anyof" } else { "allof" };
        out.push_str(&format!("if {join} ({}) {{\n", tests.join(", ")));
        if r.mark_read {
            out.push_str("    addflag \"\\\\Seen\";\n");
        }
        if r.flag {
            out.push_str("    addflag \"\\\\Flagged\";\n");
        }
        let dest = if r.trash { trash } else { r.move_to.as_deref() };
        if let Some(dest) = dest {
            out.push_str(&format!(
                "    fileinto {};\n    stop;\n",
                sieve_str(dest, false, "", "")
            ));
        } else if r.stop {
            out.push_str("    stop;\n");
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn sieve_script_for_the_enabled_rules() {
        let mut gh = Rule {
            name: "GitHub\n".into(),
            enabled: true,
            move_to: Some("Lists/Git\"Hub".into()),
            mark_read: true,
            conditions: vec![cond(Field::From, Op::EndsWith, "@github.com")],
            ..Rule::default()
        };
        let mut spam = gh.clone();
        spam.name = "Spam".into();
        spam.move_to = None;
        spam.mark_read = false;
        spam.trash = true;
        spam.match_any = true;
        spam.conditions = vec![
            cond(Field::Subject, Op::StartsWith, "WIN*"),
            cond(Field::ToCc, Op::Is, "x@y.org"),
            cond(Field::HasAttachment, Op::Contains, ""),
        ];
        let mut off = gh.clone();
        off.enabled = false;
        let s = to_sieve(&[gh.clone(), spam, off], Some("Trash"));
        assert_eq!(
            s,
            "require [\"fileinto\", \"imap4flags\"];\n\
             \n# GitHub\n\
             if allof (header :matches \"from\" \"*@github.com\") {\n    \
             addflag \"\\\\Seen\";\n    fileinto \"Lists/Git\\\"Hub\";\n    stop;\n}\n\
             \n# Spam\n\
             if anyof (header :matches \"subject\" \"WIN\\\\**\", address :is [\"to\", \"cc\"] \"x@y.org\", \
             header :contains \"content-type\" \"multipart/mixed\") {\n    \
             fileinto \"Trash\";\n    stop;\n}\n"
        );
        gh.move_to = None;
        gh.mark_read = false;
        gh.flag = true;
        assert!(to_sieve(&[gh], None).starts_with("require [\"imap4flags\"];"));
        assert_eq!(to_sieve(&[], None), "");
    }

    fn cond(field: Field, op: Op, value: &str) -> Condition {
        Condition {
            field,
            op,
            value: value.into(),
        }
    }

    fn rule(name: &str, conditions: Vec<Condition>) -> Rule {
        Rule {
            name: name.into(),
            enabled: true,
            conditions,
            ..Rule::default()
        }
    }

    fn msg() -> Fields<'static> {
        Fields {
            from_name: "João Ação",
            from_addr: "joao@GitHub.com",
            to: "\"Doe, Jane\" <jane@example.com>, me@example.org",
            cc: "Team <team@lists.example.net>",
            subject: "Your weekly Report",
            has_attach: false,
        }
    }

    fn hits(field: Field, op: Op, value: &str) -> bool {
        let mut r = rule("r", vec![cond(field, op, value)]);
        r.mark_read = true;
        evaluate(&compile(&[r], None), &msg()).any()
    }

    #[test]
    fn operators_fold_case_and_accents() {
        assert!(hits(Field::From, Op::Contains, "acao"));
        assert!(hits(Field::From, Op::EndsWith, "@github.com"));
        assert!(hits(Field::From, Op::Is, "JOAO@github.com"));
        assert!(hits(Field::From, Op::StartsWith, "João"));
        assert!(!hits(Field::From, Op::Is, "github.com"));
        assert!(hits(Field::Subject, Op::Contains, "WEEKLY"));
        assert!(hits(Field::Subject, Op::StartsWith, "your"));
        assert!(hits(Field::Subject, Op::EndsWith, "report"));
        assert!(!hits(Field::Subject, Op::NotContains, "weekly"));
        assert!(hits(Field::Subject, Op::NotContains, "invoice"));
        assert!(!hits(Field::HasAttachment, Op::Contains, ""));
    }

    #[test]
    fn recipients_compare_each_address_and_name() {
        assert!(hits(Field::ToCc, Op::Is, "jane@example.com"));
        assert!(hits(Field::ToCc, Op::Is, "doe, jane"));
        assert!(hits(Field::ToCc, Op::Is, "team@lists.example.net"));
        assert!(!hits(Field::ToCc, Op::Is, "example.com"));
        assert!(hits(Field::ToCc, Op::EndsWith, ".example.net"));
        // No address or name contains it (not: one doesn't).
        assert!(!hits(Field::ToCc, Op::NotContains, "jane"));
        assert!(hits(Field::ToCc, Op::NotContains, "boss"));
    }

    #[test]
    fn all_and_any() {
        let conds = vec![
            cond(Field::From, Op::Contains, "github"),
            cond(Field::Subject, Op::Contains, "invoice"),
        ];
        let mut all = rule("all", conds.clone());
        all.flag = true;
        let mut any = all.clone();
        any.match_any = true;
        assert!(!evaluate(&compile(&[all], None), &msg()).any());
        assert!(evaluate(&compile(&[any], None), &msg()).flag);
    }

    #[test]
    fn first_move_wins_and_stop_ends() {
        let base = || rule("x", vec![cond(Field::From, Op::Contains, "github")]);
        let mut read = base();
        read.mark_read = true;
        let mut mv = base();
        mv.name = "move".into();
        mv.move_to = Some("GitHub".into());
        let mut del = base();
        del.trash = true;
        let mut flag = base();
        flag.flag = true;
        let out = evaluate(
            &compile(&[read.clone(), mv, del, flag.clone()], None),
            &msg(),
        );
        assert!(out.mark_read && !out.flag);
        assert_eq!(out.dest, Some(Dest::Folder("GitHub".into())));
        assert_eq!(out.matched, [0, 1]);

        let mut stop = read;
        stop.stop = true;
        let out = evaluate(&compile(&[stop, flag], None), &msg());
        assert!(out.mark_read && !out.flag && out.dest.is_none());
    }

    #[test]
    fn disabled_paused_invalid_and_skipped_rules_never_run() {
        let mut r = rule("r", vec![cond(Field::From, Op::Contains, "github")]);
        r.move_to = Some("A".into());
        let mut off = r.clone();
        off.enabled = false;
        let mut paused = r.clone();
        paused.paused = true;
        let mut none = r.clone();
        none.move_to = None;
        let mut empty = r.clone();
        empty.conditions[0].value = " ".into();
        let c = compile(&[off, paused, none, empty], None);
        assert!(c.is_empty());
        assert!(compile(std::slice::from_ref(&r), Some("A")).is_empty());
        assert_eq!(compile(&[r], Some("B")).len(), 1);
    }

    #[test]
    fn check_and_caps() {
        let mut r = rule("r", vec![]);
        assert_eq!(r.check(), Err(Invalid::NoConditions));
        r.conditions
            .push(cond(Field::HasAttachment, Op::Contains, ""));
        assert_eq!(r.check(), Err(Invalid::NoActions));
        r.trash = true;
        r.move_to = Some("A".into());
        assert_eq!(r.check(), Err(Invalid::MoveAndDelete));
        r.move_to = None;
        assert_eq!(r.check(), Ok(()));
        r.conditions = vec![cond(Field::Subject, Op::Is, &"é".repeat(500)); 30];
        assert_eq!(r.check(), Err(Invalid::TooMany));
        let r = r.capped();
        assert_eq!(r.conditions.len(), MAX_CONDITIONS);
        assert_eq!(r.conditions[0].value.chars().count(), MAX_VALUE);
        let many: Vec<Rule> = (0..150)
            .map(|_| {
                let mut r = rule("r", vec![cond(Field::From, Op::Contains, "x")]);
                r.flag = true;
                r
            })
            .collect();
        assert_eq!(compile(&many, None).len(), MAX_RULES);
    }

    #[test]
    fn evaluation_is_fast() {
        let rules: Vec<Rule> = (0..50)
            .map(|i| {
                let mut r = rule(
                    "r",
                    (0..5)
                        .map(|j| cond(Field::ToCc, Op::Contains, &format!("needle{i}x{j}")))
                        .collect(),
                );
                r.flag = true;
                r
            })
            .collect();
        let c = compile(&rules, None);
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            assert!(!evaluate(&c, &msg()).any());
        }
        // 5 ms in release; the dev profile is slower, so a wide margin.
        assert!(
            t.elapsed() < std::time::Duration::from_millis(250),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn hostile_text_does_not_panic() {
        let mut r = rule("r", vec![cond(Field::ToCc, Op::EndsWith, "\u{301}")]);
        r.flag = true;
        let c = compile(&[r], None);
        for s in [
            "", "<", "\"", "\"\\", ",,,;;", "<<>>", "\u{301}a", "a <", "é\u{0}",
        ] {
            let m = Fields {
                from_name: s,
                from_addr: s,
                to: s,
                cc: s,
                subject: s,
                has_attach: true,
            };
            evaluate(&c, &m);
        }
    }
}
