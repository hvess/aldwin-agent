//! Typed shapes for the config domains, each versioned independently.
//! `deny_unknown_fields` is deliberate: a stray or renamed field must fail at
//! load, not be dropped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::ConfigError;
use crate::fsio;

pub const PERMISSIONS_VERSION: u32 = 2;
/// The `provider.yaml` schema version this build reads and writes.
pub const PROVIDER_VERSION: u32 = 1;
pub const MCP_VERSION: u32 = 1;
pub const TUI_VERSION: u32 = 1;
pub const CONNECTIONS_VERSION: u32 = 1;

/// Permissions for one scope. `roots:` is the only key read (ADR 0011).
///
/// `default:`, `allow:` (ADR 0004) and `deny:` are still parsed, as any
/// value, so older files load under `deny_unknown_fields`; do not remove
/// them. [`has_stale_keys`] reports whether they say anything.
///
/// [`has_stale_keys`]: PermissionsConfig::has_stale_keys
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsConfig {
    /// Schema version; this build reads `PERMISSIONS_VERSION`.
    pub version: u32,
    /// Extra workspace directories beyond the project root (ADR 0007).
    /// Honoured at project scope only: a global list would widen every
    /// project at once.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    default: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    allow: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    deny: Option<serde_yaml_ng::Value>,
}

impl PermissionsConfig {
    /// A file at the current version that widens nothing.
    pub fn empty() -> Self {
        Self {
            version: PERMISSIONS_VERSION,
            roots: vec![],
            default: None,
            allow: None,
            deny: None,
        }
    }

    /// Parses `text` as `permissions.yaml` at `path` is read.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] or [`ConfigError::UnknownVersion`], naming
    /// `path`.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        fsio::parse_versioned(text, path, PERMISSIONS_VERSION)
    }

    /// Whether `default:`, `allow:` or `deny:` holds anything but an empty
    /// list; earlier first launches wrote `deny: []`.
    pub fn has_stale_keys(&self) -> bool {
        [&self.default, &self.allow, &self.deny]
            .into_iter()
            .flatten()
            .any(|value| value.as_sequence().is_none_or(|items| !items.is_empty()))
    }
}

/// Which wire protocol a provider speaks; aldwin-llm picks the client
/// from it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// Anthropic's Messages API.
    Anthropic,
    /// Any server that speaks OpenAI's chat completions API; it needs a
    /// `base_url`.
    OpenaiCompatible,
}

/// One scope's `provider.yaml`. Never add a field for a plaintext key: only
/// the variable's name is stored, and aldwin-llm resolves it. A connected
/// account (ADR 0012) lives in `connections.yaml`, not here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Schema version; this build reads [`PROVIDER_VERSION`].
    pub version: u32,
    /// The protocol the provider speaks.
    pub provider: ProviderKind,
    /// The model id every request names, as the provider spells it.
    pub model: String,
    /// Where the provider's API is, when it is not the protocol's own
    /// default host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The environment variable that holds the API key.
    pub api_key_env: String,
    /// Extended-thinking token budget; `None` leaves aldwin-llm's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_thinking_budget: Option<u32>,
}

impl ProviderConfig {
    /// Whether `api_key_env` is non-blank, which the schema cannot check.
    pub fn has_valid_api_key_env(&self) -> bool {
        !self.api_key_env.trim().is_empty()
    }

    /// This file laid over `below` (project over global): required fields
    /// come from `self`, and each optional field falls back to `below`.
    pub fn over(self, below: Option<&ProviderConfig>) -> ProviderConfig {
        ProviderConfig {
            base_url: self
                .base_url
                .or_else(|| below.and_then(|b| b.base_url.clone())),
            extended_thinking_budget: self
                .extended_thinking_budget
                .or_else(|| below.and_then(|b| b.extended_thinking_budget)),
            ..self
        }
    }
}

/// How an MCP server is reached. Unknown fields are refused here, not on
/// [`McpServer`]: serde's `deny_unknown_fields` on a struct with a
/// `flatten` field refuses the flattened fields too.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum McpTransport {
    /// A server Aldwin starts as a child process and speaks to over its
    /// standard input and output.
    Stdio {
        /// The program to start.
        command: String,
        /// Its arguments, passed as they are.
        #[serde(default)]
        args: Vec<String>,
    },
    /// A server that is already running, reached over HTTP.
    Http {
        /// The server's endpoint.
        url: String,
    },
}

/// One MCP server `mcp.yaml` names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    /// What the server is known by; its tools are addressed through it.
    pub name: String,
    /// How the server is reached.
    #[serde(flatten)]
    pub transport: McpTransport,
    /// Environment variables added to Aldwin's own for a started server.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// The MCP servers for one scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    /// Schema version; this build reads `MCP_VERSION`.
    pub version: u32,
    /// The servers, in the order the file lists them.
    #[serde(default)]
    pub servers: Vec<McpServer>,
}

impl McpConfig {
    /// A file at the current version that names no server.
    pub fn empty() -> Self {
        Self {
            version: MCP_VERSION,
            servers: vec![],
        }
    }
}

/// The accounts `/connect` has connected, keyed by provider id (ADR 0012).
/// Global only, and never seeded: an empty file says nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionsConfig {
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, ConnectionRecord>,
}

impl ConnectionsConfig {
    pub fn empty() -> Self {
        Self {
            version: CONNECTIONS_VERSION,
            accounts: BTreeMap::new(),
        }
    }
}

/// One connected account's tokens, as aldwin-login stores and reads them.
/// Not aldwin-login's type: this crate is a leaf.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRecord {
    /// The bearer token requests to the provider carry.
    pub access_token: String,
    /// What a refresh trades for a new access token.
    pub refresh_token: String,
    /// When the access token expires, in Unix seconds.
    pub expires_at: u64,
}

/// Omits the tokens: they must never reach a log.
impl std::fmt::Debug for ConnectionRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionRecord")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// aldwin-tui's `tui.yaml`, as plain types: this crate is a leaf.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    /// Schema version; this build reads `TUI_VERSION`.
    pub version: u32,
    /// The theme `/theme` wrote; aldwin-tui interprets it and `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// `reduced` holds the caret and the working line still; aldwin-tui
    /// interprets it and `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<String>,
}

impl TuiConfig {
    /// A file at the current version that overrides nothing.
    pub fn empty() -> Self {
        Self {
            version: TUI_VERSION,
            theme: None,
            motion: None,
        }
    }
}
