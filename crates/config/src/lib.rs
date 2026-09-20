mod annotated;
mod domain;
mod error;
mod fsio;
mod history;
mod scope;
mod store;

pub use domain::{
    Class, ContextFilesConfig, GrantEntry, McpConfig, McpServer, McpTransport, PermissionsConfig,
    ProviderConfig, ProviderKind, Rung, TuiConfig, PROVIDER_VERSION,
};
pub use error::ConfigError;
pub use history::{
    list as list_sessions, load as load_session, project_dir as history_project_dir, HistoryStore,
    SessionHeader, SessionSummary, HISTORY_VERSION,
};
pub use scope::Scope;
pub use store::{Config, GrantList, InitOutcome, ReloadFailure};
