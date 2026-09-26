use std::time::Duration;

use reqwest::Client;
use thiserror::Error;
use tokio::time::Instant;

use crate::account::Account;
use crate::oauth::{self, Authority, Poll};
use crate::session::Credentials;

/// What the developer is shown: where to go, what to enter, and how long
/// the code is good for. The URL carries the code when the server offers
/// that form, so a click is enough; the code is still shown for a screen
/// that has to be read across.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub url: String,
    pub code: String,
    pub expires_in: Duration,
}

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("the sign-in was refused in the browser")]
    Denied,
    #[error("the code expired before it was entered")]
    Expired,
    #[error("{0}")]
    Failed(String),
}

/// One login, from the code being issued to the account approving it.
///
/// The device authorization flow (RFC 8628) is the one flow this crate
/// runs: nothing listens on a port and no browser is opened from here, so
/// the login works over SSH and inside the sandbox every process Aldwin
/// starts runs in. The developer opens the URL wherever they like.
pub struct Login {
    http: Client,
    authority: Authority,
    device_code: String,
    interval: Duration,
    expires_in: Duration,
}

/// RFC 8628 §3.5: what `slow_down` adds to the interval.
const SLOW_DOWN: Duration = Duration::from_secs(5);

impl Login {
    /// Asks the account's server for a code. The prompt is for the screen;
    /// the login is for [`Login::wait`].
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
            // The RFC lets a server name zero, which would poll flat out.
            interval: Duration::from_secs(grant.interval.max(1)),
            expires_in: prompt.expires_in,
        };
        Ok((login, prompt))
    }

    /// Polls until the account answers, or the code runs out. Meant to be
    /// spawned: it can take as long as the prompt said, and a server that
    /// cannot be reached for a while is waited out rather than given up on.
    pub async fn wait(self) -> Result<Credentials, LoginError> {
        // `None` only for a lifetime too long to add to the clock, and
        // then the server's own `expired_token` is the deadline.
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

    /// The deadline is kept here too, so a server that keeps answering
    /// `authorization_pending` past the code's lifetime cannot hold a wait
    /// open forever.
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

    /// A dropped connection or a 502 in the middle of a half-hour wait is
    /// waited out, since neither says anything about the login.
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

    /// A poll the server never answers is bounded by the request timeout,
    /// not by the wait's patience, and then waited out like any other
    /// trouble. On the real clock, with the bound cut short: see
    /// `oauth::client`.
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
