//! Typed shapes for the five V0 config domains. Each versions independently;
//! `deny_unknown_fields` is deliberate — a stray or renamed field should fail
//! loudly at load rather than be silently dropped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PERMISSIONS_VERSION: u32 = 1;
pub const PROVIDER_VERSION: u32 = 1;
pub const MCP_VERSION: u32 = 1;
pub const TUI_VERSION: u32 = 1;
pub const CONTEXT_FILES_VERSION: u32 = 1;

/// Grant entries are opaque `kind:pattern` strings. Parsing and precedence are
/// mjolnir-permissions' job; this crate only persists the two lists as given —
/// deny-wins is a structural property of keeping them separate, not something
/// this crate resolves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsConfig {
    pub version: u32,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl PermissionsConfig {
    pub fn empty() -> Self {
        Self { version: PERMISSIONS_VERSION, allow: vec![], deny: vec![] }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Anthropic,
    OpenaiCompatible,
}

/// `api_key_env` names an environment variable; resolving it is mjolnir-llm's
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
    /// override it; mjolnir-config has no opinion on what a good budget is.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum McpTransport {
    Stdio { command: String, #[serde(default)] args: Vec<String> },
    Http { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
        Self { version: MCP_VERSION, servers: vec![] }
    }
}

/// Field set owned by mjolnir-tui; this crate only persists it. Kept as
/// plain, permissive types (rather than importing mjolnir-tui's own types)
/// since mjolnir-config is a leaf crate — `depends_on: []`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keybinds: BTreeMap<String, String>,
}

impl TuiConfig {
    pub fn empty() -> Self {
        Self { version: TUI_VERSION, theme: None, layout: None, keybinds: BTreeMap::new() }
    }
}

/// Project-only. Path-keyed only, no content hash (see mjolnir-permissions).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextFilesConfig {
    pub version: u32,
    #[serde(default)]
    pub approved: Vec<PathBuf>,
}

impl ContextFilesConfig {
    pub fn empty() -> Self {
        Self { version: CONTEXT_FILES_VERSION, approved: vec![] }
    }
}
