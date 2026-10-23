//! Rules on new Inbox mail, in the sync worker (`crate::rules` decides).
//!
//! A pass starts from the Inbox's watermark: rules act only on UIDs at or
//! above it, under the same UIDVALIDITY. With no mark yet (a first sync,
//! a new account, a rebuilt cache) or a changed UIDVALIDITY the mark is set
//! to the SELECT's UIDNEXT and nothing runs, so old mail is never swept up.
//! The mark advances after every successful pass, rules or not.
//!
//! Rules are evaluated before the headers are stored: a header a rule moves
//! is never inserted in the Inbox (so it never shows there or raises a
//! notification), and one marked read or important is stored with those
//! flags already set. After the insert, one STORE per flag and one MOVE per
//! destination go to the server on the sync connection (the Inbox is still
//! selected). A failed MOVE puts its headers back in the Inbox; a failed
//! STORE is corrected by the next flag sync. Each action is written to the
//! activity log (`db::LogEntry`); the diagnostics log gets counts only.

use super::imap::{self, Imap};
use crate::db::{self, Db, Folder, LogAction, LogEntry, LogOutcome, NewHeader};
use crate::role::Role;
use crate::rules::{self, Compiled, Dest, Fields};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

/// One Inbox pass of the rules.
pub(super) struct RulePass {
    /// The first UID the rules act on.
    from: u32,
    compiled: Compiled,
    /// Headers to move, by destination server path.
    moving: BTreeMap<String, Vec<(NewHeader, String)>>,
    /// Headers whose destination (the Trash) the account doesn't have.
    homeless: Vec<(NewHeader, String)>,
    /// UIDs to mark read and to flag, with the matched rule's name and the
    /// message's log fields.
    seen: Vec<(u32, LogEntry)>,
    flagged: Vec<(u32, LogEntry)>,
    trash: Option<String>,
    /// Destinations that received mail in this pass.
    pub moved_into: HashSet<String>,
    /// Messages moved off the server's Inbox in this pass.
    pub removed: u32,
    matched: usize,
    failed: usize,
    log: Vec<LogEntry>,
    /// What the written activity rows did (for the diagnostics counts).
    done: Vec<(LogAction, LogOutcome)>,
    account_id: i64,
    started: Instant,
}

impl RulePass {
    /// The pass for folder `f` as just selected (`sel`): none unless it is
    /// the Inbox with a mark of the same UIDVALIDITY. A missing or stale
    /// mark is set to the SELECT's UIDNEXT here.
    pub(super) fn start(db: &Db, f: &Folder, sel: &imap::Selected) -> Result<Option<Self>> {
        if f.role != Role::Inbox {
            return Ok(None);
        }
        let from = match db.rule_mark(f.id)? {
            Some((v, next)) if v == sel.uidvalidity => next,
            _ => {
                db.set_rule_mark(f.id, sel.uidvalidity, sel.uid_next)?;
                return Ok(None);
            }
        };
        let compiled = rules::compile(&db.rules_for(f.account_id)?, None);
        let trash = if compiled.moves() {
            db.folder_by_role(f.account_id, Role::Trash)?
                .map(|t| t.name)
        } else {
            None
        };
        Ok(Some(Self {
            from,
            compiled,
            moving: BTreeMap::new(),
            homeless: Vec::new(),
            seen: Vec::new(),
            flagged: Vec::new(),
            trash,
            moved_into: HashSet::new(),
            removed: 0,
            matched: 0,
            failed: 0,
            log: Vec::new(),
            done: Vec::new(),
            account_id: f.account_id,
            started: Instant::now(),
        }))
    }

