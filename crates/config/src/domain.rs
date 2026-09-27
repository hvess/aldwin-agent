//! Typed shapes for the four config domains. Each versions independently;
//! `deny_unknown_fields` is deliberate — a stray or renamed field should fail
//! loudly at load rather than be silently dropped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PERMISSIONS_VERSION: u32 = 2;
/// The `provider.yaml` schema version this build reads and writes.
pub const PROVIDER_VERSION: u32 = 1;
pub const MCP_VERSION: u32 = 1;
pub const TUI_VERSION: u32 = 1;
pub const CONNECTIONS_VERSION: u32 = 1;

/// Permissions for one scope. Since ADR 0011 the workspace is the only
/// boundary, and `roots:` — which widens it — is the only key read.
///
/// Three keys from models this file has outlived are still *parsed*:
/// `default:` and `allow:` (ADR 0004's rung and grants, unread since ADR
/// 0009) and `deny:` (the lock ADR 0011 removed). Files written by an earlier
/// Aldwin carry them — every first launch wrote `deny: []` — and
/// `deny_unknown_fields` would otherwise stop every existing project from
/// starting. Their values are not interpreted, so a v1 file's `kind:pattern`
/// strings load as readily as a v2 file's `git: read`; [`has_stale_keys`]
/// says whether any of them says something, so the developer is told once
/// that it no longer does.
///
/// [`has_stale_keys`]: PermissionsConfig::has_stale_keys
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsConfig {
    /// Schema version; this build reads `PERMISSIONS_VERSION`.
    pub version: u32,
    /// Extra directories that are workspace, beyond the project root (ADR
    /// 0007): tools may be pointed at them and a run may write in them.
    ///
    /// Stated rather than inferred — nothing walks up to find sibling
    /// checkouts. Project scope only: a global root list would silently
    /// widen the workspace in every directory at once.
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

    /// Whether the file says something through a key nothing reads. An
    /// empty list says nothing — the `deny: []` every first launch wrote is
    /// not worth a notice.
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

/// `api_key_env` names an environment variable; resolving it is aldwin-llm's
/// job. A raw `api_key` field is rejected by `deny_unknown_fields` — there is
/// deliberately no field a plaintext key could go in. A provider's connected
/// account (ADR 0012) is not named here either: it lives in
/// `connections.yaml`, and whether the provider is reached through it or
/// through the variable is decided when the client is built.
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
    /// Extended-thinking token budget. `None` means "let the provider crate
    /// pick its own default" — this field only exists so the developer can
    /// override it; aldwin-config has no opinion on what a good budget is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_thinking_budget: Option<u32>,
}

impl ProviderConfig {
    /// True for a schema violation `deny_unknown_fields` can't catch by itself:
    /// the field can be present and still empty.
    pub fn has_valid_api_key_env(&self) -> bool {
        !self.api_key_env.trim().is_empty()
    }

    /// This file laid over `below` — a project `provider.yaml` over the
    /// global one. The required fields come from this file wholesale (a
    /// file that names a provider names all of them); the two optional
    /// ones fall back to `below` one at a time.
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

/// Unknown fields are refused here, rather than on
/// [`McpServer`]: serde does not support `deny_unknown_fields` together with
/// `flatten`, and on the outer struct it refused every field the flattened
/// transport owns — so no `mcp.yaml` naming a server could load at all.
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
    /// Environment variables set for a started server, on top of Aldwin's
    /// own.
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

/// One connected account's tokens, as aldwin-login hands them over and
/// reads them back. Its own type rather than aldwin-login's, for the
/// reason `TuiConfig` is not aldwin-tui's: this crate persists, it does
/// not depend.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRecord {
    /// The bearer token requests to the provider carry.
    pub access_token: String,
    /// What a refresh trades for a new access token.
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
}

/// The tokens are the one thing here that must not reach a log.
impl std::fmt::Debug for ConnectionRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionRecord")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Field set owned by aldwin-tui; this crate only persists it. Kept as
/// plain, permissive types (rather than importing aldwin-tui's own types)
/// since aldwin-config is a leaf crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    /// Schema version; this build reads `TUI_VERSION`.
    pub version: u32,
    /// The developer's theme, as `/theme` wrote it; aldwin-tui decides what
    /// the string means, and what `None` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl TuiConfig {
    /// A file at the current version that overrides nothing.
    pub fn empty() -> Self {
        Self {
            version: TUI_VERSION,
            theme: None,
        }
    }
}
