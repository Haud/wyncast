// Claude API streaming client using reqwest-eventsource.
//
// Sends messages to the Anthropic Messages API with `stream: true` and parses
// the Server-Sent Events into `LlmEvent` variants that are forwarded over an
// mpsc channel for the app orchestrator to consume.

use futures_util::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::config::Config;
use crate::protocol::LlmEvent;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ---------------------------------------------------------------------------
// ClaudeClient
// ---------------------------------------------------------------------------

/// Low-level Claude API streaming client.
pub struct ClaudeClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl ClaudeClient {
    /// Create a new client with the given API key and model identifier.
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model,
        }
    }

    /// Send a message to the Claude API and stream the response as `LlmEvent`s
    /// over `tx`.
    ///
    /// The `generation` counter is threaded through every emitted event so that
    /// the receiving side can discard stale events from cancelled tasks.
    ///
    /// The method returns when the stream is complete, an error occurs, or the
    /// receiver is dropped.
    pub async fn stream_message(
        &self,
        system: &str,
        user_content: &str,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
        generation: u64,
    ) -> anyhow::Result<()> {
        if self.api_key.is_empty() {
            let _ = tx
                .send(LlmEvent::Error {
                    message: "API key not configured".to_string(),
                    generation,
                })
                .await;
            return Ok(());
        }

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "stream": true,
            "system": system,
            "messages": [{ "role": "user", "content": user_content }]
        });

        let request = self
            .http
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body);

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

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => {
                    debug!("SSE connection opened");
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
                                    .send(LlmEvent::Token {
                                        text,
                                        generation,
                                    })
                                    .await
                                    .is_err()
                                {
                                    // Receiver dropped — abort stream.
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
                            debug!(output_tokens, "message_delta");
                        }
                        "message_stop" => {
                            debug!("message_stop — streaming complete");
                            let _ = tx
                                .send(LlmEvent::Complete {
                                    full_text,
                                    input_tokens,
                                    output_tokens,
                                    generation,
                                })
                                .await;
                            es.close();
                            return Ok(());
                        }
                        // Ignore ping, content_block_start, content_block_stop, etc.
                        _ => {
                            debug!(event_type, "ignoring SSE event");
                        }
                    }
                }
                Err(err) => {
                    warn!(?err, "SSE stream error");
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

        // Stream ended without message_stop (shouldn't normally happen).
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
                    generation,
                })
                .await;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LlmClient wrapper
// ---------------------------------------------------------------------------

/// High-level wrapper that can be either an active Claude client or disabled.
pub enum LlmClient {
    /// Claude API is configured and ready.
    Active(ClaudeClient),
    /// LLM functionality is disabled (no API key configured).
    Disabled,
}

impl LlmClient {
    /// Build an `LlmClient` from the application config.
    ///
    /// Returns `Active` if an API key is present in credentials, otherwise
    /// returns `Disabled`.
    pub fn from_config(config: &Config) -> Self {
        match &config.credentials.anthropic_api_key {
            Some(key) if !key.is_empty() => {
                let model = config.strategy.llm.model.clone();
                LlmClient::Active(ClaudeClient::new(key.clone(), model))
            }
            _ => LlmClient::Disabled,
        }
    }

    /// Stream a message, delegating to the inner `ClaudeClient` or immediately
    /// sending an error if disabled.
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

    // -- ClaudeClient with empty API key --

    #[tokio::test]
    async fn empty_api_key_sends_error_event() {
        let client = ClaudeClient::new(String::new(), "model".to_string());
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
            let _ = tx2
                .send(LlmEvent::Complete {
                    full_text,
                    input_tokens,
                    output_tokens,
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
                        }
                        "message_stop" => {
                            let _ = tx
                                .send(LlmEvent::Complete {
                                    full_text: full_text.clone(),
                                    input_tokens,
                                    output_tokens,
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

    // -- Helper to build a minimal Config for testing --

    fn make_test_config(api_key: Option<String>) -> Config {
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
                my_team: None,
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
                    model: "claude-sonnet-4-5-20250929".to_string(),
                    analysis_max_tokens: 400,
                    planning_max_tokens: 600,
                    analysis_trigger: "nomination".to_string(),
                    prefire_planning: true,
                },
            },
            credentials: CredentialsConfig {
                anthropic_api_key: api_key,
            },
            ws_port: 9001,
            db_path: "test.db".to_string(),
            data_paths: DataPaths {
                hitters: "data/hitters.csv".to_string(),
                pitchers: "data/pitchers.csv".to_string(),
            },
        }
    }
}
