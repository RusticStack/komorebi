//! The composer's typed slot in the mail view (docs/GPUI-MIGRATION.md
//! "Stage B plan"): the mail view asks for New mail, a saved draft, a reply
//! or a forward by row id ([`Draft`]), and the composer starts it with the
//! controller (`controller.rs`), which reads the message and its body from
//! the cache.

use gpui::{Context, Window};
use komorebi::mail::mime;

use super::{ComposerView, Status};

/// What the mail view asks the composer to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draft {
    /// A new message (from the first account).
    New,
    /// A saved draft (a `drafts` row id).
    Saved(i64),
    /// A reply to message `message` (a `messages` row id), to everyone with
    /// `all`.
    Reply { message: i64, all: bool },
    /// A forward of message `message`, as an attachment with `attached`.
    Forward { message: i64, attached: bool },
}

impl ComposerView {
    /// Start `draft`, or close the composer (saving what is worth keeping)
    /// with `None`. A reply or forward to a message whose body is not
    /// downloaded yet says so and opens nothing.
    pub fn open(&mut self, draft: Option<Draft>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asked) = draft else {
            if self.composing() {
                self.close(window, cx);
            }
            self.asked = None;
            return;
        };
        match asked {
            Draft::New => self.compose_new(None, window, cx),
            Draft::Saved(row) => self.open_saved(row, window, cx),
            Draft::Reply { message, .. } | Draft::Forward { message, .. } => {
                let msg = match self.db.message(message) {
                    Ok(Some(m)) => m,
                    Ok(None) => return,
                    Err(e) => return self.say(Status::Error(format!("{e:#}")), cx),
                };
                let body = msg.raw.as_deref().map(mime::parse_body);
                match asked {
                    Draft::Reply { all, .. } => self.reply(&msg, body.as_ref(), all, window, cx),
                    Draft::Forward { attached: true, .. } => {
                        self.forward_attached(&msg, window, cx)
                    }
                    _ => self.forward(&msg, body.as_ref(), window, cx),
                }
            }
        }
        self.asked = self.composing().then_some(asked);
    }

    /// A new message from `account` (none: the first), as
    /// `open(Some(Draft::New))` but from the account the pane is on.
    pub fn open_new(&mut self, account: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_new(account, window, cx);
        self.asked = self.composing().then_some(Draft::New);
    }

    /// The composer shows a draft.
    pub fn is_open(&self) -> bool {
        self.composing()
    }

    /// What the slot last asked for, while a draft shows.
    pub fn draft(&self) -> Option<Draft> {
        if self.composing() { self.asked } else { None }
    }
}
