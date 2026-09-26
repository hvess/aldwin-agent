//! The device authorization grant (RFC 8628) and the refresh grant (RFC
//! 6749 §6), as an account's authorization server speaks them. The one
//! module that knows a grant type, a wire field or an error code; what it
//! hands up is already in this crate's own words.
//!
//! Every call returns `Err(String)` for the failures a caller can only
//! report — the server unreachable, or answering outside the protocol —
//! and an `Ok` value for every answer the protocol names.

use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;

/// Where an account's authorization server is, and how this client is
/// known to it. Owned strings so a test can point one at a local server.
#[derive(Clone)]
pub(crate) struct Authority {
    pub device_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scope: String,
}

/// RFC 8628 §3.2 — what the device endpoint hands back.
#[derive(Deserialize)]
pub(crate) struct DeviceGrant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    /// Seconds between polls; the RFC's default when the server names none.
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// RFC 6749 §5.1 — a token, from either grant.
#[derive(Deserialize)]
pub(crate) struct TokenGrant {
    pub access_token: String,
    /// Recommended by the RFC, not required: a server may state no
    /// lifetime at all.
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// Absent from a refresh that does not rotate; a login always has one.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// RFC 6749 §5.2 — the shape of a refusal.
#[derive(Deserialize)]
struct Refusal {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// One poll of the token endpoint while a login waits (RFC 8628 §3.5).
pub(crate) enum Poll {
    Granted(TokenGrant),
    /// Not approved yet; ask again after the interval.
    Pending,
    /// Asked too often; the interval grows by five seconds.
    SlowDown,
    /// The developer refused in the browser.
    Denied,
    /// The code ran out before it was entered.
    Expired,
    /// The server could not be reached, or was having trouble of its own.
    /// Neither says anything about the login: ask again after the
    /// interval. A wait lasts half an hour, and a connection dropped once
    /// in that time must not end it.
    Unavailable,
}

/// What a refresh came back with.
pub(crate) enum Refresh {
    Granted(TokenGrant),
    /// The refresh token is no longer good — revoked, rotated away, or
    /// expired. Only a new login recovers from this.
    Revoked,
}

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// A token exchange is one small request, so a bound on the whole of it
/// is the right one — without it a connection the far side never answers
/// holds a wait past its deadline, or a session's lock for good.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The client both grants go over. Redirects are refused: a redirected
/// POST would carry the refresh token to wherever it was pointed.
///
/// `timeout` is always [`REQUEST_TIMEOUT`] outside a test. A test on the
/// paused clock passes `None`: a pending timer is what the paused clock
/// jumps to whenever a socket is still connecting, so a timeout there
/// fires before a local server can answer.
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
    // `invalid_grant` is the RFC's word for a refresh token the server no
    // longer honours, and a 401 or 403 *in the protocol's own shape* says
    // the same about this client: nothing but a new login helps. The shape
    // matters — a 403 from whatever sits in front of the server, an HTML
    // page from a bot check, is its passing trouble, and reading it as a
    // logout would send the developer to log in again for nothing.
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

/// A request that never got an answer, with the reason. reqwest's own
/// line stops at "error sending request"; why — timed out, refused,
/// reset — is down its source chain, and it is the part worth saying.
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

/// A failure's one line: the status, and the server's own description
/// when the body is a refusal, else the body's first stretch. Bodies are
/// cut short because an HTML error page is not worth carrying whole.
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