    /// Run the rules on the new headers above the mark: a header to move
    /// leaves `headers`; one to mark read or important gets those flags.
    pub(super) fn split(&mut self, headers: &mut Vec<NewHeader>) {
        if self.compiled.is_empty() {
            return;
        }
        let mut keep = Vec::with_capacity(headers.len());
        for mut h in headers.drain(..) {
            if h.uid < self.from {
                keep.push(h);
                continue;
            }
            let out = rules::evaluate(
                &self.compiled,
                &Fields {
                    from_name: &h.from_name,
                    from_addr: &h.from_addr,
                    to: &h.to,
                    cc: &h.cc,
                    subject: &h.subject,
                    has_attach: h.has_attach,
                },
            );
            if !out.any() {
                keep.push(h);
                continue;
            }
            self.matched += 1;
            let first = out.matched.first().map_or("", |&i| self.compiled.name(i));
            let last = out.matched.last().map_or("", |&i| self.compiled.name(i));
            let entry = |action, dest: Option<String>, name: &str| LogEntry {
                id: 0,
                at: 0,
                rule_name: name.to_string(),
                msgid: h.msgid.clone(),
                subject: h.subject.clone(),
                from_addr: h.from_addr.clone(),
                action,
                dest,
                outcome: LogOutcome::Done,
            };
            if out.mark_read && !h.seen {
                self.seen
                    .push((h.uid, entry(LogAction::MarkRead, None, first)));
            }
            if out.flag && !h.flagged {
                self.flagged
                    .push((h.uid, entry(LogAction::Flag, None, first)));
            }
            // Stored as the rules leave it (also if it is put back after a
            // failed move).
            h.seen |= out.mark_read;
            h.flagged |= out.flag;
            match out.dest {
                Some(Dest::Folder(path)) => {
                    self.moving
                        .entry(path)
                        .or_default()
                        .push((h, last.to_string()));
                }
                Some(Dest::Trash) => match &self.trash {
                    Some(t) => self
                        .moving
                        .entry(t.clone())
                        .or_default()
                        .push((h, last.to_string())),
                    None => self.homeless.push((h, last.to_string())),
                },
                None => keep.push(h),
            }
        }
        *headers = keep;
    }

    /// After the kept headers are stored: the flags, then the moves, on the
    /// server. Returns the headers whose move failed, to store in the Inbox
    /// after all. `remote` (the full path's flags, read before) learns the
    /// flags set, so its flag pass doesn't undo them in the cache.
    pub(super) fn act(
        &mut self,
        imap: &mut Imap,
        f: &Folder,
        uidvalidity: u32,
        mut remote: Option<&mut HashMap<u32, u8>>,
    ) -> Vec<NewHeader> {
        let at = (f.name.as_str(), uidvalidity);
        for (list, flag, bit) in [
            (std::mem::take(&mut self.seen), "\\Seen", db::SEEN),
            (std::mem::take(&mut self.flagged), "\\Flagged", db::FLAGGED),
        ] {
            if list.is_empty() {
                continue;
            }
            let uids: Vec<u32> = list.iter().map(|(u, _)| *u).collect();
            let ok = imap.set_flags(at, &uids, flag, true).is_ok();
            if ok && let Some(remote) = remote.as_deref_mut() {
                for u in &uids {
                    if let Some(bits) = remote.get_mut(u) {
                        *bits |= bit;
                    }
                }
            }
            if !ok {
                self.failed += list.len();
            }
            self.log.extend(list.into_iter().map(|(_, mut e)| {
                if !ok {
                    e.outcome = LogOutcome::Failed;
                }
                e
            }));
        }
        let trash = self.trash.clone();
        let mut back = Vec::new();
        for (dest, list) in std::mem::take(&mut self.moving) {
            let uids: Vec<u32> = list.iter().map(|(h, _)| h.uid).collect();
            let result = imap.move_messages(at, &uids, &dest);
            let action = if trash.as_deref() == Some(dest.as_str()) {
                LogAction::Delete
            } else {
                LogAction::Move
            };
            let ok = result.is_ok();
            if let Ok(gone) = result {
                self.moved_into.insert(dest.clone());
                if gone == imap::Expunged::Removed {
                    self.removed += uids.len() as u32;
                }
            } else {
                self.failed += list.len();
            }
            for (h, rule) in list {
                self.log.push(LogEntry {
                    id: 0,
                    at: 0,
                    rule_name: rule,
                    msgid: h.msgid.clone(),
                    subject: h.subject.clone(),
                    from_addr: h.from_addr.clone(),
                    action,
                    dest: Some(dest.clone()),
                    outcome: if ok {
                        LogOutcome::Done
                    } else {
                        LogOutcome::Failed
                    },
                });
                if let Some(bits) = remote.as_deref_mut()
                    && ok
                {
                    bits.remove(&h.uid);
                }
                if !ok {
                    back.push(h);
                }
            }
        }
        for (h, rule) in std::mem::take(&mut self.homeless) {
            self.failed += 1;
            self.log.push(LogEntry {
                id: 0,
                at: 0,
                rule_name: rule,
                msgid: h.msgid.clone(),
                subject: h.subject.clone(),
                from_addr: h.from_addr.clone(),
                action: LogAction::Delete,
                dest: None,
                outcome: LogOutcome::Failed,
            });
            back.push(h);
        }
        back
    }

