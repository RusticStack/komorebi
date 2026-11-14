//! What the reader does itself with the open message, in every place it
//! shows (the reading pane, a message tab, a message window), so no owner
//! has to: attachments opened in their default app or saved through the
//! file dialog (the Slint app's `open_attachment`, `save_attachment`), the
//! invitation's calendar file saved, the RSVP question before an answer is
//! sent (`src/app/invite.rs`), the sender notice's "Why am I seeing this?",
//! and the source commands of the More menu: Save as .eml, Show original
//! and Copy Message-ID (`src/app/original.rs`), which wait for a body that
//! is not downloaded yet. The view toggles of the More menu (View as plain
//! text, Show original colours, Load images or Stop loading images from
//! this sender) are [`ViewMenu`] and [`MenuCmd`].
//!
//! Sentences go to the owner's status line as [`ReaderEvent::Status`];
//! the invite reply's dispatch as [`ReaderEvent::ReplyQueued`].

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, ParentElement as _, Role, SharedString,
    Styled as _, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use komorebi::files;
use komorebi::invite as inv;
use komorebi::mail::ical::Answer;
use komorebi::mail::{mime, source};
use komorebi_i18n::tr;

use super::{ReaderEvent, ReaderView};
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, reader, space};
use crate::widgets::button::{Button, Variant};

/// Opens a file in its default app (on a thread of its own); false when
/// nothing opened it. Tests put one in that opens nothing.
pub type Opener = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// The system's: `ShellExecuteW`.
pub fn system_opener() -> Opener {
    Arc::new(|path: &Path| komorebi::reader::shell_execute(path.as_os_str()))
}

/// A More-menu command on the open message's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceUse {
    SaveEml,
    ShowOriginal,
    CopyId,
}

/// A view toggle of the More menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    /// View as plain text, on or off.
    Plain,
    /// Show original colours, on or off.
    Colours,
    /// Load this message's remote images.
    LoadImages,
    /// Stop loading images from this sender.
    StopImages,
}

/// The More menu's view entries for the message shown (the Slint
/// `more_menu`'s middle group): each `(enabled, checked)`; `stop_images`
/// in place of Load images when the sender's images always load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewMenu {
    pub plain: (bool, bool),
    pub colours: (bool, bool),
    pub load_images: bool,
    pub stop_images: bool,
}

/// What a More-menu toggle reads (the Slint menu's words).
pub fn menu_label(cmd: MenuCmd) -> String {
    match cmd {
        MenuCmd::Plain => tr!("View as plain text"),
        MenuCmd::Colours => tr!("Show original colours"),
        MenuCmd::LoadImages => tr!("Load images"),
        MenuCmd::StopImages => tr!("Stop loading images from this sender"),
    }
}

/// Where the save dialog starts: the Downloads Known Folder (where the
/// user moved it, OneDrive or a redirected profile), as the Slint app's
/// `save_file`.
fn downloads() -> PathBuf {
    dirs::download_dir().unwrap_or_else(std::env::temp_dir)
}

impl ReaderView {
    /// Put in another way to open files (tests: one that opens nothing).
    pub fn set_opener(&mut self, opener: Opener) {
        self.opener = opener;
    }

    fn say(&self, text: String, error: bool, cx: &mut Context<Self>) {
        cx.emit(ReaderEvent::Status { text, error });
    }

    /// Attachment `index` of the open message: its name and bytes.
    fn attachment(&self, index: usize) -> Option<(String, Vec<u8>)> {
        let raw = self.open.as_ref()?.msg.raw.as_deref()?;
        mime::attachment(raw, index)
    }

    // ---------------------------------------------------- attachments

