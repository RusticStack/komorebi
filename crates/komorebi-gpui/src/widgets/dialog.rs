//! Dialogs (cookbook_modal, cookbook_overlays): Kit's `Dialog` for the
//! overlay, focus trap, Escape and Enter, restyled: a paper card with a
//! 12 px radius and no shadow (the vendored `Styled::shadow` hook, patch
//! one of SKUA-PATCH.md) over the flat black overlay at 0.32 (Kit's
//! `overlay` colour). The
//! title is the Subtitle step; the footer is the brand's pills, right
//! aligned, the main action last.
//!
//! [`confirm`] is the confirm dialog: a question, a sentence, Cancel and
//! the action. A destructive action is still ink (the brand has no red):
//! its words say what happens.
//!
//! Kit's dialog host (gpui-base `Dialog`) is a Dialog node, an AlertDialog
//! for a destructive confirm, and takes the focus trap and the focus. The
//! vendored Kit names it by the title (`a11y_label`, KOMOREBI-PATCH.md), so
//! [`confirm`] puts only the sentence (as a description) on its content
//! group. [`region`] is the node for dialogs built on Kit's `Dialog`
//! directly: a Group named by the title. Whether Narrator says the title
//! as the dialog opens wants the Narrator pass.
//!
//! Enter runs the action of a plain confirm. A destructive one opens with
//! the focus on Cancel, Tab stays among its two buttons (Kit's focus trap),
//! and Enter presses the focused button, so Enter alone never runs it (the
//! Slint app's rule, test row 157).
//!
//! A confirm dialog does not close on a press outside it (as Kit's own
//! alert dialog): its Cancel can be a real action (Quit, for the locked
//! cache at start).

use std::rc::Rc;

use gpui::{
    App, Div, InteractiveElement as _, ParentElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;

use super::button::{Button, Variant};
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};

/// What a confirm dialog says.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm: SharedString,
    pub cancel: SharedString,
    pub destructive: bool,
}

/// The node a dialog's content goes in, inside Kit's Dialog host: a Group
/// named `title`, with `description` (the sentence) for Narrator. For
/// dialogs built on Kit's `Dialog` directly.
pub fn region(
    id: &'static str,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
) -> Stateful<Div> {
    let node = div().id(id).role(Role::Group).aria_label(title);
    match description {
        Some(d) => node.aria_description(d),
        None => node,
    }
}

/// Opens a confirm dialog; `on_confirm` runs on the action button or
/// Enter, then the dialog closes. Escape and Cancel close it.
pub fn confirm(
    spec: Confirm,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    confirm_or(spec, on_confirm, |_, _| {}, window, cx);
}

