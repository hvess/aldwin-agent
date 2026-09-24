use std::pin::Pin;

use aldwin_core::{LlmClient, LlmError, LlmEvent, LlmRequest};
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::client::LlmClientInitError;
use crate::config::ProviderConfig;
use crate::transport::{self, Dialect, Transport};
use crate::wire_openai::{self, Assembler, WireChunk};

/// V0.5 OpenAI-compatible client implementing core's `LlmClient` — a sibling
/// impl to `AnthropicClient` behind the same trait, not a refactor of it
/// (see aldwin-llm.md). No OpenAI wire type crosses this struct's public
/// surface — see `wire_openai.rs`.
pub struct OpenAiCompatibleClient {
    config: ProviderConfig,
    transport: Transport,
}

impl std::fmt::Debug for OpenAiCompatibleClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OpenAiCompatibleClient {
    /// Reads `std::env::var(config.api_key_env)` and requires
    /// `config.base_url` — there's no sane default URL for
    /// "OpenAI-compatible," unlike Anthropic's single well-known endpoint.
    pub fn new(config: ProviderConfig) -> Result<Self, LlmClientInitError> {
        let endpoint = config
            .base_url
            .clone()
            .ok_or(LlmClientInitError::MissingBaseUrl)?;
        let headers = headers(&transport::api_key(&config.api_key_env)?, &config)?;
        let transport = Transport::new(endpoint, headers)?;
        Ok(Self { config, transport })
    }

    /// Test-only: points at a local fake server instead of a real endpoint,
    /// with an injectable idle timeout — mirrors `AnthropicClient::with_endpoint`.
    #[cfg(test)]
    pub(crate) fn with_endpoint(
        config: ProviderConfig,
        api_key: &str,
        endpoint: String,
        idle_timeout: std::time::Duration,
    ) -> Self {
        let headers = headers(api_key, &config).unwrap();
        let transport = Transport::new(endpoint, headers)
            .unwrap()
            .with_idle_timeout(idle_timeout);
        Self { config, transport }
    }
}

/// The bearer key. `Content-Type` is left to reqwest's `json`, which sets it.
fn headers(api_key: &str, config: &ProviderConfig) -> Result<HeaderMap, LlmClientInitError> {
    let bearer = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
        LlmClientInitError::InvalidApiKeyValue {
            var: config.api_key_env.clone(),
        }
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, bearer);
    Ok(headers)
}

impl Dialect for Assembler {
    const PROVIDER: &'static str = "openai-compatible";
    const STEP_END: &'static str = "finish_reason";

    fn error_message(body: &str) -> Option<String> {
        wire_openai::parse_error_body(body)
    }

    fn read(&mut self, data: &str) -> Result<Vec<LlmEvent>, String> {
        // The literal end-of-stream marker. A finished step has already
        // returned on its StepEnded, or is held back for usage and completed
        // by `close` — so reaching it means the stream is over.
        if data == "[DONE]" {
            return Err(format!("stream closed before {}", Self::STEP_END));
        }
        let chunk: WireChunk =
            serde_json::from_str(data).map_err(|e| format!("malformed SSE JSON: {e}"))?;
        self.handle(chunk).map_err(|e| e.to_string())
    }

    /// The assembler holds StepEnded back when `finish_reason` arrives
    /// before usage does; the stream ending is what completes it.
    fn close(&mut self) -> Option<LlmEvent> {
        self.finish()
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        self.transport
            .stream::<Assembler>(wire_openai::build_request(&self.config, &request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{self, Canned};
    use aldwin_core::Message;
    use futures::StreamExt;
    use std::time::Duration;

    fn config() -> ProviderConfig {
        ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "mistral-small-latest".into(),
            api_key_env: "UNUSED".into(),
            base_url: Some("https://x".into()),
            extended_thinking_budget: Some(4096),
        }
    }

    fn client_at(
        server: &test_server::FakeServer,
        idle_timeout: Duration,
    ) -> OpenAiCompatibleClient {
        OpenAiCompatibleClient::with_endpoint(
            config(),
            "test-key",
            server.url("/v1/chat/completions"),
            idle_timeout,
        )
    }

    fn request<'a>(messages: &'a [Message]) -> LlmRequest<'a> {
        LlmRequest {
            system: "sys",
            tools: &[],
            messages,
            cache_breakpoint: None,
        }
    }

