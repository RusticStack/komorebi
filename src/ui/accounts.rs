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
        let syncing = self.sync_busy();
        let sync_status = self.sync_note();
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
            .child(self.settings_sections(cx, entity.clone(), accounts))
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
                    h_flex()
                        .gap_1()
                        .child({
                            let pid = acc_id.clone();
                            let ent = entity.clone();
                            Button::new(SharedString::from(format!("pause-{pid}")))
                                .label(if self.account_paused(&acc_id) {
                                    "Paused"
                                } else {
                                    "Pause"
                                })
                                .when(self.account_paused(&acc_id), |b| b.outline())
                                .when(!self.account_paused(&acc_id), |b| b.ghost())
                                .tooltip("Skip this account during sync")
                                .on_click(move |_, _, cx| {
                                    let id = pid.clone();
                                    cx.update_entity(&ent, move |this, cx| {
                                        this.toggle_account_paused(&id, cx);
                                    });
                                })
                        })
                        .child({
                            let rid = acc_id.clone();
                            let ent = entity.clone();
                            Button::new(SharedString::from(format!("resync-{rid}")))
                                .label("Resync")
                                .ghost()
                                .disabled(self.sync_busy())
                                .tooltip("Sync only this account now")
                                .on_click(move |_, _, cx| {
                                    let id = rid.clone();
                                    cx.update_entity(&ent, move |this, cx| {
                                        this.sync_one(&id, cx);
                                    });
                                })
                        })
                        .child({
                            let uid = acc_id.clone();
                            let ent = entity.clone();
                            Button::new(SharedString::from(format!("up-{uid}")))
                                .icon(IconName::ArrowUp)
                                .ghost()
                                .xsmall()
                                .tooltip("Move up")
                                .on_click(move |_, _, cx| {
                                    let id = uid.clone();
                                    cx.update_entity(&ent, move |this, cx| {
                                        this.move_account(&id, -1, cx);
                                    });
                                })
                        })
                        .child({
                            let did = acc_id.clone();
                            let ent = entity.clone();
                            Button::new(SharedString::from(format!("down-{did}")))
                                .icon(IconName::ArrowDown)
                                .ghost()
                                .xsmall()
                                .tooltip("Move down")
                                .on_click(move |_, _, cx| {
                                    let id = did.clone();
                                    cx.update_entity(&ent, move |this, cx| {
                                        this.move_account(&id, 1, cx);
                                    });
                                })
                        })
                        .child({
                            let del = acc_id.clone();
                            let ent = entity.clone();
                            Button::new(SharedString::from(format!("del-{del}")))
                                .label("Remove")
                                .ghost()
                                .danger()
                                .on_click(move |_, _, cx| {
                                    let id = del.clone();
                                    cx.update_entity(&ent, move |this, cx| {
                                        let _ = this.db.delete_account(&id);
                                        this.refresh_accounts();
                                        this.refresh_folders();
                                        this.selected_message = None;
                                        this.select_first_folder();
                                        cx.notify();
                                    });
                                })
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
    /// Sync a single account now (per-account Resync button).
    pub(crate) fn sync_one(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.sync_busy() {
            return;
        }
        let Some(target) = self
            .accounts
            .iter()
            .find(|a| a.id == account_id && a.provider_kind != "local")
            .cloned()
        else {
            return;
        };
        self.sync_phase = SyncPhase::Syncing;
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut summary = mail::SyncSummary::default();
                    match mail::sync_account(&db, &target) {
                        Ok(s) => {
                            summary.fetched = s.fetched;
                            summary.folders = s.folders;
                            summary.errors.extend(s.errors);
                        }
                        Err(e) => summary.errors.push(format!("{e}")),
                    }
                    if !summary.new_ids.is_empty() {
                        match db.apply_rules(&target.id, &summary.new_ids) {
                            Ok(n) => summary.rules_applied = n,
                            Err(e) => log::error!("rules failed: {e}"),
                        }
                    }
                    let r = mail::replay_outbox(&db, &target);
                    summary.replayed = r.applied;
                    summary.conflicts = r.conflicts.len();
                    summary
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.sync_phase = if result.errors.is_empty() {
                    SyncPhase::Done(format!(
                        "Synced {} message(s) from {}.",
                        result.fetched,
                        if result.folders == 1 {
                            "1 folder"
                        } else {
                            "folders"
                        }
                    ))
                } else {
                    SyncPhase::Failed(format!("{} error(s) during resync.", result.errors.len()))
                };
                this.refresh_folders();
                this.reload_messages();
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.sync_busy() {
            return;
        }
        let targets: Vec<Account> = self
            .accounts
            .iter()
            .filter(|a| a.provider_kind != "local" && !self.account_paused(&a.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            self.set_toast(
                "Add a real account first (the demo account cannot sync).",
                cx,
            );
            return;
        }
        self.sync_phase = SyncPhase::Syncing;
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
                                if !s.new_ids.is_empty() {
                                    match db.apply_rules(&account.id, &s.new_ids) {
                                        Ok(n) => summary.rules_applied += n,
                                        Err(e) => log::error!("rules failed: {e}"),
                                    }
                                }
                            }
                            Err(e) => summary.errors.push(format!("{}: {e}", account.email)),
                        }
                    }
                    // Replay queued offline mutations against each server.
                    let mut replayed = 0usize;
                    let mut conflicts = 0usize;
                    for account in &targets {
                        let r = mail::replay_outbox(&db, account);
                        replayed += r.applied;
                        conflicts += r.conflicts.len();
                    }
                    let _ = db.purge_applied_mutations(chrono::Utc::now().timestamp() - 30 * 86400);
                    summary.replayed = replayed;
                    summary.conflicts = conflicts;
                    summary
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                let rules_note = if result.rules_applied > 0 {
                    format!(" {} rule hit(s).", result.rules_applied)
                } else {
                    String::new()
                };
                let replay_note = if result.replayed > 0 || result.conflicts > 0 {
                    format!(
                        " {} change(s) pushed, {} deferred.",
                        result.replayed, result.conflicts
                    )
                } else {
                    String::new()
                };
                let status = if result.errors.is_empty() {
                    format!(
                        "Synced {} message(s) across {} folder(s).{replay_note}{rules_note}",
                        result.fetched, result.folders
                    )
                } else {
                    format!(
                        "Synced {} message(s); {} error(s).{replay_note}",
                        result.fetched,
                        result.errors.len()
                    )
                };
                this.sync_phase = if result.errors.is_empty() && result.conflicts == 0 {
                    SyncPhase::Done(status.clone())
                } else {
                    SyncPhase::Failed(status)
                };
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
}