    /// Write the activity of the actions sent so far (after each batch, so
    /// a pass that fails later still records what it did on the server).
    pub(super) fn flush_log(&mut self, db: &Db) {
        let log = std::mem::take(&mut self.log);
        // The activity is a convenience; the mail itself is fine.
        if db.log_rule_actions(self.account_id, &log).is_err() {
            crate::diag::record(
                self.account_id,
                "RULES LOG",
                "failed",
                self.started.elapsed(),
            );
        }
        self.done
            .extend(log.into_iter().map(|e| (e.action, e.outcome)));
    }

    /// The pass went through: write the activity and advance the mark to
    /// the SELECT's UIDNEXT (a header fetched above it was evaluated and is
    /// cached or moved, so it is never fetched again).
    pub(super) fn finish(mut self, db: &mut Db, f: &Folder, sel: &imap::Selected) {
        let next = sel.uid_next.max(self.from);
        db.set_rule_mark(f.id, sel.uidvalidity, next).ok();
        self.flush_log(db);
        if self.matched > 0 || self.failed > 0 {
            let moved = self
                .done
                .iter()
                .filter(|(action, outcome)| {
                    matches!(action, LogAction::Move | LogAction::Delete)
                        && *outcome == LogOutcome::Done
                })
                .count();
            crate::diag::record(
                f.account_id,
                "RULES",
                &format!(
                    "{} matched, {moved} moved, {} failed",
                    self.matched, self.failed
                ),
                self.started.elapsed(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake_imapd::{FakeImapd, MailBox, Profile};
    use super::super::tests::worker_with;
    use super::super::{Job, Worker};
    use crate::db::{Db, LogAction, LogOutcome, NewAccount};
    use crate::mail::imap::Imap;
    use crate::providers::{Security, Server};
    use crate::role::Role;
    use crate::rules::{Condition, Field, Op, Rule};

    fn raw(n: u32, from: &str, subject: &str) -> Vec<u8> {
        format!(
            "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
             Message-ID: <r{n}@example.com>\r\nDate: Mon, 01 Jan 2024 00:00:00 +0000\r\n\r\n\
             Body {n}\r\n"
        )
        .into_bytes()
    }

    struct Setup {
        server: FakeImapd,
        db: Db,
        w: Worker,
        account: i64,
    }

    fn setup(profile: Profile) -> Setup {
        let mut inbox = MailBox::new("INBOX", "");
        inbox.add(raw(1, "Old <old@github.com>", "old"), &[]);
        let server = FakeImapd::start(
            profile,
            vec![
                inbox,
                MailBox::new("Trash", "\\Trash"),
                MailBox::new("GitHub", ""),
                MailBox::new("Refused", ""),
            ],
        );
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
                email: "me@example.com".into(),
                name: "Me".into(),
                imap: host.clone(),
                smtp: host,
                save_sent: true,
                smtp_no_auth: false,
            })
            .unwrap();
        let (mut w, _) = worker_with(&db, None);
        let addr = server.addr;
        w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
        Setup {
            server,
            db,
            w,
            account,
        }
    }

    impl Setup {
        fn sync(&mut self) {
            self.w
                .run_job(&mut self.db, Job::SyncAll { full: true }, &mut || None);
        }

        fn arrive(&self, n: u32, from: &str, subject: &str) -> u32 {
            self.server
                .state()
                .mailbox_mut("INBOX")
                .add(raw(n, from, subject), &[])
        }

        fn cached(&self, name: &str) -> Vec<u32> {
            let f = self
                .db
                .folders()
                .unwrap()
                .into_iter()
                .find(|f| f.name == name)
                .unwrap();
            let mut uids: Vec<u32> = self.db.known_uids(f.id).unwrap().into_iter().collect();
            uids.sort_unstable();
            uids
        }

        fn rule(&mut self, value: &str, edit: impl FnOnce(&mut Rule)) -> i64 {
            let mut r = Rule {
                name: format!("rule {value}"),
                enabled: true,
                conditions: vec![Condition {
                    field: Field::From,
                    op: Op::EndsWith,
                    value: value.into(),
                }],
                ..Rule::default()
            };
            edit(&mut r);
            self.db.save_rule(self.account, &r).unwrap()
        }

        fn moves(&self) -> usize {
            let st = self.server.state();
            st.sent("UID MOVE").len() + st.sent("UID COPY").len()
        }
    }

    #[test]
    fn rules_skip_first_sync_and_set_mark() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| r.move_to = Some("GitHub".into()));
        s.sync();
        // The mail there before the first sync stays where it is.
        assert_eq!(s.cached("INBOX"), [1]);
        assert_eq!(s.moves(), 0);
        let inbox =
            s.db.folder_by_role(s.account, Role::Inbox)
                .unwrap()
                .unwrap();
        assert_eq!(s.db.rule_mark(inbox.id).unwrap(), Some((7, 2)));
    }

