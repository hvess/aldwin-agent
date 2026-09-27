//! Logs in to a provider account so a subscription stands in for an API key
//! (ADR 0012, `.claude/spec/aldwin-login.md`). A leaf crate: it depends on
//! no other Aldwin crate.
//!
//! OAuth wire types, endpoints and the client id stay in the private `oauth`
//! and `account` modules; never export them from this surface.

mod account;
mod login;
mod oauth;
mod session;

#[cfg(test)]
mod test_server;

pub use account::Account;
pub use login::{Login, LoginError, Prompt};
pub use session::{Credentials, Session, SessionError};
