//! The device authorization grant (RFC 8628) and refresh grant (RFC 6749
//! §6). The only module that knows grant types, wire fields and error codes.
//!
//! `Err(String)` is a failure the caller can only report (unreachable, or an
//! answer outside the protocol); every answer the protocol names is `Ok`.

use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;

/// An account's authorization server and this client's identity there.
/// Owned strings so a test can point one at a local server.
#[derive(Clone)]
pub(crate) struct Authority {
    pub device_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scope: String,
}

/// RFC 8628 §3.2 device authorization response.
#[derive(Deserialize)]
pub(crate) struct DeviceGrant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    /// Seconds between polls; RFC 8628's default of 5 when absent.
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// RFC 6749 §5.1 token response, from either grant.
#[derive(Deserialize)]
pub(crate) struct TokenGrant {
    pub access_token: String,
    /// Seconds; optional in the RFC.
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// Absent from a refresh that does not rotate; `Login::wait` fails a
    /// login grant without one.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// RFC 6749 §5.2 error response.
#[derive(Deserialize)]
struct Refusal {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// The outcome of one token-endpoint poll (RFC 8628 §3.5).
pub(crate) enum Poll {
    Granted(TokenGrant),
    /// `authorization_pending`: ask again after the interval.
    Pending,
    /// `slow_down`: the caller widens the interval (`login::SLOW_DOWN`).
    SlowDown,
    /// `access_denied`: the developer refused in the browser.
    Denied,
    /// `expired_token`.
    Expired,
    /// Transport failure or 5xx. Says nothing about the login: ask again,
    /// never end the wait on it.
    Unavailable,
}

/// The outcome of a refresh.
pub(crate) enum Refresh {
    Granted(TokenGrant),
    /// The refresh token is no longer honoured; only a new login recovers.
    Revoked,
}

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Bound on a whole token request. Without it an unanswered connection
/// holds a login wait past its deadline, or a `Session`'s lock for good.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The HTTP client for both grants. Never follow redirects: a redirected
/// POST would carry the refresh token wherever it pointed.
///
/// `timeout` is [`REQUEST_TIMEOUT`] outside tests. Paused-clock tests pass
/// `None`: the paused clock jumps to the pending timer while a socket
/// connects, so the timeout would fire before the local server answers.
pub(crate) fn client(timeout: Option<Duration>) -> Result<Client, String> {
    let mut client = Client::builder().redirect(Policy::none());
    if let Some(timeout) = timeout {
        client = client.timeout(timeout);
    }
    client.build().map_err(|e| e.to_string())
}

pub(crate) async fn device_grant(
    http: &Client,
    authority: &Authority,
) -> Result<DeviceGrant, String> {
    let response = http
        .post(&authority.device_url)
        .form(&[
            ("client_id", authority.client_id.as_str()),
            ("scope", authority.scope.as_str()),
        ])
        .send()
        .await
        .map_err(transport)?;
    let (status, body) = read(response).await?;
    if !status.is_success() {
        return Err(describe(status, &body));
    }
    parse(&body)
}

pub(crate) async fn poll(
    http: &Client,
    authority: &Authority,
    device_code: &str,
) -> Result<Poll, String> {
    let sent = http
        .post(&authority.token_url)
        .form(&[
            ("grant_type", DEVICE_GRANT),
            ("device_code", device_code),
            ("client_id", authority.client_id.as_str()),
        ])
        .send()
        .await;
    let Ok(response) = sent else {
        return Ok(Poll::Unavailable);
    };
    let Ok((status, body)) = read(response).await else {
        return Ok(Poll::Unavailable);
    };
    if status.is_success() {
        return parse(&body).map(Poll::Granted);
    }
    if status.is_server_error() {
        return Ok(Poll::Unavailable);
    }
    match refusal(&body).map(|r| r.error).as_deref() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown),
        Some("access_denied") => Ok(Poll::Denied),
        Some("expired_token") => Ok(Poll::Expired),
        _ => Err(describe(status, &body)),
    }
}

pub(crate) async fn refresh(
    http: &Client,
    authority: &Authority,
    refresh_token: &str,
) -> Result<Refresh, String> {
    let response = http
        .post(&authority.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", authority.client_id.as_str()),
        ])
        .send()
        .await
        .map_err(transport)?;
    let (status, body) = read(response).await?;
    if status.is_success() {
        return parse(&body).map(Refresh::Granted);
    }
    // Revoked only for `invalid_grant`, or a 401/403 with an RFC 6749 §5.2
    // body. A 403 without one (a proxy or bot-check page) is transient: do
    // not treat it as a logout.
    let revoked = refusal(&body).is_some_and(|r| {
        r.error == "invalid_grant"
            || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    });
    if revoked {
        Ok(Refresh::Revoked)
    } else {
        Err(describe(status, &body))
    }
}

async fn read(response: Response) -> Result<(StatusCode, String), String> {
    let status = response.status();
    let body = response.text().await.map_err(transport)?;
    Ok((status, body))
}

/// The error with its whole source chain: reqwest's own message stops at
/// "error sending request", and the cause (timed out, refused) is below it.
fn transport(error: reqwest::Error) -> String {
    let mut line = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        line.push_str(": ");
        line.push_str(&cause.to_string());
        source = cause.source();
    }
    line
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, String> {
    serde_json::from_str(body).map_err(|e| format!("the reply was not the expected JSON: {e}"))
}

fn refusal(body: &str) -> Option<Refusal> {
    serde_json::from_str(body).ok()
}

/// One line: the status, then the refusal's description, or else the body
/// cut to `MOST` chars (an HTML error page is not worth carrying whole).
fn describe(status: StatusCode, body: &str) -> String {
    const MOST: usize = 240;
    let detail = refusal(body)
        .map(|r| r.error_description.unwrap_or(r.error))
        .unwrap_or_else(|| {
            body.chars()
                .take(MOST)
                .collect::<String>()
                .trim()
                .to_string()
        });
    if detail.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {detail}")
    }
}
