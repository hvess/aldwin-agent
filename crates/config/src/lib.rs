//! Aldwin's settings on disk, and its conversation transcripts. See
//! `.claude/spec/archive/aldwin-config.md`.
//!
//! Four domains — permissions, provider, MCP servers and the TUI — plus the
//! connected accounts, each a versioned YAML file at project scope
//! (`<project>/.aldwin/`), global scope (`~/.aldwin/`) or both. [`Config`]
//! reads every layer once and answers from memory; a write is atomic and
//! lands on disk and in memory together. What aldwin-tui and aldwin-login
//! own, it persists as its own types rather than depending on those crates.

mod annotated;
mod domain;
mod error;
mod fsio;
mod history;
mod scope;
mod store;

pub use domain::{
    ConnectionRecord, McpConfig, McpServer, McpTransport, PermissionsConfig, ProviderConfig,
    ProviderKind, TuiConfig, PROVIDER_VERSION,
};
pub use error::ConfigError;
pub use history::{
    list as list_sessions, load as load_session, project_dir as history_project_dir, HistoryStore,
    SessionHeader, SessionSummary, HISTORY_VERSION,
};
pub use scope::Scope;
pub use store::{Config, InitOutcome, ReloadFailure};
