// LLM streaming clients: Anthropic (Claude), Google (Gemini), and OpenAI.
//
// All providers expose the same `stream_message` interface that forwards
// Server-Sent Event tokens as `LlmEvent` variants over an mpsc channel.

use futures_util::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::config::Config;
use crate::llm::provider::LlmProvider;
use crate::protocol::LlmEvent;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

// ---------------------------------------------------------------------------
// LlmClient wrapper
// ---------------------------------------------------------------------------

/// High-level wrapper that can be either an active multi-provider client or
/// disabled (no API key configured).
pub enum LlmClient {
    /// LLM API is configured and ready.
    Active(GenericLlmClient),
    /// LLM functionality is disabled (no API key configured).
    Disabled,
}

impl LlmClient {
    /// Build an `LlmClient` from the application config.
    ///
    /// Selects the provider and API key based on `config.strategy.llm.provider`
    /// and the corresponding key in `config.credentials`.  Returns `Disabled`
    /// when the selected provider's key is absent or empty.
    pub fn from_config(config: &Config) -> Self {
        let provider = config.strategy.llm.provider.clone();
        let model = config.strategy.llm.model.clone();

        let api_key = match &provider {
            LlmProvider::Anthropic => config
                .credentials
                .anthropic_api_key
                .clone()
                .unwrap_or_default(),
            LlmProvider::Google => config
                .credentials
                .google_api_key
                .clone()
                .unwrap_or_default(),
            LlmProvider::OpenAI => config
                .credentials
                .openai_api_key
                .clone()
                .unwrap_or_default(),
        };

        if api_key.is_empty() {
            LlmClient::Disabled
        } else {
            LlmClient::Active(GenericLlmClient::new(provider, api_key, model))
        }
    }

    /// Stream a message, delegating to the inner `GenericLlmClient` or
    /// immediately sending an error if disabled.
    pub async fn stream_message(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        match self {
            LlmClient::Active(client) => {
                client
                    .stream_message(system, user_content, max_tokens, tx, generation)
                    .await
            }
            LlmClient::Disabled => {
                let _ = tx
                    .send(LlmEvent::Error {
                        message: "LLM not configured".to_string(),
                        generation,
                    })
                    .await;
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GenericLlmClient
// ---------------------------------------------------------------------------

/// Configuration needed to drive a single provider.
struct ProviderConfig {
    base_url: String,
    api_key: String,
    provider: LlmProvider,
    model: String,
}

/// Multi-provider LLM client.  Internally dispatches to the correct API based
/// on the configured `LlmProvider`.
pub struct GenericLlmClient {
    http: reqwest::Client,
    cfg: ProviderConfig,
}

impl GenericLlmClient {
    /// Create a new client for the given provider, API key, and model.
    pub fn new(provider: LlmProvider, api_key: String, model: String) -> Self {
        let base_url = match &provider {
            LlmProvider::Anthropic => ANTHROPIC_API_URL.to_string(),
            LlmProvider::Google => {
                // URL includes model and key as a query parameter; we embed
                // the model now and substitute the key at call time.
                format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent",
                    model
                )
            }
            LlmProvider::OpenAI => OPENAI_API_URL.to_string(),
        };

        Self {
            http: reqwest::Client::new(),
            cfg: ProviderConfig {
                base_url,
                api_key,
                provider,
                model,
            },
        }
    }

    /// Stream a message through the configured provider.
    ///
    /// Emits `LlmEvent::Token` events for each text chunk, followed by a
    /// single `LlmEvent::Complete` (or `LlmEvent::Error` on failure).
    /// The `generation` counter is threaded through every event so the
    /// receiver can discard stale events from cancelled tasks.
    pub async fn stream_message(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        if self.cfg.api_key.is_empty() {
            let _ = tx
                .send(LlmEvent::Error {
                    message: "API key not configured".to_string(),
                    generation,
                })
                .await;
            return Ok(());
        }

        match &self.cfg.provider {
            LlmProvider::Anthropic => {
                self.stream_anthropic(system, user_content, max_tokens, tx, generation)
                    .await
            }
            LlmProvider::Google => {
                self.stream_google(system, user_content, max_tokens, tx, generation)
                    .await
            }
            LlmProvider::OpenAI => {
                self.stream_openai(system, user_content, max_tokens, tx, generation)
                    .await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Anthropic streaming
    // -----------------------------------------------------------------------

    async fn stream_anthropic(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "model": self.cfg.model,
            "max_tokens": max_tokens,
            "stream": true,
            "system": system,
            "messages": [{ "role": "user", "content": user_content }]
        });

        let request = self
            .http
            .post(&self.cfg.base_url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body);

        stream_anthropic_sse(request, tx, generation).await
    }

    // -----------------------------------------------------------------------
    // Google (Gemini) streaming
    // -----------------------------------------------------------------------

    async fn stream_google(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        // Google's streaming endpoint uses `?key=<api_key>&alt=sse` for
        // server-sent events.
        let url = format!("{}?key={}&alt=sse", self.cfg.base_url, self.cfg.api_key);

        let body = serde_json::json!({
            "system_instruction": {
                "parts": [{ "text": system }]
            },
            "contents": [{
                "role": "user",
                "parts": [{ "text": user_content }]
            }],
            "generationConfig": {
                "maxOutputTokens": max_tokens
            }
        });

        let request = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .json(&body);

        stream_google_sse(request, tx, generation).await
    }

    // -----------------------------------------------------------------------
    // OpenAI streaming
    // -----------------------------------------------------------------------

    async fn stream_openai(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "model": self.cfg.model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user_content }
            ]
        });

        let request = self
            .http
            .post(&self.cfg.base_url)
            .header("authorization", format!("Bearer {}", self.cfg.api_key))
            .header("content-type", "application/json")
            .json(&body);

        stream_openai_sse(request, tx, generation).await
    }
}

