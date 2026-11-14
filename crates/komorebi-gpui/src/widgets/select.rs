//! The select (dropdown): a pill showing the current choice with a chevron
//! (mockup `.selbox`), opening a brand menu of the choices with a check on
//! the current one. Our own rather than Kit's `Select`, whose trigger is a
//! bordered box. Its accessible name is "<title>: <choice>", as Kit's
//! settings dropdowns do.

use std::rc::Rc;

use gpui::{App, IntoElement, RenderOnce, SharedString, Window};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};

use super::button::{Button, Variant};
use super::menu;
use crate::icons::Icon;

type OnPick = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

/// A select over `(value, label)` choices.
#[derive(IntoElement)]
pub struct Dropdown {
    id: SharedString,
    title: SharedString,
    choices: Vec<(SharedString, SharedString)>,
    value: SharedString,
    on_inset: bool,
    on_pick: Option<OnPick>,
}

impl Dropdown {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            choices: Vec::new(),
            value: value.into(),
            on_inset: false,
            on_pick: None,
        }
    }

    pub fn choices(
        mut self,
        choices: impl IntoIterator<Item = (impl Into<SharedString>, impl Into<SharedString>)>,
    ) -> Self {
        self.choices = choices
            .into_iter()
            .map(|(v, l)| (v.into(), l.into()))
            .collect();
        self
    }

    /// Inside an inset card (the settings card): a paper pill.
    pub fn on_inset(mut self, on_inset: bool) -> Self {
        self.on_inset = on_inset;
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }

    /// The label of the current value (the value itself if unknown).
    pub fn current_label(&self) -> SharedString {
        self.choices
            .iter()
            .find(|(v, _)| *v == self.value)
            .map_or_else(|| self.value.clone(), |(_, l)| l.clone())
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let label = self.current_label();
        let name = format!("{}: {label}", self.title);
        let choices = self.choices;
        let value = self.value;
        let pick = self.on_pick;
        Button::new(self.id)
            .variant(if self.on_inset {
                Variant::OnInset
            } else {
                Variant::Inset
            })
            .label(label)
            .name(name)
            .expandable()
            .trailing(Icon::ChevronDown)
            .dropdown_menu(move |m, _, cx| {
                let mut m = menu::brand(m, cx);
                for (v, l) in &choices {
                    let pick = pick.clone();
                    let chosen = v.clone();
                    m = m.item(PopupMenuItem::new(l.clone()).checked(*v == value).on_click(
                        move |_, window, cx| {
                            if let Some(pick) = &pick {
                                pick(&chosen, window, cx);
                            }
                        },
                    ));
                }
                m
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::AppContext as _;
    use gpui::{Context, MouseButton, ParentElement as _, Render, Role, TestAppContext, div};
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        lang: SharedString,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            div().child(
                Dropdown::new("lang", "Language", self.lang.clone())
                    .choices([("en", "English"), ("pt-PT", "Português (Portugal)")])
                    .on_pick(move |v, _, cx| {
                        let v = v.clone();
                        let _ = view.update(cx, |d, cx| {
                            d.lang = v;
                            cx.notify();
                        });
                    }),
            )
        }
    }

    fn click_label(window: &mut Window, label: &str, cx: &mut App) {
        window.render_frame(cx);
        let center = window.find_by_label(label).bounds().center();
        window.pointer_move(center, None, cx);
        window.pointer_down(center, MouseButton::Left, cx);
        window.pointer_up(center, MouseButton::Left, cx);
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn picking_a_choice_relabels_the_select(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo { lang: "en".into() });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            // Collapsed until its menu opens.
            let select = window.find_by_label("Language: English");
            assert_eq!(select.role(), Some(Role::Button));
            assert_eq!(select.expanded(), Some(false));
            click_label(window, "Language: English", cx);
        })
        .expect("open");
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find_by_label("Language: English").expanded(),
                Some(true),
                "expanded while the menu is open"
            );
            click_label(window, "Português (Portugal)", cx)
        })
        .expect("pick");
        cx.run_until_parked();
        view.read_with(cx, |d, _| assert_eq!(d.lang.as_ref(), "pt-PT"));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let select = window.find_by_label("Language: Português (Portugal)");
            assert_eq!(select.expanded(), Some(false), "collapsed again");
        })
        .expect("relabelled");
    }
}
