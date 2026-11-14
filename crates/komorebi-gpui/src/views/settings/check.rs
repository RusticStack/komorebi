//! The account check (the Slint app's `run_check`, `show_check_steps`,
//! `finish_check`): the servers and the sign-ins checked on a background
//! thread, step by step, with the steps reported to the view as they pass.
//! Nothing is saved here. The view keeps the returned [`Task`]: dropping it
//! (Cancel, Back, Esc) drops the answer, so a late check adds or saves
//! nothing.

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{Context, Task};
use komorebi::mail::errors::{Class, Say, Step, StepError};
use komorebi::mail::pin::Kind;
use komorebi::mail::{self, Pins};
use komorebi::providers::Server;

use super::pins::review_of;

/// What checks an account's servers and sign-ins ([`mail::verify_account_with`]);
/// tests pass a fake.
pub type Verifier = fn(
    &Server,
    &Server,
    Pins,
    (&str, &str),
    &str,
    bool,
    &mut dyn FnMut(Step),
) -> Result<(), StepError>;

/// The real check.
pub const VERIFY: Verifier = mail::verify_account_with;

/// Reads the saved password of an account (`secrets::get_password`).
pub type ReadPassword = fn(i64) -> anyhow::Result<String>;

/// The state of one step of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    Running,
    Passed,
    Failed,
    Skipped,
}

/// The steps list: the first `done` steps passed and the next one is
/// running (`failed`: none is, that one failed and the rest were not tried).
pub fn steps_shown(done: usize, failed: Option<usize>) -> Vec<StepState> {
    (0..Step::ALL.len())
        .map(|i| match failed {
            Some(f) if i == f => StepState::Failed,
            Some(f) if i > f => StepState::Skipped,
            Some(_) => StepState::Passed,
            None if i < done => StepState::Passed,
            None if i == done => StepState::Running,
            None => StepState::Waiting,
        })
        .collect()
}

/// What a failed check says.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckText {
    /// A failed step.
    Say(Say),
    /// The saved password couldn't be read, so nothing was tried.
    NoSavedPassword,
}

/// A failed account check: the step that failed (none: before the first),
/// the sentence, and the server whose untrusted certificate can be reviewed.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckFailure {
    pub step: Option<usize>,
    pub text: CheckText,
    pub review: Option<Kind>,
}

/// Everything a check needs.
#[derive(Clone)]
pub struct Request {
    pub servers: (Server, Server),
    pub pins: (Option<Vec<u8>>, Option<Vec<u8>>),
    /// A manual server (its untrusted certificate can be reviewed).
    pub manual: bool,
    pub logins: (String, String),
    pub smtp_sign_in: bool,
    /// The typed password, or else the one saved for `saved`.
    pub password: Option<String>,
    pub saved: i64,
    pub verify: Verifier,
    pub read_password: ReadPassword,
}

/// What a running check tells its view.
#[derive(Debug)]
pub enum Msg {
    /// This many steps have passed.
    Progress(usize),
    Done(Result<(), CheckFailure>),
}

fn run(req: Request, report: &mut dyn FnMut(usize)) -> Result<(), CheckFailure> {
    let password = match req.password {
        Some(p) => p,
        None => (req.read_password)(req.saved).map_err(|_| CheckFailure {
            step: None,
            text: CheckText::NoSavedPassword,
            review: None,
        })?,
    };
    let logins = (req.logins.0.as_str(), req.logins.1.as_str());
    let pins = (req.pins.0.as_deref(), req.pins.1.as_deref());
    (req.verify)(
        &req.servers.0,
        &req.servers.1,
        pins,
        logins,
        &password,
        req.smtp_sign_in,
        &mut |step| report(step.index() + 1),
    )
    .map_err(|e| CheckFailure {
        step: Some(e.step.index()),
        text: CheckText::Say(e.say()),
        // Only a manual server's untrusted certificate can be reviewed; a
        // preset's is never offered for trust.
        review: review_of(&e).filter(|_| req.manual),
    })
}

/// Start a check on a background thread; `on` hears each [`Msg`] on the UI
/// thread. Keep the task: dropping it drops the rest of the answers.
pub fn start<T: 'static>(
    req: Request,
    cx: &mut Context<T>,
    on: impl Fn(&mut T, Msg, &mut Context<T>) + 'static,
) -> Task<()> {
    let (tx, mut rx) = mpsc::unbounded::<Msg>();
    let work = cx.background_executor().spawn(async move {
        let progress = tx.clone();
        let result = run(req, &mut |done| {
            // A closed channel means the check was cancelled.
            let _ = progress.unbounded_send(Msg::Progress(done));
        });
        let _ = tx.unbounded_send(Msg::Done(result));
    });
    cx.spawn(async move |this, cx| {
        // Keeps the work alive as long as this task is.
        let _work = work;
        while let Some(msg) = rx.next().await {
            let done = matches!(msg, Msg::Done(_));
            if this.update(cx, |this, cx| on(this, msg, cx)).is_err() || done {
                break;
            }
        }
    })
}