    /// Open attachment `index` in the default app for its type, from a
    /// copy beside the cache marked as downloaded. Types that run code are
    /// refused: they can be saved and checked first.
    pub fn open_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((name, data)) = self.attachment(index) else {
            return;
        };
        if files::risky(&name) {
            let text = tr!(
                "{0} can run programs, so Komorebi doesn't open it. Save it to check it first.",
                name
            );
            return self.say(text, true, cx);
        }
        let dir = files::open_dir(
            &self
                .db
                .file()
                .unwrap_or_else(|| std::env::temp_dir().join("komorebi-reader.sqlite3")),
        );
        let opener = self.opener.clone();
        // Writing can take a moment and ShellExecuteW can block (DDE, a
        // slow handler): both stay off the UI thread.
        let open = cx.background_spawn(async move {
            match files::write_open_copy(&dir, &name, &data) {
                Ok(path) if opener(&path) => None,
                Ok(_) => Some((name, None)),
                Err(e) => Some((name, Some(e.to_string()))),
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some((name, error)) = open.await {
                let text = match error {
                    None => tr!("No app on this computer opens {0}.", name),
                    Some(e) => tr!("Couldn't open {0}: {1}", name, e),
                };
                let _ = this.update(cx, |r, cx| r.say(text, true, cx));
            }
        })
        .detach();
    }

