//! Account passwords live only in the OS credential store (Windows Credential
//! Manager). Nothing secret is written to SQLite, logs or UI state.

use anyhow::{Result, anyhow};

const SERVICE: &str = "komorebi";

/// What a failure of the credential store itself says (not a wrong or
/// missing password).
pub const UNAVAILABLE: &str = "Windows Credential Manager is not available";

/// The credential store, set up on first use exactly as the `keyring` v4
/// wrapper did (`windows_native_keyring_store::Store::new()` as the default
/// store), so the passwords it saved keep their names. The wrapper is not
/// used because it also enabled the store's search, which links `regex`.
static STORE: std::sync::LazyLock<keyring_core::Result<()>> = std::sync::LazyLock::new(|| {
    #[cfg(windows)]
    {
        keyring_core::set_default_store(windows_native_keyring_store::Store::new()?);
        Ok(())
    }
    #[cfg(not(windows))]
    Err(keyring_core::Error::NoDefaultStore)
});

fn entry(account_id: i64) -> Result<keyring_core::Entry> {
    if let Err(e) = &*STORE {
        return Err(anyhow!("{UNAVAILABLE} ({e})"));
    }
    keyring_core::Entry::new(SERVICE, &format!("account:{account_id}:password"))
        .map_err(|e| anyhow!("{UNAVAILABLE} ({e})"))
}

/// The credential store has no password for the account.
#[derive(Debug)]
pub struct NoPassword;

impl std::fmt::Display for NoPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("no saved password for this account")
    }
}

impl std::error::Error for NoPassword {}

pub fn get_password(account_id: i64) -> Result<String> {
    // Development: test against a real account without writing its password
    // to the credential store. Only honored together with a throwaway cache,
    // and compiled into release builds only with `dev-env-password`.
    #[cfg(any(debug_assertions, feature = "dev-env-password"))]
    if crate::dev_var("KOMOREBI_DB").is_some()
        && let Some(p) = crate::dev_var("KOMOREBI_PASSWORD").and_then(|p| p.into_string().ok())
    {
        return Ok(p);
    }
    match entry(account_id)?.get_password() {
        Ok(v) => Ok(v),
        Err(keyring_core::Error::NoEntry) => Err(NoPassword.into()),
        Err(e) => Err(anyhow!("{UNAVAILABLE} ({e})")),
    }
}

pub fn set_password(account_id: i64, password: &str) -> Result<()> {
    entry(account_id)?
        .set_password(password)
        .map_err(|e| anyhow!("{UNAVAILABLE} ({e})"))
}

pub fn delete_password(account_id: i64) -> Result<()> {
    match entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("{UNAVAILABLE} ({e})")),
    }
}

/// The optional separate CardDAV/CalDAV password (`account:{id}:dav`),
/// for servers that want an app password of their own.
fn dav_entry(account_id: i64) -> Result<keyring_core::Entry> {
    if let Err(e) = &*STORE {
        return Err(anyhow!("{UNAVAILABLE} ({e})"));
    }
    keyring_core::Entry::new(SERVICE, &format!("account:{account_id}:dav"))
        .map_err(|e| anyhow!("{UNAVAILABLE} ({e})"))
}

pub fn get_dav_password(account_id: i64) -> Result<String> {
    match dav_entry(account_id)?.get_password() {
        Ok(v) => Ok(v),
        Err(keyring_core::Error::NoEntry) => Err(NoPassword.into()),
        Err(e) => Err(anyhow!("{UNAVAILABLE} ({e})")),
    }
}

pub fn set_dav_password(account_id: i64, password: &str) -> Result<()> {
    dav_entry(account_id)?
        .set_password(password)
        .map_err(|e| anyhow!("{UNAVAILABLE} ({e})"))
}

pub fn delete_dav_password(account_id: i64) -> Result<()> {
    match dav_entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("{UNAVAILABLE} ({e})")),
    }
}
