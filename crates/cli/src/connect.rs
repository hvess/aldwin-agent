//! How a provider is reached, decided when its client is built (ADR
//! 0012): a connected account first, the API key second, and when the
//! provider offers an account and neither is there, one sentence that the
//! session answers every request with until one is. Also where a stored
//! connection becomes a session and a finished login becomes a record:
//! aldwin-config keeps the tokens and knows nothing of sessions,
//! aldwin-login runs the session and knows nothing of files.

use std::sync::Arc;

use aldwin_config::{Config, ConnectionRecord};
use aldwin_core::Event;
use aldwin_llm::{Auth, LlmClientInitError};
use aldwin_login::{Account, Credentials, Session};
use tokio::sync::mpsc;

/// What building a client on a provider found.
pub(crate) enum Reach {
    /// The provider is reached this way.
    Through(Auth),
    /// The provider offers an account, none is connected, and the key is
    /// not exported: the sentence every request answers with.
    Neither(String),
}

/// A provider that offers an account is reached through it when one is
/// connected, else through `api_key_env` when it is set, else not at all
/// — said, not refused, so the model can still be chosen and the sentence
/// names both ways to fix it. A provider that offers no account is reached
/// through its key as it always was, and a missing key fails the build.
pub(crate) fn reach(
    api_key_env: &str,
    account: Option<Account>,
    config: &Config,
    notices: &mpsc::Sender<Event>,
) -> Result<Reach, LlmClientInitError> {
    let Some(account) = account else {
        return Ok(Reach::Through(Auth::ApiKeyEnv(api_key_env.to_string())));
    };
    if let Some(stored) = config.connection(account.id()) {
        return Ok(Reach::Through(Auth::Connection(session(
            account, stored, config, notices,
        )?)));
    }
    if std::env::var(api_key_env).is_ok() {
        return Ok(Reach::Through(Auth::ApiKeyEnv(api_key_env.to_string())));
    }
    Ok(Reach::Neither(format!(
        "No {} account is connected and {api_key_env} is not set. Connect one with /connect {}, or set {api_key_env} and start Aldwin again.",
        account.name(),
        account.id()
    )))
}

pub(crate) fn record(credentials: &Credentials) -> ConnectionRecord {
    ConnectionRecord {
        access_token: credentials.access_token.clone(),
        refresh_token: credentials.refresh_token.clone(),
        expires_at: credentials.expires_at,
    }
}

fn credentials(record: ConnectionRecord) -> Credentials {
    Credentials {
        access_token: record.access_token,
        refresh_token: record.refresh_token,
        expires_at: record.expires_at,
    }
}

/// The session on `account`'s stored connection. Every refresh that
/// rotates the token is written back through `config`, and a revocation
/// removes the entry, so the next client built on the provider falls back
/// to its key. An entry that is no longer there — the developer deleted it
/// and reloaded — is not written back. A write that fails is said once,
/// since the next start would otherwise use a token the server has already
/// retired.
fn session(
    account: Account,
    stored: ConnectionRecord,
    config: &Config,
    notices: &mpsc::Sender<Event>,
) -> Result<Arc<Session>, LlmClientInitError> {
    let persist = {
        let (config, notices) = (config.clone(), notices.clone());
        move |fresh: Option<&Credentials>| {
            let written = match fresh {
                None => config.remove_connection(account.id()),
                Some(_) if config.connection(account.id()).is_none() => Ok(()),
                Some(fresh) => config.set_connection(account.id(), record(fresh)),
            };
            if let Err(e) = written {
                let message = format!(
                    "The {} account's connection could not be saved: {e}. The next start may ask you to connect it again.",
                    account.name()
                );
                let _ = notices.try_send(Event::Notice { message });
            }
        }
    };
    Session::new(account, credentials(stored), persist)
        .map(Arc::new)
        .map_err(|e| LlmClientInitError::ConnectionSession(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        (project, global, config)
    }

    fn stored() -> ConnectionRecord {
        ConnectionRecord {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: u64::MAX,
        }
    }

    #[test]
    fn a_record_and_credentials_carry_the_same_three_fields() {
        let original = Credentials {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 7,
        };
        assert_eq!(credentials(record(&original)), original);
    }

    /// A variable no test exports, so the order can be watched from the
    /// "neither" end without touching the process environment.
    const ABSENT: &str = "ALDWIN_CONNECT_TEST_KEY_NEVER_SET";

    #[tokio::test]
    async fn a_provider_offering_an_account_is_reached_account_first_then_key_then_said() {
        let (_project, _global, config) = config();
        let (tx, _rx) = mpsc::channel(1);

        let Reach::Neither(sentence) = reach(ABSENT, Some(Account::Xai), &config, &tx).unwrap()
        else {
            panic!("nothing connected and no key: the sentence");
        };
        assert_eq!(
            sentence,
            "No x.ai account is connected and ALDWIN_CONNECT_TEST_KEY_NEVER_SET is not set. Connect one with /connect xai, or set ALDWIN_CONNECT_TEST_KEY_NEVER_SET and start Aldwin again."
        );

        const PRESENT: &str = "ALDWIN_CONNECT_TEST_KEY_SET";
        std::env::set_var(PRESENT, "not-a-real-key");
        assert!(matches!(
            reach(PRESENT, Some(Account::Xai), &config, &tx).unwrap(),
            Reach::Through(Auth::ApiKeyEnv(var)) if var == PRESENT
        ));

        config.set_connection("xai", stored()).unwrap();
        let Reach::Through(Auth::Connection(session)) =
            reach(PRESENT, Some(Account::Xai), &config, &tx).unwrap()
        else {
            panic!("a connected account wins over an exported key");
        };
        assert_eq!(session.account(), Account::Xai);
        // Fresh credentials authenticate without touching any server.
        assert!(session
            .headers()
            .await
            .unwrap()
            .contains_key("authorization"));
    }

    /// No account on offer: the key, as it always was, whether or not it
    /// is exported — a missing one fails where the client is built.
    #[test]
    fn a_provider_without_an_account_is_reached_through_its_key() {
        let (_project, _global, config) = config();
        let (tx, _rx) = mpsc::channel(1);
        assert!(matches!(
            reach(ABSENT, None, &config, &tx).unwrap(),
            Reach::Through(Auth::ApiKeyEnv(var)) if var == ABSENT
        ));
    }
}