/// [`confirm`], with `on_cancel` run on Cancel and Escape (a start-up
/// question whose Cancel is Quit).
pub fn confirm_or(
    spec: Confirm,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_confirm = Rc::new(on_confirm);
    let on_cancel = Rc::new(on_cancel);
    let t = cx.tokens().clone();
    // The two buttons' own focus handles, so a destructive confirm can start
    // on Cancel and Enter can press the button that has the focus.
    let cancel_focus = cx.focus_handle().tab_index(0).tab_stop(true);
    let confirm_focus = cx.focus_handle().tab_index(0).tab_stop(true);
    let first = cancel_focus.clone();
    let destructive = spec.destructive;
    window.open_dialog(cx, move |dialog, _window, cx| {
        let c = t.color;
        // The host node is named by the title (Kit patch), and a
        // destructive confirm is an AlertDialog node.
        let dialog = dialog.a11y_label(spec.title.clone());
        let dialog = if spec.destructive {
            dialog.alert(cx)
        } else {
            dialog
        };
        let destructive = spec.destructive;
        let ok = on_confirm.clone();
        let enter = on_confirm.clone();
        let cancel = on_cancel.clone();
        let escape = on_cancel.clone();
        let enter_cancel = on_cancel.clone();
        let pressed = confirm_focus.clone();
        dialog
            .w(px(layout::DIALOG_W))
            .bg(c.paper)
            .text_color(c.ink)
            .rounded(px(radius::CARD))
            .shadow(Vec::new())
            // Cancel and Escape close it; Kit's square-ish close glyph would
            // be an icon button without a tooltip.
            .close_button(false)
            // A press outside does not run Cancel (it can be Quit).
            .overlay_closable(false)
            .p(px(space::SP_6))
            .child(
                // The host carries the title, so the content group carries
                // only the sentence (one name per dialog for Narrator).
                div()
                    .id("confirm-dialog")
                    .role(Role::Group)
                    .aria_description(spec.message.clone())
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_2))
                    .child(div().typo(&t.text.subtitle).child(spec.title.clone()))
                    .child(
                        div()
                            .typo(&t.text.body)
                            .text_color(c.ink_2)
                            .child(spec.message.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_end()
                            .gap(px(space::SP_2))
                            .pt(px(space::SP_4))
                            .child(
                                Button::new("dialog-cancel")
                                    .label(spec.cancel.clone())
                                    .track_focus(&cancel_focus)
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        cancel(window, cx);
                                    }),
                            )
                            .child(
                                Button::new("dialog-confirm")
                                    .variant(if spec.destructive {
                                        Variant::Destructive
                                    } else {
                                        Variant::Primary
                                    })
                                    .label(spec.confirm.clone())
                                    .track_focus(&confirm_focus)
                                    .on_click(move |_, window, cx| {
                                        ok(window, cx);
                                        window.close_dialog(cx);
                                    }),
                            ),
                    )
                    .test_support(),
            )
            // Enter runs the action of a plain confirm. In a destructive one
            // it presses the focused button: the action only from the action
            // button, Cancel otherwise (the focus starts there).
            .on_ok(move |_, window, cx| {
                if destructive && !pressed.is_focused(window) {
                    enter_cancel(window, cx);
                    return true;
                }
                enter(window, cx);
                true
            })
            .on_cancel(move |_, window, cx| {
                escape(window, cx);
                true
            })
    });
    // Kit's host took the focus as the dialog opened; a destructive confirm
    // starts on Cancel instead.
    if destructive {
        window.focus(&first, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{AppContext as _, Context, IntoElement, Render, Role, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        deleted: usize,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    fn ask(view: gpui::WeakEntity<Demo>, window: &mut Window, cx: &mut App) {
        confirm(
            Confirm {
                title: "Delete 3 messages?".into(),
                message: "They move to Deleted.".into(),
                confirm: "Delete".into(),
                cancel: "Cancel".into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = view.update(cx, |v, cx| {
                    v.deleted += 1;
                    cx.notify();
                });
            },
            window,
            cx,
        );
    }

    #[gpui_kit::test]
    fn confirm_runs_once_and_closes_cancel_only_closes(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo { deleted: 0 });
        let weak = view.downgrade();
        let again = weak.clone();
        cx.update_window(window.into(), |_, window, cx| {
            ask(weak, window, cx);
            window.draw(cx).clear(cx);
            assert!(window.has_active_dialog(cx));
            assert_eq!(window.find_by_label("Delete").role(), Some(Role::Button));
            // The title names the host only: an AlertDialog node (the
            // vendored Kit and gpui-base patch), with no second dialog or
            // second name inside it.
            assert_eq!(
                window.find_by_label("Delete 3 messages?").role(),
                Some(Role::AlertDialog)
            );
            assert!(
                window.find_all_by_role(Role::Dialog).is_empty(),
                "a destructive confirm is no plain Dialog"
            );
            assert_eq!(window.find_all_by_role(Role::AlertDialog).len(), 1);
            window.click("dialog-confirm", cx);
        })
        .expect("confirm");
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            assert!(!window.has_active_dialog(cx), "closed");
            ask(again, window, cx);
            window.draw(cx).clear(cx);
            window.click("dialog-cancel", cx);
        })
        .expect("cancel");
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
        })
        .expect("closed");
        view.read_with(cx, |v, _| assert_eq!(v.deleted, 1));
    }

    /// Port of `dialogs_keep_the_keyboard_and_start_on_a_safe_button`: Enter
    /// in a destructive confirm never runs its action.
    #[gpui_kit::test]
    fn enter_in_a_destructive_confirm_never_runs_it(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo { deleted: 0 });
        let weak = view.downgrade();
        cx.update_window(window.into(), |_, window, cx| {
            ask(weak, window, cx);
            window.draw(cx).clear(cx);
            window.press("enter", cx);
        })
        .expect("enter");
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(v.deleted, 0, "nothing deleted"));
    }

    /// Row 157, the rest of the Slint app's
    /// `dialogs_keep_the_keyboard_and_start_on_a_safe_button`: a destructive
    /// confirm starts on Cancel (Enter closes it, nothing done); Tab goes
    /// Cancel, the action, back to Cancel and never behind the dialog; Enter
    /// on the action runs it.
    #[gpui_kit::test]
    fn a_destructive_confirm_starts_on_cancel_and_keeps_tab(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo { deleted: 0 });
        let step = |cx: &mut TestAppContext, keys: &[&str]| {
            let weak = view.downgrade();
            cx.update_window(window.into(), |_, window, cx| {
                ask(weak, window, cx);
                window.draw(cx).clear(cx);
                for key in keys {
                    window.press(key, cx);
                    window.draw(cx).clear(cx);
                }
            })
            .expect("keys");
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| window.has_active_dialog(cx))
                .expect("open")
        };
        let deleted = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.deleted);
        // Enter at once presses Cancel.
        assert!(!step(cx, &["enter"]), "Enter pressed Cancel");
        assert_eq!(deleted(cx), 0);
        // Tab twice is Cancel again: the focus stayed in the dialog.
        assert!(!step(cx, &["tab", "tab", "enter"]), "closed by Cancel");
        assert_eq!(deleted(cx), 0, "Tab went round to Cancel");
        // One Tab is the action, and Enter on it runs it.
        assert!(!step(cx, &["tab", "enter"]));
        assert_eq!(deleted(cx), 1, "Enter on the action runs it");
        // Three Tabs are the action again: two stops and nothing else.
        assert!(!step(cx, &["tab", "tab", "tab", "enter"]));
        assert_eq!(deleted(cx), 2, "the Tab cycle is Cancel and the action");
        // Shift+Tab from Cancel wraps to the action as well.
        assert!(!step(cx, &["shift-tab", "enter"]));
        assert_eq!(deleted(cx), 3, "Shift+Tab reached the action");
        // Many Tabs never leave it: still open, and Escape closes it.
        assert!(step(cx, &["tab"; 7]), "still open after Tabs");
        cx.update_window(window.into(), |_, window, cx| {
            window.press("escape", cx);
        })
        .expect("escape");
        cx.run_until_parked();
        assert_eq!(deleted(cx), 3);
    }
}
