use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use thiserror::Error;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::account::Account;
use crate::oauth::{self, Authority, Refresh, TokenGrant};

/// What a sign-in leaves behind, and what a [`Session`] runs on. Plain data
/// by design: whoever keeps it on disk reads and writes these three fields
/// and nothing else.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The bearer token a request is authenticated with.
    pub access_token: String,
    /// What a refresh trades for a new access token; the server may rotate
    /// it on every refresh.
    pub refresh_token: String,
    /// Unix seconds. The moment the access token stops working, as the
    /// server stated it when the token was issued.
    pub expires_at: u64,
}

/// How long before the stated expiry a token is treated as gone: a request
/// sent with seconds left could still arrive after them.
const EARLY: Duration = Duration::from_secs(120);

/// The lifetime given to a token whose server stated none. The RFC only
/// recommends stating one; a refresh is cheap, so an hour errs short.
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

/// The tokens are the one thing here that must not reach a log, so they
/// are what `Debug` leaves out.
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
    /// The refresh token was rejected. Only a new sign-in recovers from
    /// this.
    #[error("the account is no longer connected")]
    LoggedOut,
    /// A refresh failed without the server rejecting the refresh token —
    /// unreachable, a server error, or an unusable answer. The credentials
    /// are kept, so the next request tries again.
    #[error("the account's token could not be refreshed: {0}")]
    Failed(String),
}

/// Told of every change to what the server honours: `Some` with a
/// refreshed set, `None` when the server revoked the refresh token.
type Persist = Arc<dyn Fn(Option<&Credentials>) + Send + Sync>;

/// A logged-in account, ready to authenticate requests.
///
/// The credentials live behind one lock that is held across a refresh, so
/// two requests that find the token stale refresh it once between them:
/// the second waits, then reads what the first fetched. A refresh that the
/// server merely fails — a 500, a dropped connection — leaves the old
/// credentials in place for the next caller to try again; one it rejects
/// logs the session out for good, and every caller from then on is told
/// [`SessionError::LoggedOut`] until a new sign-in replaces the session.
pub struct Session {
    account: Account,
    http: Client,
    authority: Authority,
    /// `None` once the server has rejected the refresh token.
    credentials: Arc<Mutex<Option<Credentials>>>,
    /// Called with every refreshed set, since a rotated refresh token that
    /// is not written down is a connection lost at the next start — and
    /// with `None` on a revocation, so whoever stores the tokens stops
    /// offering ones the server will refuse.
    persist: Persist,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    /// A session on credentials a sign-in left behind. `persist` is called
    /// with every refreshed set, and with `None` when the server revokes
    /// the refresh token, so the caller keeps whatever it stores in step
    /// with what the server now honours.
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

    /// The headers that authenticate one request, on a token that will
    /// still be good when the request lands.
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
        // Spawned rather than awaited in place, so the request that found
        // the token stale can be cancelled — the developer stopping a turn
        // — without cancelling this. By the time the server answers it may
        // already have rotated the refresh token, and a rotation that never
        // reaches memory and disk is a connection lost. The lock travels with
        // the task, so nothing reads the credentials until it is done.
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

    /// The access token has been refused by the service it was for. The
    /// next request refreshes rather than waiting out the stated lifetime,
    /// which a revocation or a skewed clock makes wrong.
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
    // A server that does not rotate leaves the refresh token out, and the
    // one we have stays good.
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
    // Kept out of any `Debug` of the map it lands in.
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

    /// Everything a session told its persist hook, in order.
    type Persisted = Arc<StdMutex<Vec<Option<Credentials>>>>;

    /// A session over `server`, and the record of everything it persisted.
    /// No request timeout: most of these tests run on the paused clock.
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

    /// The developer stops the turn while its request is refreshing. The
    /// server has already rotated by then, so the refresh has to finish and
    /// land, or the next request would refresh on a token that is gone.
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

    /// A refresh the server merely fails is not a lost connection: the old
    /// credentials stay, and the next request tries again. A 403 that is
    /// not in the protocol's shape — a page from whatever fronts the
    /// server — is that kind of failure, not a logout.
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

    /// A refresh the server never answers is bounded, so it cannot hold
    /// the lock — and with it every request — for good. On the real
    /// clock, with the bound cut short: see `oauth::client`.
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
