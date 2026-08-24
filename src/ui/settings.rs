//! Settings sections: signatures, AI provider, notifications, language.

use super::*;

impl App {
    /// The settings half of the accounts panel: signature editor, AI
    /// provider configuration, notification toggle, and locale picker.
    pub(crate) fn settings_sections(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        accounts: Vec<Account>,
    ) -> AnyElement {
        let ai_model = self.ai_model.clone();
        let ai_base_url = self.ai_base_url.clone();
        let ai_health = self.ai_health.clone();
        let ai_health_busy = self.ai_health_busy;
        let current_provider = self.ai_config.provider.clone();
        let mut providers = v_flex().id("ai-providers").gap_1();
        for preset in ai::providers::presets() {
            let id = preset.id.to_string();
            let label = preset.label.to_string();
            let selected = current_provider == id;
            providers = providers.child(
                Button::new(SharedString::from(format!("provider-{id}")))
                    .label(SharedString::from(label))
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.outline())
                    .on_click({
                        let entity = entity.clone();
                        let model = ai_model.clone();
                        let base_url = ai_base_url.clone();
                        move |_, window, cx| {
                            let mut model_txt = String::new();
                            let mut base_txt = String::new();
                            let pid = id.clone();
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(p) = ai::providers::preset(&pid) {
                                    this.ai_config = AiConfig::from_preset(&p);
                                    model_txt = p.default_model.to_string();
                                    base_txt = p.base_url.to_string();
                                }
                                cx.notify();
                            });
                            model.update(cx, |s, cx| {
                                s.set_value(SharedString::from(model_txt), window, cx);
                            });
                            base_url.update(cx, |s, cx| {
                                s.set_value(SharedString::from(base_txt), window, cx);
                            });
                        }
                    }),
            );
        }

        v_flex()
            .gap_2()
            .child(div().w_full().h(px(1.0)).bg(cx.theme().border).mb_2())
            .child(div().text_lg().font_semibold().child("Signatures"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Appended to new messages you write."),
            )
            .child(self.signature_section(cx, entity.clone(), accounts.clone()))
            .child(div().h_4())
            .child(div().w_full().h(px(1.0)).bg(cx.theme().border).mb_2())
            .child(div().text_lg().font_semibold().child("Mail rules"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Applied to newly synced messages, in order."),
            )
            .child(self.rules_section(cx, entity.clone(), accounts))
            .child(div().h_4())
            .child(div().w_full().h(px(1.0)).bg(cx.theme().border).mb_2())
            .child(div().text_lg().font_semibold().child("AI provider"))
            .child(providers)
            .child(
                Label::new("Model")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Input::new(&ai_model))
            .child(
                Label::new("Base URL")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Input::new(&ai_base_url))
            .child(
                Button::new("save-ai")
                    .label("Save AI configuration")
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.save_ai_config(cx);
                            });
                        }
                    }),
            )
            .child(
                Button::new("check-ai-health")
                    .label(if ai_health_busy {
                        "Checking…"
                    } else {
                        "Check connection"
                    })
                    .icon(IconName::CircleCheck)
                    .outline()
                    .disabled(ai_health_busy)
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.check_ai_health(cx);
                            });
                        }
                    }),
            )
            .children(ai_health.map(|h| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(h))
            }))
            .child(div().h_2())
            .child(div().w_full().h(px(1.0)).bg(cx.theme().border).mb_2())
            .child(div().text_lg().font_semibold().child("Notifications"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Desktop notifications fire locally when new mail is synced."),
            )
            .child(
                Button::new("notify-toggle")
                    .label(if self.notify_config.enabled {
                        "Notifications: On"
                    } else {
                        "Notifications: Off"
                    })
                    .when(self.notify_config.enabled, |b| b.primary())
                    .when(!self.notify_config.enabled, |b| b.outline())
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.notify_config.enabled = !this.notify_config.enabled;
                                let _ = this.db.set_setting(
                                    "notify.enabled",
                                    if this.notify_config.enabled { "1" } else { "0" },
                                );
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(div().h_2())
            .child(div().w_full().h(px(1.0)).bg(cx.theme().border).mb_2())
            .child(div().text_lg().font_semibold().child("Language"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Interface language (restart not required)."),
            )
            .child({
                let mut locales = v_flex().id("locale-picker").gap_1();
                let current = self.locale;
                for loc in [
                    i18n::Locale::EnUs,
                    i18n::Locale::EnGb,
                    i18n::Locale::PtPt,
                    i18n::Locale::PtBr,
                    i18n::Locale::EsEs,
                    i18n::Locale::DeDe,
                    i18n::Locale::FrFr,
                    i18n::Locale::JaJp,
                ] {
                    let selected = current == loc;
                    let label = loc.display_name().to_string();
                    locales = locales.child(
                        Button::new(SharedString::from(format!("locale-{}", loc.as_str())))
                            .label(SharedString::from(label))
                            .when(selected, |b| b.primary())
                            .when(!selected, |b| b.outline())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.locale = loc;
                                        let _ = this.db.set_setting("ui.locale", loc.as_str());
                                        i18n::set_locale(loc);
                                        cx.notify();
                                    });
                                }
                            }),
                    );
                }
                locales
            })
            .into_any_element()
    }

    /// Per-account signature editor: pick an account, edit, save.
    fn signature_section(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        accounts: Vec<Account>,
    ) -> AnyElement {
        if accounts.is_empty() {
            return div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Add an account to set up a signature.")
                .into_any_element();
        }
        let selected = self
            .signature_account_id
            .clone()
            .or_else(|| accounts.first().map(|a| a.id.clone()));
        let mut picker = h_flex().gap_1().flex_wrap();
        for account in &accounts {
            let id = account.id.clone();
            let is_selected = selected.as_deref() == Some(id.as_str());
            picker = picker.child(
                Button::new(SharedString::from(format!("sig-{}", account.id)))
                    .label(SharedString::from(account.email.clone()))
                    .xsmall()
                    .when(is_selected, |b| b.primary())
                    .when(!is_selected, |b| b.outline())
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.load_signature(&id, window, cx);
                            });
                        }
                    }),
            );
        }
        v_flex()
            .gap_2()
            .child(picker)
            .child(Input::new(&self.signature_input))
            .child(
                Button::new("save-signature")
                    .label("Save signature")
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.save_signature(cx));
                        }
                    }),
            )
            .into_any_element()
    }

    fn save_ai_config(&mut self, cx: &mut Context<Self>) {
        let model = self.ai_model.read(cx).value().to_string();
        let base_url = self.ai_base_url.read(cx).value().to_string();
        self.ai_config.model = model;
        self.ai_config.base_url = base_url;
        match ai::save_config(&self.db, &self.ai_config) {
            Ok(()) => {
                self.ai = Ai::load(&self.db).expect("saved config must load");
                self.ai_output = Some("Configuration saved.".to_string());
                self.ai_error = None;
            }
            Err(e) => {
                self.ai_error = Some(e.to_string());
                self.ai_output = None;
            }
        }
        cx.notify();
    }

    /// Probe the AI endpoint reachability and available models off-thread.
    fn check_ai_health(&mut self, cx: &mut Context<Self>) {
        let base_url = self.ai_config.base_url.clone();
        let api_key = ai::resolve_api_key(&self.db, &self.ai_config.provider)
            .ok()
            .flatten();
        let entity = cx.entity();

        self.ai_health_busy = true;
        self.ai_health = Some("Checking…".to_string());
        cx.notify();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let report = ai::client::check_health(&base_url, api_key.as_deref())?;
                    let msg: anyhow::Result<String> = if report.reachable {
                        if report.models.is_empty() {
                            Ok("Reachable - no models listed.".to_string())
                        } else {
                            Ok(format!(
                                "Reachable - {} model(s): {}",
                                report.models.len(),
                                report.models.join(", ")
                            ))
                        }
                    } else {
                        Ok(
                            "Unreachable - check the base URL and that the server is running."
                                .to_string(),
                        )
                    };
                    msg
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.ai_health_busy = false;
                this.ai_health =
                    Some(result.unwrap_or_else(|e| format!("Health check failed: {e}")));
                cx.notify();
            });
        })
        .detach();
    }

    /// The stored signature for ``account_id`` (or the first account).
    pub(crate) fn signature_for_account(&self, account_id: Option<&str>) -> Option<String> {
        let id = account_id
            .map(str::to_string)
            .or_else(|| self.signature_account_id.clone());
        let account = match id {
            Some(id) => self
                .accounts
                .iter()
                .find(|a| a.id == id)
                .or_else(|| self.accounts.first()),
            None => self.accounts.first(),
        };
        account
            .and_then(|a| a.signature_html.clone())
            .filter(|s| !s.trim().is_empty())
    }

    /// Load an account's signature into the editor.
    pub(crate) fn load_signature(
        &mut self,
        account_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.signature_account_id = Some(account_id.to_string());
        let sig = self
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .and_then(|a| a.signature_html.clone())
            .unwrap_or_default();
        self.signature_input
            .update(cx, |s, cx| s.set_value(&sig, window, cx));
        cx.notify();
    }

    /// Persist the signature editor contents to the selected account.
    pub(crate) fn save_signature(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = self.signature_account_id.clone() else {
            self.set_toast("Add an account first.", cx);
            return;
        };
        let value = self.signature_input.read(cx).value().to_string();
        if let Err(e) = self.db.update_account_signature(&account_id, &value) {
            log::error!("failed to save signature: {e}");
            self.set_toast("Could not save signature.", cx);
            return;
        }
        self.refresh_accounts();
        self.set_toast("Signature saved \u{2713}", cx);
    }

    /// Rule list + composer for the active account.
    fn rules_section(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        accounts: Vec<Account>,
    ) -> AnyElement {
        let Some(account) = accounts.first() else {
            return div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Add an account to create rules.")
                .into_any_element();
        };
        let rules = self.db.list_rules(&account.id).unwrap_or_default();
        const ACTIONS: [&str; 3] = ["mark_read", "star", "archive"];
        const ACTION_LABELS: [&str; 3] = ["Mark read", "Star", "Archive"];
        let mut list = v_flex().gap_1();
        if rules.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No rules yet."),
            );
        }
        for rule in &rules {
            let rid = rule.id.clone();
            list = list.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().child(SharedString::from(format!(
                        "When {} contains \"{}\" \u{2192} {}",
                        rule.field,
                        rule.value,
                        match rule.action.as_str() {
                            "star" => "star",
                            "archive" => "archive",
                            _ => "mark read",
                        }
                    ))))
                    .child(div().flex_1())
                    .child(
                        Button::new(SharedString::from(format!("rule-del-{rid}")))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Delete rule")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    let id = rid.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        let _ = this.db.delete_rule(&id);
                                        this.set_toast("Rule deleted", cx);
                                    });
                                }
                            }),
                    ),
            );
        }

        v_flex()
            .gap_2()
            .child(list)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("rule-field-sender")
                            .label("Sender")
                            .xsmall()
                            .when(self.rule_field_sender, |b| b.primary())
                            .when(!self.rule_field_sender, |b| b.outline())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.rule_field_sender = true;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("rule-field-subject")
                            .label("Subject")
                            .xsmall()
                            .when(self.rule_field_sender, |b| b.outline())
                            .when(!self.rule_field_sender, |b| b.primary())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.rule_field_sender = false;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(Input::new(&self.rule_value_input).flex_1())
                    .child(
                        Button::new("rule-action-cycle")
                            .label(SharedString::from(format!(
                                "\u{2192} {}",
                                ACTION_LABELS[self.rule_action_idx % ACTION_LABELS.len()]
                            )))
                            .outline()
                            .xsmall()
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.rule_action_idx = (this.rule_action_idx + 1) % 3;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                Button::new("rule-add")
                    .label("Add rule")
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        let aid = account.id.clone();
                        move |_, window, cx| {
                            let account_id = aid.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                let value =
                                    this.rule_value_input.read(cx).value().trim().to_string();
                                if value.is_empty() {
                                    this.set_toast("Give the rule something to match.", cx);
                                    return;
                                }
                                let field = if this.rule_field_sender {
                                    "sender"
                                } else {
                                    "subject"
                                };
                                let action = ACTIONS[this.rule_action_idx % ACTIONS.len()];
                                let name = format!("When {field} contains \"{value}\"");
                                let _ = this.db.create_rule(
                                    &account_id,
                                    &name,
                                    field,
                                    &value,
                                    action,
                                    None,
                                );
                                this.rule_value_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.set_toast("Rule added ✓", cx);
                            });
                        }
                    }),
            )
            .into_any_element()
    }
}
