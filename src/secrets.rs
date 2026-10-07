//! Account passwords live only in the OS credential store (Windows Credential
//! Manager). Nothing secret is written to SQLite, logs or UI state.

use anyhow::{Context, Result, anyhow};

const SERVICE: &str = "komorebi";

fn entry(account_id: i64) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("account:{account_id}:password"))
        .context("opening credential entry")
}

pub fn get_password(account_id: i64) -> Result<String> {
    match entry(account_id)?.get_password() {
        Ok(v) => Ok(v),
        Err(keyring::Error::NoEntry) => Err(anyhow!("no saved password for this account")),
        Err(e) => Err(anyhow!("credential read failed: {e}")),
    }
}

pub fn set_password(account_id: i64, password: &str) -> Result<()> {
    entry(account_id)?
        .set_password(password)
        .context("saving password to Windows Credential Manager")
}

pub fn delete_password(account_id: i64) -> Result<()> {
    match entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("credential delete failed: {e}")),
    }
}
