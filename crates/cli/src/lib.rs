//! Binary crate — startup sequence, session bootstrap, impl wiring, slash-
//! command dispatch. See `.claude/spec/amundsen-cli.md`.
//!
//! Split so the testable pieces (`context`, `slash`) don't need a real
//! terminal or a live Anthropic API key: `bootstrap::run` — the actual
//! startup sequence, real TUI, real subprocess-spawning MCP bridge — is
//! integration glue in the same sense as `amundsen_tui::run`, not unit
//! tested here.

mod bootstrap;
mod context;
mod error;
mod slash;

pub use bootstrap::run;
pub use error::StartupError;
