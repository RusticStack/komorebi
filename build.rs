fn main() {
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
