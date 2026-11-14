//! The mail view's cached panes (docs/PERF.md, optimization pass): the
//! folder pane and the message list are views of their own, embedded with
//! `.cached(..)`, so that a frame drawn for one of them (a wheel step in the
//! list) does not lay out, prepaint and paint the others again. GPUI replays
//! a cached view's last frame until it, or a view inside it, is notified, or
//! its bounds change (Skua's `cookbook_cached_view`).
//!
//! A pane reads the mail view's state, so it observes the mail view and the
//! hub (a cached view goes stale otherwise): every notify of either redraws
//! the panes exactly as the uncached tree did. Only a notify of the pane
//! itself (the list's wheel, its loader) leaves the other panes replayed.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, Render, StyleRefinement, Styled,
    Subscription, WeakEntity, Window, div, px,
};

use super::MailView;
use super::hub::Hub;

/// Which pane a [`PaneView`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Which {
    Folders,
    List,
}

/// The shape the mail view gave its panes in its last render (the width
/// steps are decided there, from the window).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Shape {
    /// The folder pane shows as the rail.
    pub rail: bool,
    /// The list takes the room the reading pane leaves (or all of it).
    pub wide: bool,
    /// The list's width when it is not `wide`.
    pub list_w: f32,
}

impl Shape {
    /// The cached folder pane's box.
    pub fn folders_style(&self) -> StyleRefinement {
        let width = if self.rail {
            crate::theme::layout::RAIL_W
        } else {
            crate::theme::layout::NAV_W
        };
        StyleRefinement::default().flex_none().w(px(width)).h_full()
    }

    /// The cached list's box.
    pub fn list_style(&self) -> StyleRefinement {
        let style = StyleRefinement::default().h_full().min_w_0();
        if self.wide {
            style.flex_1()
        } else {
            style.flex_none().w(px(self.list_w))
        }
    }
}

/// A pane of the mail view as a view of its own.
pub(super) struct PaneView {
    mail: WeakEntity<MailView>,
    which: Which,
    _subscriptions: [Subscription; 2],
}

impl PaneView {
    pub fn new(
        mail: &Entity<MailView>,
        hub: &Entity<Hub>,
        which: Which,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            mail: mail.downgrade(),
            which,
            _subscriptions: [
                cx.observe(mail, |_, _, cx| cx.notify()),
                cx.observe(hub, |_, _, cx| cx.notify()),
            ],
        }
    }
}

// How many times each pane drew on this thread: folders, list (tests).
#[cfg(test)]
thread_local! {
    static RENDERS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// The draws of the folder pane and of the list so far on this thread.
#[cfg(test)]
pub(super) fn renders() -> (usize, usize) {
    RENDERS.with(std::cell::Cell::get)
}

impl Render for PaneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        RENDERS.with(|n| {
            let (folders, list) = n.get();
            n.set(match self.which {
                Which::Folders => (folders + 1, list),
                Which::List => (folders, list + 1),
            });
        });
        let Some(mail) = self.mail.upgrade() else {
            return div().size_full().into_any_element();
        };
        let which = self.which;
        mail.update(cx, |view, cx| -> AnyElement {
            let shape = view.shape;
            match which {
                Which::Folders => super::folders::render(view, shape.rail, window, cx),
                Which::List => super::list::render(view, window, cx),
            }
        })
    }
}

/// The cached panes of one mail view.
pub(super) struct Panes {
    folders: Entity<PaneView>,
    list: Entity<PaneView>,
}

impl MailView {
    /// The folder pane's and the list's views, made on first use.
    pub(super) fn panes(&mut self, cx: &mut Context<Self>) -> (Entity<PaneView>, Entity<PaneView>) {
        if self.panes.is_none() {
            let mail = cx.entity();
            let hub = self.hub.clone();
            self.panes = Some(Panes {
                folders: cx.new(|cx| PaneView::new(&mail, &hub, Which::Folders, cx)),
                list: cx.new(|cx| PaneView::new(&mail, &hub, Which::List, cx)),
            });
        }
        match &self.panes {
            Some(p) => (p.folders.clone(), p.list.clone()),
            None => unreachable!("made above"),
        }
    }

    /// Redraw the list alone (the other panes are replayed); the whole view
    /// when the panes are not made yet.
    pub(super) fn notify_list(&self, cx: &mut Context<Self>) {
        match &self.panes {
            Some(p) => p.list.update(cx, |_, cx| cx.notify()),
            None => cx.notify(),
        }
    }
}
