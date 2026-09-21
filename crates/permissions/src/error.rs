use thiserror::Error;

#[derive(Debug, Error)]
pub enum PermissionError {
    /// `edit` is outside the permissions model (ADR 0004 §3): it is not a
    /// grant, not a rung, and not a row on any prompt. Reaching this means a
    /// caller tried to persist one anyway.
    #[error("editing is never granted — every edit shows a diff and waits")]
    EditNotGrantable,

    #[error(transparent)]
    Config(#[from] aldwin_config::ConfigError),
}
