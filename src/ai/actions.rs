//! Model-based action-item extraction.
//!
//! This replaces the UI's local heuristic with a single model call. The model
//! is asked to return a JSON array of action items; the response is parsed
//! robustly so that a malformed reply degrades gracefully to "no items" rather
//! than erroring. A "no action items" outcome is never an error.

use super::{Ai, client, prompts};
use anyhow::Result;
use serde::Deserialize;

/// A single action item extracted from an email.
///
/// `owner` and `due` are optional: the model is instructed to omit them when
/// the email does not state them rather than guessing.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ActionItem {
    pub task: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
}

/// Extract action items from an email using the configured AI model.
///
/// The model is prompted to return ONLY a JSON array of action items (or `[]`
/// when there are none). The output is parsed defensively: markdown fences are
/// stripped and the JSON array is located even if the model adds surrounding
/// prose. If parsing fails or the model returns an empty array, an empty
/// `Vec` is returned - never an error.
pub fn extract_action_items(
    ai: &Ai,
    sender: Option<&str>,
    subject: Option<&str>,
    body: &str,
) -> Result<Vec<ActionItem>> {
    ai.config().validate()?;
    let (system, user) = prompts::extract_action_items(sender, subject, body);
    let raw = client::complete(ai.base_url(), ai.model(), &system, &user, ai.api_key())?;
    Ok(parse_action_items(&raw))
}

/// Parse the model's JSON array output into action items.
///
/// Robust to:
/// - Markdown code fences (` ```json ... ``` `).
/// - Leading/trailing prose around the array.
/// - An empty or `[]` response.
///
/// Any unrecoverable parse failure yields an empty `Vec` (the caller treats
/// "no items" and "couldn't parse" identically).
fn parse_action_items(raw: &str) -> Vec<ActionItem> {
    let json = extract_json_array(raw);
    serde_json::from_str::<Vec<ActionItem>>(json).unwrap_or_default()
}

/// Locate the JSON array within a possibly-fenced, possibly-noisy model reply.
///
/// Strips a single pair of markdown fences if present, then trims to the first
/// `[` and the matching last `]`. Returns the original trimmed text when no
/// array can be located (so the JSON parser can produce its own error).
fn extract_json_array(raw: &str) -> &str {
    let mut s = raw.trim();

    // Strip one layer of markdown code fences if present.
    if let Some(rest) = s.strip_prefix("```") {
        // Drop an optional language tag on the opening fence (e.g. ```json).
        let after_tag = rest.trim_start_matches(['j', 's', 'o', 'n']).trim_start();
        if let Some(inner) = after_tag.strip_suffix("```") {
            s = inner.trim();
        } else if let Some(end) = rest.find("```") {
            // Opening fence with language tag, closing fence later.
            let inner = &rest[end + 3..];
            s = inner.trim();
        } else {
            // Only an opening fence; take the rest as-is.
            s = rest.trim();
        }
    }

    // Trim to the outermost JSON array if there is surrounding prose.
    if let Some(start) = s.find('[')
        && let Some(end) = s.rfind(']')
        && end >= start
    {
        return &s[start..=end];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiConfig;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve one canned OpenAI-compatible chat response, then shut down.
    fn serve_once(response: &'static str) -> (String, AiConfig) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                let body = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = stream.write_all(body.as_bytes());
                break;
            }
        });
        let base_url = format!("http://127.0.0.1:{port}/v1");
        let config = AiConfig {
            provider: "test".into(),
            base_url: base_url.clone(),
            model: "test-model".into(),
        };
        (base_url, config)
    }

    /// Build a canned OpenAI-compatible chat completion response whose first
    /// choice's `content` is `content`, then leak it to a `&'static str` so it
    /// can be handed to `serve_once`.
    fn serve_with_content(content: &str) -> (String, AiConfig) {
        let json = serde_json::json!({
            "choices": [ { "message": { "role": "assistant", "content": content } } ]
        })
        .to_string();
        serve_once(Box::leak(json.into_boxed_str()))
    }

    fn ai_from(config: AiConfig) -> Ai {
        Ai::from_config(config, None)
    }

    #[test]
    fn parses_canned_action_items() {
        let content = r#"[{"task":"Send the deck","owner":"Pat","due":"Friday"},{"task":"Book the room","owner":null,"due":null}]"#;
        let (_base, config) = serve_with_content(content);
        let ai = ai_from(config);
        let items = extract_action_items(
            &ai,
            Some("Pat"),
            Some("Kickoff"),
            "Please send the deck by Friday and book the room.",
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].task, "Send the deck");
        assert_eq!(items[0].owner.as_deref(), Some("Pat"));
        assert_eq!(items[0].due.as_deref(), Some("Friday"));
        assert_eq!(items[1].task, "Book the room");
        assert!(items[1].owner.is_none());
        assert!(items[1].due.is_none());
    }

    #[test]
    fn empty_array_yields_no_items() {
        let (_base, config) = serve_with_content("[]");
        let ai = ai_from(config);
        let items = extract_action_items(&ai, None, None, "Nothing to do here.").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn strips_markdown_fences() {
        let content = "```json\n[{\"task\":\"Review PR\",\"owner\":\"Sam\",\"due\":null}]\n```";
        let (_base, config) = serve_with_content(content);
        let ai = ai_from(config);
        let items = extract_action_items(&ai, None, None, "Please review the PR.").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].task, "Review PR");
        assert_eq!(items[0].owner.as_deref(), Some("Sam"));
        assert!(items[0].due.is_none());
    }

    #[test]
    fn malformed_output_yields_empty_vec_not_error() {
        let (_base, config) = serve_with_content("Sorry, I couldn't parse that.");
        let ai = ai_from(config);
        let items = extract_action_items(&ai, None, None, "body").unwrap();
        assert!(items.is_empty());
    }

    // ---- parse-only unit tests (no network) ----

    #[test]
    fn parse_plain_array() {
        let raw = r#"[{"task":"A","owner":null,"due":null}]"#;
        let items = parse_action_items(raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].task, "A");
    }

    #[test]
    fn parse_array_with_surrounding_prose() {
        let raw =
            "Here are the items:\n[{\"task\":\"A\",\"owner\":null,\"due\":null}]\nHope that helps!";
        let items = parse_action_items(raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].task, "A");
    }

    #[test]
    fn parse_empty_string_yields_empty() {
        assert!(parse_action_items("").is_empty());
        assert!(parse_action_items("not json at all").is_empty());
    }
}
