use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use thiserror::Error;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::account::Account;
use crate::oauth::{self, Authority, Refresh, TokenGrant};

/// The tokens a sign-in yields and a [`Session`] runs on. Plain data: the
/// caller persists exactly these three fields.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The bearer token for requests.
    pub access_token: String,
    /// Traded for a new access token; the server may rotate it on any
    /// refresh.
    pub refresh_token: String,
    /// Unix seconds when the access token expires, per the server.
    pub expires_at: u64,
}

/// Margin before `expires_at` at which a token counts as stale, so a request
/// never lands after expiry.
const EARLY: Duration = Duration::from_secs(120);

/// Lifetime assumed when the server states none; errs short because a
/// refresh is cheap.
const UNSTATED_LIFETIME: Duration = Duration::from_secs(60 * 60);

impl Credentials {
    pub(crate) fn from_grant(grant: TokenGrant, refresh_token: String) -> Self {
        let lifetime = grant.expires_in.unwrap_or(UNSTATED_LIFETIME.as_secs());
        Self {
            access_token: grant.access_token,
            refresh_token,
            expires_at: now().saturating_add(lifetime),
        }
    }

    fn is_stale(&self) -> bool {
        now().saturating_add(EARLY.as_secs()) >= self.expires_at
    }
}

/// Omits both tokens: they must never reach a log.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Why a session could not authenticate a request.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The refresh token was rejected; only a new sign-in recovers.
    #[error("the account is no longer connected")]
    LoggedOut,
    /// A refresh failed without the refresh token being rejected. The
    /// credentials are kept, so the next request tries again.
    #[error("the account's token could not be refreshed: {0}")]
    Failed(String),
}

/// Called with `Some` refreshed credentials, or `None` on revocation.
type Persist = Arc<dyn Fn(Option<&Credentials>) + Send + Sync>;

/// A logged-in account, ready to authenticate requests.
///
/// The lock on the credentials is held across a refresh, so concurrent
/// requests finding the token stale refresh it once. A rejected refresh
/// token logs the session out for good ([`SessionError::LoggedOut`]).
pub struct Session {
    account: Account,
    http: Client,
    authority: Authority,
    /// `None` once the server has rejected the refresh token.
    credentials: Arc<Mutex<Option<Credentials>>>,
    /// Must see every refreshed set: an unpersisted rotated refresh token
    /// loses the connection at the next start.
    persist: Persist,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    /// A session on stored credentials. `persist` is called with every
    /// refreshed set, and with `None` when the refresh token is revoked.
    ///
    /// # Errors
    ///
    /// [`SessionError::Failed`] when the HTTP client cannot be built.
    pub fn new(
        account: Account,
        credentials: Credentials,
        persist: impl Fn(Option<&Credentials>) + Send + Sync + 'static,
    ) -> Result<Self, SessionError> {
        Self::at(
            account,
            account.authority(),
            credentials,
            Some(oauth::REQUEST_TIMEOUT),
            persist,
        )
    }

    pub(crate) fn at(
        account: Account,
        authority: Authority,
        credentials: Credentials,
        timeout: Option<Duration>,
        persist: impl Fn(Option<&Credentials>) + Send + Sync + 'static,
    ) -> Result<Self, SessionError> {
        Ok(Self {
            account,
            http: oauth::client(timeout).map_err(SessionError::Failed)?,
            authority,
            credentials: Arc::new(Mutex::new(Some(credentials))),
            persist: Arc::new(persist),
        })
    }

    /// Whose session this is.
    pub fn account(&self) -> Account {
        self.account
    }

    /// The headers that authenticate one request, refreshing a stale token
    /// first.
    ///
    /// # Errors
    ///
    /// [`SessionError::LoggedOut`] when the server has rejected the refresh
    /// token, now or earlier; [`SessionError::Failed`] when a needed
    /// refresh fails otherwise, or the token is not a valid header value.
    pub async fn headers(&self) -> Result<HeaderMap, SessionError> {
        let guard = self.credentials.clone().lock_owned().await;
        let Some(credentials) = guard.as_ref() else {
            return Err(SessionError::LoggedOut);
        };
        if !credentials.is_stale() {
            return bearer(&credentials.access_token);
        }
        // Spawned, not awaited in place: cancelling the caller must not
        // cancel a refresh the server may already have rotated, or the new
        // token is lost. The guard moves into the task, keeping the lock.
        let refresh = refresh_under(
            guard,
            self.http.clone(),
            self.authority.clone(),
            self.persist.clone(),
        );
        tokio::spawn(refresh)
            .await
            .map_err(|e| SessionError::Failed(format!("the refresh did not finish: {e}")))?
    }

