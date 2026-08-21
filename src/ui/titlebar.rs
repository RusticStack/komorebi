//! Custom titlebar rendering.

use super::*;

impl App {
    pub(crate) fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_dark = cx.theme().mode.is_dark();
        let entity = cx.entity();
        let ai_visible = self.ai_panel_visible;
        let sidebar_visible = self.sidebar_visible;

        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .gap_2()
                // Sidebar toggle in the title bar - consistent, discoverable navigation.
                .child(
                    Button::new("title-sidebar-toggle")
                        .icon(if sidebar_visible {
                            IconName::PanelLeftClose
                        } else {
                            IconName::PanelLeftOpen
                        })
                        .ghost()
                        .xsmall()
                        .tooltip(if sidebar_visible {
                            "Hide sidebar (Ctrl+B)"
                        } else {
                            "Show sidebar (Ctrl+B)"
                        })
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| {
                                    this.toggle_sidebar(cx);
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        // Japandi: the brand mark is understated - a soft
                        // ring with the sun glyph, not a bold filled disc.
                        .border_1()
                        .border_color(cx.theme().primary)
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Sun)
                                .size_3()
                                .text_color(cx.theme().primary),
                        ),
                )
                .child(Label::new("Komorebi").font_semibold())
                .child(
                    div()
                        .text_xs()
                        .px_1p5()
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().muted_foreground)
                        .child("AI"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("title-ai")
                        .icon(if ai_visible {
                            IconName::PanelRightClose
                        } else {
                            IconName::PanelRightOpen
                        })
                        .ghost()
                        .tooltip("Toggle AI panel")
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| {
                                    this.toggle_ai_panel(cx);
                                });
                            }
                        }),
                )
                .child(
                    Button::new("theme-toggle")
                        .icon(if is_dark {
                            IconName::Sun
                        } else {
                            IconName::Moon
                        })
                        .ghost()
                        .tooltip("Toggle theme (Ctrl+Shift+T)")
                        .on_click(move |_, window, cx| {
                            let next = if is_dark {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            Theme::change(next, Some(window), cx);
                            apply_komorebi_theme(window, cx);
                        }),
                ),
        )
    }
}
