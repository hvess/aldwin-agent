use std::pin::Pin;

use aldwin_core::{LlmClient, LlmError, LlmEvent, LlmRequest, RetryInfo};
use async_stream::try_stream;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue};
use thiserror::Error;

use crate::config::ProviderConfig;
use crate::retry::{self, backoff, is_retryable_status, should_retry, terminal_error, AttemptOutcome};
use crate::wire::{self, Assembler, WireEvent};

const PROVIDER_NAME: &str = "anthropic";

#[derive(Debug, Error)]
pub enum LlmClientInitError {
    #[error("environment variable {var:?} (provider.yaml's api_key_env) is not set")]
    MissingApiKeyEnv { var: String },
    #[error("environment variable {var:?}'s value is not a valid HTTP header value")]
    InvalidApiKeyValue { var: String },
    #[error("failed to construct the HTTP client: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("provider.yaml's base_url is required for the openai-compatible provider")]
    MissingBaseUrl,
}

/// V0 Anthropic client implementing core's `LlmClient`. No Anthropic wire
/// type crosses this struct's public surface — see `wire.rs`.
pub struct AnthropicClient {
    http:         reqwest::Client,
    config:       ProviderConfig,
    headers:      HeaderMap,
    endpoint:     String,
    idle_timeout: std::time::Duration,
}

impl std::fmt::Debug for AnthropicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicClient").field("config", &self.config).finish_non_exhaustive()
    }
}

impl AnthropicClient {
    /// Reads `std::env::var(config.api_key_env)` now — refuses to start on a
    /// missing var, surfacing the var name verbatim from the YAML (not a
    /// canonicalised form), per aldwin-llm.md's Pitfalls.
    pub fn new(config: ProviderConfig) -> Result<Self, LlmClientInitError> {
        let api_key = std::env::var(&config.api_key_env)
            .map_err(|_| LlmClientInitError::MissingApiKeyEnv { var: config.api_key_env.clone() })?;

        let mut headers = HeaderMap::new();
        let key_value = HeaderValue::from_str(&api_key)
            .map_err(|_| LlmClientInitError::InvalidApiKeyValue { var: config.api_key_env.clone() })?;
        headers.insert("x-api-key", key_value);
        headers.insert("anthropic-version", HeaderValue::from_static(wire::ANTHROPIC_VERSION));
        headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Single shared client (rustls + HTTP/2 via the crate's enabled
        // features, no native-tls competing backend) — cheap to clone,
        // expensive to construct, so built once here.
        let http = reqwest::Client::builder().build().map_err(LlmClientInitError::HttpClient)?;

        // `base_url` is deliberately NOT consulted here: per provider.yaml's
        // own annotated comment ("base_url: only used when provider is
        // openai-compatible", see aldwin-config's annotated.rs), the field
        // is scoped to the OpenAI-compatible adapter. Honoring it here would
        // silently redirect Anthropic requests for anyone who has a leftover
        // base_url set while `provider: anthropic`.
        Ok(Self { http, config, headers, endpoint: wire::ANTHROPIC_API_URL.to_string(), idle_timeout: retry::IDLE_TIMEOUT })
    }

    /// Test-only: points at a local fake server instead of the real
    /// Anthropic endpoint, with an injectable idle timeout, so the
    /// retry/SSE state machine can be exercised against controlled
    /// byte-level responses without a live API key or a real 60s wait.
    #[cfg(test)]
    pub(crate) fn with_endpoint(config: ProviderConfig, api_key: &str, endpoint: String, idle_timeout: std::time::Duration) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(api_key).unwrap());
        headers.insert("anthropic-version", HeaderValue::from_static(wire::ANTHROPIC_VERSION));
        headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder().build().unwrap();
        Self { http, config, headers, endpoint, idle_timeout }
    }
}

impl LlmClient for AnthropicClient {
    fn stream<'a>(&'a self, request: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        let body = wire::build_request(&self.config, &request);

