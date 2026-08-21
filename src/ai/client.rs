//! Minimal OpenAI-compatible chat completion client.
//!
//! One function (`complete`) talks to any server that implements the
//! `/v1/chat/completions` contract - Ollama, vLLM, Unsloth, OpenAI, and
//! self-hosted proxies. The client is blocking and intended to run on an OS
//! thread so it never stalls the UI thread.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResp,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResp {
    content: String,
}

/// Send a single-turn chat completion and return the assistant text.
///
/// `base_url` must already include the `/v1` prefix (e.g.
/// `http://127.0.0.1:11434/v1`). `api_key` is optional for local servers and
/// sent as a bearer token when present.
pub fn complete(
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let request = ChatRequest {
        model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: system,
            },
            ChatMessage {
                role: "user",
                content: user,
            },
        ],
        temperature: 0.3,
        stream: false,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("building AI HTTP client")?;

    let mut builder = client.post(&url).json(&request);
    if let Some(key) = api_key
        && !key.trim().is_empty()
    {
        builder = builder.bearer_auth(key.trim());
    }

    let resp = builder.send().context("AI request failed")?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("AI endpoint returned {status}: {}", truncate(&text, 400));
    }

    let parsed: ChatResponse =
        serde_json::from_str(&text).context("parsing AI chat completion response")?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("AI endpoint returned an empty completion"))?;
    Ok(content)
}

// ---------------------------------------------------------------------------
// Local-model health check (no chat request sent)
// ---------------------------------------------------------------------------

/// Outcome of a local-model health probe. A health check failure is a *report*,
/// not an error: `check_health` always returns `Ok`, and a failed probe is
/// expressed as `reachable: false`.
#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub reachable: bool,
    pub models: Vec<String>,
    pub source: String,
}

/// Probe an OpenAI-compatible endpoint without sending a chat request.
///
/// `base_url` follows the same convention as [`complete`] (including the `/v1`
/// prefix when applicable). The strategy is local-first:
///
/// 1. Try `GET {base_url}/models` - the OpenAI list-models endpoint, supported
///    by OpenAI, vLLM, LM Studio, and most compatible servers. Parse the
///    `data` array for each object's `id`.
/// 2. If that fails, try `GET {base_url-without-/v1}/api/tags` - the Ollama
///    native tags endpoint. Parse the `models` array for each object's `name`.
/// 3. If both fail, return `reachable: false`.
///
/// `api_key` is sent as a bearer token when present. Never returns `Err`; a
/// failure to reach either endpoint is reported as `reachable: false`.
pub fn check_health(base_url: &str, api_key: Option<&str>) -> Result<HealthReport> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building AI health-check client")?;

    // 1. OpenAI-compatible list-models endpoint.
    if let Ok(report) = probe_models(&client, base_url, api_key) {
        return Ok(report);
    }

    // 2. Ollama-native /api/tags endpoint (drop the /v1 suffix if present).
    let ollama_base = base_url.trim_end_matches('/').trim_end_matches("/v1");
    if let Ok(report) = probe_tags(&client, ollama_base, api_key) {
        return Ok(report);
    }

    // 3. Both failed: report unreachable rather than erroring.
    Ok(HealthReport {
        reachable: false,
        models: vec![],
        source: "none".to_string(),
    })
}

/// Parse the OpenAI list-models response: `{ "data": [ { "id": "..." } ] }`.
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

/// Parse the Ollama tags response: `{ "models": [ { "name": "..." } ] }`.
#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagEntry>,
}

#[derive(Debug, Deserialize)]
struct TagEntry {
    name: String,
}

/// Issue a GET to `{base_url}/models` and parse the OpenAI-style model list.
fn probe_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<HealthReport> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut builder = client.get(&url);
    if let Some(key) = api_key
        && !key.trim().is_empty()
    {
        builder = builder.bearer_auth(key.trim());
    }

    let resp = builder.send().context("GET /models failed")?;
    if !resp.status().is_success() {
        bail!("GET /models returned {}", resp.status());
    }
    let parsed: ModelsResponse = resp.json().context("parsing /models response")?;
    let models = parsed.data.into_iter().map(|m| m.id).collect();
    Ok(HealthReport {
        reachable: true,
        models,
        source: "models".to_string(),
    })
}

/// Issue a GET to `{base_url}/api/tags` and parse the Ollama tags list.
fn probe_tags(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<HealthReport> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let mut builder = client.get(&url);
    if let Some(key) = api_key
        && !key.trim().is_empty()
    {
        builder = builder.bearer_auth(key.trim());
    }

    let resp = builder.send().context("GET /api/tags failed")?;
    if !resp.status().is_success() {
        bail!("GET /api/tags returned {}", resp.status());
    }
    let parsed: TagsResponse = resp.json().context("parsing /api/tags response")?;
    let models = parsed.models.into_iter().map(|m| m.name).collect();
    Ok(HealthReport {
        reachable: true,
        models,
        source: "tags".to_string(),
    })
}

fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve one canned OpenAI-compatible response, then shut down.
    fn serve_once(response: &'static str) -> String {
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
        format!("http://127.0.0.1:{port}/v1")
    }

    #[test]
    fn parses_openai_compatible_response() {
        let json =
            r#"{"choices":[{"message":{"role":"assistant","content":"This is the summary."}}]}"#;
        let base = serve_once(json);
        let out = complete(&base, "test-model", "sys", "user", None).unwrap();
        assert_eq!(out, "This is the summary.");
    }

    #[test]
    fn errors_on_empty_choices() {
        let empty = r#"{"choices":[]}"#;
        let base = serve_once(empty);
        let err = complete(&base, "m", "s", "u", None).unwrap_err();
        assert!(err.to_string().contains("empty completion"));
    }

    /// Serve one canned response for any GET path, then shut down. Used for the
    /// health-check tests where the request target is `/models` or `/api/tags`.
    fn serve_once_get(response: &'static str, include_port: bool) -> String {
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
        if include_port {
            format!("http://127.0.0.1:{port}")
        } else {
            format!("http://127.0.0.1:{port}/v1")
        }
    }

    #[test]
    fn health_check_reads_openai_models_endpoint() {
        // OpenAI-style list-models: { "data": [ { "id": "..." } ] }
        let json = r#"{"data":[{"id":"gpt-4o-mini"},{"id":"gpt-4o"}]}"#;
        let base = serve_once_get(json, false);
        let report = check_health(&base, None).unwrap();
        assert!(report.reachable);
        assert_eq!(report.source, "models");
        assert_eq!(report.models, vec!["gpt-4o-mini", "gpt-4o"]);
    }

    #[test]
    fn health_check_reports_unreachable_when_both_endpoints_fail() {
        // Bind a port but never accept/serve - connections drop, so both
        // /models and /api/tags probes fail and we get a non-error report.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // Drop the listener immediately so accepts fail rather than hang.
        drop(listener);

        let base = format!("http://127.0.0.1:{port}/v1");
        let report = check_health(&base, None).unwrap();
        assert!(!report.reachable);
        assert_eq!(report.source, "none");
        assert!(report.models.is_empty());
    }
}
