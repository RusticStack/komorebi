//! Accounts/settings panel: setup wizard, OAuth, discovery, sync, AI config.

use super::*;

impl App {
    pub(crate) fn accounts_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let status = self.setup_status.clone();
        let accounts = self.accounts.clone();
        let setup_email = self.setup_email.clone();
        let setup_name = self.setup_name.clone();
        let setup_password = self.setup_password.clone();
        let setup_imap_host = self.setup_imap_host.clone();
        let setup_imap_port = self.setup_imap_port.clone();
        let setup_smtp_host = self.setup_smtp_host.clone();
        let setup_smtp_port = self.setup_smtp_port.clone();
        let setup_client_id = self.setup_client_id.clone();
        let setup_client_secret = self.setup_client_secret.clone();
        let setup_redirect_uri = self.setup_redirect_uri.clone();
        let discovered = self.setup_discovered.clone();
        let busy = self.setup_busy;
        let syncing = self.syncing;
        let sync_status = self.sync_status.clone();
        let ai_model = self.ai_model.clone();
        let ai_base_url = self.ai_base_url.clone();
        let ai_health = self.ai_health.clone();
        let ai_health_busy = self.ai_health_busy;
        let current_provider = self.ai_config.provider.clone();
        let oauth = discovered
            .as_ref()
            .map(|d| d.auth_mode == "oauth")
            .unwrap_or(false);

