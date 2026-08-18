//! AI provider presets and configuration.
//!
//! Komorebi is AI-first and local-first: the default targets are self-hosted
//! OpenAI-compatible servers (Ollama, vLLM, Unsloth), with hosted OpenAI and
//! any generic OpenAI-compatible API supported through the same client.
//! API keys are stored in the OS credential store, never in the config or SQLite.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// A selectable AI provider. The ID is stable and used for config + secrets.
#[derive(Debug, Clone)]
pub struct AiProviderPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// OpenAI-compatible base URL, including the `/v1` prefix when applicable.
    pub base_url: &'static str,
    /// Default model name ("" means the user must supply the served model).
    pub default_model: &'static str,
    pub local: bool,
    pub api_key_required: bool,
    pub notes: &'static [&'static str],
}

/// All selectable AI providers, local-first.
pub fn presets() -> Vec<AiProviderPreset> {
    vec![
        AiProviderPreset {
            id: "ollama",
            label: "Ollama (local)",
            base_url: "http://127.0.0.1:11434/v1",
            default_model: "llama3.2",
            local: true,
            api_key_required: false,
            notes: &[
                "Runs entirely on your machine.",
                "Pull a model first: `ollama pull llama3.2`.",
            ],
        },
        AiProviderPreset {
            id: "vllm",
            label: "vLLM (local)",
            base_url: "http://127.0.0.1:8000/v1",
            default_model: "",
            local: true,
            api_key_required: false,
            notes: &[
                "Point this at your vLLM OpenAI-compatible endpoint.",
                "Set the model to the name you served.",
            ],
        },
        AiProviderPreset {
            id: "unsloth",
            label: "Unsloth (local)",
            base_url: "http://127.0.0.1:8000/v1",
            default_model: "",
            local: true,
            api_key_required: false,
            notes: &[
                "Unsloth exports a standard OpenAI-compatible server.",
                "Set the model to the name you loaded.",
            ],
        },
        AiProviderPreset {
            id: "openai",
            label: "OpenAI API",
            base_url: "https://api.openai.com/v1",
            default_model: "gpt-4o-mini",
            local: false,
            api_key_required: true,
            notes: &["Requires an API key stored in the OS credential store."],
        },
        AiProviderPreset {
            id: "custom",
            label: "OpenAI-compatible API (self-hosted)",
            base_url: "",
            default_model: "",
            local: false,
            api_key_required: false,
            notes: &[
                "Any OpenAI-compatible endpoint (LM Studio, llama.cpp, LiteLLM…).",
                "Enter the full base URL including `/v1`.",
            ],
        },
    ]
}

pub fn preset(id: &str) -> Option<AiProviderPreset> {
    presets().into_iter().find(|p| p.id == id)
}

/// Non-secret AI configuration. Persisted in SQLite settings as JSON.
/// The API key (when required) lives only in the OS credential store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
}

impl AiConfig {
    pub fn from_preset(preset: &AiProviderPreset) -> Self {
        Self {
            provider: preset.id.to_string(),
            base_url: preset.base_url.to_string(),
            model: preset.default_model.to_string(),
        }
    }

    /// Validate the config before any request. The API key is checked by the
    /// caller (it lives in the credential store, not here).
    pub fn validate(&self) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(anyhow!("AI base URL is required"));
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(anyhow!("AI base URL must be http:// or https://"));
        }
        if self.model.trim().is_empty() {
            return Err(anyhow!("AI model name is required"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_providers_come_first() {
        let list = presets();
        assert_eq!(list[0].id, "ollama");
        assert_eq!(list[1].id, "vllm");
        assert_eq!(list[2].id, "unsloth");
        assert!(list.iter().any(|p| p.id == "openai"));
        assert!(list.iter().any(|p| p.id == "custom"));
    }

    #[test]
    fn local_providers_need_no_key() {
        for p in presets().iter().filter(|p| p.local) {
            assert!(!p.api_key_required);
        }
        assert!(preset("openai").unwrap().api_key_required);
    }

    #[test]
    fn config_validation() {
        assert!(AiConfig::default().validate().is_err());
        let good = AiConfig {
            provider: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2".into(),
        };
        assert!(good.validate().is_ok());
        let no_model = AiConfig {
            provider: "vllm".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            model: "".into(),
        };
        assert!(no_model.validate().is_err());
        let bad_scheme = AiConfig {
            provider: "x".into(),
            base_url: "ftp://host/v1".into(),
            model: "m".into(),
        };
        assert!(bad_scheme.validate().is_err());
    }
}
