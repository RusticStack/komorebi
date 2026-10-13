//! Komorebi's mail engine: the SQLite cache, IMAP sync, SMTP send, MIME and
//! HTML handling, provider presets, credential access and the Windows
//! appearance settings. The desktop app (`src/main.rs`) and development
//! tools (`examples/`) build on it.

pub mod chrome;
pub mod db;
pub mod diag;
pub mod fetch;
pub mod fold;
pub mod i18n;
pub mod mail;
pub mod onboard;
pub mod providers;
pub mod role;
pub mod secrets;
pub mod theme;
pub mod tree;
pub mod trust;

/// A development switch from the environment (`KOMOREBI_*`). Set but empty
/// counts as unset: some shells leave `VAR=` behind instead of removing it.
pub fn dev_var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}
