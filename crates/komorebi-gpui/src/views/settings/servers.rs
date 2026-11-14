//! The server fields of the account forms (Settings > Accounts > an
//! account's Server settings, and onboarding's manual setup): the incoming
//! (IMAP) and sending (SMTP) server, port, security and sign-in name, as Kit
//! inputs inside the brand's fields, and "This server does not need a
//! sign-in for sending" (the Slint app's `SmtpNoAuth`). They hold text
//! only; the form's rules read them as [`Fields`] (`komorebi::find`, the
//! lookup's own view of them) plus [`ServerForm::no_auth`].
//!
//! The account page and the setup flow word the fields as the Slint app
//! does in each place: "IMAP server" and "SSL/TLS (port 993)" on the page,
//! "Incoming (IMAP) server" and "Encrypted (SSL/TLS)" in the flow
//! ([`ServerForm::flow`]). [`find_row`] is the "Find settings" row both
//! put under the fields (the Slint app's `FindSettingsRow`).

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role, SharedString, Styled,
    Subscription, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::find::Fields;
use komorebi::providers::Security;
use komorebi_i18n::tr;

use super::card::labelled;
use super::hosts::port_for_security;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, space};
use crate::widgets::button::Button;
use crate::widgets::field::TextField;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Checkbox;

/// What the server fields tell their form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    /// The user changed a field (a certificate saved for the old server
    /// goes; a check's answer no longer fits).
    Edited,
}