    /// Marks the access token stale, so the next request refreshes. Call it
    /// when the service refuses the token (revocation, clock skew).
    pub async fn invalidate(&self) {
        if let Some(credentials) = self.credentials.lock().await.as_mut() {
            credentials.expires_at = 0;
        }
    }
}

async fn refresh_under(
    mut guard: OwnedMutexGuard<Option<Credentials>>,
    http: Client,
    authority: Authority,
    persist: Persist,
) -> Result<HeaderMap, SessionError> {
    let Some(previous) = guard.as_ref() else {
        return Err(SessionError::LoggedOut);
    };
    let previous_refresh_token = previous.refresh_token.clone();
    let grant = match oauth::refresh(&http, &authority, &previous_refresh_token)
        .await
        .map_err(SessionError::Failed)?
    {
        Refresh::Granted(grant) => grant,
        Refresh::Revoked => {
            *guard = None;
            persist(None);
            return Err(SessionError::LoggedOut);
        }
    };
    // No refresh token in the grant: the server did not rotate; keep the current one.
    let refresh_token = grant
        .refresh_token
        .clone()
        .unwrap_or(previous_refresh_token);
    let fresh = Credentials::from_grant(grant, refresh_token);
    persist(Some(&fresh));
    let headers = bearer(&fresh.access_token);
    *guard = Some(fresh);
    headers
}

