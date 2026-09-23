use thiserror::Error;

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error(transparent)]
    Config(#[from] aldwin_config::ConfigError),
}
