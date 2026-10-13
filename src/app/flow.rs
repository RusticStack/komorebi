//! Debug builds only: a scripted walk through the real app against a real
//! account, driving the controller the way clicks would, with a window
//! snapshot after each step. Run with `KOMOREBI_FLOW=<dir>` (plus
//! `KOMOREBI_DB` and `KOMOREBI_PASSWORD`); the log and snapshots go to `<dir>`.
//!
//! Safety: the list is first filtered to `[komorebi-test]` subjects, and every
//! message the flow opens or acts on is checked to be one of those first.
//! It sends two messages, both to the account itself (a reply and a new
//! formatted message with an attachment, which must arrive with its HTML
//! part), checks the new message's draft is saved locally and gone once
//! sent, moves a test message to Junk and back, and never deletes anything.
//! Run it with `KOMOREBI_NO_SERVER_DRAFTS=1` so no draft copy is added to
//! (and later removed from) the server's Drafts folder.

use super::*;
use std::io::Write as _;
use std::time::Instant;

const TEST_TAG: &str = "[komorebi-test]";
const TICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    WaitSynced,
    Filter,
    Open,
    Flag,
    Unread,
    Read,
    Unflag,
    Reply,
    ReplySent,
    Compose,
    DraftSaved,
    ComposeSent,
    MoveToJunk,
    FindInJunk,
    MoveBack,
    WaitArrivals,
    OpenAttachment,
    Done,
}

struct Flow {
    dir: PathBuf,
    step: Step,
    step_started: Instant,
    started: Instant,
    target: Option<i64>,
    last_sync: Instant,
    log: std::fs::File,
    timer: Timer,
}

thread_local! {
    static FLOW: RefCell<Option<Flow>> = const { RefCell::new(None) };
}

pub(super) fn start(dir: PathBuf) {
    std::fs::create_dir_all(&dir).ok();
    let log = std::fs::File::create(dir.join("flow.log")).expect("creating flow.log");
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, TICK, tick);
    FLOW.with(|f| {
        *f.borrow_mut() = Some(Flow {
            dir,
            step: Step::WaitSynced,
            step_started: Instant::now(),
            started: Instant::now(),
            target: None,
            last_sync: Instant::now(),
            log,
            timer,
        })
    });
}

enum Next {
    Wait,
    Go(Step),
    Fail(String),
}

fn tick() {
    FLOW.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(flow) = guard.as_mut() else { return };
        let outcome = with_app(|app| flow.advance(app)).unwrap_or(Next::Wait);
        match outcome {
            Next::Wait if flow.step_started.elapsed() > Duration::from_secs(180) => {
                flow.finish(&format!("FAIL: {:?} timed out", flow.step));
            }
            Next::Wait => {}
            Next::Go(Step::Done) => flow.finish("PASS: all steps done"),
            Next::Go(step) => {
                flow.note(&format!("-> {step:?}"));
                flow.step = step;
                flow.step_started = Instant::now();
            }
            Next::Fail(why) => flow.finish(&format!("FAIL at {:?}: {why}", flow.step)),
        }
    });
}

impl Flow {
    fn note(&mut self, line: &str) {
        let line = format!("[{:6.1} s] {line}", self.started.elapsed().as_secs_f64());
        eprintln!("[flow] {line}");
        writeln!(self.log, "{line}").ok();
    }

    fn snap(&mut self, ui: &AppWindow, name: &str) {
        crate::save_bmp(ui, &self.dir.join(format!("{name}.bmp")));
        self.note(&format!("snapshot {name}"));
    }

    fn finish(&mut self, verdict: &str) {
        self.note(verdict);
        self.timer.stop();
        slint::quit_event_loop().ok();
    }

    fn waited(&self, d: Duration) -> bool {
        self.step_started.elapsed() >= d
    }

    /// The message, if (and only if) it is one of the flow's test messages.
    fn test_message(app: &App, id: i64) -> Result<db::Message, String> {
        let m = app
            .db
            .message(id)
            .map_err(|e| format!("{e:#}"))?
            .ok_or("message vanished")?;
        if m.subject.trim_start_matches("Re: ").starts_with(TEST_TAG) {
            Ok(m)
        } else {
            Err(format!("refusing to touch a non-test message (id {id})"))
        }
    }

    fn filter(app: &mut App) {
        app.ui().set_search_text(TEST_TAG.into());
        app.search_edited(TEST_TAG);
    }

    fn folder(app: &App, role: Role) -> Option<i64> {
        app.folders.iter().find(|f| f.role == role).map(|f| f.id)
    }

    fn subjects(app: &App) -> Vec<(i64, String)> {
        (0..app.rows.len())
            .filter_map(|i| app.rows.id_at(i))
            .filter_map(|id| Some((id, app.db.message(id).ok()??.subject)))
            .collect()
    }

