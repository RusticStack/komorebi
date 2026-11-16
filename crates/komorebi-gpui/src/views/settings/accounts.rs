//! Settings > Accounts (the Slint app's account cards, `account_status`):
//! one card per account (its spot, its name and address, the line that
//! says how its last sync went, with the fix as a button), Add account,
//! and the empty page with no account yet. What the workers report comes
//! from the shell ([`SettingsView::set_account_state`]); the GPUI app's
//! Settings only shows it.

use std::collections::HashMap;

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use komorebi::mail::errors::{Attention, Problem};
use komorebi_i18n::tr;

use super::card::{CARD_MAX_W, Card, note, page_head};
use super::{Section, SettingsView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::ring_style;
use crate::widgets::spot::SenderSpot;

/// An account's last sync, for its line (this session only; the shell
/// reports it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountState {
    Syncing,
    /// Unix seconds of the last sync that went through.
    Checked(i64),
    /// The server could not be reached.
    Unreachable,
    Failed(Problem),
}

/// What the shell knows of an account's workers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountLine {
    pub state: Option<AccountState>,
    pub attention: Option<Attention>,
    /// The first download: (done, total), (0, 0) before the first count.
    pub download: Option<(usize, usize)>,
}

/// An account's line in Settings (empty before its first sync), the Slint
/// app's `account_status`.
pub fn account_status(
    state: Option<&AccountState>,
    attention: Option<Attention>,
    host: &str,
) -> String {
    match attention {
        Some(Attention::SignIn) => return text::account_sign_in_rejected(),
        Some(Attention::Secure) => return text::account_secure_failed(),
        None => {}
    }
    match state {
        None => String::new(),
        Some(AccountState::Syncing) => text::account_syncing(),
        Some(AccountState::Checked(at)) => {
            match chrono::TimeZone::timestamp_opt(&chrono::Local, *at, 0).single() {
                Some(time) => text::account_checked(&time.format("%H:%M").to_string()),
                None => text::account_up_to_date(),
            }
        }
        Some(AccountState::Unreachable) => text::account_unreachable(&crate::text::host(host)),
        Some(AccountState::Failed(p)) => text::account_sync_failed(&problem(p)),
    }
}

/// What a problem says: its sentence, or the error's own words.
fn problem(p: &Problem) -> String {
    p.say()
        .map(|s| crate::text::say(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| p.detail.clone())
}

/// Whether an account's line is a warning (shown in full, in ink with the
/// warning glyph, never cut short): a latch, an unreachable server or a
/// failed sync. The brand has no danger colour: ink and the glyph carry it
/// (the Slint app's `Theme.danger` line).
pub fn is_warning(line: &AccountLine) -> bool {
    line.attention.is_some()
        || matches!(
            line.state,
            Some(AccountState::Unreachable | AccountState::Failed(_))
        )
}

/// The first download's share done, once it has a count.
pub fn download_fraction(line: &AccountLine) -> Option<f32> {
    match line.download {
        Some((done, total)) if total > 0 => Some((done as f32 / total as f32).clamp(0., 1.)),
        _ => None,
    }
}

/// The fix an account's line offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// The sign-in was rejected: the password field, selected.
    UpdatePassword,
    /// A changed or untrusted certificate: its review.
    ReviewCertificate,
}

/// The fix for an account's line, if any.
pub fn fix_of(line: &AccountLine) -> Option<Fix> {
    match line.attention {
        Some(Attention::SignIn) => Some(Fix::UpdatePassword),
        Some(Attention::Secure) => Some(Fix::ReviewCertificate),
        None => match &line.state {
            Some(AccountState::Failed(p)) if p.changed.is_some() => Some(Fix::ReviewCertificate),
            _ => None,
        },
    }
}

impl SettingsView {
    /// What the shell knows of account `id`'s workers.
    pub fn set_account_state(&mut self, id: i64, line: AccountLine, cx: &mut Context<Self>) {
        self.lines.insert(id, line);
        cx.notify();
    }

    /// The account lines the shell reported.
    pub fn account_lines(&self) -> &HashMap<i64, AccountLine> {
        &self.lines
    }