    #[test]
    fn move_rule_files_new_mail_and_never_inserts_it_in_inbox() {
        for profile in [Profile::FULL, Profile::BARE] {
            let mut s = setup(profile);
            s.rule("@github.com", |r| r.move_to = Some("GitHub".into()));
            s.sync();
            let a = s.arrive(2, "Bot <bot@github.com>", "PR opened");
            let b = s.arrive(3, "Bot <bot@GITHUB.com>", "PR merged");
            let c = s.arrive(4, "Ann <ann@example.org>", "lunch");
            let first = s.db.new_unread_inbox(0, 100).unwrap().len();
            s.sync();
            assert_eq!(s.cached("INBOX"), [1, c], "{profile:?}");
            // The destination was synced in the same pass.
            assert_eq!(s.cached("GitHub").len(), 2, "{profile:?}");
            // One MOVE (or COPY) for the destination's batch.
            assert_eq!(s.moves(), 1, "{profile:?}");
            let st = s.server.state();
            assert_eq!(st.mailbox("GitHub").msgs.len(), 2);
            assert!(
                st.mailbox("INBOX")
                    .msgs
                    .iter()
                    .all(|m| (m.uid != a && m.uid != b) || m.has("\\Deleted")),
                "{profile:?}"
            );
            drop(st);
            // Only the unfiled one would raise a notification.
            let fresh = s.db.new_unread_inbox(0, 100).unwrap();
            assert_eq!(fresh.len() - first, 1, "{profile:?}");
            let log = s.db.rule_log(s.account, 10).unwrap();
            assert_eq!(log.len(), 2);
            assert!(log.iter().all(|e| e.action == LogAction::Move
                && e.outcome == LogOutcome::Done
                && e.dest.as_deref() == Some("GitHub")));
            // A second pass finds nothing above the mark.
            s.sync();
            assert_eq!(s.moves(), 1, "{profile:?}");
        }
    }

