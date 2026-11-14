use std::env;

fn main() {
    // Keep the existing expansion-time directory available for custom macro
    // calls. The shared enum and legacy adapter no longer rely on this path.
    let icons_dir = env::var("DEP_GPUI_KIT_DEFAULT_ICONS_ICONS_DIR")
        .expect("gpui-kit-assets publishes its icon directory through Cargo metadata");
    println!("cargo:rustc-env=GPUI_KIT_DEFAULT_ICONS_DIR={icons_dir}");
    println!("cargo:rerun-if-changed={icons_dir}");
    println!("cargo:rerun-if-changed=build.rs");
    // KOMOREBI PATCH: the `rust-i18n` table is read from here at build time (a
    // pt-PT column; KOMOREBI-PATCH.md), and a build script with
    // `rerun-if-changed` lines makes cargo watch only those paths.
    println!("cargo:rerun-if-changed=locales");
}
