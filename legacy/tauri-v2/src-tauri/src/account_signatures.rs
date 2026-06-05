use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
};

use ammonia::{Builder, UrlRelative};
use rusqlite::{params, Connection, OptionalExtension};

use crate::ipc_validation::{
    validate_entity_id, validate_payload_size, IpcValidationError, MAX_TEXT_FIELD_BYTES,
};

pub const SIGNATURE_HTML_BLOCK_START: &str = "<!-- komorebi-signature:start -->";
pub const SIGNATURE_HTML_BLOCK_END: &str = "<!-- komorebi-signature:end -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSignatureHtml(String);

impl AccountSignatureHtml {
    pub fn sanitize(value: &str) -> Result<Option<Self>, AccountSignatureError> {
        validate_payload_size("signature_html", value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;

        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let sanitized = sanitize_signature_fragment(trimmed);
        validate_payload_size("signature_html", sanitized.as_bytes(), MAX_TEXT_FIELD_BYTES)?;

        if sanitized.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(Self(sanitized)))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSignatureRecord {
    pub account_id: String,
    pub signature_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAccountSignatureRequest {
    pub account_id: String,
    pub signature_html: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSignatureError {
    IpcValidation(IpcValidationError),
    UnknownAccount,
    Database(String),
}

impl fmt::Display for AccountSignatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::UnknownAccount => write!(formatter, "account was not found"),
            Self::Database(error) => write!(formatter, "account signature failed: {error}"),
        }
    }
}

impl Error for AccountSignatureError {}

impl From<IpcValidationError> for AccountSignatureError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

impl From<rusqlite::Error> for AccountSignatureError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

pub fn update_account_signature(
    connection: &Connection,
    request: UpdateAccountSignatureRequest,
) -> Result<AccountSignatureRecord, AccountSignatureError> {
    validate_entity_id("account_id", &request.account_id)?;
    ensure_account_exists(connection, &request.account_id)?;

    let sanitized = AccountSignatureHtml::sanitize(&request.signature_html)?;
    let signature_html = sanitized.as_ref().map(|signature| signature.as_str());
    let changed = connection.execute(
        "UPDATE accounts
         SET signature_html = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND removed_at IS NULL",
        params![&request.account_id, signature_html],
    )?;

    if changed != 1 {
        return Err(AccountSignatureError::UnknownAccount);
    }

    load_account_signature(connection, &request.account_id)?
        .ok_or(AccountSignatureError::UnknownAccount)
}

pub fn load_account_signature(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<AccountSignatureRecord>, AccountSignatureError> {
    validate_entity_id("account_id", account_id)?;

    connection
        .query_row(
            "SELECT id, signature_html
             FROM accounts
             WHERE id = ?1 AND removed_at IS NULL",
            [account_id],
            |row| {
                Ok(AccountSignatureRecord {
                    account_id: row.get(0)?,
                    signature_html: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(AccountSignatureError::from)
}

pub fn signature_html_for_send(
    stored_signature_html: Option<&str>,
) -> Result<Option<AccountSignatureHtml>, AccountSignatureError> {
    match stored_signature_html {
        Some(value) => AccountSignatureHtml::sanitize(value),
        None => Ok(None),
    }
}

pub fn insert_signature_into_body_html(
    body_html: &str,
    stored_signature_html: Option<&str>,
) -> Result<String, AccountSignatureError> {
    validate_payload_size("body_html", body_html.as_bytes(), MAX_TEXT_FIELD_BYTES)?;

    let without_existing_signature = remove_signature_block(body_html);
    let Some(signature) = signature_html_for_send(stored_signature_html)? else {
        return Ok(without_existing_signature.trim_end().to_string());
    };

    let block = signature_html_block(signature.as_str());
    let body = without_existing_signature.trim_end();
    if body.is_empty() {
        Ok(block)
    } else {
        Ok(format!("{body}\n{block}"))
    }
}

pub fn insert_signature_into_body_text(
    body_text: &str,
    stored_signature_html: Option<&str>,
) -> Result<String, AccountSignatureError> {
    validate_payload_size("body_text", body_text.as_bytes(), MAX_TEXT_FIELD_BYTES)?;

    let Some(signature) = signature_html_for_send(stored_signature_html)? else {
        return Ok(body_text.to_string());
    };
    let signature_text = signature_html_to_plain_text(signature.as_str());
    if signature_text.trim().is_empty() {
        return Ok(body_text.to_string());
    }

    let body = body_text.trim_end();
    if body.is_empty() {
        Ok(signature_text)
    } else {
        Ok(format!("{body}\n\n-- \n{signature_text}"))
    }
}

fn sanitize_signature_fragment(value: &str) -> String {
    let mut tag_attributes = HashMap::new();
    tag_attributes.insert("a", set(&["href", "title", "aria-label"]));

    Builder::default()
        .tags(set(&[
            "a",
            "b",
            "blockquote",
            "br",
            "div",
            "em",
            "i",
            "li",
            "ol",
            "p",
            "span",
            "strong",
            "u",
            "ul",
        ]))
        .tag_attributes(tag_attributes)
        .generic_attributes(set(&["aria-label"]))
        .url_schemes(set(&["http", "https", "mailto"]))
        .url_relative(UrlRelative::Deny)
        .link_rel(Some("noreferrer noopener nofollow"))
        .clean(value)
        .to_string()
}

fn signature_html_block(signature_html: &str) -> String {
    format!(
        "{SIGNATURE_HTML_BLOCK_START}<div class=\"komorebi-account-signature\">{signature_html}</div>{SIGNATURE_HTML_BLOCK_END}"
    )
}

fn remove_signature_block(value: &str) -> String {
    let mut output = value.to_string();
    while let Some(start) = output.find(SIGNATURE_HTML_BLOCK_START) {
        let after_start = start + SIGNATURE_HTML_BLOCK_START.len();
        let Some(relative_end) = output[after_start..].find(SIGNATURE_HTML_BLOCK_END) else {
            break;
        };
        let end = after_start + relative_end + SIGNATURE_HTML_BLOCK_END.len();
        output.replace_range(start..end, "");
    }

    output
}

fn signature_html_to_plain_text(value: &str) -> String {
    let mut output = String::new();
    let mut tag = String::new();
    let mut in_tag = false;

    for character in value.chars() {
        match character {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let tag_name = tag
                    .trim()
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if matches!(tag_name.as_str(), "br" | "div" | "p" | "li") {
                    push_newline_once(&mut output);
                }
            }
            _ if in_tag => tag.push(character),
            _ => output.push(character),
        }
    }

    let decoded = output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'");

    decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_newline_once(output: &mut String) {
    if !output.ends_with('\n') {
        output.push('\n');
    }
}

fn ensure_account_exists(
    connection: &Connection,
    account_id: &str,
) -> Result<(), AccountSignatureError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND removed_at IS NULL)",
        [account_id],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(AccountSignatureError::UnknownAccount)
    }
}

fn set(values: &[&'static str]) -> HashSet<&'static str> {
    values.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    fn seed_account(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO accounts (
                   id,
                   provider_kind,
                   display_email,
                   username,
                   auth_kind,
                   credential_ref
                 )
                 VALUES (
                   'acct_1',
                   'gmail',
                   'sender@example.com',
                   'sender@example.com',
                   'oauth2',
                   'cred_acct_1'
                 )",
                [],
            )
            .unwrap();
    }

    #[test]
    fn updates_signature_as_sanitized_account_metadata() {
        let connection = migrated_connection();
        seed_account(&connection);

        let record = update_account_signature(
            &connection,
            UpdateAccountSignatureRequest {
                account_id: "acct_1".into(),
                signature_html: r#"<p onclick="alert(1)">Regards<br><a href="https://example.com/profile">Sender</a><img src="https://tracker.example/pixel.gif"></p>"#.into(),
            },
        )
        .unwrap();

        let signature = record.signature_html.unwrap();
        assert!(signature.contains("Regards"));
        assert!(signature.contains("https://example.com/profile"));
        assert!(signature.contains("rel=\"noreferrer noopener nofollow\""));
        assert!(!signature.contains("onclick"));
        assert!(!signature.contains("<img"));
        assert!(!signature.contains("tracker.example"));
    }

    #[test]
    fn clearing_signature_stores_null() {
        let connection = migrated_connection();
        seed_account(&connection);
        update_account_signature(
            &connection,
            UpdateAccountSignatureRequest {
                account_id: "acct_1".into(),
                signature_html: "<p>Sender</p>".into(),
            },
        )
        .unwrap();

        let record = update_account_signature(
            &connection,
            UpdateAccountSignatureRequest {
                account_id: "acct_1".into(),
                signature_html: "   ".into(),
            },
        )
        .unwrap();

        assert_eq!(record.signature_html, None);
    }

    #[test]
    fn rejects_invalid_account_ids_and_unknown_accounts() {
        let connection = migrated_connection();

        assert!(matches!(
            update_account_signature(
                &connection,
                UpdateAccountSignatureRequest {
                    account_id: "../acct".into(),
                    signature_html: "<p>Sender</p>".into(),
                },
            ),
            Err(AccountSignatureError::IpcValidation(_))
        ));
        assert_eq!(
            update_account_signature(
                &connection,
                UpdateAccountSignatureRequest {
                    account_id: "missing".into(),
                    signature_html: "<p>Sender</p>".into(),
                },
            ),
            Err(AccountSignatureError::UnknownAccount)
        );
    }

    #[test]
    fn inserts_signature_predictably_without_duplication() {
        let first = insert_signature_into_body_html(
            "<p>Hello</p>",
            Some("<p>Regards,<br><strong>Sender</strong></p>"),
        )
        .unwrap();
        let second = insert_signature_into_body_html(
            &first,
            Some("<p>Updated,<br><strong>Sender</strong></p>"),
        )
        .unwrap();

        assert!(first.starts_with("<p>Hello</p>"));
        assert!(first.contains(SIGNATURE_HTML_BLOCK_START));
        assert!(first.contains("komorebi-account-signature"));
        assert_eq!(second.matches(SIGNATURE_HTML_BLOCK_START).count(), 1);
        assert!(second.contains("Updated"));
        assert!(!second.contains("Regards"));
    }

    #[test]
    fn removes_existing_signature_when_account_signature_is_cleared() {
        let with_signature =
            insert_signature_into_body_html("<p>Hello</p>", Some("<p>Sender</p>")).unwrap();

        let without_signature = insert_signature_into_body_html(&with_signature, None).unwrap();

        assert_eq!(without_signature, "<p>Hello</p>");
    }

    #[test]
    fn adds_plain_text_signature_for_text_only_send() {
        let signed = insert_signature_into_body_text(
            "Hello",
            Some(r#"<p>Regards,<br><a href="mailto:sender@example.com">Sender</a></p>"#),
        )
        .unwrap();

        assert_eq!(signed, "Hello\n\n-- \nRegards,\nSender");
    }
}