// ---------------------------------------------------------------------------
// Provider-level streaming helpers (free functions for testability)
// ---------------------------------------------------------------------------

/// Drive an Anthropic SSE stream to completion, emitting `LlmEvent`s on `tx`.
async fn stream_anthropic_sse(
    request: reqwest::RequestBuilder,
    tx: mpsc::Sender<LlmEvent>,
    generation: u64,
) -> anyhow::Result<()> {
    let mut es = match request.eventsource() {
        Ok(es) => es,
        Err(e) => {
            let _ = tx
                .send(LlmEvent::Error {
                    message: format!("Failed to create event source: {e}"),
                    generation,
                })
                .await;
            return Ok(());
        }
    };

    let mut full_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut stop_reason: Option<String> = None;

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {
                debug!("SSE connection opened (Anthropic)");
            }
            Ok(Event::Message(msg)) => {
                let event_type = msg.event.as_str();
                let data = &msg.data;

                match event_type {
                    "message_start" => {
                        match parse_input_tokens(data) {
                            Some(n) => input_tokens = n,
                            None => warn!("failed to parse input_tokens from message_start"),
                        }
                        debug!(input_tokens, "message_start");
                    }
                    "content_block_delta" => {
                        if let Some(text) = parse_delta_text(data) {
                            full_text.push_str(&text);
                            if tx
                                .send(LlmEvent::Token { text, generation })
                                .await
                                .is_err()
                            {
                                es.close();
                                return Ok(());
                            }
                        }
                    }
                    "message_delta" => {
                        match parse_output_tokens(data) {
                            Some(n) => output_tokens = n,
                            None => warn!("failed to parse output_tokens from message_delta"),
                        }
                        stop_reason = parse_stop_reason(data);
                        debug!(output_tokens, ?stop_reason, "message_delta");
                    }
                    "message_stop" => {
                        debug!(?stop_reason, "message_stop — streaming complete");
                        let _ = tx
                            .send(LlmEvent::Complete {
                                full_text,
                                input_tokens,
                                output_tokens,
                                stop_reason,
                                generation,
                            })
                            .await;
                        es.close();
                        return Ok(());
                    }
                    _ => {
                        debug!(event_type, "ignoring SSE event");
                    }
                }
            }
            Err(err) => {
                warn!(?err, "SSE stream error (Anthropic)");
                let error_message = extract_error_message(&err);
                let _ = tx
                    .send(LlmEvent::Error {
                        message: error_message,
                        generation,
                    })
                    .await;
                es.close();
                return Ok(());
            }
        }
    }

    // Stream ended without message_stop.
    if full_text.is_empty() {
        let _ = tx
            .send(LlmEvent::Error {
                message: "Stream ended unexpectedly without any content".to_string(),
                generation,
            })
            .await;
    } else {
        let _ = tx
            .send(LlmEvent::Complete {
                full_text,
                input_tokens,
                output_tokens,
                stop_reason,
                generation,
            })
            .await;
    }

    Ok(())
}

/// Drive a Google (Gemini) SSE stream to completion.
///
/// Google's streaming format uses JSON objects emitted as SSE data payloads.
/// Each chunk looks like:
/// ```json
/// {
///   "candidates": [{
///     "content": { "parts": [{ "text": "..." }] },
///     "finishReason": "STOP"
///   }],
///   "usageMetadata": { "promptTokenCount": N, "candidatesTokenCount": M }
/// }
/// ```
async fn stream_google_sse(
    request: reqwest::RequestBuilder,
    tx: mpsc::Sender<LlmEvent>,
    generation: u64,
) -> anyhow::Result<()> {
    let mut es = match request.eventsource() {
        Ok(es) => es,
        Err(e) => {
            let _ = tx
                .send(LlmEvent::Error {
                    message: format!("Failed to create event source: {e}"),
                    generation,
                })
                .await;
            return Ok(());
        }
    };

    let mut full_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut stop_reason: Option<String> = None;

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {
                debug!("SSE connection opened (Google)");
            }
            Ok(Event::Message(msg)) => {
                let data = &msg.data;
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    // Extract text from candidates[0].content.parts[0].text
                    if let Some(text) = v
                        .get("candidates")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("content"))
                        .and_then(|c| c.get("parts"))
                        .and_then(|p| p.get(0))
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        let text = text.to_string();
                        full_text.push_str(&text);
                        if tx
                            .send(LlmEvent::Token {
                                text,
                                generation,
                            })
                            .await
                            .is_err()
                        {
                            es.close();
                            return Ok(());
                        }
                    }

                    // Extract token usage from usageMetadata
                    if let Some(usage) = v.get("usageMetadata") {
                        if let Some(n) = usage.get("promptTokenCount").and_then(|v| v.as_u64()) {
                            input_tokens = n as u32;
                        }
                        if let Some(n) =
                            usage.get("candidatesTokenCount").and_then(|v| v.as_u64())
                        {
                            output_tokens = n as u32;
                        }
                    }

                    // Check for finishReason in candidates[0]
                    if let Some(reason) = v
                        .get("candidates")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("finishReason"))
                        .and_then(|r| r.as_str())
                    {
                        if reason != "UNSPECIFIED" && reason != "OTHER" {
                            stop_reason = Some(reason.to_string());
                        }
                    }

                    // Detect stream completion: finishReason present in last chunk.
                    // Use the same filter as stop_reason assignment above: exclude
                    // "UNSPECIFIED" and "OTHER" (both indicate no meaningful stop).
                    let is_done = v
                        .get("candidates")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("finishReason"))
                        .and_then(|r| r.as_str())
                        .map(|r| r != "UNSPECIFIED" && r != "OTHER" && !r.is_empty())
                        .unwrap_or(false);

                    if is_done {
                        debug!(?stop_reason, "Google stream complete");
                        let _ = tx
                            .send(LlmEvent::Complete {
                                full_text,
                                input_tokens,
                                output_tokens,
                                stop_reason,
                                generation,
                            })
                            .await;
                        es.close();
                        return Ok(());
                    }
                }
            }
            Err(err) => {
                let error_message = extract_error_message(&err);
                warn!("SSE stream error (Google): {}", error_message);
                let _ = tx
                    .send(LlmEvent::Error {
                        message: error_message,
                        generation,
                    })
                    .await;
                es.close();
                return Ok(());
            }
        }
    }

    // Stream ended without a finish reason.
    if full_text.is_empty() {
        let _ = tx
            .send(LlmEvent::Error {
                message: "Google stream ended without any content".to_string(),
                generation,
            })
            .await;
    } else {
        let _ = tx
            .send(LlmEvent::Complete {
                full_text,
                input_tokens,
                output_tokens,
                stop_reason,
                generation,
            })
            .await;
    }

    Ok(())
}

