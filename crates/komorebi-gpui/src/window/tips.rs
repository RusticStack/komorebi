//! The teaching tips (the Slint app's `src/app/tips.rs` and `TeachingTip`,
//! docs/WINUI3.md "TeachingTip"): a few first-run tips, one at a time, over
//! the mail. Which tip comes next is `komorebi::onboard`'s; this module
//! decides when they start, where they show, and saves the `tips_seen`
//! setting.
//!
//! - The desktop series is Search, then Keys and help; with the folder
//!   pane folded into the rail, Your folders comes first (and goes with the
//!   rail when the window widens again, still unseen).
//! - A tip starts 1.5 s after the window's work starts (never in a test or
//!   a snapshot), and waits while something is in the way: Settings,
//!   Contacts, a dialog, a message or draft tab, the composer, or the
//!   What's new bar. The What's new bar waits for the tips in turn.
//! - Non-modal and never focused by itself: Got it (or Esc, before any
//!   other use of Esc) shows the next tip; Skip tips ends them in both
//!   layouts. Settings > About > Show tips again makes every tip due again.

use std::time::Duration;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, KeystrokeEvent, ParentElement as _,
    Role, Styled as _, Subscription, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::onboard::{Onboarding, Seen, Shown, Tip};
use komorebi_i18n::tr;

use super::MainWindow;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::mail::sync::StatusAction;
use crate::views::updates;
use crate::widgets::button::{Button, Variant};

/// From the start (or a tip that waited) to the next try.
pub const DELAY: Duration = Duration::from_millis(1500); // token-exempt: the Slint app's tip delay, a time not a size
/// From Show tips again to the first tip.
pub const AGAIN: Duration = Duration::from_millis(500); // token-exempt: the Slint app's Show tips again delay

/// The tips' state of this run.
#[derive(Default)]
pub(super) struct Tips {
    pub(super) onboarding: Onboarding,
    /// The layout of the last frame (the pane folded into the rail).
    compact: Option<bool>,
    /// The timer found no account (a first run): the tips arm again on
    /// the first frame with one, as the Slint app's started when the list
    /// first showed an account's rows.
    wait_account: bool,
    /// The last delay asked of [`MainWindow::arm_tips`] (tests check it;
    /// no timer runs in them).
    #[cfg(test)]
    armed: Option<Duration>,
}

impl Tips {
    /// The tips of a cache with these settings (`whats_new_due`: the bar
    /// waits for the tips).
    pub(super) fn new(tips_seen: &str, whats_new_due: bool) -> Self {
        Self {
            onboarding: Onboarding::new(Seen::parse(tips_seen), whats_new_due),
            ..Self::default()
        }
    }
}

// ----------------------------------------------------------------- words

pub fn title(tip: Tip) -> String {
    match tip {
        Tip::Folders => tr!("teaching tip" => "Your folders"),
        Tip::Search => tr!("teaching tip" => "Search"),
        Tip::Help => tr!("teaching tip" => "Keys and help"),
    }
}

pub fn body(tip: Tip) -> String {
    match tip {
        Tip::Folders => {
            tr!("Accounts and folders are behind this button when the window is narrow.")
        }
        Tip::Search => tr!("Search this folder or all folders. Ctrl+E or Ctrl+F opens it."),
        Tip::Help => tr!("Keyboard shortcuts and help are in Settings › About, or press F1."),
    }
}

pub fn place(n: usize, of: usize) -> String {
    tr!("{0} of {1}", n, of)
}

pub fn got_it() -> String {
    tr!("Got it")
}

pub fn skip() -> String {
    tr!("Skip tips")
}

// ------------------------------------------------------------ the window

impl MainWindow {
    /// The window's layout: the folder pane shows as the rail.
    fn tips_compact(&self, window: &Window, cx: &gpui::App) -> bool {
        let folded = self
            .mail
            .as_ref()
            .is_some_and(|m| m.read(cx).mail().nav_collapsed());
        folded || window.viewport_size().width.as_f32() < layout::COMPACT_BELOW_W
    }

    fn has_account(&self, cx: &gpui::App) -> bool {
        self.mail
            .as_ref()
            .is_some_and(|m| !m.read(cx).mail().accounts().is_empty())
    }

    /// Something a tip must not show over: a place (Settings, Contacts, the
    /// first-run flow), another tab, a dialog, the pane's composer.
    fn tip_blocked(&self, window: &mut Window, cx: &mut gpui::App) -> bool {
        use gpui_kit::component::WindowExt as _;
        self.place.is_some()
            || self.tabs.active() != 0
            || window.has_active_dialog(cx)
            || self
                .mail
                .as_ref()
                .is_some_and(|m| m.read(cx).composer().read(cx).is_open())
    }

