//! Typed shapes for the four config domains. Each versions independently;
//! `deny_unknown_fields` is deliberate — a stray or renamed field should fail
//! loudly at load rather than be silently dropped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PERMISSIONS_VERSION: u32 = 2;
pub const PROVIDER_VERSION: u32 = 1;
pub const MCP_VERSION: u32 = 1;
pub const TUI_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Anthropic,
    OpenaiCompatible,
}

/// `api_key_env` names an environment variable; resolving it is aldwin-llm's
/// job. A raw `api_key` field is rejected by `deny_unknown_fields` — there is
/// deliberately no field a plaintext key could go in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub version: u32,
    pub provider: ProviderKind,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
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
}

/// Unknown fields are refused here, rather than on
/// [`McpServer`]: serde does not support `deny_unknown_fields` together with
/// `flatten`, and on the outer struct it refused every field the flattened
/// transport owns — so no `mcp.yaml` naming a server could load at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Http {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    pub version: u32,
    #[serde(default)]
    pub servers: Vec<McpServer>,
}

impl McpConfig {
    pub fn empty() -> Self {
        Self {
            version: MCP_VERSION,
            servers: vec![],
        }
    }
}

/// Field set owned by aldwin-tui; this crate only persists it. Kept as
/// plain, permissive types (rather than importing aldwin-tui's own types)
/// since aldwin-config is a leaf crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl TuiConfig {
    pub fn empty() -> Self {
        Self {
            version: TUI_VERSION,
            theme: None,
        }
    }
}
