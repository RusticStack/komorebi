//! Secret storage.
//!
//! Passwords, OAuth tokens and AI API keys live **only** in the OS credential
//! store (Windows Credential Manager on Windows, via the `keyring` crate).
//! Nothing secret is written to SQLite, logs, diagnostics or UI state.

use anyhow::{Context, Result};

const SERVICE: &str = "komorebi";

/// Keys used in the credential store.
pub mod key {
    pub fn account_password(account_id: &str) -> String {
        format!("account:{account_id}:password")
    }
    pub fn account_oauth_refresh(account_id: &str) -> String {
        format!("account:{account_id}:oauth_refresh")
    }
    pub fn account_oauth_access(account_id: &str) -> String {
        format!("account:{account_id}:oauth_access")
    }
    pub fn account_oauth_client_secret(account_id: &str) -> String {
        format!("account:{account_id}:oauth_client_secret")
    }
    pub fn ai_api_key(provider_id: &str) -> String {
        format!("ai:{provider_id}:api_key")
    }
}

pub fn get(user: &str) -> Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE, user).context("opening credential entry")?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("credential read failed: {e}")),
    }
}

pub fn set(user: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, user).context("opening credential entry")?;
    entry
        .set_password(value)
        .context("writing credential to OS store")
}

pub fn delete(user: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, user).context("opening credential entry")?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("credential delete failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    // Credential-store tests are intentionally not run in CI: they touch the
    // real OS credential store. Kept here as a manual smoke path.
    #[test]
    #[ignore]
    fn roundtrip() {
        let user = "test:roundtrip";
        crate::secrets::set(user, "s3cret").unwrap();
        assert_eq!(crate::secrets::get(user).unwrap().unwrap(), "s3cret");
        crate::secrets::delete(user).unwrap();
        assert!(crate::secrets::get(user).unwrap().is_none());
    }
}
