//! The request, retry and SSE loop both provider clients share; each wire
//! dialect supplies only its reading of events, a [`Dialect`].

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

/// One provider's reading of its stream: a fresh value per attempt, fed each
/// SSE `data:` payload in order.
pub(crate) trait Dialect: Default + Send {
    /// The provider named in every `RetryAttempt`.
    const PROVIDER: &'static str;
    /// The provider's name for a step's end, used in the error when a stream
    /// closes without one.
    const STEP_END: &'static str;

    /// The message in an error body, when it has the provider's error shape.
    fn error_message(body: &str) -> Option<String>;

    /// Reads one SSE payload. `Err` says why the stream cannot be read further.
    fn read(&mut self, data: &str) -> Result<Vec<LlmEvent>, String>;

    /// Called once the stream ends, however it ends: returns a finished step
    /// the dialect held back (waiting for trailing usage), or `None`.
    fn close(&mut self) -> Option<LlmEvent> {
        None
    }
}

/// A connected account as the loop needs it (ADR 0012): one request's
/// headers, and a way to report a refused token. Implemented by [`Session`];
/// tests substitute their own to reach every answer without an account server.
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

/// Authentication by a connected account, and the error sentence for when
/// the account is logged out.
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
    /// Builds one `reqwest::Client` per session: cheap to clone, expensive
    /// to construct.
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

    /// Authenticates every request through `bearer`, resolved per attempt so
    /// a refreshed token is used. `disconnected` is the error once logged out.
    pub(crate) fn with_account(mut self, bearer: Arc<dyn Bearer>, disconnected: String) -> Self {
        self.account = Some(ConnectedAccount {
            bearer,
            disconnected,
        });
        self
    }

    /// Replaces `retry::IDLE_TIMEOUT` in tests.
    #[cfg(test)]
    pub(crate) fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    /// POSTs `body` and streams the step as `D` reads it. Retries (each
    /// yielded as a `RetryAttempt`) only until the first event is emitted:
    /// a later retry would splice two completions into one log entry.
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
            // A refused token is refreshed once; a second 401 is final.
            let mut refreshed = false;

            'attempts: loop {
                attempt += 1;

                let mut headers = self.headers.clone();
                if let Some(account) = &self.account {
                    match account.bearer.headers().await {
                        Ok(bearer) => headers.extend(bearer),
                        // Nothing was sent: `attempts: 0` makes the failure
                        // row show `disconnected` as written.
                        Err(SessionError::LoggedOut) => {
                            Err(LlmError::Terminal { attempts: 0, message: account.disconnected.clone() })?;
                            continue;
                        }
                        // Retried like a network failure.
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
                        Err(_elapsed) => Err(format!(
                            "idle timeout: no SSE activity for {:?}",
                            self.idle_timeout
                        )),
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
                            // Every stream end reaches this arm, so a held-back
                            // step completes here; only a stream without one
                            // is a failure.
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

/// Reads the key `api_key_env` names. A missing variable refuses the session,
/// named exactly as `provider.yaml` spells it (aldwin-llm.md, Pitfalls).
pub(crate) fn api_key(var: &str) -> Result<String, LlmClientInitError> {
    std::env::var(var).map_err(|_| LlmClientInitError::MissingApiKeyEnv {
        var: var.to_string(),
    })
}