fn bearer(access_token: &str) -> Result<HeaderMap, SessionError> {
    let mut value = HeaderValue::from_str(&format!("Bearer {access_token}"))
        .map_err(|_| SessionError::Failed("the access token is not a valid header value".into()))?;
    // Hides the token from `Debug`.
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{self, Canned, FakeServer};
    use std::sync::Mutex as StdMutex;

    fn authority(server: &FakeServer) -> Authority {
        Authority {
            device_url: server.url("/device"),
            token_url: server.url("/token"),
            client_id: "the-client".into(),
            scope: "openid".into(),
        }
    }

    fn credentials(expires_at: u64) -> Credentials {
        Credentials {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at,
        }
    }

    const FRESH: u64 = u64::MAX;
    const STALE: u64 = 0;

    fn refreshed_body(rotated: bool) -> String {
        let rotated = if rotated {
            r#","refresh_token":"new-refresh""#
        } else {
            ""
        };
        format!(
            r#"{{"access_token":"new-access","token_type":"bearer","expires_in":21600{rotated}}}"#
        )
    }

    fn refreshed(rotated: bool) -> Canned {
        Canned::Status(200, refreshed_body(rotated))
    }

    const REFRESH: &str =
        "POST /token grant_type=refresh_token&refresh_token=old-refresh&client_id=the-client";

    /// Every `persist` call, in order.
    type Persisted = Arc<StdMutex<Vec<Option<Credentials>>>>;

    /// No request timeout: see `oauth::client` on the paused clock.
    fn session(server: &FakeServer, credentials: Credentials) -> (Arc<Session>, Persisted) {
        session_bounded(server, credentials, None)
    }

    fn session_bounded(
        server: &FakeServer,
        credentials: Credentials,
        timeout: Option<Duration>,
    ) -> (Arc<Session>, Persisted) {
        let persisted = Arc::new(StdMutex::new(Vec::new()));
        let record = persisted.clone();
        let session = Session::at(
            Account::Xai,
            authority(server),
            credentials,
            timeout,
            move |c: Option<&Credentials>| record.lock().unwrap().push(c.cloned()),
        )
        .unwrap();
        (Arc::new(session), persisted)
    }

    fn authorization(headers: &HeaderMap) -> &str {
        headers[AUTHORIZATION].to_str().unwrap()
    }

    #[tokio::test]
    async fn a_fresh_token_is_used_as_it_is() {
        let server = test_server::spawn(vec![]);
        let (session, persisted) = session(&server, credentials(FRESH));

        let headers = session.headers().await.unwrap();

        assert_eq!(authorization(&headers), "Bearer old-access");
        assert!(server.requests().is_empty(), "nothing to refresh");
        assert!(persisted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_token_is_refreshed_rotated_and_persisted() {
        let server = test_server::spawn(vec![refreshed(true)]);
        let (session, persisted) = session(&server, credentials(STALE));

        let headers = session.headers().await.unwrap();

        assert_eq!(authorization(&headers), "Bearer new-access");
        assert_eq!(server.requests(), vec![REFRESH]);
        let persisted = persisted.lock().unwrap();
        assert_eq!(persisted.len(), 1);
        let fresh = persisted[0].as_ref().expect("a refreshed set");
        assert_eq!(fresh.access_token, "new-access");
        assert_eq!(fresh.refresh_token, "new-refresh");
        assert!(fresh.expires_at > now() + 21_000);
    }

    #[tokio::test]
    async fn a_refresh_that_does_not_rotate_keeps_the_refresh_token() {
        let server = test_server::spawn(vec![refreshed(false)]);
        let (session, persisted) = session(&server, credentials(STALE));

        session.headers().await.unwrap();

        assert_eq!(
            persisted.lock().unwrap()[0].as_ref().unwrap().refresh_token,
            "old-refresh"
        );
    }

    #[tokio::test]
    async fn a_token_granted_without_a_lifetime_gets_an_hour() {
        let server = test_server::spawn(vec![Canned::Status(
            200,
            r#"{"access_token":"new-access","token_type":"bearer"}"#.into(),
        )]);
        let (session, persisted) = session(&server, credentials(STALE));

        session.headers().await.unwrap();

        let expires_at = persisted.lock().unwrap()[0].as_ref().unwrap().expires_at;
        assert!(expires_at > now() + 3_500 && expires_at <= now() + 3_600);
    }

    #[tokio::test]
    async fn once_refreshed_the_next_request_needs_no_refresh() {
        let server = test_server::spawn(vec![refreshed(true)]);
        let (session, _) = session(&server, credentials(STALE));

        session.headers().await.unwrap();
        let headers = session.headers().await.unwrap();

        assert_eq!(authorization(&headers), "Bearer new-access");
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn two_requests_finding_the_token_stale_refresh_it_once() {
        let server = test_server::spawn(vec![refreshed(true)]);
        let (session, persisted) = session(&server, credentials(STALE));

        let (first, second) = tokio::join!(session.headers(), session.headers());

        assert_eq!(authorization(&first.unwrap()), "Bearer new-access");
        assert_eq!(authorization(&second.unwrap()), "Bearer new-access");
        assert_eq!(
            server.requests().len(),
            1,
            "the second waited for the first"
        );
        assert_eq!(persisted.lock().unwrap().len(), 1);
    }

    /// Pins the spawned refresh in `headers`: a cancelled caller must not
    /// lose a rotation the server already made.
    #[tokio::test(start_paused = true)]
    async fn a_request_cancelled_mid_refresh_still_lands_the_rotation() {
        let server = test_server::spawn(vec![Canned::Delayed(
            Duration::from_secs(1),
            200,
            refreshed_body(true),
        )]);
        let (session, persisted) = session(&server, credentials(STALE));

        let cancelled = tokio::spawn({
            let session = session.clone();
            async move { session.headers().await }
        });
        while server.requests().is_empty() {
            tokio::task::yield_now().await;
        }
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());

        let headers = session.headers().await.unwrap();

        assert_eq!(authorization(&headers), "Bearer new-access");
        assert_eq!(server.requests().len(), 1, "no second refresh");
        assert_eq!(persisted.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_rejected_refresh_token_means_logged_out_and_stays_so() {
        for refusal in [
            Canned::Status(400, r#"{"error":"invalid_grant"}"#.into()),
            Canned::Status(401, r#"{"error":"invalid_token"}"#.into()),
            Canned::Status(403, r#"{"error":"access_denied"}"#.into()),
        ] {
            let server = test_server::spawn(vec![refusal, refreshed(true)]);
            let (session, persisted) = session(&server, credentials(STALE));

            assert!(matches!(
                session.headers().await,
                Err(SessionError::LoggedOut)
            ));
            assert!(matches!(
                session.headers().await,
                Err(SessionError::LoggedOut)
            ));
            assert_eq!(
                server.requests().len(),
                1,
                "a logged-out session does not keep asking"
            );
            assert_eq!(
                *persisted.lock().unwrap(),
                vec![None],
                "the revocation is reported once, so the stored tokens can go"
            );
        }
    }

    /// A 403 without an RFC 6749 §5.2 body is transient, not a logout.
    #[tokio::test]
    async fn a_failed_refresh_is_retried_by_the_next_request() {
        let server = test_server::spawn(vec![
            Canned::Status(500, "gateway trouble".into()),
            Canned::Status(403, "<html>checking your browser</html>".into()),
            refreshed(true),
        ]);
        let (session, _) = session(&server, credentials(STALE));

        let Err(SessionError::Failed(message)) = session.headers().await else {
            panic!("a 500 must fail the refresh, not log out");
        };
        assert_eq!(message, "HTTP 500 Internal Server Error: gateway trouble");
        let Err(SessionError::Failed(message)) = session.headers().await else {
            panic!("a bare 403 must fail the refresh, not log out");
        };
        assert_eq!(
            message,
            "HTTP 403 Forbidden: <html>checking your browser</html>"
        );

        let headers = session.headers().await.unwrap();
        assert_eq!(authorization(&headers), "Bearer new-access");
    }

    #[tokio::test]
    async fn a_hung_up_refresh_fails_naming_the_request() {
        let server = test_server::spawn(vec![Canned::HangUp]);
        let (session, _) = session(&server, credentials(STALE));
        let Err(SessionError::Failed(message)) = session.headers().await else {
            panic!("a hang-up must fail the refresh");
        };
        assert!(message.contains(&server.url("/token")), "{message}");
    }

    /// An unanswered refresh must not hold the lock for good. Real clock,
    /// short timeout: a paused clock fires it early (`oauth::client`).
    #[tokio::test]
    async fn a_stalled_refresh_times_out_and_the_next_request_tries_again() {
        let timeout = Duration::from_millis(200);
        let server = test_server::spawn(vec![Canned::Stall, refreshed(true)]);
        let (session, _) = session_bounded(&server, credentials(STALE), Some(timeout));
        let started = std::time::Instant::now();

        let Err(SessionError::Failed(message)) = session.headers().await else {
            panic!("a stalled refresh must time out");
        };
        assert!(message.contains("timed out"), "{message}");
        assert!(started.elapsed() >= timeout);

        let headers = session.headers().await.unwrap();
        assert_eq!(authorization(&headers), "Bearer new-access");
    }

    #[tokio::test]
    async fn an_invalidated_token_is_refreshed_on_the_next_request() {
        let server = test_server::spawn(vec![refreshed(true)]);
        let (session, _) = session(&server, credentials(FRESH));

        session.invalidate().await;
        let headers = session.headers().await.unwrap();

        assert_eq!(authorization(&headers), "Bearer new-access");
        assert_eq!(server.requests(), vec![REFRESH]);
    }

    #[test]
    fn neither_debug_output_carries_a_token() {
        let printed = format!("{:?}", credentials(7));
        assert!(!printed.contains("old-access"), "{printed}");
        assert!(!printed.contains("old-refresh"), "{printed}");
        assert!(printed.contains("expires_at: 7"), "{printed}");

        let printed = format!("{:?}", bearer("old-access").unwrap());
        assert!(!printed.contains("old-access"), "{printed}");
    }

    #[test]
    fn a_token_within_two_minutes_of_expiry_is_stale() {
        let now = now();
        assert!(credentials(now + 118).is_stale());
        assert!(!credentials(now + 122).is_stale());
    }
}
