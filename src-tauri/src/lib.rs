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
            composer::list_recent_recipients
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
