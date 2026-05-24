use std::{error::Error, fmt, path::Path};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    account_management::{mail_cache_path, AccountCommandError},
    schema::{apply_migrations, SchemaMigrationError},
};

const UI_LANGUAGE_SETTING_KEY: &str = "ui:language";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiLanguagePreference {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "pt-PT")]
    PortuguesePortugal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferencesView {
    pub language: Option<UiLanguagePreference>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPreferencesUpdateRequest {
    pub language: Option<UiLanguagePreference>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferencesError {
    pub reason: String,
}

impl UiPreferencesError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for UiPreferencesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for UiPreferencesError {}

impl From<rusqlite::Error> for UiPreferencesError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SchemaMigrationError> for UiPreferencesError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<serde_json::Error> for UiPreferencesError {
    fn from(value: serde_json::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tauri::Error> for UiPreferencesError {
    fn from(value: tauri::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AccountCommandError> for UiPreferencesError {
    fn from(value: AccountCommandError) -> Self {
        Self::new(value.reason)
    }
}

#[tauri::command]
pub fn get_ui_preferences(app: tauri::AppHandle) -> Result<UiPreferencesView, UiPreferencesError> {
    let cache_path = mail_cache_path(&app)?;
    get_ui_preferences_from_path(&cache_path)
}

#[tauri::command]
pub fn update_ui_preferences(
    app: tauri::AppHandle,
    request: UiPreferencesUpdateRequest,
) -> Result<UiPreferencesView, UiPreferencesError> {
    let cache_path = mail_cache_path(&app)?;
    update_ui_preferences_from_path(&cache_path, request)
}

pub fn get_ui_preferences_from_path(
    cache_path: &Path,
) -> Result<UiPreferencesView, UiPreferencesError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    get_ui_preferences_from_connection(&connection)
}

pub fn update_ui_preferences_from_path(
    cache_path: &Path,
    request: UiPreferencesUpdateRequest,
) -> Result<UiPreferencesView, UiPreferencesError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    update_ui_preferences_from_connection(&connection, request)
}

pub fn get_ui_preferences_from_connection(
    connection: &Connection,
) -> Result<UiPreferencesView, UiPreferencesError> {
    Ok(UiPreferencesView {
        language: read_language_preference(connection)?,
    })
}

pub fn update_ui_preferences_from_connection(
    connection: &Connection,
    request: UiPreferencesUpdateRequest,
) -> Result<UiPreferencesView, UiPreferencesError> {
    match request.language {
        Some(language) => {
            let value_json = serde_json::to_string(&language)?;
            connection.execute(
                "INSERT INTO settings (key, value_json, value_type)
                 VALUES (?1, ?2, 'string')
                 ON CONFLICT(key) DO UPDATE SET
                   value_json = excluded.value_json,
                   value_type = excluded.value_type",
                params![UI_LANGUAGE_SETTING_KEY, value_json],
            )?;
        }
        None => {
            connection.execute(
                "DELETE FROM settings WHERE key = ?1",
                [UI_LANGUAGE_SETTING_KEY],
            )?;
        }
    }

    get_ui_preferences_from_connection(connection)
}

fn read_language_preference(
    connection: &Connection,
) -> Result<Option<UiLanguagePreference>, UiPreferencesError> {
    let value_json = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            [UI_LANGUAGE_SETTING_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    value_json
        .map(|json| {
            serde_json::from_str::<UiLanguagePreference>(&json)
                .map_err(|_| UiPreferencesError::new("stored language preference is invalid"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open sqlite");
        apply_migrations(&mut connection).expect("apply migrations");
        connection
    }

    #[test]
    fn language_preference_defaults_to_system_detection() {
        let connection = migrated_connection();

        let preferences =
            get_ui_preferences_from_connection(&connection).expect("read preferences");

        assert_eq!(preferences.language, None);
    }

    #[test]
    fn language_preference_persists_reviewed_languages() {
        let connection = migrated_connection();

        let saved = update_ui_preferences_from_connection(
            &connection,
            UiPreferencesUpdateRequest {
                language: Some(UiLanguagePreference::PortuguesePortugal),
            },
        )
        .expect("save language");
        assert_eq!(
            saved.language,
            Some(UiLanguagePreference::PortuguesePortugal)
        );

        let stored_json: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [UI_LANGUAGE_SETTING_KEY],
                |row| row.get(0),
            )
            .expect("stored language");
        assert_eq!(stored_json, "\"pt-PT\"");

        let saved = update_ui_preferences_from_connection(
            &connection,
            UiPreferencesUpdateRequest {
                language: Some(UiLanguagePreference::English),
            },
        )
        .expect("save english");
        assert_eq!(saved.language, Some(UiLanguagePreference::English));
    }

    #[test]
    fn null_language_preference_removes_override() {
        let connection = migrated_connection();
        update_ui_preferences_from_connection(
            &connection,
            UiPreferencesUpdateRequest {
                language: Some(UiLanguagePreference::English),
            },
        )
        .expect("save language");

        let reset = update_ui_preferences_from_connection(
            &connection,
            UiPreferencesUpdateRequest { language: None },
        )
        .expect("reset language");

        assert_eq!(reset.language, None);
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = ?1",
                [UI_LANGUAGE_SETTING_KEY],
                |row| row.get(0),
            )
            .expect("setting count");
        assert_eq!(count, 0);
    }

    #[test]
    fn invalid_stored_language_preference_is_rejected() {
        let connection = migrated_connection();
        connection
            .execute(
                "INSERT INTO settings (key, value_json, value_type)
                 VALUES (?1, ?2, 'string')",
                params![UI_LANGUAGE_SETTING_KEY, "\"fr-FR\""],
            )
            .expect("insert corrupt preference");

        let error = get_ui_preferences_from_connection(&connection)
            .expect_err("invalid language should fail");

        assert_eq!(error.reason, "stored language preference is invalid");
    }
}
