//! Komorebi's mail engine: the SQLite cache, IMAP sync, SMTP send, MIME and
//! HTML handling, provider presets and credential access. The desktop app
//! (`src/main.rs`) and development tools (`examples/`) build on it.

pub mod db;
pub mod diag;
pub mod fetch;
pub mod fold;
pub mod mail;
pub mod providers;
pub mod role;
pub mod secrets;
pub mod trust;

/// A development switch from the environment (`KOMOREBI_*`). Set but empty
/// counts as unset: some shells leave `VAR=` behind instead of removing it.
pub fn dev_var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}
