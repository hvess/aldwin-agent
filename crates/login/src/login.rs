use std::fmt;
use std::time::Duration;

use reqwest::Client;
use thiserror::Error;
use tokio::time::Instant;

use crate::account::Account;
use crate::oauth::{self, Authority, Poll};
use crate::session::Credentials;

/// What the developer is shown to approve a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Where the developer signs in; carries the code when the server
    /// offers `verification_uri_complete`.
    pub url: String,
    /// The code the developer enters at the URL.
    pub code: String,
    /// The code's lifetime, from when it was issued.
    pub expires_in: Duration,
}

/// Why a login ended without credentials.
#[derive(Debug, Error)]
pub enum LoginError {
    /// The developer turned the sign-in down at the account's page.
    #[error("the sign-in was refused in the browser")]
    Denied,
    /// The code ran out before the developer approved it.
    #[error("the code expired before it was entered")]
    Expired,
    /// The server could not be reached or answered outside the protocol;
    /// the message says which.
    #[error("{0}")]
    Failed(String),
}

/// One device-authorization login (RFC 8628), from code issued to approval.
///
/// Never listen on a port or open a browser here: the login must work over
/// SSH and inside the ADR 0011 sandbox.
pub struct Login {
    http: Client,
    authority: Authority,
    device_code: String,
    interval: Duration,
    expires_in: Duration,
}

/// Omits `device_code`: it is what the poll trades for tokens.
impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("interval", &self.interval)
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// RFC 8628 §3.5: what `slow_down` adds to the interval.
const SLOW_DOWN: Duration = Duration::from_secs(5);

impl Login {
    /// Asks the account's server for a code; show the [`Prompt`], then call
    /// [`Login::wait`].
    ///
    /// # Errors
    ///
    /// [`LoginError::Failed`] when the HTTP client cannot be built, or the
    /// server cannot be reached or does not answer with a code.
    pub async fn start(account: Account) -> Result<(Login, Prompt), LoginError> {
        Self::start_at(account.authority(), Some(oauth::REQUEST_TIMEOUT)).await
    }

    pub(crate) async fn start_at(
        authority: Authority,
        timeout: Option<Duration>,
    ) -> Result<(Login, Prompt), LoginError> {
        let http = oauth::client(timeout).map_err(LoginError::Failed)?;
        let grant = oauth::device_grant(&http, &authority)
            .await
            .map_err(LoginError::Failed)?;
        let prompt = Prompt {
            url: grant
                .verification_uri_complete
                .unwrap_or(grant.verification_uri),
            code: grant.user_code,
            expires_in: Duration::from_secs(grant.expires_in),
        };
        let login = Login {
            http,
            authority,
            device_code: grant.device_code,
            // A server may name zero; never poll flat out.
            interval: Duration::from_secs(grant.interval.max(1)),
            expires_in: prompt.expires_in,
        };
        Ok((login, prompt))
    }

