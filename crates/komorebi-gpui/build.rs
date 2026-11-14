fn main() {
    // The icons, version information and manifest of komorebi.exe, from
    // assets/komorebi.rc: the seal icon and the three tray icons
    // (docs/BRANDING.md), and the manifest (assets/komorebi.manifest).
    // Linked into the binary only, not tests. The
    // manifest carries what GPUI's own did (PerMonitorV2 DPI awareness,
    // Common Controls 6, asInvoker, Windows 10 and 11); the
    // `windows-manifest-provided-by-host` feature keeps GPUI's out.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for file in [
            "komorebi.rc",
            "komorebi.ico",
            "tray/tray-light.ico",
            "tray/tray-dark.ico",
            "tray/tray-unread.ico",
            "komorebi.manifest",
        ] {
            println!("cargo:rerun-if-changed=../../assets/{file}");
        }
        let part = |name: &str| std::env::var(name).expect("Cargo sets the package version");
        let macros = [
            format!("VER_MAJOR={}", part("CARGO_PKG_VERSION_MAJOR")),
            format!("VER_MINOR={}", part("CARGO_PKG_VERSION_MINOR")),
            format!("VER_PATCH={}", part("CARGO_PKG_VERSION_PATCH")),
            format!("VER_STRING=\"{}\"", part("CARGO_PKG_VERSION")),
        ];
        embed_resource::compile("../../assets/komorebi.rc", &macros)
            .manifest_required()
            .expect("compiling assets/komorebi.rc (rc.exe from the Windows SDK)");
        // The development examples (the reader's and the composer's snapshots)
        // open real windows too and need the same manifest (Common Controls 6,
        // DPI awareness): without it they fail to start.
        embed_resource::compile_for_examples("../../assets/komorebi.rc", &macros)
            .manifest_required()
            .expect("compiling assets/komorebi.rc for the examples");
    }
}