    fn success_sse() -> String {
        [
            r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            "data: [DONE]",
            "",
        ]
        .join("\n\n")
    }

    #[tokio::test]
    async fn successful_stream_yields_text_and_step_ended() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        let events: Vec<LlmEvent> = events.into_iter().map(|e| e.unwrap()).collect();
        assert!(matches!(&events[0], LlmEvent::TextDelta { text } if text == "hi"));
        assert!(matches!(&events[1], LlmEvent::StepEnded { .. }));
    }

    /// Frames copied from a live `lumo-api.proton.me/ai/v1` stream: usage
    /// trails `finish_reason` in its own choice-less chunk. The trailing
    /// `[DONE]` is never read — usage completes the step first — but is kept
    /// so the fixture stays the shape the wire actually has.
    fn lumo_sse() -> String {
        [
            r#"data:{"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello","reasoning":null}}]}"#,
            r#"data:{"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":null,"content":null,"reasoning":null},"finish_reason":"stop"}]}"#,
            r#"data: {"object":"chat.completion.chunk","choices":[],"usage":{"completion_tokens":7,"prompt_tokens":77,"total_tokens":84}}"#,
            "data:[DONE]",
            "",
        ]
        .join("\n\n")
    }

    #[tokio::test]
    async fn lumo_trailing_usage_lands_on_step_ended() {
        let server = test_server::spawn(vec![Canned::Sse(lumo_sse())]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<LlmEvent> = client
            .stream(request(&messages))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        let [LlmEvent::TextDelta { text }, LlmEvent::StepEnded { outcome }] = &events[..] else {
            panic!("expected TextDelta then StepEnded, got {events:?}")
        };
        assert_eq!(text, "Hello");
        assert_eq!(outcome.usage.input_tokens, 77);
        assert_eq!(outcome.usage.output_tokens, 7);
    }

    /// A stream that ends after `finish_reason` without ever sending usage is
    /// a finished turn, not a dropped connection — it must not retry. Ends on
    /// the literal `data:[DONE]` (no space, as Lumo writes it), which is the
    /// same client arm a bare connection close, an idle timeout and a framing
    /// error all reach.
    #[tokio::test]
    async fn stream_ending_after_finish_reason_completes_instead_of_retrying() {
        let sse = [
            r#"data: {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
            "data:[DONE]",
            "",
        ]
        .join("\n\n");
        let server = test_server::spawn(vec![Canned::Sse(sse), Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<LlmEvent> = client
            .stream(request(&messages))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmEvent::RetryAttempt { .. })),
            "got {events:?}"
        );
        let [LlmEvent::TextDelta { .. }, LlmEvent::StepEnded { outcome }] = &events[..] else {
            panic!("got {events:?}")
        };
        assert_eq!(outcome.usage.input_tokens, 0);
    }

    #[tokio::test]
    async fn retryable_status_retries_then_succeeds() {
        let server = test_server::spawn(vec![
            Canned::Status(503, r#"{"detail":"overloaded"}"#.into()),
            Canned::Sse(success_sse()),
        ]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        let mut saw_retry = false;
        let mut saw_step_ended = false;
        for e in events {
            match e.unwrap() {
                LlmEvent::RetryAttempt { .. } => saw_retry = true,
                LlmEvent::StepEnded { .. } => saw_step_ended = true,
                _ => {}
            }
        }
        assert!(saw_retry);
        assert!(saw_step_ended);
    }

    #[tokio::test]
    async fn transport_failure_before_any_response_is_retried() {
        let server = test_server::spawn(vec![Canned::HangUp, Canned::Sse(success_sse())]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(LlmEvent::StepEnded { .. }))));
    }

    #[tokio::test]
    async fn retries_exhaust_into_a_terminal_error() {
        let server = test_server::spawn(vec![
            Canned::HangUp,
            Canned::HangUp,
            Canned::HangUp,
            Canned::HangUp,
        ]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        let last = events.last().unwrap();
        assert!(matches!(last, Err(LlmError::Terminal { attempts: 4, .. })));
    }

    #[test]
    fn missing_base_url_is_a_construction_time_error() {
        let config = ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            api_key_env: "UNUSED".into(),
            base_url: None,
            extended_thinking_budget: Some(4096),
        };
        assert!(matches!(
            OpenAiCompatibleClient::new(config),
            Err(LlmClientInitError::MissingBaseUrl)
        ));
    }
}
