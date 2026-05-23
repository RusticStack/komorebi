use std::{error::Error, fmt, ptr, slice};

use crate::{
    ipc_validation::{validate_entity_id, IpcValidationError},
    token_lifecycle::{CredentialStore, SecretStoreError, SecretString},
};

pub const CREDENTIAL_NAMESPACE: &str = "Komorebi/mail-account";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialSlot {
    OAuthRefresh,
    OAuthAccess,
    GenericPassword,
}

impl CredentialSlot {
    pub const fn name(self) -> &'static str {
        match self {
            Self::OAuthRefresh => "oauth-refresh",
            Self::OAuthAccess => "oauth-access",
            Self::GenericPassword => "generic-password",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    IpcValidation(IpcValidationError),
    SecretTooLarge { max_bytes: u32, actual_bytes: usize },
    WindowsUnavailable { operation: &'static str, code: u32 },
    NotFound,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::SecretTooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "credential secret exceeds {max_bytes} bytes ({actual_bytes} bytes provided)"
            ),
            Self::WindowsUnavailable { operation, code } => {
                write!(
                    formatter,
                    "Windows credential {operation} failed with {code}"
                )
            }
            Self::NotFound => write!(formatter, "credential was not found"),
        }
    }
}

impl Error for CredentialError {}

impl From<IpcValidationError> for CredentialError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

pub fn credential_target_name(
    account_id: &str,
    slot: CredentialSlot,
) -> Result<String, CredentialError> {
    validate_entity_id("account_id", account_id)?;

    Ok(format!(
        "{CREDENTIAL_NAMESPACE}/{account_id}/{}",
        slot.name()
    ))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsCredentialManager;

impl WindowsCredentialManager {
    pub fn write_secret(
        &self,
        account_id: &str,
        slot: CredentialSlot,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        let target_name = credential_target_name(account_id, slot)?;
        let username = format!("{}:{account_id}", slot.name());

        write_windows_credential(&target_name, &username, secret.expose_secret())
    }

    pub fn read_secret(
        &self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<SecretString, CredentialError> {
        let target_name = credential_target_name(account_id, slot)?;

        read_windows_credential(&target_name)
    }

    pub fn delete_secret(
        &self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<(), CredentialError> {
        let target_name = credential_target_name(account_id, slot)?;

        delete_windows_credential(&target_name)
    }

    pub fn delete_all_account_secrets(&self, account_id: &str) -> Result<(), CredentialError> {
        credential_target_name(account_id, CredentialSlot::OAuthRefresh)?;

        for slot in [
            CredentialSlot::OAuthRefresh,
            CredentialSlot::OAuthAccess,
            CredentialSlot::GenericPassword,
        ] {
            self.delete_secret(account_id, slot)?;
        }

        Ok(())
    }
}

impl CredentialStore for WindowsCredentialManager {
    fn load_refresh_token(
        &mut self,
        account_id: &str,
    ) -> Result<Option<SecretString>, SecretStoreError> {
        match self.read_secret(account_id, CredentialSlot::OAuthRefresh) {
            Ok(secret) => Ok(Some(secret)),
            Err(CredentialError::NotFound) => Ok(None),
            Err(_) => Err(SecretStoreError::Unavailable),
        }
    }

    fn store_refresh_token(
        &mut self,
        account_id: &str,
        token: SecretString,
    ) -> Result<(), SecretStoreError> {
        self.write_secret(account_id, CredentialSlot::OAuthRefresh, &token)
            .map_err(|_| SecretStoreError::Unavailable)
    }

    fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), SecretStoreError> {
        self.delete_all_account_secrets(account_id)
            .map_err(|_| SecretStoreError::Unavailable)
    }
}

#[cfg(windows)]
fn write_windows_credential(
    target_name: &str,
    username: &str,
    secret: &str,
) -> Result<(), CredentialError> {
    use windows_sys::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    let secret_bytes = secret.as_bytes();
    if secret_bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(CredentialError::SecretTooLarge {
            max_bytes: CRED_MAX_CREDENTIAL_BLOB_SIZE,
            actual_bytes: secret_bytes.len(),
        });
    }

    let mut target_name = to_wide_null(target_name);
    let mut username = to_wide_null(username);
    let mut secret_bytes = secret_bytes.to_vec();

    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_name.as_mut_ptr(),
        Comment: ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: secret_bytes.len() as u32,
        CredentialBlob: secret_bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: ptr::null_mut(),
        TargetAlias: ptr::null_mut(),
        UserName: username.as_mut_ptr(),
    };

    // SAFETY: pointers reference live, null-terminated UTF-16 buffers and a live
    // credential blob for the duration of the Win32 call.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(windows_error("write"));
    }

    Ok(())
}

