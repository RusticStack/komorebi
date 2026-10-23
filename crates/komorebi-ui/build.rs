fn main() {
    // Slint names the translation catalog after the crate being built
    // (`<lang>/LC_MESSAGES/<crate>.po`). The catalogs, the template and
    // tests/i18n_catalog.rs belong to the app, `komorebi`, so name that.
    // SAFETY: a build script is single-threaded at this point.
    unsafe { std::env::set_var("CARGO_PKG_NAME", "komorebi") };
    // English is the source language; `translations/<tag>/LC_MESSAGES/
    // komorebi.po` adds one (the folder name is the tag src/i18n.rs selects).
    // No implicit context (the component name): a string has one only when its
    // `@tr("context" => ...)` names it, so the catalog does not depend on how
    // the UI is split into components.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .with_bundled_translations("../../translations");
    println!("cargo:rerun-if-changed=../../translations");
    slint_build::compile_with_config("../../ui/app.slint", config).expect("compiling Slint UI");
}
