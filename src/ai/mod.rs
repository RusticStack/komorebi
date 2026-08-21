//! AI-first features: OpenAI-compatible chat client with local (Ollama, vLLM,
//! Unsloth) and hosted API providers, plus prompt templates and persistence.
//!
//! Configuration (provider, base URL, model) is stored in SQLite settings;
//! API keys live only in the OS credential store (`crate::secrets`).

pub mod actions;
pub mod client;
pub mod prompts;
pub mod providers;

use crate::db::Database;
use crate::secrets;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use providers::AiConfig;

const CONFIG_KEY: &str = "ai.config";

/// Result of an email triage classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Work,
    Personal,
    Finance,
    Newsletters,
    Updates,
}

impl Category {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "work" => Some(Self::Work),
            "personal" => Some(Self::Personal),
            "finance" => Some(Self::Finance),
            "newsletters" | "newsletter" => Some(Self::Newsletters),
            "updates" | "update" => Some(Self::Updates),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Finance => "finance",
            Self::Newsletters => "newsletters",
            Self::Updates => "updates",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Work => "Work",
            Self::Personal => "Personal",
            Self::Finance => "Finance",
            Self::Newsletters => "Newsletters",
            Self::Updates => "Updates",
        }
    }
}

/// A runnable AI session bound to one provider configuration.
#[derive(Clone)]
pub struct Ai {
    config: AiConfig,
    api_key: Option<String>,
}

impl Ai {
    /// Load config from SQLite and the API key from the OS credential store.
    pub fn load(db: &Database) -> Result<Self> {
        let config = load_config(db)?;
        let api_key = resolve_api_key(db, &config.provider)?;
        Ok(Self { config, api_key })
    }

    /// Construct a session directly from a config and an optional API key.
    ///
    /// Primarily for tests that pin a fake base URL without touching SQLite or
    /// the OS credential store.
    pub fn from_config(config: AiConfig, api_key: Option<String>) -> Self {
        Self { config, api_key }
    }

    pub fn config(&self) -> &AiConfig {
        &self.config
    }

    /// Base URL for the configured provider (including the `/v1` prefix).
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Configured model name.
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// API key (when present), for bearer auth. May be `None` for local
    /// servers.
    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    /// Summarize an email into 2-3 sentences.
    pub fn summarize(
        &self,
        sender: Option<&str>,
        subject: Option<&str>,
        body: &str,
    ) -> Result<String> {
        self.config.validate()?;
        let (system, user) = prompts::summarize(sender, subject, body);
        client::complete(
            &self.config.base_url,
            &self.config.model,
            &system,
            &user,
            self.api_key.as_deref(),
        )
    }

    /// Summarize an entire conversation (already formatted as per-message
    /// blocks with sender + body).
    pub fn summarize_thread(&self, thread: &str) -> Result<String> {
        self.config.validate()?;
        let (system, user) = prompts::summarize_thread(thread);
        client::complete(
            &self.config.base_url,
            &self.config.model,
            &system,
            &user,
            self.api_key.as_deref(),
        )
    }

    /// Draft a reply to an email.
    pub fn draft_reply(
        &self,
        sender: Option<&str>,
        subject: Option<&str>,
        body: &str,
        tone: &str,
    ) -> Result<String> {
        self.config.validate()?;
        let (system, user) = prompts::draft_reply(sender, subject, body, tone);
        client::complete(
            &self.config.base_url,
            &self.config.model,
            &system,
            &user,
            self.api_key.as_deref(),
        )
    }

    /// Polish a user-written draft.
    pub fn polish(&self, draft: &str, tone: &str) -> Result<String> {
        self.config.validate()?;
        let (system, user) = prompts::polish(draft, tone);
        client::complete(
            &self.config.base_url,
            &self.config.model,
            &system,
            &user,
            self.api_key.as_deref(),
        )
    }

    /// Classify an email. Returns the raw lowercase category word from the
    /// model (use `Category::parse` to map it to a known category).
    pub fn triage(
        &self,
        sender: Option<&str>,
        subject: Option<&str>,
        body: &str,
    ) -> Result<String> {
        self.config.validate()?;
        let (system, user) = prompts::triage(sender, subject, body);
        let raw = client::complete(
            &self.config.base_url,
            &self.config.model,
            &system,
            &user,
            self.api_key.as_deref(),
        )?;
        Ok(raw.trim().to_ascii_lowercase())
    }
}

// ---------------------------------------------------------------------------
// Config + secret persistence
// ---------------------------------------------------------------------------