    /// Save attachment `index` where the user picks.
    pub fn save_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some((name, data)) = self.attachment(index) {
            self.save_file(name, data, cx);
        }
    }

    /// Save .ics on the invite card: the invitation's calendar file.
    pub fn save_invite(&mut self, cx: &mut Context<Self>) {
        let data = self.open.as_ref().and_then(|o| inv::calendar_file(&o.msg));
        if let Some(data) = data {
            self.save_file("invite.ics".into(), data, cx);
        }
    }

    /// Save `data` where the user picks (offered as `name` in Downloads),
    /// marked as downloaded; the file is written off the UI thread.
    fn save_file(&mut self, name: String, data: Vec<u8>, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_new_path(&downloads(), Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let target = path.clone();
            let write = cx.background_spawn(async move {
                let result = std::fs::write(&target, data);
                if result.is_ok() {
                    // Mark of the Web, as a browser download has.
                    files::mark_downloaded(&target).ok();
                }
                result.map_err(|e| e.to_string())
            });
            let result = write.await;
            let _ = this.update(cx, |r, cx| match result {
                Ok(()) => r.say(tr!("Saved {0}", path.display()), false, cx),
                Err(e) => r.say(tr!("Couldn't save {0}: {1}", name, e), true, cx),
            });
        })
        .detach();
    }

    // ---------------------------------------------------- the invitation

    /// Accept, Tentative or Decline on the card: the question first (who
    /// the reply goes to, exactly, and the warnings), then
    /// [`ReaderView::answer_invite`] with the reply (Send reply, Enter)
    /// or without (Don't send a reply). Cancel and Escape do nothing.
    pub fn ask_rsvp(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        let Some(ev) = open.body.as_ref().and_then(|b| b.invite.as_deref()) else {
            return;
        };
        let msg = open.msg.id;
        let email = self
            .accounts
            .iter()
            .find(|a| a.id == open.msg.account_id)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let p = inv::prompt(&email, &open.msg, ev);
        let title: SharedString = rsvp_title(answer).into();
        let text: SharedString = rsvp_text(&p).into();
        let view = cx.entity().downgrade();
        let t = cx.tokens().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let c = t.color;
            let (send, keep, enter) = (view.clone(), view.clone(), view.clone());
            dialog
                .w(px(layout::DIALOG_W))
                .bg(c.paper)
                .text_color(c.ink)
                .rounded(px(radius::CARD))
                .shadow(Vec::new())
                .close_button(false)
                .p(px(space::SP_6))
                .title(div().typo(&t.text.subtitle).child(title.clone()))
                .child(
                    div()
                        .id("rsvp-text")
                        .role(Role::Label)
                        .aria_label(text.clone())
                        .typo(&t.text.body)
                        .text_color(c.ink_2)
                        .child(text.clone())
                        .test_support(),
                )
                .footer(
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(space::SP_2))
                        .pt(px(space::SP_4))
                        .child(
                            Button::new("rsvp-cancel")
                                .label(tr!("Cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("rsvp-keep")
                                .label(tr!("Don't send a reply"))
                                .on_click(move |_, window, cx| {
                                    let _ = keep.update(cx, |r, cx| {
                                        r.answer_invite(msg, answer, false, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("rsvp-send")
                                .variant(Variant::Primary)
                                .label(tr!("Send reply"))
                                .on_click(move |_, window, cx| {
                                    let _ = send
                                        .update(cx, |r, cx| r.answer_invite(msg, answer, true, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_ok(move |_, _window, cx| {
                    let _ = enter.update(cx, |r, cx| r.answer_invite(msg, answer, true, cx));
                    true
                })
        });
    }

    // ---------------------------------------------------- the sender notice

    /// "Why?" on the sender notice: what the check is and what to do, with
    /// one Close button.
    pub fn why_sender(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let t = cx.tokens().clone();
        let (what, todo): (SharedString, SharedString) = (
            tr!(
                "Komorebi looks at the checks your mail server made when this message arrived, and at the sender's name and address. When they don't add up, someone may be pretending to be the sender."
            )
            .into(),
            tr!(
                "Don't open its links or attachments, reply, or enter a password unless you're sure. If you know the sender, check with them another way. Report junk moves it to your Junk folder."
            )
            .into(),
        );
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let c = t.color;
            let para = |id: &'static str, s: &SharedString| {
                div()
                    .id(id)
                    .role(Role::Label)
                    .aria_label(s.clone())
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(s.clone())
                    .test_support()
            };
            dialog
                .w(px(layout::DIALOG_W))
                .bg(c.paper)
                .text_color(c.ink)
                .rounded(px(radius::CARD))
                .shadow(Vec::new())
                .close_button(false)
                .p(px(space::SP_6))
                .title(
                    div()
                        .typo(&t.text.subtitle)
                        .child(tr!("Why am I seeing this?")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_3))
                        .child(para("why-what", &what))
                        .child(para("why-todo", &todo)),
                )
                .footer(
                    div()
                        .w_full()
                        .flex()
                        .justify_end()
                        .pt(px(space::SP_4))
                        .child(
                            Button::new("why-close")
                                .variant(Variant::Primary)
                                .label(tr!("Close"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
                .on_ok(|_, _, _| true)
        });
    }

    // ---------------------------------------------------- the source

    /// Run `what` on the open message once its body is here: at once when
    /// it is cached, else after the download it asks for (the status line
    /// says so after a second, as the Slint app).
    pub fn source(&mut self, what: SourceUse, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        // The Message-ID is in the cached header: no body is needed.
        if open.msg.raw.is_some() || what == SourceUse::CopyId {
            return self.run_source(what, window, cx);
        }
        let id = open.msg.id;
        self.source_wait = Some((id, what));
        // Asked again: a first download may have failed.
        cx.emit(ReaderEvent::FetchBody(id));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1)) // timer-exempt: the wait UX's first second before "Fetching message"
                .await;
            let _ = this.update(cx, |r, cx| {
                if r.source_wait.is_some_and(|(wait, _)| wait == id) && r.message() == Some(id) {
                    r.say(tr!("Fetching message\u{a0}…"), false, cx);
                }
            });
        })
        .detach();
    }

    /// The body of `id` arrived: the source command waiting for it runs, if
    /// `id` is still the message shown.
    pub(super) fn source_arrived(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((wait, what)) = self.source_wait else {
            return;
        };
        if wait != id {
            return;
        }
        self.source_wait = None;
        if self.message() == Some(id) {
            self.run_source(what, window, cx);
        }
    }

    /// The download of `id`'s body failed: the source command waiting for
    /// it says so.
    pub fn body_failed(&mut self, id: i64, error: &str, cx: &mut Context<Self>) {
        if self.source_wait.is_some_and(|(wait, _)| wait == id) {
            self.source_wait = None;
            self.say(tr!("Couldn't download this message: {0}", error), true, cx);
        }
    }

    /// A source command waits for message `id`'s body.
    pub fn source_waiting(&self) -> Option<(i64, SourceUse)> {
        self.source_wait
    }

    fn run_source(&mut self, what: SourceUse, window: &mut Window, cx: &mut Context<Self>) {
        match what {
            SourceUse::SaveEml => {
                if let Some((name, raw)) = self.eml_export() {
                    self.save_file(name, raw, cx);
                }
            }
            SourceUse::ShowOriginal => self.show_source(window, cx),
            SourceUse::CopyId => self.copy_message_id(cx),
        }
    }

    /// The open message's file name and exact bytes, for Save as .eml.
    pub fn eml_export(&self) -> Option<(String, Vec<u8>)> {
        let m = &self.open.as_ref()?.msg;
        Some((source::eml_name(&m.subject, m.date), m.raw.clone()?))
    }

    /// The open message's Message-ID with its angle brackets (empty: none).
    pub fn message_id(&self) -> String {
        self.open
            .as_ref()
            .map_or_else(String::new, |o| bracketed(&o.msg.msgid))
    }

    /// Copy the open message's Message-ID (`<id@host>`).
    fn copy_message_id(&mut self, cx: &mut Context<Self>) {
        let id = self.message_id();
        self.copy_id_text(id, cx);
    }

    /// Copy Message-ID of message `message`, shown or not, from its cached
    /// header alone (`Db::message_id`): the row menu's, which need not open
    /// the message or load its body.
    pub fn copy_message_id_of(&mut self, message: i64, cx: &mut Context<Self>) {
        let id = match self.db.message_id(message) {
            Ok(Some(raw)) => bracketed(&raw),
            Ok(None) => return,
            Err(e) => return self.say(format!("{e:#}"), true, cx),
        };
        self.copy_id_text(id, cx);
    }

    fn copy_id_text(&mut self, id: String, cx: &mut Context<Self>) {
        if id.is_empty() {
            return self.say(tr!("This message has no Message-ID."), false, cx);
        }
        cx.write_to_clipboard(ClipboardItem::new_string(id));
        self.say(tr!("Message-ID copied."), false, cx);
    }

    /// Show original: the Message-ID, the header fields and the source,
    /// selectable, with Copy Message-ID, Copy headers, Copy all and Close.
    /// Escape closes it; the focus goes back to where it was.
    fn show_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(raw) = self.open.as_ref().and_then(|o| o.msg.raw.as_deref()) else {
            return;
        };
        let original = source::original(raw);
        let msgid = self.message_id();
        let field = |text: &str, window: &mut Window, cx: &mut Context<Self>| {
            let text = text.to_string();
            cx.new(|cx| {
                let mut s = TextareaState::new(window, cx).auto_grow(1, usize::MAX);
                s.set_value(text, window, cx);
                s
            })
        };
        let headers = field(&original.headers, window, cx);
        let body = field(&original.body, window, cx);
        let all = format!("{}\n\n{}", original.headers, original.body);
        let source = Rc::new(SourceTexts {
            msgid: msgid.into(),
            headers_text: original.headers.into(),
            all: all.into(),
            headers,
            body,
        });
        let t = cx.tokens().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let c = t.color;
            let s = source.clone();
            let heading = |text: String| div().typo(&t.text.body_strong).child(text);
            let copy = |id: &'static str, label: String, text: SharedString| {
                Button::new(id).label(label).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()))
                })
            };
            let mut buttons = div()
                .w_full()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(space::SP_2))
                .pt(px(space::SP_4));
            if !s.msgid.is_empty() {
                buttons = buttons.child(copy(
                    "source-copy-id",
                    tr!("Copy Message-ID"),
                    s.msgid.clone(),
                ));
            }
            let buttons = buttons
                .child(copy(
                    "source-copy-headers",
                    tr!("Copy headers"),
                    s.headers_text.clone(),
                ))
                .child(copy("source-copy-all", tr!("Copy all"), s.all.clone()))
                .child(
                    Button::new("source-close")
                        .variant(Variant::Primary)
                        .label(tr!("Close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                );
            dialog
                .w(px(reader::SOURCE_DIALOG_W))
                .bg(c.paper)
                .text_color(c.ink)
                .rounded(px(radius::CARD))
                .shadow(Vec::new())
                .close_button(false)
                .p(px(space::SP_6))
                .title(div().typo(&t.text.subtitle).child(tr!("Original message")))
                .child(
                    div()
                        .id("source")
                        .role(Role::Group)
                        .aria_label(tr!("Original message"))
                        .max_h(px(reader::SOURCE_DIALOG_MAX_H))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_3))
                        .when(!s.msgid.is_empty(), |el| {
                            let line = tr!("Message-ID: {0}", s.msgid);
                            el.child(
                                div()
                                    .id("source-msgid")
                                    .role(Role::Label)
                                    .aria_label(line.clone())
                                    .typo(&t.text.caption)
                                    .text_color(c.ink_2)
                                    .child(line)
                                    .test_support(),
                            )
                        })
                        .child(heading(tr!("Header fields")))
                        .child(
                            Textarea::new(&s.headers)
                                .readonly(true)
                                .appearance(false)
                                .aria_label(tr!("Header fields"))
                                .typo(&t.text.caption),
                        )
                        .child(heading(tr!("Source")))
                        .child(
                            Textarea::new(&s.body)
                                .readonly(true)
                                .appearance(false)
                                .aria_label(tr!("Message source"))
                                .typo(&t.text.caption),
                        )
                        .test_support(),
                )
                .footer(buttons)
        });
    }

    // ---------------------------------------------------- the More menu

    /// The More menu's view entries for the message shown.
    pub fn view_menu(&self) -> ViewMenu {
        let s = &self.session;
        let (html, plain) = (s.html(), s.plain());
        let trusted = self
            .open
            .as_ref()
            .is_some_and(|o| self.db.remote_allowed(&o.msg.from_addr).unwrap_or(false));
        ViewMenu {
            plain: (html || plain, plain),
            colours: (html && !plain && (s.dimmed() || s.original()), s.original()),
            load_images: !trusted && html && !plain && s.remote_blocked() > 0,
            stop_images: trusted,
        }
    }

    /// Run a view toggle of the More menu.
    pub fn menu(&mut self, cmd: MenuCmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            MenuCmd::Plain => {
                let on = !self.session.plain();
                self.set_plain(on, window, cx);
            }
            MenuCmd::Colours => {
                let on = !self.session.original();
                self.show_original(on, window, cx);
            }
            MenuCmd::LoadImages => self.load_remote(false, window, cx),
            MenuCmd::StopImages => {
                let Some(addr) = self.open.as_ref().map(|o| o.msg.from_addr.clone()) else {
                    return;
                };
                if let Err(e) = self.db.forget_remote(&addr) {
                    return self.say(format!("{e:#}"), true, cx);
                }
                self.remote_allowed = false;
                self.remote_always = None;
                cx.notify();
            }
        }
    }
}

