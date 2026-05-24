pub mod account_connection;
pub mod account_management;
pub mod account_signatures;
pub mod app_preferences;
pub mod attachment_commands;
pub mod attachment_safety;
pub mod auth;
pub mod composer;
pub mod credentials;
pub mod draft_persistence;
pub mod external_links;
pub mod imap_live;
pub mod imap_sync;
pub mod ipc_boundary;
pub mod ipc_validation;
pub mod mail_actions;
pub mod mail_shell;
pub mod mime_normalization;
pub mod oauth;
pub mod oauth_loopback;
pub mod schema;
pub mod smtp_send;
pub mod sync_scheduler;
pub mod token_lifecycle;

use std::sync::Arc;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(sync_scheduler::BackgroundSyncScheduler::default())
        .setup(|app| {
            let cache_path = account_management::mail_cache_path(app.handle())?;
            let scheduler = app
                .state::<sync_scheduler::BackgroundSyncScheduler>()
                .inner()
                .clone();
            scheduler.set_status_emitter(Arc::new(
                sync_scheduler::TauriSyncStatusEventEmitter::new(app.handle().clone()),
            ));
            let _ = scheduler.start_all_configured_accounts(cache_path);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let scheduler = window
                    .state::<sync_scheduler::BackgroundSyncScheduler>()
                    .inner()
                    .clone();
                scheduler.stop_all();
            }
        })
        .invoke_handler(tauri::generate_handler![
            mail_shell::load_mail_shell,
            mail_shell::load_message_page,
            mail_shell::search_mail,
            mail_shell::allow_remote_content,
            app_preferences::get_ui_preferences,
            app_preferences::update_ui_preferences,
            mail_actions::perform_mail_action,
            external_links::open_external_link,
            attachment_commands::preview_attachment,
            attachment_commands::save_attachment,
            attachment_commands::download_attachment,
            composer::autosave_composer_draft,
            composer::discard_composer_draft,
            composer::stage_composer_attachment,
            composer::stage_forwarded_attachments,
            composer::send_composer_email,
            composer::list_recent_recipients,
            composer::list_composer_drafts,
            account_management::create_account,
            account_management::reauth_account,
            account_management::remove_account,
            account_management::update_account_settings,
            account_management::test_account_connection,
            account_management::get_account_auth_status,
            oauth_loopback::start_oauth_account_setup,
            oauth_loopback::start_oauth_account_reauth
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