    /// A send is done once the outbox is empty again (after the undo
    /// window); it failed if the composer stayed open or the outbox kept
    /// the message with an error.
    fn sent_done(app: &App) -> Option<Result<String, String>> {
        let ui = app.ui();
        let status = ui.get_status().to_string();
        if ui.get_composing() {
            let reason = [ui.get_c_guard(), ui.get_c_send_reason()].join(" ");
            return Some(Err(format!("{status} {reason}")));
        }
        match app.db.count_outbox() {
            Ok(0) => Some(Ok(status)),
            Ok(_) if ui.get_status_severity() == Severity::Critical => Some(Err(status)),
            Ok(_) => None,
            Err(e) => Some(Err(format!("{e:#}"))),
        }
    }

    fn advance(&mut self, app: &mut App) -> Next {
        let ui = app.ui();
        let flag = |app: &App, id| {
            app.db
                .message(id)
                .ok()
                .flatten()
                .map(|m| (m.seen, m.flagged))
        };
        match self.step {
            Step::WaitSynced => {
                if app.started_sync && app.syncing.is_empty() && self.waited(Duration::from_secs(5))
                {
                    self.snap(&ui, "00-synced");
                    return Next::Go(Step::Filter);
                }
                Next::Wait
            }
            Step::Filter => {
                Self::filter(app);
                let n = app.rows.len();
                self.note(&format!("{n} test messages listed"));
                self.snap(&ui, "01-list");
                if n == 0 {
                    return Next::Fail("no [komorebi-test] messages".into());
                }
                Next::Go(Step::Open)
            }
            Step::Open => {
                if self.target.is_none() {
                    let Some(id) = app.rows.id_at(0) else {
                        return Next::Fail("empty list".into());
                    };
                    if let Err(e) = Self::test_message(app, id) {
                        return Next::Fail(e);
                    }
                    app.select_message(id);
                    self.target = Some(id);
                }
                if app.open.as_ref().is_some_and(|o| o.body.is_some()) {
                    let atts = app
                        .open
                        .as_ref()
                        .and_then(|o| o.body.as_ref())
                        .map_or(0, |b| b.attachments.len());
                    self.note(&format!(
                        "opened id {:?}, body ready, {atts} attachments",
                        self.target
                    ));
                    self.snap(&ui, "02-reader");
                    return Next::Go(Step::Flag);
                }
                Next::Wait
            }
            Step::Flag => {
                app.toggle_flag();
                self.note(&format!(
                    "flag now (seen, flagged) = {:?}",
                    flag(app, self.target.unwrap())
                ));
                self.snap(&ui, "03-flagged");
                Next::Go(Step::Unread)
            }
            Step::Unread => {
                if !self.waited(Duration::from_secs(2)) {
                    return Next::Wait;
                }
                app.toggle_read();
                self.note(&format!(
                    "marked unread: {:?}",
                    flag(app, self.target.unwrap())
                ));
                Next::Go(Step::Read)
            }
            Step::Read => {
                if !self.waited(Duration::from_secs(2)) {
                    return Next::Wait;
                }
                app.toggle_read();
                self.note(&format!(
                    "marked read: {:?}",
                    flag(app, self.target.unwrap())
                ));
                Next::Go(Step::Unflag)
            }
            Step::Unflag => {
                if !self.waited(Duration::from_secs(2)) {
                    return Next::Wait;
                }
                app.toggle_flag();
                self.note(&format!("unflagged: {:?}", flag(app, self.target.unwrap())));
                Next::Go(Step::Reply)
            }
            Step::Reply => {
                if !self.waited(Duration::from_secs(2)) {
                    return Next::Wait;
                }
                app.reply(false);
                if !ui.get_composing() {
                    return Next::Fail("reply did not open the composer".into());
                }
                let body = format!("Reply from Komorebi's app flow test.{}", ui.get_c_body());
                app.set_draft_field(ComposeField::Body, &body);
                self.note(&format!(
                    "reply to: {} / subject: {}",
                    ui.get_c_to(),
                    ui.get_c_subject()
                ));
                self.snap(&ui, "04-reply");
                app.send();
                Next::Go(Step::ReplySent)
            }
            Step::ReplySent => match Self::sent_done(app) {
                Some(Ok(status)) => {
                    self.note(&format!("reply sent: {status}"));
                    Next::Go(Step::Compose)
                }
                Some(Err(status)) => Next::Fail(format!("reply failed: {status}")),
                None => Next::Wait,
            },
            Step::Compose => {
                let me = app
                    .accounts
                    .first()
                    .map(|a| a.email.clone())
                    .unwrap_or_default();
                app.compose_new();
                app.set_draft_field(ComposeField::To, &me);
                app.set_draft_field(
                    ComposeField::Subject,
                    &format!("{TEST_TAG} flow attachment"),
                );
                app.set_draft_field(
                    ComposeField::Body,
                    "A new message with an attachment, from Komorebi's app flow test.\n\n\
                     **Bold** and _italic_ text, and a list:\n\n- one\n- two",
                );
                if let Some(d) = app.drafts.active_mut() {
                    d.attachments.push(
                        OutAttachment {
                            filename: "komorebi-test.txt".into(),
                            data: b"Attachment written by Komorebi's app flow test.\n".to_vec(),
                        }
                        .into(),
                    );
                }
                app.show_compose_attachments();
                self.snap(&ui, "05-compose");
                Next::Go(Step::DraftSaved)
            }
            // The draft is saved to the cache a moment after the last edit.
            Step::DraftSaved => match app.db.count_drafts() {
                Ok(0) => Next::Wait,
                Ok(n) => {
                    self.note(&format!("{n} draft(s) saved locally; sending"));
                    app.send();
                    Next::Go(Step::ComposeSent)
                }
                Err(e) => Next::Fail(format!("{e:#}")),
            },
            Step::ComposeSent => match Self::sent_done(app) {
                Some(Ok(status)) => {
                    let left = app.db.count_drafts().unwrap_or(-1);
                    self.note(&format!("new message sent: {status}; drafts left: {left}"));
                    if left != 0 {
                        return Next::Fail("the sent message's draft was kept".into());
                    }
                    Next::Go(Step::MoveToJunk)
                }
                Some(Err(status)) => Next::Fail(format!("send failed: {status}")),
                None => Next::Wait,
            },
            Step::MoveToJunk => {
                let id = self.target.unwrap();
                if let Err(e) = Self::test_message(app, id) {
                    return Next::Fail(e);
                }
                let Some(junk) = Self::folder(app, Role::Junk) else {
                    return Next::Fail("no Junk folder".into());
                };
                app.select_message(id);
                app.move_to(junk);
                // Past the undo window at once.
                app.flush_held();
                self.note("moved the test message to Junk");
                app.set_view(View::Folder(junk));
                Self::filter(app);
                Next::Go(Step::FindInJunk)
            }
            Step::FindInJunk => {
                Self::filter(app);
                if let Some((id, subject)) = Self::subjects(app)
                    .into_iter()
                    .find(|(_, s)| s.starts_with(TEST_TAG))
                {
                    self.note(&format!("found in Junk: {subject}"));
                    self.snap(&ui, "06-junk");
                    if let Err(e) = Self::test_message(app, id) {
                        return Next::Fail(e);
                    }
                    app.select_message(id);
                    self.target = Some(id);
                    return Next::Go(Step::MoveBack);
                }
                Next::Wait
            }
            Step::MoveBack => {
                if !self.waited(Duration::from_secs(1)) {
                    return Next::Wait;
                }
                let Some(inbox) = Self::folder(app, Role::Inbox) else {
                    return Next::Fail("no Inbox".into());
                };
                app.move_to(inbox);
                app.flush_held();
                self.note("moved it back to the Inbox");
                // All inboxes is in the pane only with several accounts.
                app.set_view(if app.accounts.len() > 1 {
                    View::AllInboxes
                } else {
                    View::Folder(inbox)
                });
                Self::filter(app);
                self.last_sync = Instant::now();
                Next::Go(Step::WaitArrivals)
            }
            Step::WaitArrivals => {
                if self.last_sync.elapsed() > Duration::from_secs(10) {
                    app.sync_all(false);
                    self.last_sync = Instant::now();
                }
                Self::filter(app);
                let subjects = Self::subjects(app);
                let reply = subjects
                    .iter()
                    .any(|(_, s)| s.starts_with("Re: [komorebi-test]"));
                let attach = subjects
                    .iter()
                    .find(|(_, s)| s.ends_with("flow attachment"));
                let back = subjects.len();
                if reply && let Some(&(id, _)) = attach {
                    self.note(&format!(
                        "arrived: reply and attachment message ({back} test messages listed)"
                    ));
                    self.target = Some(id);
                    if let Err(e) = Self::test_message(app, id) {
                        return Next::Fail(e);
                    }
                    app.select_message(id);
                    return Next::Go(Step::OpenAttachment);
                }
                Next::Wait
            }
            Step::OpenAttachment => {
                if let Some(body) = app.open.as_ref().and_then(|o| o.body.as_ref()) {
                    let names: Vec<_> = body.attachments.iter().map(|a| a.name.clone()).collect();
                    self.note(&format!(
                        "attachment message opened, attachments: {names:?}"
                    ));
                    self.snap(&ui, "07-attachment");
                    if !names.iter().any(|n| n == "komorebi-test.txt") {
                        return Next::Fail("attachment missing".into());
                    }
                    let html = app
                        .open
                        .as_ref()
                        .and_then(|o| o.msg.raw.as_deref())
                        .and_then(mime::html_body)
                        .unwrap_or_default();
                    let formatted = ["<b>Bold</b>", "<i>italic</i>", "<li>one</li>"]
                        .iter()
                        .all(|t| html.contains(t));
                    self.note(&format!(
                        "HTML part: {} bytes, formatting kept: {formatted}",
                        html.len()
                    ));
                    if !formatted {
                        return Next::Fail(
                            "the HTML part is missing or lost its formatting".into(),
                        );
                    }
                    return Next::Go(Step::Done);
                }
                Next::Wait
            }
            Step::Done => Next::Go(Step::Done),
        }
    }
}
