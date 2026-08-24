//! Command palette (Ctrl+K): fuzzy-filtered quick actions overlay.

use super::*;

impl App {
    pub(crate) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            let input = self.palette_input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |s, cx| s.focus(window, cx));
            });
        }
        cx.notify();
    }

    /// The ordered command list; ids are stable strings matched in `run_palette`.
    pub(crate) fn palette_items(&self) -> Vec<&'static str> {
        vec![
            "New message",
            "Sync all accounts",
            "Toggle theme",
            "Toggle sidebar",
            "Toggle AI panel",
            "Go to Inbox",
            "Go to Priority",
            "Go to Snoozed",
            "Go to Drafts",
            "Go to Sent",
            "Go to Archive",
            "Go to Trash",
            "Accounts && settings",
            "Undo last action",
        ]
    }

    pub(crate) fn run_palette_item(
        &mut self,
        item: &'static str,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match item {
            "New message" => match window.take() {
                Some(window) => self.new_message(window, cx),
                None => self.set_toast("Press Ctrl+N for a new message.", cx),
            },
            "Sync all accounts" => self.sync_now(cx),
            "Toggle theme" => {}
            "Undo last action" => self.undo_last(cx),
            "Accounts && settings" => {
                self.panel = Panel::Accounts;
            }
            other if other.starts_with("Go to ") => {
                let view = &other["Go to ".len()..];
                let target = match view {
                    "Priority" => Some(PRIORITY.to_string()),
                    "Snoozed" => Some(SNOOZED.to_string()),
                    "Drafts" => self
                        .folders
                        .iter()
                        .find(|f| f.special_use.as_deref() == Some("drafts"))
                        .map(|f| f.id.clone()),
                    "Sent" => self
                        .folders
                        .iter()
                        .find(|f| f.special_use.as_deref() == Some("sent"))
                        .map(|f| f.id.clone()),
                    "Archive" => self
                        .folders
                        .iter()
                        .find(|f| f.special_use.as_deref() == Some("archive"))
                        .map(|f| f.id.clone()),
                    "Trash" => self
                        .folders
                        .iter()
                        .find(|f| f.special_use.as_deref() == Some("trash"))
                        .map(|f| f.id.clone()),
                    _ => self
                        .folders
                        .iter()
                        .find(|f| f.special_use.as_deref() == Some("inbox"))
                        .map(|f| f.id.clone()),
                };
                if let Some(id) = target {
                    self.selected_folder_id = Some(id);
                    self.selected_message = None;
                    self.panel = Panel::Mail;
                    self.reload_messages();
                }
            }
            _ => {}
        }
        self.palette_open = false;
        cx.notify();
    }

    /// The floating palette overlay; rendered above everything when open.
    pub(crate) fn palette_overlay(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let query = self.palette_input.read(cx).value().to_lowercase();
        let items: Vec<&'static str> = self
            .palette_items()
            .into_iter()
            .filter(|label| label.to_lowercase().contains(&query))
            .take(8)
            .collect();

        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .bg(gpui::black().opacity(0.35))
            .on_mouse_down(gpui::MouseButton::Left, {
                let entity = entity.clone();
                move |_, window, cx| {
                    cx.update_entity(&entity, |this, cx| this.toggle_palette(window, cx));
                }
            })
            .child(
                v_flex()
                    .w(px(520.0))
                    .mt_16()
                    .mx_auto()
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .shadow_lg()
                    .p_2()
                    .gap_2()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, _| {})
                    .child(Input::new(&self.palette_input))
                    .children({
                        let entity = entity.clone();
                        items.into_iter().map(move |item| {
                            let entity = entity.clone();
                            Button::new(SharedString::from(item))
                                .label(item.replace("&&", "&"))
                                .ghost()
                                .small()
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.run_palette_item(item, Some(window), cx);
                                        });
                                    }
                                })
                                .into_any_element()
                        })
                    }),
            )
    }
}