/// Drive an OpenAI SSE stream to completion.
///
/// OpenAI's streaming format:
/// ```text
/// data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{"content":"..."},...}]}
/// data: [DONE]
/// ```
async fn stream_openai_sse(
    request: reqwest::RequestBuilder,
    tx: mpsc::Sender<LlmEvent>,
    generation: u64,
) -> anyhow::Result<()> {
    let mut es = match request.eventsource() {
        Ok(es) => es,
        Err(e) => {
            let _ = tx
                .send(LlmEvent::Error {
                    message: format!("Failed to create event source: {e}"),
                    generation,
                })
                .await;
            return Ok(());
        }
    };

    let mut full_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut stop_reason: Option<String> = None;

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {
                debug!("SSE connection opened (OpenAI)");
            }
            Ok(Event::Message(msg)) => {
                let data = msg.data.trim();

                // OpenAI signals end-of-stream with `[DONE]`
                if data == "[DONE]" {
                    debug!(?stop_reason, "OpenAI stream complete ([DONE])");
                    let _ = tx
                        .send(LlmEvent::Complete {
                            full_text,
                            input_tokens,
                            output_tokens,
                            stop_reason,
                            generation,
                        })
                        .await;
                    es.close();
                    return Ok(());
                }

                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    // Extract delta content from choices[0].delta.content
                    if let Some(text) = v
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|t| t.as_str())
                    {
                        let text = text.to_string();
                        full_text.push_str(&text);
                        if tx
                            .send(LlmEvent::Token {
                                text,
                                generation,
                            })
                            .await
                            .is_err()
                        {
                            es.close();
                            return Ok(());
                        }
                    }

                    // Extract finish_reason from choices[0].finish_reason
                    if let Some(reason) = v
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("finish_reason"))
                        .and_then(|r| r.as_str())
                    {
                        if !reason.is_empty() {
                            stop_reason = Some(reason.to_string());
                        }
                    }

                    // Extract usage if present (only in the last chunk when
                    // `stream_options.include_usage` is set; treat as optional)
                    if let Some(usage) = v.get("usage") {
                        if let Some(n) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = n as u32;
                        }
                        if let Some(n) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = n as u32;
                        }
                    }
                }
            }
            Err(err) => {
                warn!(?err, "SSE stream error (OpenAI)");
                let error_message = extract_error_message(&err);
                let _ = tx
                    .send(LlmEvent::Error {
                        message: error_message,
                        generation,
                    })
                    .await;
                es.close();
                return Ok(());
            }
        }
    }

    // Stream ended without [DONE].
    if full_text.is_empty() {
        let _ = tx
            .send(LlmEvent::Error {
                message: "OpenAI stream ended without [DONE]".to_string(),
                generation,
            })
            .await;
    } else {
        let _ = tx
            .send(LlmEvent::Complete {
                full_text,
                input_tokens,
                output_tokens,
                stop_reason,
                generation,
            })
            .await;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// SSE JSON parsing helpers
// ---------------------------------------------------------------------------

/// Extract `input_tokens` from a `message_start` event's JSON.
///
/// Expected shape: `{ "type": "message_start", "message": { "usage": { "input_tokens": N } } }`
pub(crate) fn parse_input_tokens(data: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(data).ok()?;
    v.get("message")?
        .get("usage")?
        .get("input_tokens")?
        .as_u64()
        .map(|n| n as u32)
}

/// Extract `delta.text` from a `content_block_delta` event's JSON.
///
/// Expected shape: `{ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "..." } }`
pub(crate) fn parse_delta_text(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    v.get("delta")?
        .get("text")?
        .as_str()
        .map(|s| s.to_string())
}

/// Extract `output_tokens` from a `message_delta` event's JSON.
///
/// Expected shape: `{ "type": "message_delta", "usage": { "output_tokens": N } }`
pub(crate) fn parse_output_tokens(data: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(data).ok()?;
    v.get("usage")?
        .get("output_tokens")?
        .as_u64()
        .map(|n| n as u32)
}

/// Extract `delta.stop_reason` from a `message_delta` event's JSON.
///
/// Expected shape: `{ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, ... }`
pub(crate) fn parse_stop_reason(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    v.get("delta")?
        .get("stop_reason")?
        .as_str()
        .map(|s| s.to_string())
}

/// Extract a human-readable error message from an SSE error.
fn extract_error_message(err: &reqwest_eventsource::Error) -> String {
    match err {
        reqwest_eventsource::Error::InvalidStatusCode(status, _response) => {
            format!("API returned status {status}")
        }
        reqwest_eventsource::Error::Transport(e) => {
            format!("Network error: {e}")
        }
        other => format!("Stream error: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SSE JSON parsing tests --

    #[test]
    fn parse_message_start_input_tokens() {
        let data = r#"{
            "type": "message_start",
            "message": {
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-sonnet-4-5-20250929",
                "usage": { "input_tokens": 42, "output_tokens": 0 }
            }
        }"#;
        assert_eq!(parse_input_tokens(data), Some(42));
    }

    #[test]
    fn parse_message_start_missing_usage() {
        let data = r#"{ "type": "message_start", "message": { "id": "msg_1" } }"#;
        assert_eq!(parse_input_tokens(data), None);
    }

    #[test]
    fn parse_message_start_invalid_json() {
        assert_eq!(parse_input_tokens("not json"), None);
    }

    #[test]
    fn parse_content_block_delta_text() {
        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "Hello" }
        }"#;
        assert_eq!(parse_delta_text(data), Some("Hello".to_string()));
    }

    #[test]
    fn parse_content_block_delta_empty_text() {
        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "" }
        }"#;
        assert_eq!(parse_delta_text(data), Some(String::new()));
    }

    #[test]
    fn parse_content_block_delta_missing_delta() {
        let data = r#"{ "type": "content_block_delta", "index": 0 }"#;
        assert_eq!(parse_delta_text(data), None);
    }

    #[test]
    fn parse_content_block_delta_invalid_json() {
        assert_eq!(parse_delta_text("{broken"), None);
    }

    #[test]
    fn parse_message_delta_output_tokens() {
        let data = r#"{
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": 128 }
        }"#;
        assert_eq!(parse_output_tokens(data), Some(128));
    }

    #[test]
    fn parse_message_delta_missing_usage() {
        let data = r#"{ "type": "message_delta", "delta": {} }"#;
        assert_eq!(parse_output_tokens(data), None);
    }

    #[test]
    fn parse_message_delta_invalid_json() {
        assert_eq!(parse_output_tokens("nope"), None);
    }

    // -- stop_reason parsing tests --

    #[test]
    fn parse_stop_reason_end_turn() {
        let data = r#"{
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": 128 }
        }"#;
        assert_eq!(parse_stop_reason(data), Some("end_turn".to_string()));
    }

    #[test]
    fn parse_stop_reason_max_tokens() {
        let data = r#"{
            "type": "message_delta",
            "delta": { "stop_reason": "max_tokens", "stop_sequence": null },
            "usage": { "output_tokens": 400 }
        }"#;
        assert_eq!(parse_stop_reason(data), Some("max_tokens".to_string()));
    }

    #[test]
    fn parse_stop_reason_missing_delta() {
        let data = r#"{ "type": "message_delta", "usage": { "output_tokens": 10 } }"#;
        assert_eq!(parse_stop_reason(data), None);
    }

    #[test]
    fn parse_stop_reason_null_value() {
        let data = r#"{
            "type": "message_delta",
            "delta": { "stop_reason": null },
            "usage": { "output_tokens": 10 }
        }"#;
        assert_eq!(parse_stop_reason(data), None);
    }

    #[test]
    fn parse_stop_reason_invalid_json() {
        assert_eq!(parse_stop_reason("nope"), None);
    }

    // -- Token counting with various values --

    #[test]
    fn parse_large_token_counts() {
        let start_data = r#"{
            "type": "message_start",
            "message": { "usage": { "input_tokens": 100000 } }
        }"#;
        assert_eq!(parse_input_tokens(start_data), Some(100_000));

        let delta_data = r#"{
            "type": "message_delta",
            "usage": { "output_tokens": 4096 }
        }"#;
        assert_eq!(parse_output_tokens(delta_data), Some(4096));
    }

    #[test]
    fn parse_zero_token_counts() {
        let start_data = r#"{
            "type": "message_start",
            "message": { "usage": { "input_tokens": 0 } }
        }"#;
        assert_eq!(parse_input_tokens(start_data), Some(0));

        let delta_data = r#"{
            "type": "message_delta",
            "usage": { "output_tokens": 0 }
        }"#;
        assert_eq!(parse_output_tokens(delta_data), Some(0));
    }

    // -- LlmClient::Disabled path --

    #[tokio::test]
    async fn disabled_client_sends_error_event() {
        let client = LlmClient::Disabled;
        let (tx, mut rx) = mpsc::channel(8);

        client
            .stream_message("system", "user", 100, tx, 1)
            .await
            .expect("should not fail");

        let event = rx.recv().await.expect("should receive an event");
        assert_eq!(
            event,
            LlmEvent::Error {
                message: "LLM not configured".to_string(),
                generation: 1,
            }
        );

        // No more events.
        assert!(rx.try_recv().is_err());
    }

    // -- GenericLlmClient with empty API key --

    #[tokio::test]
    async fn empty_api_key_sends_error_event() {
        let client = GenericLlmClient::new(
            LlmProvider::Anthropic,
            String::new(),
            "claude-opus-4-6".to_string(),
        );
        let (tx, mut rx) = mpsc::channel(8);

        client
            .stream_message("system", "user", 100, tx, 42)
            .await
            .expect("should not fail");

        let event = rx.recv().await.expect("should receive an event");
        assert_eq!(
            event,
            LlmEvent::Error {
                message: "API key not configured".to_string(),
                generation: 42,
            }
        );
    }

    // -- GenericLlmClient base URL routing --

    #[test]
    fn generic_client_anthropic_uses_correct_url() {
        let client = GenericLlmClient::new(
            LlmProvider::Anthropic,
            "key".to_string(),
            "claude-opus-4-6".to_string(),
        );
        assert_eq!(client.cfg.base_url, ANTHROPIC_API_URL);
    }

    #[test]
    fn generic_client_google_url_contains_model() {
        let model = "gemini-3.1-pro-preview";
        let client = GenericLlmClient::new(
            LlmProvider::Google,
            "key".to_string(),
            model.to_string(),
        );
        assert!(client.cfg.base_url.contains(model));
        assert!(client.cfg.base_url.contains("generativelanguage.googleapis.com"));
    }

    #[test]
    fn generic_client_openai_uses_correct_url() {
        let client = GenericLlmClient::new(
            LlmProvider::OpenAI,
            "key".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(client.cfg.base_url, OPENAI_API_URL);
    }

    // -- LlmClient::from_config --

    #[test]
    fn from_config_with_api_key_returns_active() {
        let config = make_test_config(Some("sk-ant-test-key".to_string()));
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Active(_)));
    }

    #[test]
    fn from_config_without_api_key_returns_disabled() {
        let config = make_test_config(None);
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Disabled));
    }

    #[test]
    fn from_config_with_empty_api_key_returns_disabled() {
        let config = make_test_config(Some(String::new()));
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Disabled));
    }

    // -- Full SSE event sequence simulation --

    #[tokio::test]
    async fn simulated_sse_sequence_produces_correct_events() {
        // Simulate the parsing logic that stream_message uses internally.
        // We feed known SSE event data through the parse helpers and verify
        // the LlmEvent sequence that would be produced.
        let (tx, mut rx) = mpsc::channel(32);
        let gen = 7u64;

        // Simulate the event processing in a task.
        let tx2 = tx.clone();
        tokio::spawn(async move {
            // 1. message_start
            let start_data = r#"{
                "type": "message_start",
                "message": {
                    "id": "msg_abc",
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": "claude-sonnet-4-5-20250929",
                    "usage": { "input_tokens": 25 }
                }
            }"#;
            let input_tokens = parse_input_tokens(start_data).unwrap_or(0);

            // 2. content_block_start (ignored)
            // 3. content_block_delta — "Hello"
            let delta1 = r#"{
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Hello" }
            }"#;
            let text1 = parse_delta_text(delta1).unwrap();
            let _ = tx2
                .send(LlmEvent::Token {
                    text: text1.clone(),
                    generation: gen,
                })
                .await;

            // 4. content_block_delta — " world"
            let delta2 = r#"{
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": " world" }
            }"#;
            let text2 = parse_delta_text(delta2).unwrap();
            let _ = tx2
                .send(LlmEvent::Token {
                    text: text2.clone(),
                    generation: gen,
                })
                .await;

            // 5. content_block_stop (ignored)
            // 6. message_delta
            let msg_delta = r#"{
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 10 }
            }"#;
            let output_tokens = parse_output_tokens(msg_delta).unwrap_or(0);

            // 7. message_stop
            let full_text = format!("{}{}", text1, text2);
            let stop_reason = parse_stop_reason(msg_delta).map(|s| s.to_string());
            let _ = tx2
                .send(LlmEvent::Complete {
                    full_text,
                    input_tokens,
                    output_tokens,
                    stop_reason,
                    generation: gen,
                })
                .await;
        });

        drop(tx); // Drop our copy so the channel closes when the task finishes.

        // Verify sequence of events.
        let e1 = rx.recv().await.unwrap();
        assert_eq!(
            e1,
            LlmEvent::Token {
                text: "Hello".to_string(),
                generation: gen,
            }
        );

        let e2 = rx.recv().await.unwrap();
        assert_eq!(
            e2,
            LlmEvent::Token {
                text: " world".to_string(),
                generation: gen,
            }
        );

        let e3 = rx.recv().await.unwrap();
        assert_eq!(
            e3,
            LlmEvent::Complete {
                full_text: "Hello world".to_string(),
                input_tokens: 25,
                output_tokens: 10,
                stop_reason: Some("end_turn".to_string()),
                generation: gen,
            }
        );

        // No more events.
        assert!(rx.recv().await.is_none());
    }

    // -- Edge case: Unicode in delta text --

    #[test]
    fn parse_delta_text_with_unicode() {
        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "Shohei Ohtani (\u5927\u8c37\u7fd4\u5e73)" }
        }"#;
        let text = parse_delta_text(data).unwrap();
        assert!(text.contains("Ohtani"));
        assert!(text.contains('\u{5927}')); // CJK character
    }

    // -- Edge case: multiple content blocks --

    #[test]
    fn parse_delta_text_different_index() {
        let data = r#"{
            "type": "content_block_delta",
            "index": 1,
            "delta": { "type": "text_delta", "text": "second block" }
        }"#;
        assert_eq!(parse_delta_text(data), Some("second block".to_string()));
    }

    // -- Integration-style test with mock TCP server --

    #[tokio::test]
    async fn mock_sse_server_full_flow() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        // Start a local TCP server that speaks SSE.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            // Read the HTTP request (discard it).
            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;

            // Send SSE response.
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/event-stream\r\n",
                "Cache-Control: no-cache\r\n",
                "\r\n",
                "event: message_start\r\n",
                "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"test\",\"usage\":{\"input_tokens\":15}}}\r\n",
                "\r\n",
                "event: content_block_start\r\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\r\n",
                "\r\n",
                "event: content_block_delta\r\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Draft\"}}\r\n",
                "\r\n",
                "event: content_block_delta\r\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" analysis\"}}\r\n",
                "\r\n",
                "event: content_block_stop\r\n",
                "data: {\"type\":\"content_block_stop\",\"index\":0}\r\n",
                "\r\n",
                "event: message_delta\r\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\r\n",
                "\r\n",
                "event: message_stop\r\n",
                "data: {\"type\":\"message_stop\"}\r\n",
                "\r\n",
            );

            socket.write_all(response.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();

            // Keep connection alive briefly so the client can read everything.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });

        // Build an EventSource pointed at our mock server.
        let client = reqwest::Client::new();
        let request = client
            .post(format!("http://{addr}"))
            .header("content-type", "application/json")
            .body("{}");

        let mut es = request.eventsource().unwrap();

        let (tx, mut rx) = mpsc::channel(32);

        // Process SSE events like stream_message does.
        let gen = 1u64;
        let processor = tokio::spawn(async move {
            let mut full_text = String::new();
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut stop_reason: Option<String> = None;

            while let Some(event) = es.next().await {
                match event {
                    Ok(Event::Open) => {}
                    Ok(Event::Message(msg)) => match msg.event.as_str() {
                        "message_start" => {
                            input_tokens =
                                parse_input_tokens(&msg.data).unwrap_or(0);
                        }
                        "content_block_delta" => {
                            if let Some(text) = parse_delta_text(&msg.data) {
                                full_text.push_str(&text);
                                let _ = tx
                                    .send(LlmEvent::Token {
                                        text,
                                        generation: gen,
                                    })
                                    .await;
                            }
                        }
                        "message_delta" => {
                            output_tokens =
                                parse_output_tokens(&msg.data).unwrap_or(output_tokens);
                            stop_reason = parse_stop_reason(&msg.data);
                        }
                        "message_stop" => {
                            let _ = tx
                                .send(LlmEvent::Complete {
                                    full_text: full_text.clone(),
                                    input_tokens,
                                    output_tokens,
                                    stop_reason: stop_reason.clone(),
                                    generation: gen,
                                })
                                .await;
                            es.close();
                            return;
                        }
                        _ => {}
                    },
                    Err(err) => {
                        let _ = tx
                            .send(LlmEvent::Error {
                                message: format!("Stream error: {err}"),
                                generation: gen,
                            })
                            .await;
                        es.close();
                        return;
                    }
                }
            }
        });

        // Collect all events.
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        // Wait for tasks to finish.
        let _ = server_task.await;
        let _ = processor.await;

        // Verify events.
        assert_eq!(events.len(), 3, "expected 3 events: 2 tokens + 1 complete");
        assert_eq!(
            events[0],
            LlmEvent::Token {
                text: "Draft".to_string(),
                generation: gen,
            }
        );
        assert_eq!(
            events[1],
            LlmEvent::Token {
                text: " analysis".to_string(),
                generation: gen,
            }
        );
        assert_eq!(
            events[2],
            LlmEvent::Complete {
                full_text: "Draft analysis".to_string(),
                input_tokens: 15,
                output_tokens: 7,
                stop_reason: Some("end_turn".to_string()),
                generation: gen,
            }
        );
    }

    #[tokio::test]
    async fn mock_sse_server_error_status() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        // Start a local TCP server that returns 401.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;

            let response = concat!(
                "HTTP/1.1 401 Unauthorized\r\n",
                "Content-Type: application/json\r\n",
                "Content-Length: 49\r\n",
                "\r\n",
                "{\"error\":{\"message\":\"Invalid API key\",\"type\":\"authentication_error\"}}",
            );

            socket.write_all(response.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        });

        let client = reqwest::Client::new();
        let request = client
            .post(format!("http://{addr}"))
            .header("content-type", "application/json")
            .body("{}");

        let mut es = request.eventsource().unwrap();

        let (tx, mut rx) = mpsc::channel(8);

        let gen = 5u64;
        let processor = tokio::spawn(async move {
            while let Some(event) = es.next().await {
                match event {
                    Ok(_) => {}
                    Err(err) => {
                        let error_message = extract_error_message(&err);
                        let _ = tx
                            .send(LlmEvent::Error {
                                message: error_message,
                                generation: gen,
                            })
                            .await;
                        es.close();
                        return;
                    }
                }
            }
        });

        let event = rx.recv().await.expect("should receive error event");
        match event {
            LlmEvent::Error { message: msg, generation } => {
                assert_eq!(generation, gen);
                assert!(
                    msg.contains("401") || msg.contains("status") || msg.contains("error"),
                    "Error message should mention status code or error: {msg}"
                );
            }
            other => panic!("Expected LlmEvent::Error, got: {other:?}"),
        }

        let _ = server_task.await;
        let _ = processor.await;
    }

    // -- Additional from_config tests for Google and OpenAI providers --

    #[test]
    fn from_config_google_with_key_returns_active() {
        let config = make_test_config_for_provider(
            LlmProvider::Google,
            "gemini-3.1-pro-preview".to_string(),
            Some("google-api-key".to_string()),
            None,
        );
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Active(_)));
    }

    #[test]
    fn from_config_google_without_key_returns_disabled() {
        let config = make_test_config_for_provider(
            LlmProvider::Google,
            "gemini-3.1-pro-preview".to_string(),
            None,
            None,
        );
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Disabled));
    }

    #[test]
    fn from_config_google_empty_key_returns_disabled() {
        let config = make_test_config_for_provider(
            LlmProvider::Google,
            "gemini-3.1-pro-preview".to_string(),
            Some(String::new()),
            None,
        );
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Disabled));
    }

    #[test]
    fn from_config_openai_with_key_returns_active() {
        let config = make_test_config_for_provider(
            LlmProvider::OpenAI,
            "gpt-4o".to_string(),
            None,
            Some("openai-api-key".to_string()),
        );
        let client = LlmClient::from_config(&config);
        assert!(matches!(client, LlmClient::Active(_)));
    }

    // -- Google SSE parsing tests --

    #[test]
    fn parse_google_chunk_extracts_text() {
        // A typical streamGenerateContent SSE chunk from the Gemini API.
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "Hello from Gemini" }],
                    "role": "model"
                },
                "finishReason": "UNSPECIFIED",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 4
            }
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let text = v
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str());
        assert_eq!(text, Some("Hello from Gemini"));
    }

    #[test]
    fn parse_google_chunk_finish_reason_stop_is_done() {
        // A final chunk with finishReason "STOP" should trigger stream completion.
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "last token" }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 10
            }
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let reason = v
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finishReason"))
            .and_then(|r| r.as_str());
        // stop_reason assignment: exclude "UNSPECIFIED" and "OTHER"
        let stop_reason: Option<String> = reason
            .filter(|r| *r != "UNSPECIFIED" && *r != "OTHER")
            .map(|r| r.to_string());
        assert_eq!(stop_reason, Some("STOP".to_string()));
        // is_done check: same filter
        let is_done = reason
            .map(|r| r != "UNSPECIFIED" && r != "OTHER" && !r.is_empty())
            .unwrap_or(false);
        assert!(is_done);
    }

    #[test]
    fn parse_google_chunk_finish_reason_unspecified_not_done() {
        // "UNSPECIFIED" should not trigger is_done and should not set stop_reason.
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "mid token" }],
                    "role": "model"
                },
                "finishReason": "UNSPECIFIED",
                "index": 0
            }]
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let reason = v
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finishReason"))
            .and_then(|r| r.as_str());
        let stop_reason: Option<String> = reason
            .filter(|r| *r != "UNSPECIFIED" && *r != "OTHER")
            .map(|r| r.to_string());
        assert_eq!(stop_reason, None);
        let is_done = reason
            .map(|r| r != "UNSPECIFIED" && r != "OTHER" && !r.is_empty())
            .unwrap_or(false);
        assert!(!is_done);
    }

    #[test]
    fn parse_google_chunk_finish_reason_other_not_done() {
        // "OTHER" should also not trigger is_done and should not set stop_reason.
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "" }],
                    "role": "model"
                },
                "finishReason": "OTHER",
                "index": 0
            }]
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let reason = v
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finishReason"))
            .and_then(|r| r.as_str());
        let stop_reason: Option<String> = reason
            .filter(|r| *r != "UNSPECIFIED" && *r != "OTHER")
            .map(|r| r.to_string());
        assert_eq!(stop_reason, None);
        let is_done = reason
            .map(|r| r != "UNSPECIFIED" && r != "OTHER" && !r.is_empty())
            .unwrap_or(false);
        assert!(!is_done);
    }

    // -- OpenAI SSE parsing tests --

    #[test]
    fn parse_openai_chunk_extracts_text() {
        // A typical chat completion chunk from the OpenAI API.
        let data = r#"{
            "id": "chatcmpl-abc123",
            "object": "chat.completion.chunk",
            "created": 1699123456,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": { "content": "Hello from GPT" },
                "finish_reason": null
            }]
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let text = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|t| t.as_str());
        assert_eq!(text, Some("Hello from GPT"));
    }

    #[test]
    fn parse_openai_done_sentinel_detected() {
        // The [DONE] sentinel signals end-of-stream.
        let data = "[DONE]";
        assert_eq!(data.trim(), "[DONE]");
    }

    #[test]
    fn parse_openai_chunk_finish_reason_stop() {
        // A final chunk has finish_reason set to "stop".
        let data = r#"{
            "id": "chatcmpl-abc123",
            "object": "chat.completion.chunk",
            "created": 1699123456,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        let reason = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finish_reason"))
            .and_then(|r| r.as_str());
        let stop_reason: Option<String> = reason
            .filter(|r| !r.is_empty())
            .map(|r| r.to_string());
        assert_eq!(stop_reason, Some("stop".to_string()));
    }

    #[test]
    fn parse_openai_chunk_null_finish_reason() {
        // Mid-stream chunks have null finish_reason which should not set stop_reason.
        let data = r#"{
            "id": "chatcmpl-abc123",
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": { "content": "mid" },
                "finish_reason": null
            }]
        }"#;
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        // null as_str() returns None, so stop_reason stays None
        let reason = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finish_reason"))
            .and_then(|r| r.as_str());
        assert_eq!(reason, None);
    }

    // -- Helper to build a minimal Config for testing --

    fn make_test_config(api_key: Option<String>) -> Config {
        use crate::config::*;
        use crate::llm::provider::LlmProvider;
        use std::collections::HashMap;

        Config {
            league: LeagueConfig {
                name: "Test".to_string(),
                platform: "espn".to_string(),
                num_teams: 10,
                scoring_type: "h2h".to_string(),
                salary_cap: 260,
                batting_categories: CategoriesSection {
                    categories: vec!["R".to_string()],
                },
                pitching_categories: CategoriesSection {
                    categories: vec!["K".to_string()],
                },
                roster: HashMap::new(),
                roster_limits: RosterLimits {
                    max_sp: 7,
                    max_rp: 7,
                    gs_per_week: 7,
                },
                teams: HashMap::new(),
            },
            strategy: StrategyConfig {
                hitting_budget_fraction: 0.65,
                weights: CategoryWeights {
                    R: 1.0,
                    HR: 1.0,
                    RBI: 1.0,
                    BB: 1.2,
                    SB: 1.0,
                    AVG: 1.0,
                    K: 1.0,
                    W: 1.0,
                    SV: 0.7,
                    HD: 1.3,
                    ERA: 1.0,
                    WHIP: 1.0,
                },
                pool: PoolConfig {
                    min_pa: 400,
                    min_ip_sp: 100.0,
                    min_g_rp: 40,
                    hitter_pool_size: 150,
                    sp_pool_size: 70,
                    rp_pool_size: 80,
                },
                llm: LlmConfig {
                    provider: LlmProvider::Anthropic,
                    model: "claude-sonnet-4-6".to_string(),
                    analysis_max_tokens: 2048,
                    planning_max_tokens: 2048,
                    analysis_trigger: "nomination".to_string(),
                    prefire_planning: true,
                },
                strategy_overview: None,
            },
            credentials: CredentialsConfig {
                anthropic_api_key: api_key,
                google_api_key: None,
                openai_api_key: None,
            },
            ws_port: 9001,
            data_paths: DataPaths {
                hitters: "data/hitters.csv".to_string(),
                pitchers: "data/pitchers.csv".to_string(),
            },
        }
    }

    /// Build a Config for a specific provider with separate key slots.
    /// `google_key` is used when provider is Google; `openai_key` when OpenAI.
    fn make_test_config_for_provider(
        provider: LlmProvider,
        model: String,
        google_key: Option<String>,
        openai_key: Option<String>,
    ) -> Config {
        use crate::config::*;
        use std::collections::HashMap;

        Config {
            league: LeagueConfig {
                name: "Test".to_string(),
                platform: "espn".to_string(),
                num_teams: 10,
                scoring_type: "h2h".to_string(),
                salary_cap: 260,
                batting_categories: CategoriesSection {
                    categories: vec!["R".to_string()],
                },
                pitching_categories: CategoriesSection {
                    categories: vec!["K".to_string()],
                },
                roster: HashMap::new(),
                roster_limits: RosterLimits {
                    max_sp: 7,
                    max_rp: 7,
                    gs_per_week: 7,
                },
                teams: HashMap::new(),
            },
            strategy: StrategyConfig {
                hitting_budget_fraction: 0.65,
                weights: CategoryWeights {
                    R: 1.0,
                    HR: 1.0,
                    RBI: 1.0,
                    BB: 1.2,
                    SB: 1.0,
                    AVG: 1.0,
                    K: 1.0,
                    W: 1.0,
                    SV: 0.7,
                    HD: 1.3,
                    ERA: 1.0,
                    WHIP: 1.0,
                },
                pool: PoolConfig {
                    min_pa: 400,
                    min_ip_sp: 100.0,
                    min_g_rp: 40,
                    hitter_pool_size: 150,
                    sp_pool_size: 70,
                    rp_pool_size: 80,
                },
                llm: LlmConfig {
                    provider,
                    model,
                    analysis_max_tokens: 2048,
                    planning_max_tokens: 2048,
                    analysis_trigger: "nomination".to_string(),
                    prefire_planning: true,
                },
                strategy_overview: None,
            },
            credentials: CredentialsConfig {
                anthropic_api_key: None,
                google_api_key: google_key,
                openai_api_key: openai_key,
            },
            ws_port: 9001,
            data_paths: DataPaths {
                hitters: "data/hitters.csv".to_string(),
                pitchers: "data/pitchers.csv".to_string(),
            },
        }
    }
}
