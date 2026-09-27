//! Binary crate — startup sequence, session bootstrap, impl wiring, slash-
//! command dispatch. See `.claude/spec/aldwin-cli.md`.
//!
//! Split so the testable pieces (`context`, `slash`) don't need a real
//! terminal or a live Anthropic API key: `bootstrap::run` — the actual
//! startup sequence, real TUI, real subprocess-spawning MCP bridge — is
//! integration glue in the same sense as `aldwin_tui::run`, not unit
//! tested here.

mod bootstrap;
mod connect;
mod context;
mod error;
// Unix only, as the sandbox is: it needs `exec` and a symlink (ADR 0013).
#[cfg(unix)]
pub mod git_shim;
mod history;
mod slash;

pub use bootstrap::run;
pub use error::{ShimError, StartupError};