    fn account_card(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let a = &self.accounts[ix];
        let id = a.id;
        let line = self.lines.get(&id).cloned().unwrap_or_default();
        let shown = if a.name.trim().is_empty() {
            a.email.clone()
        } else {
            a.name.clone()
        };
        let status = match line.download {
            Some(_) if a.enabled => tr!("Downloading the mail of {0}", a.email),
            _ if !a.enabled => tr!("account switched off" => "Off"),
            _ => account_status(line.state.as_ref(), line.attention, &a.imap.host),
        };
        let fix = fix_of(&line).filter(|_| a.enabled);
        let downloading = a.enabled && line.download.is_some();
        let warning = a.enabled && !downloading && is_warning(&line);
        let fraction = download_fraction(&line).filter(|_| downloading);
        let email = a.email.clone();
        let caption = if status.is_empty() || shown == email {
            if status.is_empty() {
                email.clone()
            } else {
                status.clone()
            }
        } else {
            format!("{email} · {status}")
        };
        let progress_name = tr!("Downloading the mail of {0}", email);
        div()
            .id(SharedString::from(format!("settings-account-{id}")))
            .role(Role::Button)
            .aria_label(tr!("Edit {0}", email))
            .tab_index(0)
            .w_full()
            .max_w(px(CARD_MAX_W))
            .min_h(px(layout::CARD_H))
            .mb(px(space::SP_1))
            .px(px(space::SP_4))
            .flex()
            .items_center()
            .gap(px(space::SP_4))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .hover(move |s| s.bg(c.wash))
            .focus_visible(ring_style(&t))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.edit_account(id, window, cx)))
            .child(SenderSpot::new(&shown).on_wash(true))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(space::SP_3))
                    .child(div().typo(&t.text.body).truncate().child(shown.clone()))
                    .when(!warning, |el| {
                        el.child(
                            div()
                                .mt(px(space::SP_0_5))
                                .typo(&t.text.caption)
                                .text_color(c.ink_3)
                                .truncate()
                                .child(caption.clone()),
                        )
                    })
                    .when(warning, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("settings-account-warning-{id}")))
                                .role(Role::Label)
                                .aria_label(tr!("Warning: {0}", caption))
                                .mt(px(space::SP_0_5))
                                .flex()
                                .items_start()
                                .gap(px(space::SP_1))
                                .typo(&t.text.caption)
                                .text_color(c.ink)
                                .child(
                                    svg()
                                        .path(Icon::Warning.path())
                                        .size(px(layout::ICON))
                                        .flex_none()
                                        .text_color(c.ink),
                                )
                                .child(div().flex_1().min_w_0().child(caption.clone()))
                                .test_support(),
                        )
                    })
                    .when(downloading, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!(
                                    "settings-account-progress-{id}"
                                )))
                                .role(Role::ProgressIndicator)
                                .aria_label(progress_name.clone())
                                .mt(px(space::SP_2))
                                .w_full()
                                .h(px(space::SP_1))
                                .rounded(px(radius::FULL))
                                .bg(c.wash)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded(px(radius::FULL))
                                        .bg(c.accent)
                                        .w(gpui::relative(fraction.unwrap_or(0.))),
                                )
                                .test_support(),
                        )
                    }),
            )
            .when_some(fix, |el, fix| {
                let (label, name) = match fix {
                    Fix::UpdatePassword => (
                        tr!("Update password"),
                        tr!("Update password for {0}", email),
                    ),
                    Fix::ReviewCertificate => (
                        tr!("Review certificate"),
                        tr!("Review the certificate of {0}", email),
                    ),
                };
                el.child(
                    Button::new(SharedString::from(format!("settings-account-fix-{id}")))
                        .label(label)
                        .name(name)
                        .variant(Variant::OnInset)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.fix_account(id, fix, window, cx);
                            cx.stop_propagation();
                        })),
                )
            })
            .child(
                svg()
                    .path(Icon::ChevronRight.path())
                    .size(px(layout::ICON))
                    .flex_none()
                    .text_color(c.ink_2),
            )
            .test_support()
    }

    pub(crate) fn render_accounts(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cards: Vec<_> = (0..self.accounts.len())
            .map(|ix| self.account_card(ix, cx).into_any_element())
            .collect();
        let empty = cards.is_empty();
        let add = Card::new("settings-add-account", tr!("Add account"))
            .icon_as_spot(Icon::Add)
            .note(tr!(
                "Gmail, iCloud, Fastmail, Postale.io and more, or any IMAP and SMTP server."
            ))
            .control(
                Button::new("settings-add-account-button")
                    .label(if empty {
                        tr!("Add account")
                    } else {
                        tr!("Add another account")
                    })
                    .variant(if empty {
                        Variant::Primary
                    } else {
                        Variant::OnInset
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.add_account(window, cx))),
            );
        let import = Card::new("settings-import-outlook", tr!("Import from Outlook"))
            .icon_as_spot(Icon::MoveTo)
            .note(tr!(
                "Your accounts and signatures from classic Outlook on this PC. Your mail downloads again from the server."
            ))
            .control(
                Button::new("settings-import-outlook-button")
                    .label(tr!("Import"))
                    .name(tr!("Import from Outlook"))
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(|this, _, window, cx| this.import_outlook(window, cx))),
            );
        div()
            .flex()
            .flex_col()
            .child(page_head(Section::Accounts.name(), None, cx))
            .when(empty, |el| {
                el.child(
                    div()
                        .mt(px(space::SP_4))
                        .child(note(tr!("No accounts yet"), cx)),
                )
            })
            .when(!empty, |el| {
                el.child(div().mt(px(space::SP_6)).children(cards))
            })
            .child(div().mt(px(space::SP_4)).child(add))
            .child(div().mt(px(space::SP_2)).child(import))
    }

    /// The fix on an account's line: its page, with the password field
    /// selected or the certificate review started.
    pub fn fix_account(&mut self, id: i64, fix: Fix, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_account(id, window, cx);
        let changed = match self.lines.get(&id).and_then(|l| l.state.clone()) {
            Some(AccountState::Failed(p)) => p.changed,
            _ => None,
        };
        if let Some(editor) = self.editor.clone() {
            editor.update(cx, |e, cx| match fix {
                Fix::UpdatePassword => e.focus_password(window, cx),
                Fix::ReviewCertificate => e.review_changed(changed.as_deref(), window, cx),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use komorebi::i18n::Lang;
    use komorebi::mail::errors::Class;

    use super::*;

    /// Ports `account_status_lines` (src/app.rs).
    #[test]
    fn account_status_lines() {
        komorebi_i18n::with_lang(Lang::En, || {
            let host = "imap.example.org";
            assert_eq!(account_status(None, None, host), "");
            assert_eq!(
                account_status(Some(&AccountState::Syncing), None, host),
                "Syncing\u{a0}…"
            );
            let at = chrono::Utc::now().timestamp();
            let checked = account_status(Some(&AccountState::Checked(at)), None, host);
            assert!(checked.starts_with("Up to date, checked "), "{checked}");
            assert_eq!(checked.len(), "Up to date, checked 12:41".len());
            assert_eq!(
                account_status(Some(&AccountState::Unreachable), None, host),
                "Can't reach imap.example.org"
            );
            // A rejected sign-in wins over the last pass.
            assert_eq!(
                account_status(
                    Some(&AccountState::Checked(at)),
                    Some(Attention::SignIn),
                    host
                ),
                "Sign-in rejected"
            );
            assert_eq!(
                account_status(None, Some(Attention::Secure), host),
                "Secure connection failed"
            );
            let failed = AccountState::Failed(Problem {
                class: Class::Other,
                detail: "the server said no".into(),
                changed: None,
            });
            assert_eq!(
                account_status(Some(&failed), None, host),
                "Last sync failed: the server said no"
            );
        });
    }

    #[test]
    fn a_changed_certificate_or_a_latch_offers_its_fix() {
        let line = |state, attention| AccountLine {
            state,
            attention,
            download: None,
        };
        assert_eq!(fix_of(&line(None, None)), None);
        assert_eq!(
            fix_of(&line(None, Some(Attention::SignIn))),
            Some(Fix::UpdatePassword)
        );
        let changed = AccountState::Failed(Problem {
            class: Class::TlsCert,
            detail: String::new(),
            changed: Some("mail.example.org".into()),
        });
        assert_eq!(
            fix_of(&line(Some(changed), None)),
            Some(Fix::ReviewCertificate)
        );
    }
}