    /// The What's new bar is on the status line.
    fn whats_new_up(&self, cx: &gpui::App) -> bool {
        self.hub.as_ref().is_some_and(|h| {
            h.read(cx)
                .sync()
                .line()
                .is_some_and(|l| l.action == StatusAction::WhatsNew)
        })
    }

    /// The tip on screen (shown or waiting behind something).
    pub fn tip(&self) -> Option<Shown> {
        self.tips.onboarding.current()
    }

    /// The tip drawn now: one is running, nothing is in its way, and a
    /// Your folders tip has its rail.
    pub(super) fn tip_visible(&self, window: &mut Window, cx: &mut gpui::App) -> Option<Shown> {
        let shown = self.tip()?;
        if self.tip_blocked(window, cx)
            || (shown.tip == Tip::Folders && !self.tips_compact(window, cx))
        {
            return None;
        }
        Some(shown)
    }

    /// Try again after `after` (never in a test or a snapshot).
    #[cfg_attr(test, allow(unused_variables))]
    pub(super) fn arm_tips(
        &mut self,
        after: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        {
            self.tips.armed = Some(after);
        }
        #[cfg(not(test))]
        if self.snapshot.is_none() {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(after).await; // timer-exempt: the Slint app's teaching tips start 1.5 s after the list shows
                let _ = this.update_in(cx, |this, window, cx| this.tips_due_now(window, cx));
            })
            .detach();
        }
    }

    /// The tips' timer: start the series due in this layout, or wait
    /// while something is in the way (no timer runs in tests). With no
    /// account yet (a first run) it waits for the first frame with one.
    pub(super) fn tips_due_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_account(cx) {
            self.tips.wait_account = true;
            return;
        }
        let compact = self.tips_compact(window, cx);
        if !self.tips.onboarding.tips_due(compact) || self.tip().is_some() {
            return;
        }
        if self.tip_blocked(window, cx) || self.whats_new_up(cx) {
            return self.arm_tips(DELAY, window, cx);
        }
        self.start_tips(window, cx);
    }

    /// Start the series due in this layout now (the timer, and tests).
    /// Returns the tip shown.
    pub fn start_tips(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Shown> {
        if !self.has_account(cx) || self.whats_new_up(cx) {
            return None;
        }
        let compact = self.tips_compact(window, cx);
        self.tips.compact = Some(compact);
        let shown = self.tips.onboarding.start(compact);
        cx.notify();
        shown
    }

    fn save_tips(&self) {
        let seen = self.tips.onboarding.seen().format();
        if let Some(db) = &self.db
            && let Err(e) = db.set_setting("tips_seen", &seen)
        {
            eprintln!("komorebi-gpui: saving tips_seen: {e:#}");
        }
    }

    /// Got it (or Esc): the next tip, or after the last the What's new bar
    /// if it waited (and Your folders if the window folded meanwhile).
    pub fn tip_got_it(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.tips.onboarding.got_it();
        self.save_tips();
        if next.is_none() {
            self.whats_new_due_now(cx);
            if self.tips.onboarding.tips_due(self.tips_compact(window, cx)) {
                self.arm_tips(DELAY, window, cx);
            }
        }
        cx.notify();
    }

    /// Skip tips: none again, in either layout.
    pub fn tip_skip(&mut self, cx: &mut Context<Self>) {
        self.tips.onboarding.skip();
        self.save_tips();
        self.whats_new_due_now(cx);
        cx.notify();
    }

    /// Settings > About > Show tips again: every tip is due again, and the
    /// series starts shortly, over the mail.
    pub(super) fn show_tips_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tips.onboarding.reset();
        self.save_tips();
        self.close_place(window, cx);
        self.arm_tips(AGAIN, window, cx);
        cx.notify();
    }

    /// The What's new bar, if it is due and no tip is up (its timer, and
    /// the end of a series). Once a run.
    pub(super) fn whats_new_due_now(&mut self, cx: &mut Context<Self>) {
        if self.tip().is_some() || !self.tips.onboarding.bar_ready() {
            return;
        }
        if let (Some(db), Some(hub)) = (self.db.clone(), self.hub.clone())
            && updates::whats_new::show_if_due(&db, &hub, cx)
        {
            self.tips.onboarding.bar_shown();
        }
    }

    /// Each frame: the layout may have changed (a Your folders tip goes
    /// with the rail), and a first run may have its first account now.
    pub(super) fn tips_follow_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tips.wait_account && self.has_account(cx) {
            self.tips.wait_account = false;
            self.arm_tips(DELAY, window, cx);
        }
        let compact = self.tips_compact(window, cx);
        if self.tips.compact.replace(compact) != Some(compact) {
            self.tips.onboarding.layout(compact);
        }
        if self.tips.onboarding.bar_up() && !self.whats_new_up(cx) {
            self.tips.onboarding.bar_closed();
        }
    }

    /// Esc closes the tip first (as Got it), before any view's Esc.
    pub(super) fn tips_esc(window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        let this = cx.entity().downgrade();
        let main = window.window_handle();
        cx.intercept_keystrokes(move |event: &KeystrokeEvent, window, cx| {
            if event.keystroke.key != "escape"
                || event.keystroke.modifiers.modified()
                || window.window_handle() != main
            {
                return;
            }
            let took = this
                .update(cx, |this, cx| {
                    if this.tip_visible(window, cx).is_some() {
                        this.tip_got_it(window, cx);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            if took {
                cx.stop_propagation();
            }
        })
    }

    /// The tip, over the content beside its target: Search under the
    /// title bar's search, Keys and help at the foot of the folder pane,
    /// Your folders beside the rail.
    pub(super) fn tip_layer(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let shown = self.tip_visible(window, cx)?;
        let t = cx.tokens().clone();
        let c = t.color;
        let w = window.viewport_size().width.as_f32();
        let width = layout::TOOLTIP_MAX_W.min(w - space::SP_2 - space::SP_2);
        let top = layout::TITLEBAR_H + space::SP_2;
        let left = match shown.tip {
            Tip::Search => ((w - width) / 2.).max(space::SP_2),
            Tip::Help if !self.tips_compact(window, cx) => space::SP_2,
            Tip::Help | Tip::Folders => layout::RAIL_W + space::SP_2,
        };
        let title = title(shown.tip);
        let this = cx.entity().downgrade();
        let skip_this = this.clone();
        let card = div()
            .id("teaching-tip")
            .role(Role::Group)
            .aria_label(title.clone())
            .aria_description(body(shown.tip))
            .absolute()
            .left(px(left))
            .w(px(width))
            .map(|el| match shown.tip {
                Tip::Help => el.bottom(px(space::SP_2)),
                _ => el.top(px(top)),
            })
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .p(px(space::SP_4))
            .rounded(px(radius::CARD))
            .border(px(layout::HAIRLINE))
            .border_color(c.line)
            .bg(c.paper)
            .text_color(c.ink)
            .occlude()
            .child(div().typo(&t.text.subtitle).child(title))
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(body(shown.tip)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SP_2))
                    .child(
                        div()
                            .flex_1()
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(place(shown.n, shown.of)),
                    )
                    .child(
                        Button::new("tip-skip")
                            .label(skip())
                            .variant(Variant::Subtle)
                            .on_click(move |_, _, cx| {
                                let _ = skip_this.update(cx, |this, cx| this.tip_skip(cx));
                            }),
                    )
                    .child(
                        Button::new("tip-got-it")
                            .label(got_it())
                            .primary()
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |this, cx| this.tip_got_it(window, cx));
                            }),
                    ),
            )
            .test_support();
        Some(card.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, assert_named, click_named, in_window, names, press};
    use gpui::{AnyWindowHandle, Entity, TestAppContext, px, size};

    fn open(
        name: &str,
        cx: &mut TestAppContext,
    ) -> (
        test_support::TempCache,
        komorebi::db::Db,
        AnyWindowHandle,
        Entity<MainWindow>,
    ) {
        let (cache, db, _) = test_support::seeded(name, 3);
        // Told already: the What's new bar is not due.
        updates::whats_new::seen(&db);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
        (cache, db, window, view)
    }

    fn start(
        window: AnyWindowHandle,
        view: &Entity<MainWindow>,
        cx: &mut TestAppContext,
    ) -> Option<(Tip, usize, usize)> {
        let v = view.clone();
        let shown = cx
            .update_window(window, move |_, w, cx| {
                v.update(cx, |v, cx| v.start_tips(w, cx))
            })
            .expect("start");
        cx.run_until_parked();
        shown.map(|s| (s.tip, s.n, s.of))
    }

    fn tip(view: &Entity<MainWindow>, cx: &mut TestAppContext) -> Option<(Tip, usize, usize)> {
        view.read_with(cx, |v, _| v.tip().map(|s| (s.tip, s.n, s.of)))
    }

    fn seen(db: &komorebi::db::Db) -> String {
        db.settings().expect("settings").tips_seen
    }

    fn resize(window: AnyWindowHandle, w: f32, h: f32, cx: &mut TestAppContext) {
        cx.simulate_window_resize(window, size(px(w), px(h)));
        cx.run_until_parked();
    }

    /// Port of `tips_follow_the_layout_and_esc_closes_only_the_tip` (row
    /// 175): the desktop series is Search then Keys and help; the tip is a
    /// named group with its buttons; Esc closes the tip only (as Got it),
    /// not the open message; nothing more on the desktop; a dialog or
    /// Settings hides the tip, which comes back after.
    #[gpui_kit::test]
    fn tips_follow_the_layout_and_esc_closes_only_the_tip(cx: &mut TestAppContext) {
        let (_cache, db, window, view) = open("tips", cx);
        assert_eq!(start(window, &view, cx), Some((Tip::Search, 1, 2)));
        in_window(cx, window, |window, _| {
            assert_named(window, "Search", Role::Group);
            assert_named(window, "Got it", Role::Button);
            assert_named(window, "Skip tips", Role::Button);
        });
        // A message open: Esc closes the tip only.
        let mail = view.read_with(cx, |v, _| v.mail().cloned()).expect("mail");
        let ids = db
            .message_ids(&[db.folders().expect("folders")[0].id], "")
            .expect("ids");
        let m = mail.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.select_message(ids[0]);
                })
            })
        });
        cx.run_until_parked();
        let open = |cx: &mut TestAppContext| mail.read_with(cx, |m, _| m.mail().open_id());
        assert!(open(cx).is_some());
        in_window(cx, window, |window, cx| press(window, &["escape"], cx));
        cx.run_until_parked();
        assert_eq!(tip(&view, cx), Some((Tip::Help, 2, 2)));
        assert!(open(cx).is_some(), "the message stays open");
        assert_eq!(seen(&db), "search");
        // Settings hides it; closing Settings brings it back.
        let shown = |cx: &mut TestAppContext| {
            in_window(cx, window, |window, _| {
                names(window).iter().any(|(_, n)| n == "Keys and help")
            })
        };
        assert!(shown(cx));
        let v = view.clone();
        cx.update_window(window, move |_, w, cx| {
            v.update(cx, |v, cx| {
                v.show_place(crate::views::mail::Place::Settings, w, cx)
            })
        })
        .expect("settings");
        cx.run_until_parked();
        assert!(!shown(cx), "hidden under Settings");
        let v = view.clone();
        cx.update_window(window, move |_, w, cx| {
            v.update(cx, |v, cx| v.close_place(w, cx))
        })
        .expect("close");
        cx.run_until_parked();
        assert!(shown(cx), "back");
        in_window(cx, window, |w, cx| click_named(w, "Got it", cx));
        cx.run_until_parked();
        assert_eq!(tip(&view, cx), None);
        assert_eq!(seen(&db), "search,help");
        assert_eq!(
            start(window, &view, cx),
            None,
            "nothing more on the desktop"
        );
    }

    /// Port of `the_compact_series_has_three_tips_and_skip_ends_both_layouts`
    /// (row 176): folded into the rail, Your folders comes first, 1 of 3;
    /// it goes with the rail; Skip tips ends them in both layouts; Show
    /// tips again makes them due again and closes Settings.
    #[gpui_kit::test]
    fn the_compact_series_has_three_tips_and_skip_ends_both_layouts(cx: &mut TestAppContext) {
        let (_cache, db, window, view) = open("tips-compact", cx);
        resize(window, 900., 700., cx);
        assert_eq!(start(window, &view, cx), Some((Tip::Folders, 1, 3)));
        in_window(cx, window, |window, _| {
            assert_named(window, "Your folders", Role::Group)
        });
        let v = view.clone();
        cx.update_window(window, move |_, w, cx| {
            v.update(cx, |v, cx| v.tip_got_it(w, cx))
        })
        .expect("got it");
        cx.run_until_parked();
        assert_eq!(tip(&view, cx), Some((Tip::Search, 2, 3)));
        assert_eq!(seen(&db), "folders");
        in_window(cx, window, |w, cx| click_named(w, "Skip tips", cx));
        cx.run_until_parked();
        assert_eq!(tip(&view, cx), None);
        assert_eq!(seen(&db), "folders,search,help");
        assert_eq!(start(window, &view, cx), None);
        resize(window, 1280., 800., cx);
        assert_eq!(start(window, &view, cx), None);
        // Settings > About > Show tips again.
        let v = view.clone();
        cx.update_window(window, move |_, w, cx| {
            v.update(cx, |v, cx| {
                v.show_place(crate::views::mail::Place::Settings, w, cx);
                v.show_tips_again(w, cx);
            })
        })
        .expect("again");
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |v, _| v.place().is_none()),
            "Settings closed"
        );
        assert_eq!(seen(&db), "");
        assert_eq!(start(window, &view, cx), Some((Tip::Search, 1, 2)));
    }

    /// A Your folders tip goes with the rail when the window widens, still
    /// unseen.
    #[gpui_kit::test]
    fn the_folders_tip_goes_with_the_rail(cx: &mut TestAppContext) {
        let (_cache, db, window, view) = open("tips-rail", cx);
        resize(window, 900., 700., cx);
        assert_eq!(start(window, &view, cx), Some((Tip::Folders, 1, 3)));
        resize(window, 1280., 800., cx);
        in_window(cx, window, |_, _| {});
        assert_eq!(tip(&view, cx), None);
        assert_eq!(seen(&db), "");
    }

    /// A first run: the window starts with no account, so the tips' timer
    /// finds none and waits; the first frame with an account (after the
    /// first-run flow adds it) arms the tips again, once, and they start.
    #[gpui_kit::test]
    fn a_first_run_starts_the_tips_once_it_has_an_account(cx: &mut TestAppContext) {
        let cache = test_support::TempCache::new("tips-first-run");
        let mut db = cache.open();
        db.set_setting("language", "en").expect("language");
        updates::whats_new::seen(&db);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
        let due = |cx: &mut TestAppContext| {
            let v = view.clone();
            cx.update_window(window, move |_, w, cx| {
                v.update(cx, |v, cx| v.tips_due_now(w, cx))
            })
            .expect("due");
            cx.run_until_parked();
        };
        // The window's work started (its own arm); the timer then fires.
        in_window(cx, window, |_, _| {});
        view.update(cx, |v, _| v.tips.armed = None);
        due(cx);
        assert_eq!(tip(&view, cx), None, "no account, no tip");
        in_window(cx, window, |_, _| {});
        assert_eq!(view.read_with(cx, |v, _| v.tips.armed), None);
        // The first-run flow adds the first account and closes.
        komorebi::seed::seed(&mut db, 3).expect("seed");
        let mail = view.read_with(cx, |v, _| v.mail().cloned()).expect("mail");
        let v = view.clone();
        cx.update_window(window, move |_, w, cx| {
            mail.update(cx, |m, cx| m.reload_settings(w, cx));
            v.update(cx, |v, cx| {
                v.close_place(w, cx);
                cx.notify();
            })
        })
        .expect("account");
        cx.run_until_parked();
        in_window(cx, window, |_, _| {});
        assert_eq!(
            view.read_with(cx, |v, _| (v.tips.armed, v.tips.wait_account)),
            (Some(DELAY), false),
            "armed again on the first frame with an account"
        );
        // The timer fires: the series starts.
        due(cx);
        assert_eq!(tip(&view, cx), Some((Tip::Search, 1, 2)));
    }

    /// Port of `whats_new_shows_once_and_never_with_a_tip` (row 177, its
    /// tips half): while a tip is up the What's new bar waits; after the
    /// last tip it shows; while it is up no tip starts.
    #[gpui_kit::test]
    fn whats_new_never_shows_with_a_tip(cx: &mut TestAppContext) {
        let (cache, db, _) = test_support::seeded("tips-bar", 3);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
        assert!(updates::whats_new::due(&db), "after an update");
        assert_eq!(start(window, &view, cx), Some((Tip::Search, 1, 2)));
        view.update(cx, |v, cx| v.whats_new_due_now(cx));
        let bar = |cx: &mut TestAppContext| view.read_with(cx, |v, cx| v.whats_new_up(cx));
        assert!(!bar(cx), "not with a tip");
        view.update(cx, |v, cx| v.tip_skip(cx));
        cx.run_until_parked();
        assert!(bar(cx), "after the last tip");
        // The bar up: no tip starts.
        view.update(cx, |v, _| v.tips.onboarding.reset());
        assert_eq!(start(window, &view, cx), None, "never with the bar");
    }
}