    #[test]
    fn mark_read_rule_stores_seen_and_no_notification() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| {
            r.mark_read = true;
            r.flag = true;
        });
        s.sync();
        let before = s.db.new_unread_inbox(0, 100).unwrap().len();
        let uid = s.arrive(2, "Bot <bot@github.com>", "PR opened");
        s.sync();
        assert_eq!(s.cached("INBOX"), [1, uid]);
        assert_eq!(s.db.new_unread_inbox(0, 100).unwrap().len(), before);
        let st = s.server.state();
        let m = st.mailbox("INBOX").msg(uid).unwrap();
        assert!(m.has("\\Seen") && m.has("\\Flagged"));
        assert_eq!(st.sent("UID STORE").len(), 2);
        drop(st);
        // The next pass reads the same flags back: nothing changes.
        s.sync();
        assert_eq!(s.db.new_unread_inbox(0, 100).unwrap().len(), before);
    }

    #[test]
    fn failed_move_inserts_in_inbox_and_logs_failure() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| r.move_to = Some("Refused".into()));
        s.sync();
        s.server.state().refuse_copy_to = Some("Refused".into());
        let uid = s.arrive(2, "Bot <bot@github.com>", "PR opened");
        s.sync();
        assert_eq!(s.cached("INBOX"), [1, uid]);
        let log = s.db.rule_log(s.account, 10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].outcome, LogOutcome::Failed);
    }

    #[test]
    fn delete_rule_moves_to_trash() {
        let mut s = setup(Profile::FULL);
        s.rule("@spam.example", |r| r.trash = true);
        s.sync();
        s.arrive(2, "X <x@spam.example>", "win");
        s.sync();
        assert_eq!(s.cached("INBOX"), [1]);
        assert_eq!(s.cached("Trash").len(), 1);
        assert_eq!(
            s.db.rule_log(s.account, 10).unwrap()[0].action,
            LogAction::Delete
        );
    }

    #[test]
    fn uidvalidity_change_resets_mark_without_running() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| r.move_to = Some("GitHub".into()));
        s.sync();
        {
            let mut st = s.server.state();
            let inbox = st.mailbox_mut("INBOX");
            inbox.uidvalidity = 8;
            inbox.add(raw(2, "Bot <bot@github.com>", "PR"), &[]);
        }
        s.sync();
        assert_eq!(s.moves(), 0);
        let inbox =
            s.db.folder_by_role(s.account, Role::Inbox)
                .unwrap()
                .unwrap();
        assert_eq!(s.db.rule_mark(inbox.id).unwrap(), Some((8, 3)));
    }

    #[test]
    fn stop_and_first_move_wins_order() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| {
            r.mark_read = true;
            r.stop = true;
        });
        s.rule("@github.com", |r| r.move_to = Some("GitHub".into()));
        s.sync();
        let uid = s.arrive(2, "Bot <bot@github.com>", "PR");
        s.sync();
        assert_eq!(s.cached("INBOX"), [1, uid]);
        assert_eq!(s.moves(), 0);
    }

    #[test]
    fn missing_folder_pauses_rule() {
        let mut s = setup(Profile::FULL);
        s.rule("@github.com", |r| r.move_to = Some("Gone".into()));
        s.sync();
        assert!(s.db.rules_for(s.account).unwrap()[0].paused);
        let uid = s.arrive(2, "Bot <bot@github.com>", "PR");
        s.sync();
        assert_eq!(s.cached("INBOX"), [1, uid]);
        assert_eq!(s.moves(), 0);
    }

    #[test]
    fn no_rules_costs_no_imap_commands() {
        let mut s = setup(Profile::FULL);
        s.sync();
        s.arrive(2, "Bot <bot@github.com>", "PR");
        s.sync();
        let st = s.server.state();
        assert!(st.sent("UID STORE").is_empty());
        assert!(st.sent("UID MOVE").is_empty() && st.sent("UID COPY").is_empty());
    }
}
