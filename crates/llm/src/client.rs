use std::pin::Pin;

use aldwin_core::{LlmClient, LlmError, LlmEvent, LlmRequest};
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue};
use thiserror::Error;

use crate::config::{Auth, ProviderConfig};
use crate::transport::{self, Dialect, Transport};
use crate::wire::{self, Assembler, WireEvent};

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
    #[error("this provider takes an API key, not a connected account")]
    AccountNotOffered,
    #[error("the connected account's session could not be set up: {0}")]
    ConnectionSession(String),
}

/// V0 Anthropic client implementing core's `LlmClient`. No Anthropic wire
/// type crosses this struct's public surface — see `wire.rs`.
pub struct AnthropicClient {
    config: ProviderConfig,
    transport: Transport,
}

impl std::fmt::Debug for AnthropicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AnthropicClient {
    /// Reads `std::env::var(config.api_key_env)` now — refuses to start on a
    /// missing var, surfacing the var name verbatim from the YAML (not a
    /// canonicalised form), per aldwin-llm.md's Pitfalls.
    ///
    /// `base_url` is deliberately not consulted: per provider.yaml's own
    /// annotated comment ("base_url: only used when provider is
    /// openai-compatible", see aldwin-config's annotated.rs), the field is
    /// scoped to the OpenAI-compatible adapter. Honouring it here would
    /// silently redirect Anthropic requests for anyone who has a leftover
    /// base_url set while `provider: anthropic`.
    pub fn new(config: ProviderConfig) -> Result<Self, LlmClientInitError> {
        let Auth::ApiKeyEnv(var) = &config.auth else {
            return Err(LlmClientInitError::AccountNotOffered);
        };
        let headers = headers(&transport::api_key(var)?, var)?;
        let transport = Transport::new(wire::ANTHROPIC_API_URL.to_string(), headers)?;
        Ok(Self { config, transport })
    }

    /// Test-only: points at a local fake server instead of the real
    /// Anthropic endpoint, with an injectable idle timeout, so the
    /// retry/SSE state machine can be exercised against controlled
    /// byte-level responses without a live API key or a real 60s wait.
    #[cfg(test)]
    pub(crate) fn with_endpoint(
        config: ProviderConfig,
        api_key: &str,
        endpoint: String,
        idle_timeout: std::time::Duration,
    ) -> Self {
        let headers = headers(api_key, "test").unwrap();
        let transport = Transport::new(endpoint, headers)
            .unwrap()
            .with_idle_timeout(idle_timeout);
        Self { config, transport }
    }
}

/// The key and the pinned API version. `Content-Type` is left to reqwest's
/// `json`, which sets it. `var` is named when the key will not go in a
/// header.
fn headers(api_key: &str, var: &str) -> Result<HeaderMap, LlmClientInitError> {
    let key =
        HeaderValue::from_str(api_key).map_err(|_| LlmClientInitError::InvalidApiKeyValue {
            var: var.to_string(),
        })?;
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key);
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static(wire::ANTHROPIC_VERSION),
    );
    Ok(headers)
}

impl Dialect for Assembler {
    const PROVIDER: &'static str = "anthropic";
    const STEP_END: &'static str = "message_stop";

    fn error_message(body: &str) -> Option<String> {
        wire::parse_error_body(body)
    }

    fn read(&mut self, data: &str) -> Result<Vec<LlmEvent>, String> {
        let event: WireEvent =
            serde_json::from_str(data).map_err(|e| format!("malformed SSE JSON: {e}"))?;
        self.handle(event).map_err(|e| e.to_string())
    }
}

impl LlmClient for AnthropicClient {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        self.transport
            .stream::<Assembler>(wire::build_request(&self.config, &request))
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
            auth: Auth::ApiKeyEnv("UNUSED".into()),
            base_url: None,
            extended_thinking_budget: Some(1000),
        }
    }

    fn client_at(server: &test_server::FakeServer, idle_timeout: Duration) -> AnthropicClient {
        AnthropicClient::with_endpoint(
            config(),
            "test-key",
            server.url("/v1/messages"),
            idle_timeout,
        )
    }

    fn empty_messages() -> Vec<Message> {
        vec![]
    }

    fn request<'a>(messages: &'a [Message], tools: &'a [ToolDefinition]) -> LlmRequest<'a> {
        LlmRequest {
            system: "sys",
            tools,
            messages,
            cache_breakpoint: None,
        }
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

    async fn collect(
        client: &AnthropicClient,
        messages: &[Message],
    ) -> Vec<Result<LlmEvent, LlmError>> {
        client.stream(request(messages, &[])).collect().await
    }

    #[tokio::test]
    async fn successful_stream_yields_text_and_step_ended() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(
            events.iter().all(|e| e.is_ok()),
            "unexpected error: {:?}",
            events
        );

        let texts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Ok(LlmEvent::TextDelta { text }) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Hello".to_string()]);

        assert!(matches!(
            events.last(),
            Some(Ok(LlmEvent::StepEnded { .. }))
        ));
    }

    #[tokio::test]
    async fn retryable_status_retries_then_succeeds() {
        let server = test_server::spawn(vec![
            Canned::Status(
                503,
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
                    .to_string(),
            ),
            Canned::Sse(success_sse()),
        ]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        let retries: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. })))
            .collect();
        assert_eq!(retries.len(), 1);
        assert!(matches!(
            events.last(),
            Some(Ok(LlmEvent::StepEnded { .. }))
        ));
    }

    #[tokio::test]
    async fn non_retryable_status_is_terminal_without_retry() {
        let server = test_server::spawn(vec![Canned::Status(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad request"}}"#
                .to_string(),
        )]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(!events
            .iter()
            .any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        match events.last() {
            Some(Err(LlmError::Provider {
                status: 400,
                message,
            })) => assert_eq!(message, "bad request"),
            other => panic!("expected a terminal Provider(400) error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn idle_timeout_with_no_events_yet_retries_then_succeeds() {
        let server = test_server::spawn(vec![
            Canned::SseThenStall(String::new()),
            Canned::Sse(success_sse()),
        ]);
        let client = client_at(&server, Duration::from_millis(50));

        let events = collect(&client, &empty_messages()).await;
        let retries: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. })))
            .collect();
        assert_eq!(retries.len(), 1);
        assert!(matches!(
            events.last(),
            Some(Ok(LlmEvent::StepEnded { .. }))
        ));
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
        assert!(!events
            .iter()
            .any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(matches!(
            events.last(),
            Some(Err(LlmError::StreamInterrupted(_)))
        ));
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
        let retries = events
            .iter()
            .filter(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. })))
            .count();
        assert_eq!(
            retries, 3,
            "3 retries + the 4th (final) failing attempt = MAX_ATTEMPTS"
        );
        assert!(matches!(
            events.last(),
            Some(Err(LlmError::Terminal { attempts: 4, .. }))
        ));
    }

    #[tokio::test]
    async fn transport_failure_before_any_response_is_retried() {
        let server = test_server::spawn(vec![Canned::HangUp, Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));

        let events = collect(&client, &empty_messages()).await;
        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(matches!(
            events.last(),
            Some(Ok(LlmEvent::StepEnded { .. }))
        ));
    }
}