        Box::pin(try_stream! {
            let mut attempt: u32 = 0;

            'attempts: loop {
                attempt += 1;

                let sent = self.http.post(&self.endpoint).headers(self.headers.clone()).json(&body).send().await;
                let resp = match sent {
                    Ok(r) => r,
                    Err(e) => {
                        if should_retry(attempt) {
                            yield LlmEvent::RetryAttempt {
                                info: RetryInfo { provider: PROVIDER_NAME.into(), status: None, message: e.to_string(), attempt },
                            };
                            tokio::time::sleep(backoff(attempt)).await;
                            continue 'attempts;
                        }
                        Err(terminal_error(attempt, None, e.to_string()))?;
                        continue;
                    }
                };

                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    let message = wire::parse_error_body(&text).unwrap_or(text);
                    if is_retryable_status(status.as_u16()) && should_retry(attempt) {
                        yield LlmEvent::RetryAttempt {
                            info: RetryInfo { provider: PROVIDER_NAME.into(), status: Some(status.as_u16()), message, attempt },
                        };
                        tokio::time::sleep(backoff(attempt)).await;
                        continue 'attempts;
                    }
                    Err(terminal_error(attempt, Some(status.as_u16()), message))?;
                    continue;
                }

                let mut sse = resp.bytes_stream().eventsource();
                let mut assembler = Assembler::new();
                let mut emitted_any = false;

                loop {
                    let outcome = match tokio::time::timeout(self.idle_timeout, sse.next()).await {
                        Err(_elapsed) => AttemptOutcome::Failed("idle timeout: no SSE activity for 60s".to_string()),
                        Ok(None) => AttemptOutcome::Failed("stream closed before message_stop".to_string()),
                        Ok(Some(Err(e))) => AttemptOutcome::Failed(format!("SSE framing error: {e}")),
                        Ok(Some(Ok(raw))) if raw.data.is_empty() => continue,
                        Ok(Some(Ok(raw))) => match serde_json::from_str::<WireEvent>(&raw.data) {
                            Err(e) => AttemptOutcome::Failed(format!("malformed SSE JSON: {e}")),
                            Ok(wire_event) => match assembler.handle(wire_event) {
                                Ok(events) => AttemptOutcome::Events(events),
                                Err(e) => AttemptOutcome::Failed(e.to_string()),
                            },
                        },
                    };

                    match outcome {
                        AttemptOutcome::Events(events) => {
                            let mut step_ended = false;
                            for event in events {
                                if matches!(event, LlmEvent::StepEnded { .. }) {
                                    step_ended = true;
                                }
                                emitted_any = true;
                                yield event;
                            }
                            if step_ended {
                                return;
                            }
                        }
                        AttemptOutcome::Failed(message) => {
                            if !emitted_any && should_retry(attempt) {
                                yield LlmEvent::RetryAttempt {
                                    info: RetryInfo { provider: PROVIDER_NAME.into(), status: None, message, attempt },
                                };
                                tokio::time::sleep(backoff(attempt)).await;
                                continue 'attempts;
                            }
                            // Per aldwin-llm.md: mid-stream errors after the
                            // first event are never retried, even if attempts
                            // remain — retrying now would splice two
                            // different completions into one log entry.
                            if emitted_any {
                                Err(LlmError::StreamInterrupted(message))?;
                            } else {
                                Err(LlmError::Terminal { attempts: attempt, message })?;
                            }
                            continue;
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{self, Canned};
    use aldwin_core::{Message, ToolDefinition};
    use futures::StreamExt;
    use std::time::Duration;

    fn config() -> ProviderConfig {
        ProviderConfig {
            kind: aldwin_config::ProviderKind::Anthropic,
            model: "claude-sonnet-5".into(),
            api_key_env: "UNUSED".into(),
            base_url: None,
            extended_thinking_budget: 1000,
        }
    }

    fn client_at(server: &test_server::FakeServer, idle_timeout: Duration) -> AnthropicClient {
        AnthropicClient::with_endpoint(config(), "test-key", server.url("/v1/messages"), idle_timeout)
    }

    fn empty_messages() -> Vec<Message> {
        vec![]
    }

    fn request<'a>(messages: &'a [Message], tools: &'a [ToolDefinition]) -> LlmRequest<'a> {
        LlmRequest { model: "unused", system: "sys", tools, messages, cache_breakpoints: &[] }
    }

    fn success_sse() -> String {
        [
            r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}

"#,
            r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

"#,
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

"#,
            r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#,
            r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}

"#,
            r#"event: message_stop
data: {"type":"message_stop"}

"#,
        ]
        .concat()
    }

    async fn collect(client: &AnthropicClient, messages: &[Message]) -> Vec<Result<LlmEvent, LlmError>> {
        client.stream(request(messages, &[])).collect().await
    }

    #[tokio::test]
    async fn successful_stream_yields_text_and_step_ended() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(events.iter().all(|e| e.is_ok()), "unexpected error: {:?}", events);

        let texts: Vec<_> = events.iter().filter_map(|e| match e {
            Ok(LlmEvent::TextDelta { text }) => Some(text.clone()),
            _ => None,
        }).collect();
        assert_eq!(texts, vec!["Hello".to_string()]);

        assert!(matches!(events.last(), Some(Ok(LlmEvent::StepEnded { .. }))));
    }

    #[tokio::test]
    async fn retryable_status_retries_then_succeeds() {
        let server = test_server::spawn(vec![
            Canned::Status(503, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#.to_string()),
            Canned::Sse(success_sse()),
        ]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        let retries: Vec<_> = events.iter().filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))).collect();
        assert_eq!(retries.len(), 1);
        assert!(matches!(events.last(), Some(Ok(LlmEvent::StepEnded { .. }))));
    }

