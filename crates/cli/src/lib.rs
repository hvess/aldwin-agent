//! Startup sequence, session bootstrap and slash-command dispatch
//! (`docs/spec/archive/aldwin-cli.md`).
//!
//! `bootstrap::run` is integration glue (real terminal, provider, MCP
//! processes) and is not unit tested; `context` and `slash` are.

mod bootstrap;
mod connect;
mod context;
mod error;
// Unix only: it needs `exec` and a symlink (ADR 0013).
#[cfg(unix)]
pub mod git_shim;
mod history;
mod slash;

pub use bootstrap::run;
pub use error::{ShimError, StartupError};
