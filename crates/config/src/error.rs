use std::path::PathBuf;
use thiserror::Error;

/// Why a config file or transcript could not be read or written.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// There is no home directory to put `~/.aldwin/` in.
    #[error("could not determine the home directory")]
    NoHomeDir,

    /// A file or directory could not be read, written or created.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the filesystem said.
        #[source]
        source: std::io::Error,
    },

    /// A file is not valid YAML, or not valid for its domain's schema.
    #[error("{path}: failed to parse YAML: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What the parser said, naming the line or the field.
        #[source]
        source: serde_yaml_ng::Error,
    },

    /// A value could not be turned into YAML to be written.
    #[error("{path}: failed to serialise config: {source}")]
    Serialize {
        /// The file it was for.
        path: PathBuf,
        /// What the serialiser said.
        #[source]
        source: serde_yaml_ng::Error,
    },

    /// A file states a version this build does not read — zero, or one
    /// written by a newer build.
    #[error("{path}: unknown config version {found} (this build understands version {expected})")]
    UnknownVersion {
        /// The file.
        path: PathBuf,
        /// The version the file states.
        found: u32,
        /// The newest version this build reads.
        expected: u32,
    },

    /// A `provider.yaml` names no environment variable for its key.
    #[error("{path}: api_key_env is missing or empty")]
    MissingApiKeyEnv {
        /// The `provider.yaml`.
        path: PathBuf,
    },

    /// There is no global `provider.yaml`, so no provider to fall back to.
    #[error("{path}: no provider configured; run first-launch init or write this file")]
    ProviderNotConfigured {
        /// Where the file was looked for.
        path: PathBuf,
    },
}
