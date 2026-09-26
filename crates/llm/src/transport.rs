//! The request, retry and SSE loop both provider clients run.
//!
//! The two providers differ in what a stream's events mean, not in how a
//! stream is fetched: the same POST, the same retry policy (`retry.rs`), the
//! same idle timeout, and the same rule that nothing is retried once an event
//! has crossed the boundary. So the loop is written once, here, and each
//! wire dialect supplies only what is its own — a [`Dialect`].

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use aldwin_core::{LlmError, LlmEvent, RetryInfo};
use aldwin_login::{Session, SessionError};
use async_stream::try_stream;
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::header::HeaderMap;
use serde::Serialize;

use crate::client::LlmClientInitError;
use crate::retry::{self, backoff, is_retryable_status, should_retry, terminal_error};

/// One provider's reading of its own stream: a fresh value per attempt,
/// fed each SSE `data:` payload in order.
pub(crate) trait Dialect: Default + Send {
    /// Named in every `RetryAttempt`, so a retry is attributed to its
    /// provider.
    const PROVIDER: &'static str;
    /// What the provider calls the end of a step, for the message a stream
    /// that ended without one carries.
    const STEP_END: &'static str;

    /// The message in an error response's body, when the body is the
    /// provider's own error shape.
    fn error_message(body: &str) -> Option<String>;

    /// One SSE payload. `Err` is why the stream cannot be read any further.
    fn read(&mut self, data: &str) -> Result<Vec<LlmEvent>, String>;

    /// The stream ended, however it ended. A dialect that holds a finished
    /// step back — waiting for usage that may come after it — completes it
    /// here; `None` means there was nothing to complete.
    fn close(&mut self) -> Option<LlmEvent> {
        None
    }
}

/// A connected account behind a request, as the loop needs it (ADR 0012):
/// the headers for one request, and a way to say the endpoint refused the
/// token. The one implementation outside a test is aldwin-login's
/// [`Session`]; a test's stands in for it so every answer a session can
/// give is reachable without an account server.
#[async_trait]
pub(crate) trait Bearer: Send + Sync {
    async fn headers(&self) -> Result<HeaderMap, SessionError>;
    async fn invalidate(&self);
}

#[async_trait]
impl Bearer for Session {
    async fn headers(&self) -> Result<HeaderMap, SessionError> {
        Session::headers(self).await
    }

    async fn invalidate(&self) {
        Session::invalidate(self).await;
    }
}

/// A request authenticated by a connected account rather than a fixed
/// header, and the sentence for the day the account is gone.
struct ConnectedAccount {
    bearer: Arc<dyn Bearer>,
    disconnected: String,
}

/// Where a client's requests go, and the shared HTTP client they go over.
pub(crate) struct Transport {
    http: reqwest::Client,
    endpoint: String,
    headers: HeaderMap,
    account: Option<ConnectedAccount>,
    idle_timeout: Duration,
}

