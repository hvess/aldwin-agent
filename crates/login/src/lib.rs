//! Logging in to a provider's account, so a subscription — SuperGrok today —
//! can stand in for an API key. See `.claude/spec/aldwin-login.md`.
//!
//! The crate knows no other part of Aldwin. It is handed an [`Account`] and
//! gives back a [`Prompt`] to show the developer, [`Credentials`] to keep,
//! and a [`Session`] that turns the credentials into request headers,
//! refreshing them as they age. Everything OAuth-shaped — the grants, the
//! endpoints, the client id, the wire JSON — stays in the private `oauth`
//! and `account` modules, and none of it crosses this surface: the same
//! rule aldwin-llm applies to a provider's wire types.

mod account;
mod login;
mod oauth;
mod session;

#[cfg(test)]
mod test_server;

pub use account::Account;
pub use login::{Login, LoginError, Prompt};
pub use session::{Credentials, Session, SessionError};
