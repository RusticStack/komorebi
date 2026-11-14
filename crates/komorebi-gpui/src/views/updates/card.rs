//! The update state on Settings > About (the Slint app's update card,
//! `ui/app.slint`): the state as the card's title, what it means under it,
//! Check for updates at its end, a progress bar while it checks or
//! downloads, and under the card what can be done with a version found
//! (Release notes, Download and install or Try again, Later).
//!
//! It reads the window's [`Updater`] and calls its `check`, `download` and
//! `later`; the work and the words of the bar are in `super`.

use gpui::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement, Render,
    Role, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::update::{self, Phase};
use komorebi_i18n::tr;

use super::Updater;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, radius, space};
use crate::views::settings::card::Card;
use crate::widgets::button::{Button, Variant};

/// The card's thin progress bar.
const BAR_H: f32 = 4.;

/// The update card over the window's updater.
pub struct UpdatesCard {
    updater: Entity<Updater>,
    _observe: Subscription,
}

impl UpdatesCard {
    pub fn new(updater: Entity<Updater>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&updater, |_, _, cx| cx.notify());
        Self {
            updater,
            _observe: observe,
        }
    }
}

/// The card's title for a phase.
pub fn title(phase: &Phase) -> String {
    match phase {
        Phase::Idle => tr!("Check for updates"),
        Phase::Checking => tr!("Checking for updates…"),
        Phase::UpToDate => tr!("Komorebi is up to date."),
        Phase::Available(r) => tr!("Version {0} is available.", r.version.to_string()),
        Phase::Downloading(r) => tr!("Downloading version {0}…", r.version.to_string()),
        Phase::Failed(..) => tr!("The update didn't work."),
        Phase::Installing => tr!("Starting the installer…"),
    }
}

/// What the phase means, under the title.
pub fn note(phase: &Phase, progress: Option<(u64, Option<u64>)>) -> String {
    match phase {
        Phase::Failed(e, _) => match update::error_code(e) {
            0 => tr!("Komorebi is offline, so it can't look for updates."),
            1 => tr!("GitHub couldn't be reached. Check the connection and try again."),
            2 => tr!("The release on GitHub isn't complete yet. Try again later."),
            3 => tr!("The download was larger than expected, so it was stopped."),
            4 => tr!("The download didn't pass the signature check, so it wasn't installed."),
            _ => tr!("The installer couldn't be saved or started."),
        },
        Phase::Downloading(_) => match progress {
            Some((done, Some(total))) if total > 0 => {
                let lang = komorebi_i18n::current();
                tr!(
                    "{0} of {1}",
                    komorebi::format::size(lang, done.min(total) as usize),
                    komorebi::format::size(lang, total as usize)
                )
            }
            _ => tr!("The installer is checked against the project's signature before it runs."),
        },
        Phase::Installing => tr!("Komorebi closes now and opens again when the update is done."),
        Phase::Available(_) => tr!("Nothing is downloaded until you press Download and install."),
        _ => tr!(
            "Komorebi asks GitHub for the latest release. Nothing about you or your mail is sent."
        ),
    }
}

/// The progress bar's fill: `None` is indeterminate (a checking phase).
fn bar(fill: Option<f32>, cx: &App) -> AnyElement {
    let c = cx.tokens().color;
    let track = div()
        .h(px(BAR_H))
        .w_full()
        .rounded(px(radius::FULL))
        .bg(c.line)
        .overflow_hidden();
    match fill {
        Some(f) => track
            .child(
                div()
                    .h_full()
                    .w(gpui::relative(f.clamp(0., 1.)))
                    .rounded(px(radius::FULL))
                    .bg(c.ink),
            )
            .into_any_element(),
        None => track
            .child(
                div()
                    .h_full()
                    .w(gpui::relative(0.3))
                    .rounded(px(radius::FULL))
                    .bg(c.ink),
            )
            .into_any_element(),
    }
}

