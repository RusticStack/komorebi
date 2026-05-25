use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsStr,
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::schema::{apply_migrations, current_schema_version};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const LOG_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "komorebi.ndjson";
const MAX_LOG_READ_BYTES: u64 = 1024 * 1024;
const MAX_EXPORTED_LOG_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticLogEntry {
    pub timestamp_unix_ms: i64,
    pub level: DiagnosticLogLevel,
    pub component: String,
    pub event: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticsExportRequest {
    pub user_requested: bool,
    pub include_synthetic_fixtures: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExportResponse {
    pub destination_label: String,
    pub bytes_written: u64,
    pub included_log_entries: usize,
    pub included_synthetic_fixtures: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsCommandError {
    pub reason: String,
}

impl DiagnosticsCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for DiagnosticsCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for DiagnosticsCommandError {}

impl From<tauri::Error> for DiagnosticsCommandError {
    fn from(_value: tauri::Error) -> Self {
        Self::new("diagnostics app data is unavailable")
    }
}

impl From<io::Error> for DiagnosticsCommandError {
    fn from(_value: io::Error) -> Self {
        Self::new("diagnostics filesystem operation failed")
    }
}

impl From<serde_json::Error> for DiagnosticsCommandError {
    fn from(_value: serde_json::Error) -> Self {
        Self::new("diagnostics serialization failed")
    }
}

#[tauri::command]
pub fn export_diagnostics_bundle(
    app: tauri::AppHandle,
    request: DiagnosticsExportRequest,
) -> Result<DiagnosticsExportResponse, DiagnosticsCommandError> {
    if !request.user_requested {
        return Err(DiagnosticsCommandError::new(
            "diagnostics export requires explicit user intent",
        ));
    }

    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)
        .map_err(|_| DiagnosticsCommandError::new("diagnostics app data is unavailable"))?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    let destination = selected_diagnostics_destination(&app)?;
    export_diagnostics_bundle_to_path(&app_data_dir, &cache_path, &destination, request)
}

pub fn diagnostics_log_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOG_DIR_NAME).join(LOG_FILE_NAME)
}

