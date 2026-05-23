pub mod account_connection;
pub mod account_management;
pub mod account_signatures;
pub mod attachment_safety;
pub mod auth;
pub mod composer;
pub mod credentials;
pub mod draft_persistence;
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
pub mod token_lifecycle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            mail_shell::load_mail_shell,
            mail_shell::search_mail,
            composer::autosave_composer_draft,
            composer::discard_composer_draft,
            composer::stage_composer_attachment,
            composer::send_composer_email,
            composer::list_recent_recipients,
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
