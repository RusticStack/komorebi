const APP_COMMANDS: &[&str] = &[
    "load_mail_shell",
    "search_mail",
    "autosave_composer_draft",
    "discard_composer_draft",
    "stage_composer_attachment",
    "send_composer_email",
    "list_recent_recipients",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to run Tauri build script");
}
