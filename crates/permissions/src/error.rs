use thiserror::Error;

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("malformed grant entry {entry:?} (expected kind:pattern)")]
    MalformedGrant { entry: String },

    #[error("edit-class tools are never allowlistable; route approval through the per-call binary gate instead")]
    EditNotAllowlistable,

    #[error(transparent)]
    Config(#[from] amundsen_config::ConfigError),
}
