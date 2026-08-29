use std::pin::Pin;

use mjolnir_core::{LlmClient, LlmError, LlmEvent, LlmRequest, RetryInfo};
use async_stream::try_stream;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue};

use crate::client::LlmClientInitError;
use crate::config::ProviderConfig;
use crate::retry::{backoff, is_retryable_status, should_retry, terminal_error, AttemptOutcome};
use crate::wire_openai::{self, Assembler, WireChunk};

const PROVIDER_NAME: &str = "openai-compatible";

/// V0.5 OpenAI-compatible client implementing core's `LlmClient` — a sibling
/// impl to `AnthropicClient` behind the same trait, not a refactor of it
/// (see mjolnir-llm.md). No OpenAI wire type crosses this struct's public
/// surface — see `wire_openai.rs`.
pub struct OpenAiCompatibleClient {
    http:         reqwest::Client,
    config:       ProviderConfig,
    headers:      HeaderMap,
    endpoint:     String,
    idle_timeout: std::time::Duration,
}

impl std::fmt::Debug for OpenAiCompatibleClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleClient").field("config", &self.config).finish_non_exhaustive()
    }
}

impl OpenAiCompatibleClient {
    /// Reads `std::env::var(config.api_key_env)` and requires
    /// `config.base_url` — there's no sane default URL for
    /// "OpenAI-compatible," unlike Anthropic's single well-known endpoint.
    pub fn new(config: ProviderConfig) -> Result<Self, LlmClientInitError> {
        let endpoint = config.base_url.clone().ok_or(LlmClientInitError::MissingBaseUrl)?;

        let api_key = std::env::var(&config.api_key_env)
            .map_err(|_| LlmClientInitError::MissingApiKeyEnv { var: config.api_key_env.clone() })?;

        let mut headers = HeaderMap::new();
        let auth_value = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| LlmClientInitError::InvalidApiKeyValue { var: config.api_key_env.clone() })?;
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let http = reqwest::Client::builder().build().map_err(LlmClientInitError::HttpClient)?;

        Ok(Self { http, config, headers, endpoint, idle_timeout: crate::retry::IDLE_TIMEOUT })
    }

    /// Test-only: points at a local fake server instead of a real endpoint,
    /// with an injectable idle timeout — mirrors `AnthropicClient::with_endpoint`.
    #[cfg(test)]
    pub(crate) fn with_endpoint(config: ProviderConfig, api_key: &str, endpoint: String, idle_timeout: std::time::Duration) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {api_key}")).unwrap());
        headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder().build().unwrap();
        Self { http, config, headers, endpoint, idle_timeout }
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn stream<'a>(&'a self, request: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        let body = wire_openai::build_request(&self.config, &request);

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
                    let message = wire_openai::parse_error_body(&text).unwrap_or(text);
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
                        Ok(None) => AttemptOutcome::Failed("stream closed before finish_reason".to_string()),
                        Ok(Some(Err(e))) => AttemptOutcome::Failed(format!("SSE framing error: {e}")),
                        Ok(Some(Ok(raw))) if raw.data.is_empty() => continue,
                        // The literal end-of-stream marker: the expected path
                        // already returns on `finish_reason`'s StepEnded
                        // before ever reaching this line, so getting here
                        // means the stream ended without one — treat like a
                        // closed connection, not like valid JSON.
                        Ok(Some(Ok(raw))) if raw.data == "[DONE]" => AttemptOutcome::Failed("stream closed before finish_reason".to_string()),
                        Ok(Some(Ok(raw))) => match serde_json::from_str::<WireChunk>(&raw.data) {
                            Err(e) => AttemptOutcome::Failed(format!("malformed SSE JSON: {e}")),
                            Ok(wire_chunk) => match assembler.handle(wire_chunk) {
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
                            // Mid-stream errors after the first event are
                            // never retried, matching AnthropicClient's rule
                            // — retrying now would splice two completions
                            // into one log entry.
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
    use mjolnir_core::Message;
    use std::time::Duration;

    fn config() -> ProviderConfig {
        ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "mistral-small-latest".into(),
            api_key_env: "UNUSED".into(),
            base_url: Some("https://x".into()),
            extended_thinking_budget: 4096,
        }
    }

    fn client_at(server: &test_server::FakeServer, idle_timeout: Duration) -> OpenAiCompatibleClient {
        OpenAiCompatibleClient::with_endpoint(config(), "test-key", server.url("/v1/chat/completions"), idle_timeout)
    }

    fn request<'a>(messages: &'a [Message]) -> LlmRequest<'a> {
        LlmRequest { model: "unused", system: "sys", tools: &[], messages, cache_breakpoints: &[] }
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

    #[tokio::test]
    async fn retryable_status_retries_then_succeeds() {
        let server = test_server::spawn(vec![Canned::Status(503, r#"{"detail":"overloaded"}"#.into()), Canned::Sse(success_sse())]);
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

        assert!(events.iter().any(|e| matches!(e, Ok(LlmEvent::RetryAttempt { .. }))));
        assert!(events.iter().any(|e| matches!(e, Ok(LlmEvent::StepEnded { .. }))));
    }

    #[tokio::test]
    async fn retries_exhaust_into_a_terminal_error() {
        let server = test_server::spawn(vec![Canned::HangUp, Canned::HangUp, Canned::HangUp, Canned::HangUp]);
        let client = client_at(&server, Duration::from_secs(5));
        let messages = vec![];
        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        let last = events.last().unwrap();
        assert!(matches!(last, Err(LlmError::Terminal { attempts: 4, .. })));
    }

    #[test]
    fn missing_base_url_is_a_construction_time_error() {
        let config = ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            api_key_env: "UNUSED".into(),
            base_url: None,
            extended_thinking_budget: 4096,
        };
        assert!(matches!(OpenAiCompatibleClient::new(config), Err(LlmClientInitError::MissingBaseUrl)));
    }
}