/// Load AI config from SQLite settings, defaulting to the Ollama preset.
pub fn load_config(db: &Database) -> Result<AiConfig> {
    match db.get_setting(CONFIG_KEY)? {
        Some(json) => serde_json::from_str(&json).with_context(|| format!("parsing {CONFIG_KEY}")),
        None => {
            let preset = providers::preset("ollama").expect("ollama preset must exist");
            Ok(AiConfig::from_preset(&preset))
        }
    }
}

pub fn save_config(db: &Database, config: &AiConfig) -> Result<()> {
    config.validate()?;
    let json = serde_json::to_string(config).context("serializing AI config")?;
    db.set_setting(CONFIG_KEY, &json)
}

/// Read the API key for a provider from the OS credential store.
pub fn resolve_api_key(db: &Database, provider: &str) -> Result<Option<String>> {
    // `db` is accepted for signature symmetry; keys live only in the OS store.
    let _ = db;
    secrets::get(&secrets::key::ai_api_key(provider))
}

pub fn set_api_key(db: &Database, provider: &str, key: &str) -> Result<()> {
    let _ = db;
    if key.trim().is_empty() {
        secrets::delete(&secrets::key::ai_api_key(provider))
    } else {
        secrets::set(&secrets::key::ai_api_key(provider), key.trim())
    }
}

pub fn clear_api_key(db: &Database, provider: &str) -> Result<()> {
    let _ = db;
    secrets::delete(&secrets::key::ai_api_key(provider))
}

// ---------------------------------------------------------------------------
// Persistence of AI results into the local cache
// ---------------------------------------------------------------------------

/// Record an AI action in the ledger and write the result back to a message.
pub fn record_and_apply(
    db: &Database,
    account_id: Option<&str>,
    message_id: Option<&str>,
    kind: &str,
    config: &AiConfig,
    output: &str,
) -> Result<()> {
    db.insert_ai_action(
        account_id,
        message_id,
        kind,
        Some(&config.provider),
        Some(&config.model),
        output,
    )?;
    Ok(())
}

/// Apply a triage category to a message.
pub fn apply_triage(db: &Database, message_id: &str, category: Category) -> Result<()> {
    db.set_ai_fields(message_id, Some(category.as_str()), None)
}

/// Apply a summary to a message.
pub fn apply_summary(db: &Database, message_id: &str, summary: &str) -> Result<()> {
    db.set_ai_fields(message_id, None, Some(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{AccountDraft, MessageDraft};

    #[test]
    fn category_parsing() {
        assert_eq!(Category::parse("Work"), Some(Category::Work));
        assert_eq!(Category::parse(" newsletter "), Some(Category::Newsletters));
        assert_eq!(Category::parse("UPDATE"), Some(Category::Updates));
        assert_eq!(Category::parse("shopping"), None);
    }

    #[test]
    fn config_defaults_to_ollama() {
        let db = Database::in_memory().unwrap();
        let config = load_config(&db).unwrap();
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.model, "llama3.2");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn config_roundtrips_through_settings() {
        let db = Database::in_memory().unwrap();
        let config = AiConfig {
            provider: "vllm".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            model: "Qwen2.5-7B-Instruct".into(),
        };
        save_config(&db, &config).unwrap();
        let loaded = load_config(&db).unwrap();
        assert_eq!(loaded.provider, "vllm");
        assert_eq!(loaded.model, "Qwen2.5-7B-Instruct");
    }

    #[test]
    fn triage_result_is_recorded_and_applied() {
        let db = Database::in_memory().unwrap();
        let account = db
            .insert_account(&AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let folder = db
            .upsert_folder(&account.id, "INBOX", None, Some("inbox"), None)
            .unwrap();
        let msg = db
            .insert_message(&MessageDraft {
                account_id: account.id.clone(),
                folder_id: folder.id.clone(),
                ..Default::default()
            })
            .unwrap();

        let config = AiConfig {
            provider: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2".into(),
        };
        record_and_apply(
            &db,
            Some(&account.id),
            Some(&msg),
            "triage",
            &config,
            "work",
        )
        .unwrap();
        apply_triage(&db, &msg, Category::Work).unwrap();

        let actions = db.list_ai_actions(&msg, 10).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "triage");
        assert_eq!(actions[0].provider.as_deref(), Some("ollama"));

        let detail = db.get_message_detail(&msg).unwrap().unwrap();
        assert_eq!(detail.ai_category.as_deref(), Some("work"));
    }
}
