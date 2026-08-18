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
    if let Some(key) = api_key {
        if !key.trim().is_empty() {
            builder = builder.bearer_auth(key.trim());
        }
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
}
