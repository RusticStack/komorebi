pub mod attachment_safety;
pub mod auth;
pub mod credentials;
pub mod draft_persistence;
pub mod imap_sync;
pub mod ipc_boundary;
pub mod ipc_validation;
pub mod mail_actions;
pub mod mime_normalization;
pub mod oauth;
pub mod schema;
pub mod smtp_send;
pub mod token_lifecycle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