        let add_or_authorize: AnyElement = if oauth {
            v_flex()
                .gap_2()
                .child(Label::new("OAuth client id").text_xs())
                .child(Input::new(&setup_client_id))
                .child(Label::new("OAuth client secret").text_xs())
                .child(Input::new(&setup_client_secret))
                .child(Label::new("Redirect URI").text_xs())
                .child(Input::new(&setup_redirect_uri))
                .child(
                    Button::new("authorize-oauth")
                        .label(if busy {
                            "Authorizing…"
                        } else {
                            "Authorize & add account"
                        })
                        .icon(IconName::Globe)
                        .primary()
                        .disabled(busy)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| this.authorize_oauth(cx));
                            }
                        }),
                )
                .into_any_element()
        } else {
            Button::new("add-account")
                .label("Add account")
                .icon(IconName::Plus)
                .primary()
                .on_click({
                    let entity = entity.clone();
                    let email = setup_email.clone();
                    let name = setup_name.clone();
                    move |_, window, cx| {
                        let mut ok = false;
                        cx.update_entity(&entity, |this, cx| {
                            ok = this.add_account(cx);
                        });
                        if ok {
                            email.update(cx, |s, cx| s.set_value("", window, cx));
                            name.update(cx, |s, cx| s.set_value("", window, cx));
                        }
                    }
                })
                .into_any_element()
        };

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

        let onboarding = accounts.is_empty();
        v_flex()
            .id("accounts")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p_4()
            .gap_2()
            // Back button for consistent navigation back to
            // the mail view from settings.
            .when(!onboarding, |this| {
                this.child(
                    h_flex()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Button::new("accounts-back")
                                        .icon(IconName::ChevronLeft)
                                        .ghost()
                                        .tooltip("Back to mail")
                                        .on_click({
                                            let entity = entity.clone();
                                            move |_, _, cx| {
                                                cx.update_entity(&entity, |this, cx| {
                                                    this.panel = Panel::Mail;
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                )
                                .child(div().text_xl().font_semibold().child("Accounts")),
                        ),
                )
            })
            .when(onboarding, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .mt_4()
                        .child(div().text_2xl().font_semibold().child("Welcome to Komorebi"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "A privacy-first email client. Your mail stays in a local \
                                     database on this machine, passwords stay in Windows Credential \
                                     Manager, and nothing is ever sent to a server you don't own. \
                                     Add your first account to get started.",
                                ),
                        )
                        .child(
                            h_flex().gap_2().pt_2().child(
                                Button::new("seed-demo")
                                    .label("Or explore with demo data")
                                    .outline()
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                this.seed_demo(cx);
                                            });
                                        }
                                    }),
                            ),
                        ),
                )
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Self-hosted first: Mailcow, Zimbra, Poste.io, or any IMAP/SMTP server. Gmail and Microsoft 365 are supported through OAuth.",
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
            .child(div().text_lg().font_semibold().child("Add account"))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Email address")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(Input::new(&setup_email)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Display name (optional)")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(Input::new(&setup_name)),
            )
            .child(
                Button::new("discover")
                    .label(if busy { "Discovering…" } else { "Discover settings" })
                    .icon(IconName::Search)
                    .outline()
                    .disabled(busy)
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.discover_settings(cx));
                        }
                    }),
            )
            .children(discovered.as_ref().map(|d| self.discovered_summary(cx, d)))
            .child(
                Label::new("Password")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Input::new(&setup_password))
            .child(
                Label::new("Manual server settings (used when discovery finds nothing)")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                Label::new("IMAP")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&setup_imap_host).flex_1())
                    .child(Input::new(&setup_imap_port).w(px(96.0))),
            )
            .child(self.security_buttons(entity.clone(), true))
            .child(
                Label::new("SMTP")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&setup_smtp_host).flex_1())
                    .child(Input::new(&setup_smtp_port).w(px(96.0))),
            )
            .child(self.security_buttons(entity.clone(), false))
            .child(add_or_authorize)
            .children(status.map(|s| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(s))
            }))
            .child(div().h_2())
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
            .child(div().text_lg().font_semibold().child("Sync"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("sync")
                            .label(if syncing { "Syncing…" } else { "Sync all accounts" })
                            .icon(IconName::Globe)
                            .primary()
                            .disabled(syncing)
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| this.sync_now(cx));
                                }
                            }),
                    )
                    .children(sync_status.map(|s| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(s))
                    })),
            )
            .child(self.account_rows(cx, entity.clone(), accounts.clone()))
            .child(div().h_4())
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
            .child(div().text_lg().font_semibold().child("Signatures"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Appended to new messages you write."),
            )
            .child(self.signature_section(cx, entity.clone(), accounts))
            .child(div().h_4())
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
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
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
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
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(cx.theme().border)
                    .mb_2(),
            )
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
                for loc in [i18n::Locale::EnUs, i18n::Locale::EnGb, i18n::Locale::PtPt, i18n::Locale::PtBr, i18n::Locale::EsEs, i18n::Locale::DeDe, i18n::Locale::FrFr, i18n::Locale::JaJp] {
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

    fn account_rows(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        accounts: Vec<Account>,
    ) -> impl IntoElement + use<> {
        let mut list = v_flex().gap_1();
        for account in accounts {
            let acc_id = account.id.clone();
            let email = account.email.clone();
            let provider = account.provider_kind.clone();
            let name = account
                .display_name
                .clone()
                .unwrap_or_else(|| email.clone());
            let row = h_flex()
                .w_full()
                .px_2()
                .py_2()
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(cx.theme().border)
                .justify_between()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Avatar::new().name(SharedString::from(name)).small())
                        .child(
                            v_flex().gap_0p5().child(SharedString::from(email)).child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(provider)),
                            ),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("del-{acc_id}")))
                        .label("Remove")
                        .ghost()
                        .danger()
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                let id = acc_id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_account(&id);
                                    this.refresh_accounts();
                                    this.refresh_folders();
                                    this.selected_message = None;
                                    this.select_first_folder();
                                    cx.notify();
                                });
                            }
                        }),
                );
            list = list.child(row);
        }
        let _ = cx;
        list
    }

    fn add_account(&mut self, cx: &mut Context<Self>) -> bool {
        let email = self.setup_email.read(cx).value().to_string();
        let name = self.setup_name.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return false;
        }
        let email = email.trim().to_string();
        let display_name = if name.trim().is_empty() {
            None
        } else {
            Some(name.trim().to_string())
        };

        // Prefer discovered settings; otherwise read the manual fields.
        let discovered = self.setup_discovered.clone();
        let (kind, auth_mode, imap, smtp) = match &discovered {
            Some(d) => (
                ProviderKind::parse_or_default(&d.provider_kind),
                d.auth_mode.clone(),
                d.imap.clone(),
                d.smtp.clone(),
            ),
            None => (
                ProviderKind::Custom,
                "password".to_string(),
                self.manual_imap(cx),
                self.manual_smtp(cx),
            ),
        };

        if auth_mode == "oauth" {
            self.setup_status = Some(
                "This account needs OAuth - fill in the client credentials and click Authorize."
                    .to_string(),
            );
            cx.notify();
            return false;
        }

        let password = self.setup_password.read(cx).value().to_string();
        if password.trim().is_empty() {
            self.setup_status = Some("Enter the mailbox password.".to_string());
            cx.notify();
            return false;
        }

        match mail::create_password_account(
            &self.db,
            &email,
            display_name.as_deref(),
            kind,
            imap.as_ref(),
            smtp.as_ref(),
            None,
            password.trim(),
        ) {
            Ok(acc) => {
                self.setup_status = Some(format!(
                    "Added {} - use Sync in the sidebar to fetch mail.",
                    acc.email
                ));
                self.setup_discovered = None;
                self.refresh_accounts();
                self.refresh_folders();
                cx.notify();
                true
            }
            Err(e) => {
                self.setup_status = Some(format!("Could not add account: {e}"));
                cx.notify();
                false
            }
        }
    }

    /// Read manual IMAP settings from the setup inputs (None if host is blank).
    fn manual_imap(&self, cx: &mut Context<Self>) -> Option<ServerSettings> {
        let host = self.setup_imap_host.read(cx).value().to_string();
        if host.trim().is_empty() {
            return None;
        }
        let port = self.setup_imap_port.read(cx).value().to_string();
        Some(ServerSettings {
            host: host.trim().to_string(),
            port: port.trim().parse().unwrap_or(993),
            security: self.setup_imap_security.security(),
        })
    }

    /// Read manual SMTP settings from the setup inputs (None if host is blank).
    fn manual_smtp(&self, cx: &mut Context<Self>) -> Option<ServerSettings> {
        let host = self.setup_smtp_host.read(cx).value().to_string();
        if host.trim().is_empty() {
            return None;
        }
        let port = self.setup_smtp_port.read(cx).value().to_string();
        Some(ServerSettings {
            host: host.trim().to_string(),
            port: port.trim().parse().unwrap_or(587),
            security: self.setup_smtp_security.security(),
        })
    }

    /// Run standards-based autodiscovery for the entered email address,
    /// off the UI thread, and store the result for the add/authorize buttons.
    fn discover_settings(&mut self, cx: &mut Context<Self>) {
        let email = self.setup_email.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return;
        }
        let email = email.trim().to_string();
        self.setup_busy = true;
        self.setup_discovered = None;
        self.setup_status = Some("Discovering server settings…".to_string());
        cx.notify();

        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { mail::discover_account(&email) })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.setup_busy = false;
                match result {
                    Ok(d) => {
                        this.setup_status = Some(format!(
                            "Discovered via {} (confidence {}%) - review and add.",
                            d.source, d.confidence
                        ));
                        this.setup_discovered = Some(d);
                    }
                    Err(e) => {
                        this.setup_status = Some(format!(
                            "Discovery failed: {e} - enter server settings manually."
                        ));
                        this.setup_discovered = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run the OAuth PKCE browser flow for a discovered Gmail/Microsoft account.
    fn authorize_oauth(&mut self, cx: &mut Context<Self>) {
        let email = self.setup_email.read(cx).value().to_string();
        let name = self.setup_name.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return;
        }
        let Some(discovered) = self.setup_discovered.clone() else {
            self.setup_status =
                Some("Run discovery first to detect the OAuth provider.".to_string());
            cx.notify();
            return;
        };
        if discovered.auth_mode != "oauth" {
            self.setup_status = Some("This account uses password auth, not OAuth.".to_string());
            cx.notify();
            return;
        }

        let client_id = self.setup_client_id.read(cx).value().to_string();
        let client_secret = self.setup_client_secret.read(cx).value().to_string();
        let redirect = self.setup_redirect_uri.read(cx).value().to_string();
        if client_id.trim().is_empty() {
            self.setup_status = Some("Enter your OAuth client id.".to_string());
            cx.notify();
            return;
        }
        let redirect = if redirect.trim().is_empty() {
            "http://127.0.0.1:8765/callback".to_string()
        } else {
            redirect.trim().to_string()
        };
        let email = email.trim().to_string();
        let display_name = if name.trim().is_empty() {
            None
        } else {
            Some(name.trim().to_string())
        };
        let kind = ProviderKind::parse_or_default(&discovered.provider_kind);
        let imap = discovered.imap.clone();
        let smtp = discovered.smtp.clone();

        self.setup_busy = true;
        self.setup_status = Some("Opening your browser for authorization…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        let client_id = client_id.trim().to_string();
        let client_secret = client_secret.trim().to_string();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    mail::create_oauth_account(
                        &db,
                        &email,
                        display_name.as_deref(),
                        kind,
                        &client_id,
                        &client_secret,
                        &redirect,
                        imap.as_ref(),
                        smtp.as_ref(),
                        None,
                    )
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.setup_busy = false;
                match result {
                    Ok(acc) => {
                        this.setup_status = Some(format!(
                            "Authorized {} - use Sync to fetch mail.",
                            acc.email
                        ));
                        this.setup_discovered = None;
                        this.refresh_accounts();
                        this.refresh_folders();
                    }
                    Err(e) => this.setup_status = Some(format!("Authorization failed: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sync every non-demo account off the UI thread, with visible status.
    pub(crate) fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let targets: Vec<Account> = self
            .accounts
            .iter()
            .filter(|a| a.provider_kind != "local")
            .cloned()
            .collect();
        if targets.is_empty() {
            self.set_toast(
                "Add a real account first (the demo account cannot sync).",
                cx,
            );
            return;
        }
        self.syncing = true;
        self.sync_status = Some("Syncing…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut summary = mail::SyncSummary::default();
                    for account in &targets {
                        match mail::sync_account(&db, account) {
                            Ok(s) => {
                                summary.fetched += s.fetched;
                                summary.folders += s.folders;
                                summary.errors.extend(s.errors);
                            }
                            Err(e) => summary.errors.push(format!("{}: {e}", account.email)),
                        }
                    }
                    summary
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.syncing = false;
                this.sync_status = Some(if result.errors.is_empty() {
                    format!(
                        "Synced {} message(s) across {} folder(s).",
                        result.fetched, result.folders
                    )
                } else {
                    format!(
                        "Synced {} message(s); {} error(s).",
                        result.fetched,
                        result.errors.len()
                    )
                });
                this.refresh_folders();
                this.reload_messages();
                // Fire a desktop notification when new mail arrives (if enabled).
                if result.fetched > 0 && this.notify_config.enabled {
                    let _ = notify::notify_message(
                        &this.notify_config,
                        "",
                        "",
                        "Komorebi",
                        &format!("{} new message(s) synced.", result.fetched),
                        notify::Priority::Normal,
                    );
                }
                cx.notify();
            });
        })
        .detach();
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
}