/// The server fields.
pub struct ServerForm {
    pub imap_host: Entity<InputState>,
    pub imap_port: Entity<InputState>,
    pub imap_user: Entity<InputState>,
    pub smtp_host: Entity<InputState>,
    pub smtp_port: Entity<InputState>,
    pub smtp_user: Entity<InputState>,
    imap_starttls: bool,
    smtp_starttls: bool,
    smtp_own_login: bool,
    /// The SMTP server takes mail without a sign-in (a relay).
    smtp_no_auth: bool,
    /// The setup flow's wording ([`ServerForm::flow`]).
    flow: bool,
    /// Set while the fields are filled from code (no `Edited`).
    filling: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ServerEvent> for ServerForm {}

fn security(starttls: bool) -> Security {
    if starttls {
        Security::StartTls
    } else {
        Security::Tls
    }
}

impl ServerForm {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: Option<String>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let state = InputState::new(window, cx);
                match placeholder {
                    Some(p) => state.placeholder(p),
                    None => state,
                }
            })
        };
        let imap_host = input(Some("imap.example.com".into()), window, cx);
        let imap_port = input(None, window, cx);
        let imap_user = input(Some(tr!("Same as email address")), window, cx);
        let smtp_host = input(Some("smtp.example.com".into()), window, cx);
        let smtp_port = input(None, window, cx);
        let smtp_user = input(Some(tr!("Same as email address")), window, cx);
        let mut subscriptions = Vec::new();
        for state in [
            &imap_host, &imap_port, &imap_user, &smtp_host, &smtp_port, &smtp_user,
        ] {
            subscriptions.push(
                cx.subscribe(state, |this: &mut Self, _, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change) && !this.filling {
                        cx.emit(ServerEvent::Edited);
                    }
                }),
            );
        }
        Self {
            imap_host,
            imap_port,
            imap_user,
            smtp_host,
            smtp_port,
            smtp_user,
            imap_starttls: false,
            smtp_starttls: false,
            smtp_own_login: false,
            smtp_no_auth: false,
            flow: false,
            filling: false,
            _subscriptions: subscriptions,
        }
    }

    /// The setup flow's wording: "Incoming (IMAP) server", "Outgoing (SMTP)
    /// server", "Username" and "Encrypted (SSL/TLS)", as the Slint flow's
    /// manual setup.
    pub fn flow(mut self) -> Self {
        self.flow = true;
        self
    }

    /// Whether the SMTP server is marked as needing no sign-in.
    pub fn no_auth(&self) -> bool {
        self.smtp_no_auth
    }

    /// Mark the SMTP server as needing no sign-in (an account opened, or
    /// the check box).
    pub fn set_no_auth(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.smtp_no_auth != on {
            self.smtp_no_auth = on;
            cx.notify();
        }
    }

    /// Fill the fields (an account opened, a lookup answered).
    pub fn set(&mut self, f: &Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.filling = true;
        for (state, value) in [
            (&self.imap_host, &f.imap_host),
            (&self.imap_port, &f.imap_port),
            (&self.imap_user, &f.imap_user),
            (&self.smtp_host, &f.smtp_host),
            (&self.smtp_port, &f.smtp_port),
            (&self.smtp_user, &f.smtp_user),
        ] {
            let value = value.clone();
            state.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.imap_starttls = f.imap_starttls;
        self.smtp_starttls = f.smtp_starttls;
        self.smtp_own_login = f.smtp_own_login;
        self.filling = false;
        cx.notify();
    }

    /// The fields as typed.
    pub fn fields(&self, cx: &App) -> Fields {
        let value = |s: &Entity<InputState>| s.read(cx).value().to_string();
        Fields {
            imap_host: value(&self.imap_host),
            imap_port: value(&self.imap_port),
            imap_starttls: self.imap_starttls,
            imap_user: value(&self.imap_user),
            smtp_host: value(&self.smtp_host),
            smtp_port: value(&self.smtp_port),
            smtp_starttls: self.smtp_starttls,
            smtp_user: value(&self.smtp_user),
            smtp_own_login: self.smtp_own_login,
        }
    }

    /// Focus the incoming sign-in name (Set username).
    pub fn focus_user(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.imap_user.update(cx, |s, cx| s.focus(window, cx));
    }

    fn set_security(
        &mut self,
        imap: bool,
        starttls: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (old, port) = if imap {
            (self.imap_starttls, &self.imap_port)
        } else {
            (self.smtp_starttls, &self.smtp_port)
        };
        if old == starttls {
            return;
        }
        let typed = port.read(cx).value().to_string();
        let next = port_for_security(imap, security(old), security(starttls), &typed);
        let port = port.clone();
        port.update(cx, |s, cx| s.set_value(next, window, cx));
        if imap {
            self.imap_starttls = starttls;
        } else {
            self.smtp_starttls = starttls;
        }
        cx.emit(ServerEvent::Edited);
        cx.notify();
    }

    fn security_select(&self, imap: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (id, name, on) = if imap {
            (
                "server-imap-security",
                tr!("IMAP security"),
                self.imap_starttls,
            )
        } else {
            (
                "server-smtp-security",
                tr!("SMTP security"),
                self.smtp_starttls,
            )
        };
        let (tls, starttls) = security_labels(imap, self.flow);
        let this = cx.entity().downgrade();
        Dropdown::new(id, name, if on { "starttls" } else { "tls" })
            .choices([("tls", tls), ("starttls", starttls)])
            .on_pick(move |value: &SharedString, window, cx| {
                let starttls = value == "starttls";
                let _ = this.update(cx, |this, cx| this.set_security(imap, starttls, window, cx));
            })
    }
}

/// The two security choices' words: the account page names the default
/// port, the setup flow says what each does.
pub fn security_labels(imap: bool, flow: bool) -> (String, String) {
    match (flow, imap) {
        (true, _) => (
            tr!("Encrypted (SSL/TLS)"),
            tr!("Encrypted after connecting (STARTTLS)"),
        ),
        (false, true) => (tr!("SSL/TLS (port 993)"), tr!("STARTTLS (port 143)")),
        (false, false) => (tr!("SSL/TLS (port 465)"), tr!("STARTTLS (port 587)")),
    }
}

/// The server fields' labels: (incoming server, outgoing server, incoming
/// username).
pub fn host_labels(flow: bool) -> (String, String, String) {
    if flow {
        (
            tr!("Incoming (IMAP) server"),
            tr!("Outgoing (SMTP) server"),
            tr!("Username"),
        )
    } else {
        (tr!("IMAP server"), tr!("SMTP server"), tr!("IMAP username"))
    }
}