    #[tokio::test]
    async fn non_retryable_status_is_terminal_without_retry() {
        let server = test_server::spawn(vec![Canned::Status(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad request"}}"#.to_string(),
        )]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(!events.iter().any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        match events.last() {
            Some(Err(LlmError::Provider { status: 400, message })) => assert_eq!(message, "bad request"),
            other => panic!("expected a terminal Provider(400) error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn idle_timeout_with_no_events_yet_retries_then_succeeds() {
        let server = test_server::spawn(vec![Canned::SseThenStall(String::new()), Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_millis(50));

        let events = collect(&client, &empty_messages()).await;
        let retries: Vec<_> = events.iter().filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))).collect();
        assert_eq!(retries.len(), 1);
        assert!(matches!(events.last(), Some(Ok(LlmEvent::StepEnded { .. }))));
    }

    #[tokio::test]
    async fn mid_stream_failure_after_first_event_is_never_retried() {
        // message_start + one text delta, then the connection just ends
        // (no message_stop) — per spec, once at least one event has been
        // emitted this attempt, a subsequent failure is terminal even
        // though retry attempts remain.
        let partial = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}

"#;
        let server = test_server::spawn(vec![Canned::Sse(partial.to_string())]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(!events.iter().any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(matches!(events.last(), Some(Err(LlmError::StreamInterrupted(_)))));
    }

    #[tokio::test]
    async fn retries_exhaust_into_a_terminal_error() {
        let server = test_server::spawn(vec![
            Canned::Status(503, "{}".to_string()),
            Canned::Status(503, "{}".to_string()),
            Canned::Status(503, "{}".to_string()),
            Canned::Status(503, "{}".to_string()),
        ]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        let retries = events.iter().filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))).count();
        assert_eq!(retries, 3, "3 retries + the 4th (final) failing attempt = MAX_ATTEMPTS");
        assert!(matches!(events.last(), Some(Err(LlmError::Terminal { attempts: 4, .. }))));
    }

    #[tokio::test]
    async fn transport_failure_before_any_response_is_retried() {
        let server = test_server::spawn(vec![Canned::HangUp, Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(events.iter().any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(matches!(events.last(), Some(Ok(LlmEvent::StepEnded { .. }))));
    }
}
