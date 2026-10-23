//! The generated Slint user interface (`ui/*.slint`), a crate of its own so
//! that the release profile can size-optimize it apart from the app's mail,
//! database and rendering code (docs/PERF.md 4.1).

slint::include_modules!();
