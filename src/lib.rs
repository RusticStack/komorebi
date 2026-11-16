//! Komorebi's mail engine: the SQLite cache, IMAP sync, SMTP send, MIME and
//! HTML handling, provider presets, credential access and the Windows
//! appearance settings. The desktop app (`crates/komorebi-gpui`) and
//! development tools (`examples/`) build on it.

#[cfg(windows)]
pub mod cachecrypt;
pub mod cal;
pub mod chrome;
pub mod compose;
pub mod contacts;
pub mod contacts_page;
pub mod contacts_sync;
pub mod crash;
pub mod dav;
pub mod db;
pub mod diag;
pub mod favicon;
pub mod fetch;
pub mod files;
pub mod find;
pub mod fold;
pub mod format;
pub mod i18n;
pub mod import;
pub mod instance;
pub mod invite;
pub mod launch;
pub mod mail;
pub mod notify;
pub mod onboard;
pub mod providers;
pub mod reader;
pub mod role;
pub mod rows;
pub mod rules;
pub mod rules_page;
pub mod secrets;
pub mod seed;
pub mod tabs;
#[cfg(test)]
pub(crate) mod testdir;
pub mod theme;
pub mod tree;
pub mod trust;
pub mod update;
#[cfg(windows)]
pub mod win_shell;

/// A development switch from the environment (`KOMOREBI_*`). Set but empty
/// counts as unset: some shells leave `VAR=` behind instead of removing it.
pub fn dev_var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}