impl Transport {
    /// One `reqwest::Client` per session — cheap to clone, expensive to
    /// construct (rustls, HTTP/2 via the crate's features).
    pub(crate) fn new(endpoint: String, headers: HeaderMap) -> Result<Self, LlmClientInitError> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(LlmClientInitError::HttpClient)?;
        Ok(Self {
            http,
            endpoint,
            headers,
            account: None,
            idle_timeout: retry::IDLE_TIMEOUT,
        })
    }

    /// Authenticates every request through `bearer`, resolved per attempt
    /// so a token refreshed between two is the one sent. `disconnected` is
    /// what the developer reads when the account is no longer good.
    pub(crate) fn with_account(mut self, bearer: Arc<dyn Bearer>, disconnected: String) -> Self {
        self.account = Some(ConnectedAccount {
            bearer,
            disconnected,
        });
        self
    }

    /// A test's stand-in for the 60s idle timeout, so the retry path can be
    /// exercised without waiting it out.
    #[cfg(test)]
    pub(crate) fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    /// POSTs `body` and streams the step back as `D` reads it, retrying what
    /// `retry.rs` says is retryable — every attempt visible as a
    /// `RetryAttempt` — until an event has been emitted, and never after:
    /// a second attempt then would splice two completions into one log entry.
    pub(crate) fn stream<'a, D: Dialect + 'a>(
        &'a self,
        body: impl Serialize + Send + Sync + 'a,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        let retrying =
            |status: Option<u16>, message: String, attempt: u32| LlmEvent::RetryAttempt {
                info: RetryInfo {
                    provider: D::PROVIDER.into(),
                    status,
                    message,
                    attempt,
                },
            };

        Box::pin(try_stream! {
            let mut attempt: u32 = 0;
            // A token the endpoint refuses is refreshed once and the request
            // sent again; a second refusal is the endpoint's answer.
            let mut refreshed = false;

            'attempts: loop {
                attempt += 1;

                let mut headers = self.headers.clone();
                if let Some(account) = &self.account {
                    match account.bearer.headers().await {
                        Ok(bearer) => headers.extend(bearer),
                        // Nothing is sent, so the error is Aldwin's own
                        // sentence: zero attempts, which the failure row
                        // leads with as written.
                        Err(SessionError::LoggedOut) => {
                            Err(LlmError::Terminal { attempts: 0, message: account.disconnected.clone() })?;
                            continue;
                        }
                        // The account server's passing trouble is the same
                        // kind of failure as not reaching the provider.
                        Err(SessionError::Failed(message)) => {
                            if should_retry(attempt) {
                                yield retrying(None, message, attempt);
                                tokio::time::sleep(backoff(attempt)).await;
                                continue 'attempts;
                            }
                            Err(terminal_error(attempt, None, message))?;
                            continue;
                        }
                    }
                }

                let sent = self.http.post(&self.endpoint).headers(headers).json(&body).send().await;
                let resp = match sent {
                    Ok(r) => r,
                    Err(e) => {
                        if should_retry(attempt) {
                            yield retrying(None, e.to_string(), attempt);
                            tokio::time::sleep(backoff(attempt)).await;
                            continue 'attempts;
                        }
                        Err(terminal_error(attempt, None, e.to_string()))?;
                        continue;
                    }
                };

                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    let message = D::error_message(&text).unwrap_or(text);
                    if status == 401 {
                        if let Some(account) = &self.account {
                            if !refreshed && should_retry(attempt) {
                                account.bearer.invalidate().await;
                                refreshed = true;
                                yield retrying(Some(status), message, attempt);
                                continue 'attempts;
                            }
                            Err(LlmError::Provider { status, message })?;
                            continue;
                        }
                    }
                    if is_retryable_status(status) && should_retry(attempt) {
                        yield retrying(Some(status), message, attempt);
                        tokio::time::sleep(backoff(attempt)).await;
                        continue 'attempts;
                    }
                    Err(terminal_error(attempt, Some(status), message))?;
                    continue;
                }

                let mut sse = resp.bytes_stream().eventsource();
                let mut dialect = D::default();
                let mut emitted_any = false;

                loop {
                    let read = match tokio::time::timeout(self.idle_timeout, sse.next()).await {
                        Err(_elapsed) => Err("idle timeout: no SSE activity for 60s".to_string()),
                        Ok(None) => Err(format!("stream closed before {}", D::STEP_END)),
                        Ok(Some(Err(e))) => Err(format!("SSE framing error: {e}")),
                        Ok(Some(Ok(raw))) if raw.data.is_empty() => continue,
                        Ok(Some(Ok(raw))) => dialect.read(&raw.data),
                    };

                    match read {
                        Ok(events) => {
                            let mut step_ended = false;
                            for event in events {
                                step_ended |= matches!(event, LlmEvent::StepEnded { .. });
                                emitted_any = true;
                                yield event;
                            }
                            if step_ended {
                                return;
                            }
                        }
                        Err(message) => {
                            // Every way a stream can end reaches this arm, so
                            // this is where a step the dialect held back gets
                            // completed. Only a stream that ended without
                            // one falls through to the failure paths below.
                            if let Some(event) = dialect.close() {
                                yield event;
                                return;
                            }
                            if !emitted_any && should_retry(attempt) {
                                yield retrying(None, message, attempt);
                                tokio::time::sleep(backoff(attempt)).await;
                                continue 'attempts;
                            }
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

/// The key the provider's `api_key_env` names, read now — a session refuses
/// to start on a missing variable, naming it exactly as `provider.yaml`
/// spelled it (aldwin-llm.md's Pitfalls).
pub(crate) fn api_key(var: &str) -> Result<String, LlmClientInitError> {
    std::env::var(var).map_err(|_| LlmClientInitError::MissingApiKeyEnv {
        var: var.to_string(),
    })
}
