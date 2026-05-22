pub mod attachment_safety;
pub mod auth;
pub mod ipc_boundary;
pub mod ipc_validation;
pub mod token_lifecycle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
