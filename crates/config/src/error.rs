use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the home directory")]
    NoHomeDir,

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}: failed to parse YAML: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },

    #[error("{path}: failed to serialise config: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },

    #[error("{path}: unknown config version {found} (this build understands version {expected})")]
    UnknownVersion {
        path: PathBuf,
        found: u32,
        expected: u32,
    },

    #[error("{path}: api_key_env is missing or empty")]
    MissingApiKeyEnv { path: PathBuf },

    #[error("{path}: no provider configured; run first-launch init or write this file")]
    ProviderNotConfigured { path: PathBuf },
}
