use std::pin::Pin;

use aldwin_core::{LlmClient, LlmError, LlmEvent, LlmRequest};
use aldwin_login::Account;
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::client::LlmClientInitError;
use crate::config::{Auth, ProviderConfig};
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
    /// Requires `config.base_url` — there's no sane default URL for
    /// "OpenAI-compatible," unlike Anthropic's single well-known endpoint —
    /// and reads `std::env::var` for a key, or takes the connected
    /// account's session (ADR 0012).
    pub fn new(config: ProviderConfig) -> Result<Self, LlmClientInitError> {
        let endpoint = config
            .base_url
            .clone()
            .ok_or(LlmClientInitError::MissingBaseUrl)?;
        let transport = match &config.auth {
            Auth::ApiKeyEnv(var) => {
                Transport::new(endpoint, bearer(&transport::api_key(var)?, var)?)?
            }
            Auth::Connection(session) => Transport::new(endpoint, HeaderMap::new())?
                .with_account(session.clone(), disconnected(session.account())),
        };
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
        let headers = bearer(api_key, "test").unwrap();
        let transport = Transport::new(endpoint, headers)
            .unwrap()
            .with_idle_timeout(idle_timeout);
        Self { config, transport }
    }

    /// Test-only: a client on a connected account, against a local fake
    /// server, with the account itself a stand-in.
    #[cfg(test)]
    pub(crate) fn with_account_at(
        config: ProviderConfig,
        account: std::sync::Arc<dyn transport::Bearer>,
        endpoint: String,
        idle_timeout: std::time::Duration,
    ) -> Self {
        let transport = Transport::new(endpoint, HeaderMap::new())
            .unwrap()
            .with_account(account, disconnected(Account::Xai))
            .with_idle_timeout(idle_timeout);
        Self { config, transport }
    }
}

/// The bearer key. `Content-Type` is left to reqwest's `json`, which sets
/// it. `var` is named when the key will not go in a header.
fn bearer(api_key: &str, var: &str) -> Result<HeaderMap, LlmClientInitError> {
    let bearer = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
        LlmClientInitError::InvalidApiKeyValue {
            var: var.to_string(),
        }
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, bearer);
    Ok(headers)
}

