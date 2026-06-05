use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

use crate::ipc_validation::{validate_external_link_url, IpcValidationError};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLinkOpenRequest {
    pub url: String,
    pub user_requested: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLinkOpenResponse {
    pub opened_url: String,
    pub opened: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLinkCommandError {
    pub reason: String,
}

impl ExternalLinkCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ExternalLinkCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for ExternalLinkCommandError {}

impl From<IpcValidationError> for ExternalLinkCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

pub trait ExternalLinkOpener {
    fn open_external_link(&self, url: &str) -> Result<(), ExternalLinkCommandError>;
}

pub struct SystemExternalLinkOpener;

impl ExternalLinkOpener for SystemExternalLinkOpener {
    fn open_external_link(&self, url: &str) -> Result<(), ExternalLinkCommandError> {
        open_system_link(url)
    }
}

#[tauri::command]
pub fn open_external_link(
    request: ExternalLinkOpenRequest,
) -> Result<ExternalLinkOpenResponse, ExternalLinkCommandError> {
    open_external_link_with_opener(request, &SystemExternalLinkOpener)
}

pub fn open_external_link_with_opener(
    request: ExternalLinkOpenRequest,
    opener: &impl ExternalLinkOpener,
) -> Result<ExternalLinkOpenResponse, ExternalLinkCommandError> {
    if !request.user_requested {
        return Err(ExternalLinkCommandError::new(
            "external link opening requires explicit user intent",
        ));
    }

    let validated_url = validate_external_link_url("url", &request.url)?;
    let opened_url = validated_url.as_str().to_string();
    opener.open_external_link(&opened_url)?;

    Ok(ExternalLinkOpenResponse {
        opened_url,
        opened: true,
    })
}

#[cfg(windows)]
fn open_system_link(url: &str) -> Result<(), ExternalLinkCommandError> {
    use std::ptr;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    let operation = wide_null("open");
    let file = wide_null(url);
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    if result as isize <= 32 {
        return Err(ExternalLinkCommandError::new(
            "external link could not be opened",
        ));
    }

    Ok(())
}

#[cfg(not(windows))]
fn open_system_link(_url: &str) -> Result<(), ExternalLinkCommandError> {
    Err(ExternalLinkCommandError::new(
        "external link opening is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Default)]
    struct FakeExternalLinkOpener {
        opened_urls: Rc<RefCell<Vec<String>>>,
    }

    impl ExternalLinkOpener for FakeExternalLinkOpener {
        fn open_external_link(&self, url: &str) -> Result<(), ExternalLinkCommandError> {
            self.opened_urls.borrow_mut().push(url.to_string());
            Ok(())
        }
    }

    fn request(url: &str, user_requested: bool) -> ExternalLinkOpenRequest {
        ExternalLinkOpenRequest {
            url: url.to_string(),
            user_requested,
        }
    }

    #[test]
    fn opens_only_valid_external_links_after_user_intent() {
        let opener = FakeExternalLinkOpener::default();

        for url in [
            "https://example.com/message",
            "http://example.com/message",
            "mailto:person@example.com?subject=Hello",
        ] {
            let response = open_external_link_with_opener(request(url, true), &opener)
                .expect("valid external link opens");
            assert_eq!(response.opened_url, url);
            assert!(response.opened);
        }

        assert_eq!(
            opener.opened_urls.borrow().as_slice(),
            [
                "https://example.com/message",
                "http://example.com/message",
                "mailto:person@example.com?subject=Hello",
            ]
        );
    }

    #[test]
    fn rejects_missing_intent_and_unsafe_links_before_opening() {
        let opener = FakeExternalLinkOpener::default();

        let missing_intent =
            open_external_link_with_opener(request("https://example.com/message", false), &opener)
                .expect_err("user intent is required");
        assert_eq!(
            missing_intent.reason,
            "external link opening requires explicit user intent"
        );

        for url in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///C:/Windows/win.ini",
            "C:\\Windows\\win.ini",
            "https://user:pass@example.com/message",
            "https://localhost/message",
            "http://127.0.0.1:8080/message",
            "mailto:person@example.com?body=hidden",
            "mailto:person@example.com?attach=file:///C:/secret.txt",
            "mailto:person@example.com%0D%0ABcc:other@example.com",
        ] {
            let error = open_external_link_with_opener(request(url, true), &opener)
                .expect_err("unsafe external link is rejected");
            assert!(!error.reason.contains(url));
        }

        assert!(opener.opened_urls.borrow().is_empty());
    }
}