/// A form's check: the running task (dropping it drops the answer), how
/// many steps passed, and which failed.
#[derive(Default)]
pub struct CheckState {
    task: Option<Task<()>>,
    done: usize,
    failed: Option<usize>,
    /// The steps list shows (a check ran or runs).
    shown: bool,
}

impl CheckState {
    /// A check started (its task kept here).
    pub fn start(&mut self, task: Task<()>) {
        self.task = Some(task);
        self.done = 0;
        self.failed = None;
        self.shown = true;
    }

    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    /// Cancel, Back, Esc: the answer is dropped and the steps go.
    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    /// Hide the steps (the form changed).
    pub fn clear(&mut self) {
        if !self.running() {
            *self = Self::default();
        }
    }

    /// This many steps passed.
    pub fn progress(&mut self, done: usize) {
        if self.running() {
            self.done = done;
        }
    }

    /// The check answered.
    pub fn finish(&mut self, result: &Result<(), CheckFailure>) {
        self.task = None;
        match result {
            Ok(()) => self.done = Step::ALL.len(),
            Err(f) => {
                self.failed = f.step;
                self.shown = f.step.is_some();
            }
        }
    }

    /// The steps' states, if the list shows.
    pub fn steps(&self) -> Option<Vec<StepState>> {
        self.shown.then(|| steps_shown(self.done, self.failed))
    }
}

/// What a failed check says: a rejected password by provider
/// (`text::rejected`), any other step's sentence, or that the saved
/// password could not be read.
pub fn failure_text(f: &CheckFailure, preset: Option<usize>, email: &str) -> String {
    match &f.text {
        CheckText::Say(Say::Rejected { host, login }) => {
            super::text::rejected(preset, email, &crate::text::host(host), login)
        }
        CheckText::Say(say) => crate::text::say(say),
        CheckText::NoSavedPassword => super::text::err_saved_password(),
    }
}

/// A step's name in the steps list.
pub fn step_label(index: usize) -> String {
    use komorebi_i18n::tr;
    match index {
        0 => tr!("Find the incoming server"),
        1 => tr!("Connect to the incoming server"),
        2 => tr!("Secure the connection"),
        3 => tr!("Sign in to incoming mail"),
        4 => tr!("Read the folder list"),
        5 => tr!("Connect to the sending server"),
        _ => tr!("Sign in to sending mail"),
    }
}

/// A step's accessible name: its label and its state.
pub fn step_name(label: &str, state: StepState) -> String {
    use komorebi_i18n::tr;
    match state {
        StepState::Waiting => tr!("{0}, waiting", label),
        StepState::Running => tr!("{0}, in progress", label),
        StepState::Passed => tr!("{0}, passed", label),
        StepState::Failed => tr!("{0}, failed", label),
        StepState::Skipped => tr!("{0}, skipped", label),
    }
}

/// True for a sentence that means the sign-in was rejected (the password
/// box's error state, and Set username).
pub fn rejected(text: &CheckText) -> bool {
    matches!(text, CheckText::Say(Say::Rejected { .. }))
}

/// The error kind of a failure (`a-error-kind`): the short title and
/// sentence the onboarding flow shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind3 {
    Other,
    Rejected,
    Unreachable,
    Certificate,
}

/// What kind of failure a check's answer is.
pub fn error_kind(text: &CheckText, reviewable: bool) -> Kind3 {
    match text {
        CheckText::Say(Say::Rejected { .. }) => Kind3::Rejected,
        CheckText::Say(
            Say::NotFound { .. }
            | Say::NoAnswer { .. }
            | Say::NoConnection { .. }
            | Say::Lost { .. }
            | Say::Class(Class::Offline),
        ) => Kind3::Unreachable,
        CheckText::Say(
            Say::SecureFailed { .. } | Say::Class(Class::TlsCert) | Say::CertChanged { .. },
        ) => Kind3::Certificate,
        _ if reviewable => Kind3::Certificate,
        _ => Kind3::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_show_the_first_passed_the_next_running_and_after_a_failure_the_rest_skipped() {
        use StepState::*;
        assert_eq!(
            steps_shown(2, None),
            [Passed, Passed, Running, Waiting, Waiting, Waiting, Waiting]
        );
        assert_eq!(
            steps_shown(0, Some(3)),
            [Passed, Passed, Passed, Failed, Skipped, Skipped, Skipped]
        );
        assert_eq!(steps_shown(7, None), vec![Passed; 7]);
    }
}
