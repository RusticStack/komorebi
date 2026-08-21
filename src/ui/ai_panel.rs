//! AI sidebar: insights, action cards, tone selection, AI job runners.

use super::send::perform_ai;
use super::*;

impl App {
    // ---------------------------------------------------------------------
    // AI panel
    // ---------------------------------------------------------------------
    pub(crate) fn ai_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let (name, email) = self.user_profile();
        let first_name = name
            .split_whitespace()
            .next()
            .unwrap_or("there")
            .to_string();
        let busy = self.ai_busy;
        let output = self.ai_output.clone();
        let error = self.ai_error.clone();
        let tone = self.ai_tone.clone();

        let topic = self
            .selected_message
            .as_ref()
            .and_then(|m| m.ai_category.clone())
            .unwrap_or_else(|| "General".to_string());
        let last_response = self
            .selected_message
            .as_ref()
            .and_then(|m| m.date_received)
            .map(format_relative_long)
            .unwrap_or_else(|| " - ".to_string());
        let sender_name = self
            .selected_message
            .as_ref()
            .and_then(|m| m.sender_name.clone())
            .unwrap_or_else(|| "Sender".to_string());

        // Compute real thread stats from the loaded thread messages.
        let thread_count = self.thread_messages.len();
        let thread_participants: Vec<String> = self
            .thread_messages
            .iter()
            .filter_map(|m| m.sender_name.clone().or_else(|| m.sender_email.clone()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let participant_count = thread_participants.len().max(1);
        let extra_participants = participant_count.saturating_sub(1);
        let topic_badge = label_badge(cx, &topic);

        v_flex()
            .id("ai-panel")
            .h_full()
            .w_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .overflow_y_scroll()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Icon::new(IconName::Bot)
                                    .size_4()
                                    .text_color(cx.theme().primary),
                            )
                            .child(Label::new("Komorebi AI").font_semibold())
                            .child(
                                div()
                                    .text_xs()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .child("BETA"),
                            ),
                    )
                    .child(
                        Button::new("close-ai")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Collapse AI panel")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.ai_panel_visible = false;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .items_center()
                    .child(Avatar::new().name(SharedString::from(name.clone())).small())
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(SharedString::from(name)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(email)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size_3p5()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                v_flex()
                    .px_3()
                    .pb_2()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_medium()
                            .child(SharedString::from(format!("How can I help, {first_name}?"))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Choose an action for this email."),
                    ),
            )
            .child(
                v_flex()
                    .px_2()
                    .gap_1()
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::BookOpen,
                        "Summarize thread",
                        "Get a concise summary of this conversation",
                        AiKind::Summarize,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::ArrowLeft,
                        "Draft a reply",
                        "Create a smart reply in your tone",
                        AiKind::DraftReply,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::ALargeSmall,
                        "Improve writing",
                        "Enhance clarity, tone, and grammar",
                        AiKind::Polish,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::CircleCheck,
                        "Detect action items",
                        "Identify tasks and next steps",
                        AiKind::Triage,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::Folder,
                        "Categorize email",
                        "Suggest the best label",
                        AiKind::Triage,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::Calendar,
                        "Schedule follow-up",
                        "Set a reminder to follow up",
                        AiKind::FollowUp,
                    )),
            )
            .child(self.section_header(cx, "AI Insights"))
            .child(
                v_flex()
                    .px_2()
                    .pb_2()
                    .gap_2()
                    .child(insight_row(cx, "This thread is about:", Some(topic_badge)))
                    .child(
                        h_flex()
                            .w_full()
                            .px_2()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Key people"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .child(
                                        Avatar::new()
                                            .name(SharedString::from(sender_name.clone()))
                                            .small(),
                                    )
                                    .when(extra_participants > 0, |d| {
                                        d.child(
                                            div()
                                                .text_xs()
                                                .px_1p5()
                                                .rounded_full()
                                                .bg(cx.theme().muted)
                                                .text_color(cx.theme().muted_foreground)
                                                .child(SharedString::from(format!(
                                                    "+{extra_participants}"
                                                ))),
                                        )
                                    }),
                            ),
                    )
                    .child(insight_row(
                        cx,
                        "Last response time",
                        Some(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(last_response))
                                .into_any_element(),
                        ),
                    ))
                    .child(insight_row(
                        cx,
                        "Messages in thread",
                        Some(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(thread_count.to_string()))
                                .into_any_element(),
                        ),
                    )),
            )
            .children(output.clone().map(|o| {
                v_flex()
                    .id("ai-output")
                    .mx_2()
                    .mb_2()
                    .p_3()
                    .rounded(RADIUS_LG)
                    .bg(cx.theme().muted)
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new("Result").text_xs().font_semibold())
                            .child(if busy {
                                Icon::new(IconName::LoaderCircle)
                                    .size_3p5()
                                    .text_color(cx.theme().muted_foreground)
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
                    .child(SharedString::from(o))
            }))
            .when(busy && output.is_none(), |d| {
                d.child(
                    h_flex()
                        .id("ai-loading")
                        .mx_2()
                        .mb_2()
                        .p_3()
                        .rounded(RADIUS_LG)
                        .bg(cx.theme().muted)
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(IconName::LoaderCircle)
                                .size_4()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Working on it…"),
                        ),
                )
            })
            .children(error.map(|e| {
                div()
                    .id("ai-error")
                    .mx_2()
                    .mb_2()
                    .p_2()
                    .rounded(RADIUS_SM)
                    .text_color(cx.theme().danger)
                    .child(SharedString::from(e))
            }))
            .child(div().flex_1())
            .child(self.tone_footer(cx, entity, &tone))
    }

    fn ai_action_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        icon: IconName,
        title: &'static str,
        subtitle: &'static str,
        kind: AiKind,
    ) -> AnyElement {
        h_flex()
            .id(SharedString::from(format!("ai-card-{title}")))
            .w_full()
            .px_2()
            .py_2()
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .gap_2()
            .items_center()
            .hover(|s| s.bg(cx.theme().muted))
            .child(Icon::new(icon).size_4().text_color(cx.theme().primary))
            .child(
                v_flex()
                    .flex_1()
                    .gap_0p5()
                    .child(div().text_sm().font_medium().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(subtitle),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size_3p5()
                    .text_color(cx.theme().muted_foreground),
            )
            .on_click(move |_, window, cx| {
                cx.update_entity(&entity, move |this, cx| match kind {
                    AiKind::Summarize => this.run_ai(cx, AiKind::Summarize),
                    AiKind::DraftReply => this.run_ai_reply(window, cx),
                    AiKind::Polish => this.run_ai(cx, AiKind::Polish),
                    AiKind::FollowUp => this.schedule_followup(cx),
                    AiKind::Triage => {
                        let _ = title;
                        if title == "Detect action items" {
                            this.run_detect_actions(cx);
                        } else {
                            this.run_ai(cx, AiKind::Triage);
                        }
                    }
                });
            })
            .into_any_element()
    }

    fn tone_footer(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        tone: &str,
    ) -> impl IntoElement + use<> {
        h_flex()
            .id("tone-footer")
            .w_full()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Palette)
                            .size_4()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("AI tone"),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("tone-select")
                            .label(SharedString::from(tone.to_string()))
                            .icon(IconName::ChevronDown)
                            .ghost()
                            .small()
                            .dropdown_menu({
                                let entity = entity.clone();
                                move |menu, _, _| {
                                    let tones = [
                                        "Professional",
                                        "Warm",
                                        "Concise",
                                        "Direct",
                                        "Friendly",
                                        "Formal",
                                    ];
                                    let mut m = menu;
                                    for t in tones {
                                        m = m.item(
                                            PopupMenuItem::new(SharedString::from(t)).on_click({
                                                let entity = entity.clone();
                                                move |_, _, cx| {
                                                    let t = t.to_string();
                                                    cx.update_entity(&entity, move |this, cx| {
                                                        let _ = this.db.set_setting("ai.tone", &t);
                                                        this.ai_tone = t;
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                        );
                                    }
                                    m
                                }
                            }),
                    )
                    .child(
                        Button::new("ai-settings")
                            .icon(IconName::Settings)
                            .ghost()
                            .xsmall()
                            .tooltip("AI settings")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.panel = Panel::Accounts;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
    }

    // ---------------------------------------------------------------------
    // AI execution
    // ---------------------------------------------------------------------
    fn run_ai(&mut self, cx: &mut Context<Self>, kind: AiKind) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        let sender = msg.sender_name.clone();
        let subject = msg.subject.clone();
        // Summarize covers the whole conversation; other actions use the
        // selected message body only.
        let body = if matches!(kind, AiKind::Summarize) {
            format_thread_text(&self.thread_messages)
        } else {
            msg.body_text.clone().unwrap_or_default()
        };
        let message_id = msg.id.clone();
        let account_id = msg.account_id.clone();
        let ai = self.ai.clone();
        let db = self.db.clone();
        let tone = self.ai_tone.clone();
        let entity = cx.entity();

        self.ai_busy = true;
        self.ai_output = None;
        self.ai_error = None;
        cx.notify();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    perform_ai(
                        &ai,
                        &db,
                        kind,
                        &account_id,
                        &message_id,
                        sender.as_deref(),
                        subject.as_deref(),
                        &body,
                        &tone,
                    )
                })
                .await;
            let (output, error) = match result {
                Ok(text) => (Some(text), None),
                Err(e) => (None, Some(e.to_string())),
            };
            let _ = entity.update(cx, move |this, cx| {
                this.ai_busy = false;
                this.ai_output = output;
                this.ai_error = error;
                cx.notify();
            });
        })
        .detach();
    }

    fn run_detect_actions(&mut self, cx: &mut Context<Self>) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        let sender = msg.sender_name.clone();
        let subject = msg.subject.clone();
        let body = msg.body_text.clone().unwrap_or_default();
        let message_id = msg.id.clone();
        let account_id = msg.account_id.clone();
        let ai = self.ai.clone();
        let db = self.db.clone();
        let entity = cx.entity();

        self.ai_busy = true;
        self.ai_output = None;
        self.ai_error = None;
        cx.notify();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let items = ai::actions::extract_action_items(
                        &ai,
                        sender.as_deref(),
                        subject.as_deref(),
                        &body,
                    )?;
                    let summary: String = if items.is_empty() {
                        "No action items detected in this message.".to_string()
                    } else {
                        let s = items
                            .iter()
                            .map(|i| {
                                let owner = i.owner.as_deref().unwrap_or("(unassigned)");
                                let due = i.due.as_deref().unwrap_or("(no date)");
                                format!("• {} - {} {}", i.task, owner, due)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !items.is_empty() {
                            ai::record_and_apply(
                                &db,
                                Some(&account_id),
                                Some(&message_id),
                                "extract_action_items",
                                ai.config(),
                                &s,
                            )?;
                        }
                        s
                    };
                    Ok::<String, anyhow::Error>(summary)
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.ai_busy = false;
                match result {
                    Ok(text) => {
                        this.ai_output = Some(text);
                        this.ai_error = None;
                    }
                    Err(e) => {
                        this.ai_error = Some(e.to_string());
                        this.ai_output = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