/// The "Find settings" row (the Slint app's `FindSettingsRow`): the
/// button, "Looking up settings …" while the lookup runs, and the note
/// under it: what the lookup does, what it found, or that the form changed
/// while it looked (`note`, empty for the first).
pub fn find_row(
    id: &'static str,
    finding: bool,
    can_find: bool,
    note: String,
    on_find: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let t = cx.tokens();
    let c = t.color;
    let note = if note.is_empty() {
        tr!(
            "Fills in the servers for your address. Only your domain name is looked up, and nothing is saved."
        )
    } else {
        note
    };
    div()
        .flex()
        .flex_col()
        .gap(px(space::SP_2))
        .max_w(px(geo::FORM_W))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::SP_2))
                .child(
                    Button::new(id)
                        .label(tr!("Find settings"))
                        .disabled(!can_find)
                        .on_click(on_find),
                )
                .when(finding, |el| {
                    el.child(
                        div()
                            .typo(&t.text.body)
                            .text_color(c.ink)
                            .child(tr!("Looking up settings\u{a0}…")),
                    )
                }),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-note")))
                .role(Role::Status)
                .aria_label(note.clone())
                .typo(&t.text.caption)
                .text_color(c.ink_2)
                .child(note)
                .test_support(),
        )
        .into_any_element()
}

/// A host field with its port beside it.
fn server_row(
    host_label: String,
    host: &Entity<InputState>,
    port_name: String,
    port: &Entity<InputState>,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .gap(px(geo::FIELD_GAP))
        .child(div().flex_1().min_w_0().child(labelled(
            host_label.clone(),
            TextField::new(host, host_label),
            None,
            cx,
        )))
        .child(div().w(px(geo::PORT_W)).flex_none().child(labelled(
            tr!("Port"),
            TextField::new(port, port_name),
            None,
            cx,
        )))
}

impl Render for ServerForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let no_auth_this = this.clone();
        let own = self.smtp_own_login;
        let no_auth = self.smtp_no_auth;
        let (imap_label, smtp_label, user_label) = host_labels(self.flow);
        let t = cx.tokens().clone();
        let user = labelled(
            user_label.clone(),
            TextField::new(&self.imap_user, user_label),
            None,
            cx,
        );
        let (user_early, user_late) = if self.flow {
            (None, Some(user))
        } else {
            (Some(user), None)
        };
        div()
            .w_full()
            .max_w(px(geo::FORM_W))
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(server_row(
                imap_label,
                &self.imap_host,
                tr!("IMAP port"),
                &self.imap_port,
                cx,
            ))
            .child(labelled(
                tr!("IMAP security"),
                self.security_select(true, cx),
                None,
                cx,
            ))
            .children(user_early)
            .child(server_row(
                smtp_label,
                &self.smtp_host,
                tr!("SMTP port"),
                &self.smtp_port,
                cx,
            ))
            .child(labelled(
                tr!("SMTP security"),
                self.security_select(false, cx),
                None,
                cx,
            ))
            .child(
                Checkbox::new(
                    "server-no-auth",
                    tr!("This server does not need a sign-in for sending"),
                    no_auth,
                )
                .on_change(move |on, _, cx| {
                    let _ = no_auth_this.update(cx, |this, cx| {
                        this.smtp_no_auth = on;
                        cx.emit(ServerEvent::Edited);
                        cx.notify();
                    });
                }),
            )
            .when(no_auth, |el| {
                el.child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(t.color.ink_2)
                        .child(tr!("Komorebi sends without a password, for a relay that trusts your network. The connection still has to be encrypted: a server without TLS is refused.")),
                )
            })
            .when(!no_auth, |el| {
                el.child(
                    Checkbox::new("server-same-login", tr!("Same sign-in as incoming"), !own)
                        .on_change(move |same, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.smtp_own_login = !same;
                                cx.emit(ServerEvent::Edited);
                                cx.notify();
                            });
                        }),
                )
            })
            .children(user_late)
            .when(own && !no_auth, |el| {
                el.child(labelled(
                    tr!("SMTP username"),
                    TextField::new(&self.smtp_user, tr!("SMTP username")),
                    None,
                    cx,
                ))
            })
    }
}