    /// Polls until the account answers or the code runs out. Spawn it: it
    /// can run for the prompt's whole lifetime, and an unreachable server is
    /// waited out, not given up on.
    ///
    /// # Errors
    ///
    /// [`LoginError::Denied`] when the developer refuses the sign-in,
    /// [`LoginError::Expired`] when the code runs out first, and
    /// [`LoginError::Failed`] when the server answers outside the protocol
    /// or grants no refresh token.
    pub async fn wait(self) -> Result<Credentials, LoginError> {
        // `None` on overflow; the server's `expired_token` is then the deadline.
        let deadline = Instant::now().checked_add(self.expires_in);
        let mut interval = self.interval;
        loop {
            tokio::time::sleep(interval).await;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(LoginError::Expired);
            }
            match oauth::poll(&self.http, &self.authority, &self.device_code)
                .await
                .map_err(LoginError::Failed)?
            {
                Poll::Granted(grant) => {
                    let refresh_token = grant.refresh_token.clone().ok_or_else(|| {
                        LoginError::Failed("the server issued no refresh token".into())
                    })?;
                    return Ok(Credentials::from_grant(grant, refresh_token));
                }
                Poll::Pending | Poll::Unavailable => {}
                Poll::SlowDown => interval += SLOW_DOWN,
                Poll::Denied => return Err(LoginError::Denied),
                Poll::Expired => return Err(LoginError::Expired),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::now;
    use crate::test_server::{self, Canned, FakeServer};

    fn authority(server: &FakeServer) -> Authority {
        Authority {
            device_url: server.url("/device"),
            token_url: server.url("/token"),
            client_id: "the-client".into(),
            scope: "openid offline_access".into(),
        }
    }

    fn device_grant(complete: bool) -> Canned {
        let complete = if complete {
            r#","verification_uri_complete":"https://accounts.example/device?user_code=ABCD-EFGH""#
        } else {
            ""
        };
        Canned::Status(
            200,
            format!(
                r#"{{"device_code":"dev-1","user_code":"ABCD-EFGH","verification_uri":"https://accounts.example/device"{complete},"expires_in":1800,"interval":5}}"#
            ),
        )
    }

    fn pending() -> Canned {
        Canned::Status(400, r#"{"error":"authorization_pending"}"#.into())
    }

    fn granted() -> Canned {
        Canned::Status(
            200,
            r#"{"access_token":"at-1","token_type":"bearer","expires_in":21600,"refresh_token":"rt-1"}"#.into(),
        )
    }

    const POLL: &str = "POST /token grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=dev-1&client_id=the-client";

    #[tokio::test]
    async fn start_asks_for_a_code_as_this_client_and_shows_the_complete_url() {
        let server = test_server::spawn(vec![device_grant(true)]);
        let (_, prompt) = Login::start_at(authority(&server), None).await.unwrap();

        assert_eq!(
            prompt,
            Prompt {
                url: "https://accounts.example/device?user_code=ABCD-EFGH".into(),
                code: "ABCD-EFGH".into(),
                expires_in: Duration::from_secs(1800),
            }
        );
        assert_eq!(
            server.requests(),
            vec!["POST /device client_id=the-client&scope=openid+offline_access"]
        );
    }

    #[tokio::test]
    async fn the_plain_url_is_shown_when_no_complete_one_is_offered() {
        let server = test_server::spawn(vec![device_grant(false)]);
        let (_, prompt) = Login::start_at(authority(&server), None).await.unwrap();
        assert_eq!(prompt.url, "https://accounts.example/device");
    }

    #[tokio::test(start_paused = true)]
    async fn wait_polls_with_the_device_grant_until_approved() {
        let server = test_server::spawn(vec![device_grant(true), pending(), pending(), granted()]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let started = Instant::now();

        let credentials = login.wait().await.unwrap();

        assert_eq!(credentials.access_token, "at-1");
        assert_eq!(credentials.refresh_token, "rt-1");
        assert!(
            credentials.expires_at > now() + 21_000,
            "the expiry the grant stated"
        );
        assert_eq!(server.requests()[1..], [POLL, POLL, POLL]);
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(15),
            "one interval before each of the three polls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_names_no_interval_is_polled_every_five_seconds() {
        let server = test_server::spawn(vec![
            Canned::Status(
                200,
                r#"{"device_code":"dev-1","user_code":"X","verification_uri":"u","expires_in":1800}"#.into(),
            ),
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let started = Instant::now();

        login.wait().await.unwrap();

        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn a_zero_interval_is_polled_once_a_second_not_flat_out() {
        let server = test_server::spawn(vec![
            Canned::Status(
                200,
                r#"{"device_code":"dev-1","user_code":"X","verification_uri":"u","expires_in":1800,"interval":0}"#.into(),
            ),
            pending(),
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let started = Instant::now();

        login.wait().await.unwrap();

        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_down_widens_the_interval() {
        let server = test_server::spawn(vec![
            device_grant(true),
            Canned::Status(400, r#"{"error":"slow_down"}"#.into()),
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let started = Instant::now();

        login.wait().await.unwrap();

        assert_eq!(
            started.elapsed(),
            Duration::from_secs(5 + 10),
            "five seconds to the first poll, then ten to the next"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_in_the_browser_is_denied() {
        let server = test_server::spawn(vec![
            device_grant(true),
            Canned::Status(400, r#"{"error":"access_denied"}"#.into()),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        assert!(matches!(login.wait().await, Err(LoginError::Denied)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_code_the_server_has_forgotten_is_expired() {
        let server = test_server::spawn(vec![
            device_grant(true),
            Canned::Status(400, r#"{"error":"expired_token"}"#.into()),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        assert!(matches!(login.wait().await, Err(LoginError::Expired)));
    }

    /// Pins the local deadline: a server answering `authorization_pending`
    /// forever must not hold the wait open.
    #[tokio::test(start_paused = true)]
    async fn the_wait_gives_up_when_the_code_is_out_of_time() {
        let server = test_server::spawn(vec![
            Canned::Status(
                200,
                r#"{"device_code":"dev-1","user_code":"X","verification_uri":"u","expires_in":12,"interval":5}"#.into(),
            ),
            pending(),
            pending(),
            pending(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();

        assert!(matches!(login.wait().await, Err(LoginError::Expired)));
        assert_eq!(
            server.requests().len(),
            3,
            "two polls fit before twelve seconds"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn trouble_reaching_the_server_is_waited_out_not_given_up_on() {
        let server = test_server::spawn(vec![
            device_grant(true),
            Canned::HangUp,
            Canned::Status(502, "bad gateway".into()),
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let started = Instant::now();

        let credentials = login.wait().await.unwrap();

        assert_eq!(credentials.access_token, "at-1");
        assert_eq!(server.requests().len(), 4);
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(3 * 5),
            "one interval before each poll, the failed ones included"
        );
    }

    /// Real clock, short timeout: a paused clock fires it early (`oauth::client`).
    #[tokio::test]
    async fn a_stalled_poll_is_given_up_on_and_the_wait_goes_on() {
        let timeout = Duration::from_millis(200);
        let server = test_server::spawn(vec![
            Canned::Status(
                200,
                r#"{"device_code":"dev-1","user_code":"X","verification_uri":"u","expires_in":1800,"interval":0}"#.into(),
            ),
            Canned::Stall,
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), Some(timeout))
            .await
            .unwrap();

        let credentials = login.wait().await.unwrap();

        assert_eq!(credentials.access_token, "at-1");
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_lifetime_too_long_for_the_clock_does_not_panic() {
        let server = test_server::spawn(vec![
            Canned::Status(
                200,
                format!(
                    r#"{{"device_code":"dev-1","user_code":"X","verification_uri":"u","expires_in":{},"interval":5}}"#,
                    u64::MAX
                ),
            ),
            granted(),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        assert!(login.wait().await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_grant_without_a_refresh_token_is_a_failure_not_a_login() {
        let server = test_server::spawn(vec![
            device_grant(true),
            Canned::Status(
                200,
                r#"{"access_token":"at-1","token_type":"bearer","expires_in":21600}"#.into(),
            ),
        ]);
        let (login, _) = Login::start_at(authority(&server), None).await.unwrap();
        let Err(LoginError::Failed(message)) = login.wait().await else {
            panic!("a grant with nothing to refresh on is not a login");
        };
        assert_eq!(message, "the server issued no refresh token");
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_fails_the_start_naming_the_request() {
        let server = test_server::spawn(vec![Canned::HangUp]);
        let Err(LoginError::Failed(message)) = Login::start_at(authority(&server), None).await
        else {
            panic!("a hang-up must fail the start");
        };
        assert!(
            message.contains(&server.url("/device")),
            "reqwest's own message names the request that failed: {message}"
        );
    }

    #[tokio::test]
    async fn a_refusal_at_the_start_carries_the_servers_description() {
        let server = test_server::spawn(vec![Canned::Status(
            400,
            r#"{"error":"invalid_client","error_description":"unknown client"}"#.into(),
        )]);
        let Err(LoginError::Failed(message)) = Login::start_at(authority(&server), None).await
        else {
            panic!("a refusal must fail the start");
        };
        assert_eq!(message, "HTTP 400 Bad Request: unknown client");
    }
}
