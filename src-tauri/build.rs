const APP_COMMANDS: &[&str] = &[
    "load_mail_shell",
    "search_mail",
    "allow_remote_content",
    "open_external_link",
    "preview_attachment",
    "save_attachment",
    "download_attachment",
    "autosave_composer_draft",
    "discard_composer_draft",
    "stage_composer_attachment",
    "send_composer_email",
    "list_recent_recipients",
    "list_composer_drafts",
    "create_account",
    "reauth_account",
    "remove_account",
    "update_account_settings",
    "test_account_connection",
    "get_account_auth_status",
    "start_oauth_account_setup",
    "start_oauth_account_reauth",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to run Tauri build script");
}