#[cfg(not(windows))]
fn write_windows_credential(
    _target_name: &str,
    _username: &str,
    _secret: &str,
) -> Result<(), CredentialError> {
    Err(CredentialError::WindowsUnavailable {
        operation: "write",
        code: 0,
    })
}

#[cfg(windows)]
fn read_windows_credential(target_name: &str) -> Result<SecretString, CredentialError> {
    use windows_sys::Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC},
    };

    let target_name = to_wide_null(target_name);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();

    // SAFETY: target_name is a live, null-terminated UTF-16 buffer; Credential
    // receives an allocated pointer that is freed with CredFree below.
    if unsafe { CredReadW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        let error = windows_error("read");
        if matches!(
            error,
            CredentialError::WindowsUnavailable { code, .. } if code == ERROR_NOT_FOUND
        ) {
            return Err(CredentialError::NotFound);
        }

        return Err(error);
    }

    if credential.is_null() {
        return Err(CredentialError::WindowsUnavailable {
            operation: "read",
            code: 0,
        });
    }

    // SAFETY: CredReadW returned a non-null pointer to a CREDENTIALW block. The
    // blob pointer/size are owned by that block until CredFree is called.
    let secret = unsafe {
        let credential_ref = &*credential;
        let bytes = slice::from_raw_parts(
            credential_ref.CredentialBlob,
            credential_ref.CredentialBlobSize as usize,
        );
        String::from_utf8_lossy(bytes).into_owned()
    };

    // SAFETY: credential was allocated by CredReadW and must be released with
    // CredFree exactly once after copying the blob.
    unsafe { CredFree(credential.cast()) };

    Ok(SecretString::new(secret))
}

#[cfg(not(windows))]
fn read_windows_credential(_target_name: &str) -> Result<SecretString, CredentialError> {
    Err(CredentialError::WindowsUnavailable {
        operation: "read",
        code: 0,
    })
}

#[cfg(windows)]
fn delete_windows_credential(target_name: &str) -> Result<(), CredentialError> {
    use windows_sys::Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC},
    };

    let target_name = to_wide_null(target_name);

    // SAFETY: target_name is a live, null-terminated UTF-16 buffer.
    if unsafe { CredDeleteW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        let error = windows_error("delete");
        if matches!(
            error,
            CredentialError::WindowsUnavailable { code, .. } if code == ERROR_NOT_FOUND
        ) {
            return Ok(());
        }

        return Err(error);
    }

    Ok(())
}

#[cfg(not(windows))]
fn delete_windows_credential(_target_name: &str) -> Result<(), CredentialError> {
    Err(CredentialError::WindowsUnavailable {
        operation: "delete",
        code: 0,
    })
}

#[cfg(windows)]
fn windows_error(operation: &'static str) -> CredentialError {
    use windows_sys::Win32::Foundation::GetLastError;

    CredentialError::WindowsUnavailable {
        operation,
        // SAFETY: GetLastError reads the calling thread's last Win32 error code.
        code: unsafe { GetLastError() },
    }
}

#[cfg(windows)]
fn to_wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names_are_namespaced_and_slot_specific() {
        assert_eq!(
            credential_target_name("acct_1", CredentialSlot::OAuthRefresh).unwrap(),
            "Komorebi/mail-account/acct_1/oauth-refresh"
        );
        assert_eq!(
            credential_target_name("acct_1", CredentialSlot::GenericPassword).unwrap(),
            "Komorebi/mail-account/acct_1/generic-password"
        );
    }

    #[test]
    fn target_names_reject_path_like_account_ids() {
        assert!(matches!(
            credential_target_name("..\\secret", CredentialSlot::OAuthRefresh),
            Err(CredentialError::IpcValidation(
                IpcValidationError::InvalidId { .. }
            ))
        ));
    }

    #[test]
    fn secret_debug_is_redacted_before_reaching_the_backend() {
        let secret = SecretString::new("refresh-token-value");

        assert_eq!(format!("{secret:?}"), "[redacted]");
    }
}
