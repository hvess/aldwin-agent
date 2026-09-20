mod annotated;
mod domain;
mod error;
mod fsio;
mod scope;
mod store;

pub use domain::{
    Class, ContextFilesConfig, GrantEntry, McpConfig, McpServer, McpTransport, PermissionsConfig,
    ProviderConfig, ProviderKind, Rung, TuiConfig, PROVIDER_VERSION,
};
pub use error::ConfigError;
pub use scope::Scope;
pub use store::{Config, GrantList, InitOutcome, ReloadFailure};