pub fn append_log_event(
    app_data_dir: &Path,
    level: DiagnosticLogLevel,
    component: &str,
    event: &str,
    fields: BTreeMap<String, String>,
) -> Result<DiagnosticLogEntry, DiagnosticsCommandError> {
    let entry = DiagnosticLogEntry {
        timestamp_unix_ms: unix_now_millis(),
        level,
        component: sanitize_identifier(component),
        event: sanitize_identifier(event),
        fields: redact_fields(fields),
    };
    let path = diagnostics_log_path(app_data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(&entry)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(entry)
}

pub fn log_startup_migration_status(cache_path: &Path) -> Result<(), DiagnosticsCommandError> {
    let Some(app_data_dir) = cache_path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(app_data_dir)?;

    let mut connection = match Connection::open(cache_path) {
        Ok(connection) => connection,
        Err(_error) => {
            return log_component_error(
                app_data_dir,
                "schema",
                "migration_status",
                "mail cache could not be opened",
            );
        }
    };

    match apply_migrations(&mut connection) {
        Ok(report) => {
            let mut fields = BTreeMap::new();
            fields.insert(
                "current_version".to_string(),
                report.current_version.to_string(),
            );
            fields.insert(
                "latest_version".to_string(),
                report.latest_version.to_string(),
            );
            fields.insert(
                "applied_versions".to_string(),
                if report.applied.is_empty() {
                    "none".to_string()
                } else {
                    report
                        .applied
                        .iter()
                        .map(|migration| migration.version.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                },
            );
            append_log_event(
                app_data_dir,
                DiagnosticLogLevel::Info,
                "schema",
                "migration_status",
                fields,
            )?;
        }
        Err(error) => {
            log_component_error(
                app_data_dir,
                "schema",
                "migration_status",
                &error.to_string(),
            )?;
        }
    }

    Ok(())
}

pub fn log_sync_status(
    app_data_dir: &Path,
    account_id: &str,
    state: &str,
    message: Option<&str>,
    action: Option<&str>,
    code: Option<&str>,
) -> Result<(), DiagnosticsCommandError> {
    let mut fields = BTreeMap::new();
    fields.insert("account_id".to_string(), account_id.to_string());
    fields.insert("state".to_string(), state.to_string());
    if let Some(message) = message {
        fields.insert("message".to_string(), message.to_string());
    }
    if let Some(action) = action {
        fields.insert("action".to_string(), action.to_string());
    }
    if let Some(code) = code {
        fields.insert("code".to_string(), code.to_string());
    }

    let level = match state {
        "error" => DiagnosticLogLevel::Error,
        "reconnect-needed" | "offline" => DiagnosticLogLevel::Warn,
        _ => DiagnosticLogLevel::Info,
    };

    append_log_event(
        app_data_dir,
        level,
        "sync_scheduler",
        "sync_status",
        fields.clone(),
    )?;
    if action == Some("retry_later") {
        append_log_event(
            app_data_dir,
            DiagnosticLogLevel::Warn,
            "sync_scheduler",
            "retry_scheduled",
            fields,
        )?;
    }
    Ok(())
}

pub fn log_command_failure(
    app_data_dir: &Path,
    command: &str,
    reason: &str,
) -> Result<(), DiagnosticsCommandError> {
    let mut fields = BTreeMap::new();
    fields.insert("command".to_string(), command.to_string());
    fields.insert("reason".to_string(), reason.to_string());
    append_log_event(
        app_data_dir,
        DiagnosticLogLevel::Error,
        "ipc",
        "command_failure",
        fields,
    )?;
    Ok(())
}

fn log_component_error(
    app_data_dir: &Path,
    component: &str,
    event: &str,
    reason: &str,
) -> Result<(), DiagnosticsCommandError> {
    let mut fields = BTreeMap::new();
    fields.insert("reason".to_string(), reason.to_string());
    append_log_event(
        app_data_dir,
        DiagnosticLogLevel::Error,
        component,
        event,
        fields,
    )?;
    Ok(())
}

pub fn export_diagnostics_bundle_to_path(
    app_data_dir: &Path,
    cache_path: &Path,
    destination: &Path,
    request: DiagnosticsExportRequest,
) -> Result<DiagnosticsExportResponse, DiagnosticsCommandError> {
    if !request.user_requested {
        return Err(DiagnosticsCommandError::new(
            "diagnostics export requires explicit user intent",
        ));
    }

    let destination = checked_diagnostics_destination(destination)?;
    let logs = read_redacted_log_entries(app_data_dir)?;
    let bundle = DiagnosticBundle {
        schema_version: 1,
        generated_at_unix_ms: unix_now_millis(),
        privacy: DiagnosticBundlePrivacy {
            redacted: true,
            excludes_credentials: true,
            excludes_tokens: true,
            excludes_raw_message_bodies: true,
            excludes_attachment_bytes: true,
            excludes_private_subjects: true,
            synthetic_fixtures_only: request.include_synthetic_fixtures,
        },
        database: summarize_database(cache_path),
        logs,
        synthetic_fixtures: request
            .include_synthetic_fixtures
            .then(synthetic_diagnostic_fixtures),
    };
    let included_log_entries = bundle.logs.len();
    let included_synthetic_fixtures = bundle.synthetic_fixtures.is_some();
    let bytes = serde_json::to_vec_pretty(&bundle)?;
    let bytes_written = write_new_file(&destination, &bytes)?;
    let destination_label = destination
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("komorebi-diagnostics.json")
        .to_string();

    let _ = append_log_event(
        app_data_dir,
        DiagnosticLogLevel::Info,
        "diagnostics",
        "export_completed",
        BTreeMap::from([
            ("destination_label".to_string(), destination_label.clone()),
            ("bytes_written".to_string(), bytes_written.to_string()),
        ]),
    );

    Ok(DiagnosticsExportResponse {
        destination_label,
        bytes_written,
        included_log_entries,
        included_synthetic_fixtures,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticBundle {
    schema_version: u32,
    generated_at_unix_ms: i64,
    privacy: DiagnosticBundlePrivacy,
    database: DatabaseDiagnosticSummary,
    logs: Vec<DiagnosticLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    synthetic_fixtures: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticBundlePrivacy {
    redacted: bool,
    excludes_credentials: bool,
    excludes_tokens: bool,
    excludes_raw_message_bodies: bool,
    excludes_attachment_bytes: bool,
    excludes_private_subjects: bool,
    synthetic_fixtures_only: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseDiagnosticSummary {
    mail_cache_present: bool,
    schema_version: Option<u32>,
    counts: BTreeMap<String, i64>,
    auth_status_counts: BTreeMap<String, i64>,
    sync_status_counts: BTreeMap<String, i64>,
    error: Option<String>,
}

fn summarize_database(cache_path: &Path) -> DatabaseDiagnosticSummary {
    let mut summary = DatabaseDiagnosticSummary {
        mail_cache_present: cache_path.exists(),
        schema_version: None,
        counts: BTreeMap::new(),
        auth_status_counts: BTreeMap::new(),
        sync_status_counts: BTreeMap::new(),
        error: None,
    };

    if !summary.mail_cache_present {
        return summary;
    }

    let connection = match Connection::open_with_flags(cache_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    {
        Ok(connection) => connection,
        Err(_error) => {
            summary.error = Some("database summary unavailable".to_string());
            return summary;
        }
    };

    summary.schema_version = current_schema_version(&connection).ok();
    for table in [
        "accounts",
        "folders",
        "messages",
        "attachments",
        "settings",
        "composer_drafts",
        "sent_mail_copies",
    ] {
        if let Some(count) = table_count(&connection, table) {
            summary.counts.insert(table.to_string(), count);
        }
    }

    summary.auth_status_counts = grouped_count(&connection, "accounts", "auth_status");
    summary.sync_status_counts = sync_status_counts(&connection);
    summary
}

fn table_count(connection: &Connection, table: &str) -> Option<i64> {
    if !table_exists(connection, table) {
        return None;
    }
    let sql = format!("SELECT COUNT(*) FROM {table}");
    connection.query_row(&sql, [], |row| row.get(0)).ok()
}

fn grouped_count(connection: &Connection, table: &str, column: &str) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    if !table_exists(connection, table) {
        return counts;
    }
    let sql = format!("SELECT {column}, COUNT(*) FROM {table} GROUP BY {column}");
    let Ok(mut statement) = connection.prepare(&sql) else {
        return counts;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) else {
        return counts;
    };
    for row in rows.flatten() {
        counts.insert(row.0, row.1);
    }
    counts
}

fn sync_status_counts(connection: &Connection) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    if !table_exists(connection, "settings") {
        return counts;
    }
    let Ok(mut statement) = connection.prepare(
        "SELECT value_json
         FROM settings
         WHERE key LIKE 'account:%:sync_worker_status'",
    ) else {
        return counts;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return counts;
    };
    for value_json in rows.flatten() {
        let Ok(value) = serde_json::from_str::<Value>(&value_json) else {
            continue;
        };
        let Some(state) = value.get("state").and_then(Value::as_str) else {
            continue;
        };
        *counts.entry(state.to_string()).or_insert(0) += 1;
    }
    counts
}

fn table_exists(connection: &Connection, table: &str) -> bool {
    connection
        .query_row(
            "SELECT 1
             FROM sqlite_master
             WHERE type = 'table'
               AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
}

fn selected_diagnostics_destination(
    app: &tauri::AppHandle,
) -> Result<PathBuf, DiagnosticsCommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Export diagnostics")
        .set_file_name("komorebi-diagnostics.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .ok_or_else(|| DiagnosticsCommandError::new("diagnostics export was cancelled"))?;

    selected
        .into_path()
        .map_err(|_| DiagnosticsCommandError::new("diagnostics export destination is unavailable"))
}

fn checked_diagnostics_destination(destination: &Path) -> Result<PathBuf, DiagnosticsCommandError> {
    if destination
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DiagnosticsCommandError::new(
            "diagnostics export destination is outside the selected scope",
        ));
    }

    let mut candidate = destination.to_path_buf();
    match candidate.extension().and_then(OsStr::to_str) {
        Some(extension) if extension.eq_ignore_ascii_case("json") => {}
        Some(_) => {
            return Err(DiagnosticsCommandError::new(
                "diagnostics export must use a .json file",
            ));
        }
        None => {
            candidate.set_extension("json");
        }
    }

    let file_name = candidate.file_name().ok_or_else(|| {
        DiagnosticsCommandError::new("diagnostics export destination is unavailable")
    })?;
    let parent = candidate.parent().ok_or_else(|| {
        DiagnosticsCommandError::new("diagnostics export destination is unavailable")
    })?;
    let root = parent.canonicalize().map_err(|_| {
        DiagnosticsCommandError::new("diagnostics export destination is unavailable")
    })?;
    if !root.is_dir() {
        return Err(DiagnosticsCommandError::new(
            "diagnostics export destination is unavailable",
        ));
    }
    let checked = root.join(file_name);
    if checked.parent() != Some(root.as_path()) || !checked.starts_with(&root) {
        return Err(DiagnosticsCommandError::new(
            "diagnostics export destination is outside the selected scope",
        ));
    }
    Ok(checked)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<u64, DiagnosticsCommandError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            if let Err(_error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                let _ = fs::remove_file(path);
                return Err(DiagnosticsCommandError::new("diagnostics export failed"));
            }
            Ok(bytes.len() as u64)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(
            DiagnosticsCommandError::new("diagnostics export destination already exists"),
        ),
        Err(_error) => Err(DiagnosticsCommandError::new("diagnostics export failed")),
    }
}

fn read_redacted_log_entries(
    app_data_dir: &Path,
) -> Result<Vec<DiagnosticLogEntry>, DiagnosticsCommandError> {
    let path = diagnostics_log_path(app_data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = File::open(&path)?;
    let length = file.metadata()?.len();
    let mut skip_first_partial_line = false;
    if length > MAX_LOG_READ_BYTES {
        file.seek(SeekFrom::Start(length - MAX_LOG_READ_BYTES))?;
        skip_first_partial_line = true;
    }

    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let mut lines = content.lines().collect::<Vec<_>>();
    if skip_first_partial_line && !lines.is_empty() {
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(MAX_EXPORTED_LOG_ENTRIES);
    let mut entries = Vec::new();
    for line in &lines[start..] {
        if let Ok(entry) = serde_json::from_str::<DiagnosticLogEntry>(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn synthetic_diagnostic_fixtures() -> Value {
    json!({
        "accounts": [
            {
                "label": "Synthetic Personal",
                "status": "connected",
                "unreadCount": 3
            }
        ],
        "syncEvents": [
            {
                "component": "sync_scheduler",
                "event": "sync_status",
                "state": "idle"
            },
            {
                "component": "smtp_send",
                "event": "provider_error",
                "state": "retryable"
            }
        ],
        "messages": [
            {
                "subject": "Synthetic fixture subject",
                "body": "Synthetic fixture body; no user mailbox content."
            }
        ]
    })
}

fn redact_fields(fields: BTreeMap<String, String>) -> BTreeMap<String, String> {
    fields
        .into_iter()
        .map(|(key, value)| {
            let redacted = if is_sensitive_field_key(&key) {
                "[redacted]".to_string()
            } else {
                redact_freeform_text(&value)
            };
            (sanitize_identifier(&key), redacted)
        })
        .collect()
}

fn is_sensitive_field_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "password",
        "secret",
        "token",
        "credential",
        "authorization",
        "auth_code",
        "refresh_token",
        "access_token",
        "client_secret",
        "subject",
        "body",
        "body_html",
        "body_text",
        "raw_mime",
        "message_body",
        "email",
        "address",
        "username",
        "sender",
        "recipient",
        "to",
        "cc",
        "bcc",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

fn redact_freeform_text(value: &str) -> String {
    let lowered = value.to_ascii_lowercase();
    if [
        "bearer ",
        "refresh_token",
        "access_token",
        "token",
        "client_secret",
        "secret",
        "authorization:",
        "password=",
        "password:",
        "subject=",
        "subject:",
        "body_html",
        "body_text",
        "raw_mime",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "[redacted]".to_string();
    }

    value
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_token(token: &str) -> String {
    let trimmed = token.trim_matches(|character: char| {
        matches!(
            character,
            ',' | ';' | ':' | '.' | ')' | '(' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\''
        )
    });
    if looks_like_email(trimmed) {
        return token.replace(trimmed, "[redacted-email]");
    }
    if looks_like_secret_token(trimmed) {
        return token.replace(trimmed, "[redacted-token]");
    }
    token.to_string()
}

fn looks_like_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && domain
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'))
}

fn looks_like_secret_token(value: &str) -> bool {
    if value.len() < 32 {
        return false;
    }
    let relevant = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .count();
    relevant >= value.len().saturating_sub(2)
}

fn sanitize_identifier(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/')
        })
        .take(96)
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn unix_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::{env, process};

    #[test]
    fn structured_logs_redact_secrets_addresses_subjects_and_bodies() {
        let app_data_dir = temp_app_data_dir("redaction");
        append_log_event(
            &app_data_dir,
            DiagnosticLogLevel::Error,
            "smtp_send",
            "provider_error",
            BTreeMap::from([
                ("email".to_string(), "ada@example.com".to_string()),
                (
                    "provider_error".to_string(),
                    "Connection failed for ada@example.com".to_string(),
                ),
                (
                    "reason".to_string(),
                    "Bearer abcdefghijklmnopqrstuvwxyz1234567890 for ada@example.com failed"
                        .to_string(),
                ),
                ("subject".to_string(), "Private launch".to_string()),
                ("body_html".to_string(), "<p>Private body</p>".to_string()),
            ]),
        )
        .unwrap();

        let content = fs::read_to_string(diagnostics_log_path(&app_data_dir)).unwrap();
        assert!(content.contains("\"component\":\"smtp_send\""));
        assert!(content.contains("\"event\":\"provider_error\""));
        assert!(content.contains("[redacted]"));
        assert!(content.contains("[redacted-email]"));
        assert!(!content.contains("ada@example.com"));
        assert!(!content.contains("Private launch"));
        assert!(!content.contains("Private body"));
        assert!(!content.contains("abcdefghijklmnopqrstuvwxyz"));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn export_bundle_excludes_private_mailbox_data_and_attachment_bytes() {
        let app_data_dir = temp_app_data_dir("export-private");
        let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
        seed_private_cache(&cache_path);
        append_log_event(
            &app_data_dir,
            DiagnosticLogLevel::Warn,
            "sync_scheduler",
            "sync_status",
            BTreeMap::from([
                ("account_id".to_string(), "acct_private".to_string()),
                ("state".to_string(), "error".to_string()),
                (
                    "provider_error".to_string(),
                    "Connection failed for ada@example.com".to_string(),
                ),
                (
                    "message".to_string(),
                    "Connection failed for ada@example.com with token secret-secret-secret-secret"
                        .to_string(),
                ),
            ]),
        )
        .unwrap();

        let destination = app_data_dir.join("diagnostics.json");
        let response = export_diagnostics_bundle_to_path(
            &app_data_dir,
            &cache_path,
            &destination,
            DiagnosticsExportRequest {
                user_requested: true,
                include_synthetic_fixtures: false,
            },
        )
        .unwrap();

        assert_eq!(response.destination_label, "diagnostics.json");
        assert_eq!(response.included_log_entries, 1);
        assert!(!response.included_synthetic_fixtures);

        let content = fs::read_to_string(destination).unwrap();
        assert!(content.contains("\"redacted\": true"));
        assert!(content.contains("\"messages\": 1"));
        assert!(content.contains("\"attachments\": 1"));
        assert!(content.contains("[redacted-email]"));
        assert!(!content.contains("ada@example.com"));
        assert!(!content.contains("Private acquisition"));
        assert!(!content.contains("Private body"));
        assert!(!content.contains("credential-private"));
        assert!(!content.contains("attachment secret bytes"));
        assert!(!content.contains("secret-secret"));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn export_requires_user_intent_and_does_not_overwrite_existing_files() {
        let app_data_dir = temp_app_data_dir("export-safety");
        let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
        let destination = app_data_dir.join("diagnostics.json");
        fs::write(&destination, "{}").unwrap();

        let missing_intent = export_diagnostics_bundle_to_path(
            &app_data_dir,
            &cache_path,
            &app_data_dir.join("missing-intent.json"),
            DiagnosticsExportRequest {
                user_requested: false,
                include_synthetic_fixtures: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            missing_intent.reason,
            "diagnostics export requires explicit user intent"
        );

        let overwrite = export_diagnostics_bundle_to_path(
            &app_data_dir,
            &cache_path,
            &destination,
            DiagnosticsExportRequest {
                user_requested: true,
                include_synthetic_fixtures: true,
            },
        )
        .unwrap_err();
        assert_eq!(
            overwrite.reason,
            "diagnostics export destination already exists"
        );

        let synthetic_destination = app_data_dir.join("synthetic.json");
        let response = export_diagnostics_bundle_to_path(
            &app_data_dir,
            &cache_path,
            &synthetic_destination,
            DiagnosticsExportRequest {
                user_requested: true,
                include_synthetic_fixtures: true,
            },
        )
        .unwrap();
        assert!(response.included_synthetic_fixtures);
        assert!(fs::read_to_string(synthetic_destination)
            .unwrap()
            .contains("Synthetic fixture subject"));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn logs_migration_sync_and_command_failure_events() {
        let app_data_dir = temp_app_data_dir("events");
        let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

        log_startup_migration_status(&cache_path).unwrap();
        log_sync_status(
            &app_data_dir,
            "acct_private",
            "error",
            Some("Provider rejected access_token for ada@example.com"),
            Some("retry_later"),
            Some("authentication_failed"),
        )
        .unwrap();
        log_command_failure(
            &app_data_dir,
            "send_composer_email",
            "SMTP error for subject: Private launch",
        )
        .unwrap();

        let content = fs::read_to_string(diagnostics_log_path(&app_data_dir)).unwrap();
        assert!(content.contains("migration_status"));
        assert!(content.contains("sync_status"));
        assert!(content.contains("retry_scheduled"));
        assert!(content.contains("command_failure"));
        assert!(!content.contains("ada@example.com"));
        assert!(!content.contains("Private launch"));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    fn seed_private_cache(cache_path: &Path) {
        let mut connection = Connection::open(cache_path).unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO accounts (
               id, provider_kind, display_email, username, auth_kind, credential_ref,
               imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security,
               auth_status
             ) VALUES (
               ?1, 'generic_imap_smtp', ?2, ?2, 'app_password', ?3,
               'imap.example.test', 993, 'tls', 'smtp.example.test', 587, 'starttls',
               'connected'
             )",
                params!["acct_private", "ada@example.com", "credential-private"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO folders (
               id, account_id, remote_id, name, display_name, kind
             ) VALUES (?1, ?2, ?3, 'INBOX', 'Inbox', 'inbox')",
                params!["folder_private", "acct_private", "INBOX"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (
               id, account_id, folder_id, subject, sender_email, body_text, body_html
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    "msg_private",
                    "acct_private",
                    "folder_private",
                    "Private acquisition",
                    "sender@example.com",
                    "Private body",
                    "<p>Private body</p>"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO attachments (
               id, account_id, folder_id, message_id, display_filename, canonical_mime,
               size_bytes, storage_relative_path, sha256
             ) VALUES (?1, ?2, ?3, ?4, 'secret.txt', 'text/plain', 23, ?5, ?6)",
                params![
                    "att_private",
                    "acct_private",
                    "folder_private",
                    "msg_private",
                    "attachments/acct_private/msg_private/secret.txt",
                    "a".repeat(64)
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO settings (key, value_type, value_json)
             VALUES ('account:acct_private:sync_worker_status', 'object', ?1)",
                [r#"{"state":"error","message":"Private failure"}"#],
            )
            .unwrap();
    }

    fn temp_app_data_dir(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "komorebi-diagnostics-{label}-{}-{}",
            process::id(),
            unix_now_millis()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