/// A Message-ID with its angle brackets (empty: none).
fn bracketed(msgid: &str) -> String {
    let id = msgid.trim().trim_matches(['<', '>']);
    if id.is_empty() {
        String::new()
    } else {
        format!("<{id}>")
    }
}

/// What the source dialog shows and copies.
struct SourceTexts {
    msgid: SharedString,
    headers_text: SharedString,
    all: SharedString,
    headers: Entity<TextareaState>,
    body: Entity<TextareaState>,
}

/// The RSVP question's title (the Slint app's `rsvp_prompt`).
fn rsvp_title(answer: Answer) -> String {
    match answer {
        Answer::Accepted => tr!("Reply \u{201c}Accepted\u{201d}?"),
        Answer::Tentative => tr!("Reply \u{201c}Tentative\u{201d}?"),
        Answer::Declined => tr!("Reply \u{201c}Declined\u{201d}?"),
    }
}

/// The RSVP question's text: who the reply goes to (exactly), then a
/// warning when the organizer is not in the sender's domain or the user is
/// not an attendee, then what not sending does. Whole sentences, one after
/// the other.
pub fn rsvp_text(p: &inv::Prompt) -> String {
    let mut text = tr!(
        "The reply goes only to the organizer, {0}, from {1}. No one else gets it.",
        p.organizer,
        p.account
    );
    let mut add = |s: String| {
        text.push(' ');
        text.push_str(&s);
    };
    if let Some((theirs, sender)) = &p.domains {
        add(tr!(
            "Check the address: the organizer's domain ({0}) is not the sender's ({1}).",
            theirs,
            sender
        ));
    }
    if p.unlisted {
        add(tr!(
            "You are not listed as an attendee, so the reply answers as {0}.",
            p.account
        ));
    } else if let Some(alias) = &p.alias {
        add(tr!(
            "The reply answers as {0}, an address this message was sent to.",
            alias
        ));
    }
    add(tr!(
        "\u{201c}Don't send a reply\u{201d} keeps your answer on this computer only."
    ));
    text
}