/// What the developer reads when the account server no longer honours the
/// connection: the one failure a refresh cannot recover from (ADR 0012).
fn disconnected(account: Account) -> String {
    format!(
        "Your {} account is no longer connected. Connect it again with /connect {}.",
        account.name(),
        account.id()
    )
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
    use aldwin_login::SessionError;
    use futures::StreamExt;
    use std::time::Duration;

    fn config() -> ProviderConfig {
        ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "mistral-small-latest".into(),
            auth: Auth::ApiKeyEnv("UNUSED".into()),
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
            auth: Auth::ApiKeyEnv("UNUSED".into()),
            base_url: None,
            extended_thinking_budget: Some(4096),
        };
        assert!(matches!(
            OpenAiCompatibleClient::new(config),
            Err(LlmClientInitError::MissingBaseUrl)
        ));
    }

    /// A connected account, as the transport sees one: each attempt's
    /// answer in turn, and a count of how often the endpoint's refusal was
    /// passed back.
    struct FakeAccount {
        answers: std::sync::Mutex<std::collections::VecDeque<Result<&'static str, SessionError>>>,
        invalidated: std::sync::atomic::AtomicUsize,
    }

    impl FakeAccount {
        fn answering(answers: Vec<Result<&'static str, SessionError>>) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                answers: std::sync::Mutex::new(answers.into()),
                invalidated: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn invalidated(&self) -> usize {
            self.invalidated.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl transport::Bearer for FakeAccount {
        async fn headers(&self) -> Result<HeaderMap, SessionError> {
            let token = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("an answer for every attempt")?;
            bearer(token, "test").map_err(|e| SessionError::Failed(e.to_string()))
        }

        async fn invalidate(&self) {
            self.invalidated
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn account_client_at(
        server: &test_server::FakeServer,
        account: std::sync::Arc<FakeAccount>,
    ) -> OpenAiCompatibleClient {
        OpenAiCompatibleClient::with_account_at(
            config(),
            account,
            server.url("/v1/chat/completions"),
            Duration::from_secs(5),
        )
    }

    /// The header on each request, lowercased — hyper writes standard
    /// names in lowercase, and the test should not care either way.
    fn authorizations(server: &test_server::FakeServer) -> Vec<String> {
        server
            .requests()
            .iter()
            .map(|head| {
                head.lines()
                    .map(str::to_ascii_lowercase)
                    .find(|line| line.starts_with("authorization:"))
                    .unwrap_or_default()
            })
            .collect()
    }

    #[tokio::test]
    async fn a_client_on_a_connected_account_sends_the_sessions_bearer() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let account = FakeAccount::answering(vec![Ok("tok-1")]);
        let client = account_client_at(&server, account);
        let messages = vec![];

        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        assert!(events.iter().all(Result::is_ok), "{events:?}");
        assert_eq!(authorizations(&server), vec!["authorization: bearer tok-1"]);
    }

    /// The session says the account is gone: nothing is sent, and the
    /// error is the sentence that says what to do — at zero attempts, so
    /// the failure row leads with it rather than with a provider's refusal.
    #[tokio::test]
    async fn a_disconnected_account_is_the_sentence_and_no_request() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let account = FakeAccount::answering(vec![Err(SessionError::LoggedOut)]);
        let client = account_client_at(&server, account);
        let messages = vec![];

        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        let [Err(LlmError::Terminal {
            attempts: 0,
            message,
        })] = &events[..]
        else {
            panic!("expected Aldwin's own sentence, got {events:?}");
        };
        assert_eq!(
            message,
            "Your x.ai account is no longer connected. Connect it again with /connect xai."
        );
        assert!(server.requests().is_empty());
    }

    /// The endpoint refuses the token the session thought good — revoked,
    /// or a skewed clock. The session is told, and the request goes again
    /// on what it hands out next; the retry is visible like any other.
    #[tokio::test]
    async fn a_token_the_endpoint_refuses_is_refreshed_once_and_the_request_sent_again() {
        let server = test_server::spawn(vec![
            Canned::Status(401, r#"{"error":{"message":"token expired"}}"#.into()),
            Canned::Sse(success_sse()),
        ]);
        let account = FakeAccount::answering(vec![Ok("tok-1"), Ok("tok-2")]);
        let client = account_client_at(&server, account.clone());
        let messages = vec![];

        let events: Vec<LlmEvent> = client
            .stream(request(&messages))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        assert!(
            matches!(&events[0], LlmEvent::RetryAttempt { info } if info.status == Some(401)),
            "{events:?}"
        );
        assert!(matches!(&events[1], LlmEvent::TextDelta { text } if text == "hi"));
        assert_eq!(account.invalidated(), 1);
        assert_eq!(
            authorizations(&server),
            vec!["authorization: bearer tok-1", "authorization: bearer tok-2"]
        );
    }

    /// A second refusal is the endpoint's answer, not a reason to keep
    /// refreshing — and it is reported as the 401 it is.
    #[tokio::test]
    async fn a_second_refusal_is_the_endpoints_answer() {
        let server = test_server::spawn(vec![
            Canned::Status(401, r#"{"error":{"message":"no"}}"#.into()),
            Canned::Status(401, r#"{"error":{"message":"still no"}}"#.into()),
        ]);
        let account = FakeAccount::answering(vec![Ok("tok-1"), Ok("tok-2")]);
        let client = account_client_at(&server, account.clone());
        let messages = vec![];

        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        assert!(matches!(events[0], Ok(LlmEvent::RetryAttempt { .. })));
        assert!(
            matches!(&events[1], Err(LlmError::Provider { status: 401, message }) if message.contains("still no")),
            "{events:?}"
        );
        assert_eq!(account.invalidated(), 1);
    }

    /// The account server's passing trouble reads like a lost connection:
    /// a visible retry, then the request as normal.
    #[tokio::test]
    async fn a_refresh_the_account_server_fails_is_retried_like_a_lost_connection() {
        let server = test_server::spawn(vec![Canned::Sse(success_sse())]);
        let account = FakeAccount::answering(vec![
            Err(SessionError::Failed("HTTP 502".into())),
            Ok("tok-1"),
        ]);
        let client = account_client_at(&server, account);
        let messages = vec![];

        let events: Vec<LlmEvent> = client
            .stream(request(&messages))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|e| e.unwrap())
            .collect();

        assert!(
            matches!(&events[0], LlmEvent::RetryAttempt { info } if info.status.is_none() && info.message == "HTTP 502"),
            "{events:?}"
        );
        assert!(matches!(&events[1], LlmEvent::TextDelta { .. }));
        assert_eq!(server.requests().len(), 1);
    }

    /// The resend after a refused token is an attempt like any other, so a
    /// 401 on the last attempt is the answer rather than a fifth request.
    #[tokio::test(start_paused = true)]
    async fn a_refused_token_on_the_last_attempt_is_not_sent_again() {
        let server = test_server::spawn(vec![
            Canned::Status(503, "{}".into()),
            Canned::Status(503, "{}".into()),
            Canned::Status(503, "{}".into()),
            Canned::Status(401, r#"{"error":{"message":"expired"}}"#.into()),
        ]);
        let account = FakeAccount::answering(vec![Ok("a"), Ok("b"), Ok("c"), Ok("d")]);
        let client = account_client_at(&server, account.clone());
        let messages = vec![];

        let events: Vec<_> = client.stream(request(&messages)).collect().await;

        assert_eq!(server.requests().len(), 4, "four attempts and no more");
        assert_eq!(account.invalidated(), 0);
        assert!(matches!(events.last(), Some(Err(_))), "{events:?}");
    }
}
