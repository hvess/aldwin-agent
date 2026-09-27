//! Aldwin's settings on disk and its conversation transcripts
//! (`docs/spec/archive/aldwin-config.md`).
//!
//! Each domain (permissions, provider, MCP, TUI, connected accounts) is a
//! versioned YAML file at project scope (`<project>/.aldwin/`), global scope
//! (`~/.aldwin/`) or both. [`Config`] reads every layer once and answers from
//! memory; a write is atomic and updates disk and memory together. Leaf
//! crate: what aldwin-tui and aldwin-login own is persisted as this crate's
//! own types, never by depending on them.

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