impl Render for UpdatesCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.clone();
        let (phase, progress) = {
            let u = updater.read(cx);
            (u.phase().clone(), u.state().progress)
        };
        let title = title(&phase);
        let note = note(&phase, progress);
        let checking = matches!(phase, Phase::Checking);
        let busy = matches!(
            phase,
            Phase::Checking | Phase::Downloading(_) | Phase::Installing
        );
        let mut card = Card::new("about-update", title.clone())
            .icon(Icon::Sync)
            .note(note);
        if !matches!(
            phase,
            Phase::Available(_) | Phase::Downloading(_) | Phase::Installing
        ) {
            let check = updater.clone();
            card = card.control(
                Button::new("about-update-check")
                    .label(tr!("Check for updates"))
                    .name(tr!("Check GitHub for a newer version of Komorebi"))
                    .variant(Variant::OnInset)
                    .disabled(checking)
                    .on_click(move |_, _, cx| check.update(cx, |u, cx| u.check(false, cx))),
            );
        }
        let release = phase.release().cloned();
        let c = cx.tokens().color;
        let mut col = div().flex().flex_col().child(card);
        if busy && !matches!(phase, Phase::Installing) {
            let fill = match (&phase, progress) {
                (Phase::Downloading(_), Some((done, total))) => update::fraction(done, total),
                _ => None,
            };
            col = col.child(
                div()
                    .id("about-update-progress")
                    .role(Role::ProgressIndicator)
                    .aria_label(match &release {
                        Some(r) => tr!("Downloading version {0}", r.version.to_string()),
                        None => tr!("Checking for updates"),
                    })
                    // How far the download is, for Narrator (0 to 100).
                    .when_some(fill, |el, f| {
                        el.aria_min_numeric_value(0.)
                            .aria_max_numeric_value(100.)
                            .aria_numeric_value((f64::from(f) * 100.).round())
                    })
                    .px(px(space::SP_4))
                    .pb(px(space::SP_2))
                    .child(bar(fill, cx))
                    .test_support(),
            );
        }
        let retry = matches!(phase, Phase::Failed(_, Some(_)));
        if let (Some(r), true) = (release, matches!(phase, Phase::Available(_)) || retry) {
            let version = r.version.to_string();
            let notes = updater.clone();
            let install = updater.clone();
            let later = updater.clone();
            let available = matches!(phase, Phase::Available(_));
            col = col.child(
                div()
                    .mb(px(space::SP_1))
                    .px(px(space::SP_4))
                    .py(px(space::SP_3))
                    .rounded(px(radius::CARD))
                    .bg(c.inset)
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_2))
                    .items_start()
                    .child(
                        Button::new("about-update-notes")
                            .variant(Variant::Subtle)
                            .label(tr!("Release notes"))
                            .name(tr!(
                                "Open the release notes of version {0} in the browser",
                                version.clone()
                            ))
                            .on_click(move |_, _, cx| {
                                notes.update(cx, |u, cx| u.open_notes(cx));
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(space::SP_2))
                            .child(
                                Button::new("about-update-install")
                                    .variant(Variant::Primary)
                                    .label(if available {
                                        tr!("Download and install")
                                    } else {
                                        tr!("Try again")
                                    })
                                    .name(tr!(
                                        "Download version {0}, check its signature, then close Komorebi and install it",
                                        version
                                    ))
                                    .on_click(move |_, _, cx| {
                                        install.update(cx, |u, cx| u.download(cx));
                                    }),
                            )
                            .when(available, |el| {
                                el.child(
                                    Button::new("about-update-later")
                                        .variant(Variant::OnInset)
                                        .label(tr!("update" => "Later"))
                                        .name(tr!("Hide the update bar until tomorrow"))
                                        .on_click(move |_, _, cx| {
                                            later.update(cx, |u, cx| u.later(cx));
                                        }),
                                )
                            }),
                    ),
            );
        }
        col
    }
}
