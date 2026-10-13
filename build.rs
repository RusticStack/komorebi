fn main() {
    // English is the source language; `translations/<tag>/LC_MESSAGES/
    // komorebi.po` adds one (the folder name is the tag src/i18n.rs selects).
    // No implicit context (the component name): a string has one only when its
    // `@tr("context" => ...)` names it, so the catalog does not depend on how
    // the UI is split into components.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .with_bundled_translations("translations");
    println!("cargo:rerun-if-changed=translations");
    slint_build::compile_with_config("ui/app.slint", config).expect("compiling Slint UI");

    // The icon, version information and manifest of komorebi.exe (linked
    // into the app binary only, not tests or examples).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for file in ["komorebi.rc", "komorebi.ico", "komorebi.manifest"] {
            println!("cargo:rerun-if-changed=assets/{file}");
        }
        let part = |name: &str| std::env::var(name).expect("Cargo sets the package version");
        let macros = [
            format!("VER_MAJOR={}", part("CARGO_PKG_VERSION_MAJOR")),
            format!("VER_MINOR={}", part("CARGO_PKG_VERSION_MINOR")),
            format!("VER_PATCH={}", part("CARGO_PKG_VERSION_PATCH")),
            format!("VER_STRING=\"{}\"", part("CARGO_PKG_VERSION")),
        ];
        embed_resource::compile("assets/komorebi.rc", &macros)
            .manifest_required()
            .expect("compiling assets/komorebi.rc (rc.exe from the Windows SDK)");
    }
}
