//! Typed shapes for the five V0 config domains. Each versions independently;
//! `deny_unknown_fields` is deliberate — a stray or renamed field should fail
//! loudly at load rather than be silently dropped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PERMISSIONS_VERSION: u32 = 2;
pub const PROVIDER_VERSION: u32 = 1;
pub const MCP_VERSION: u32 = 1;
pub const TUI_VERSION: u32 = 1;
pub const CONTEXT_FILES_VERSION: u32 = 1;

/// What a call does. The class belongs to the *call*, not the program —
/// `git status` is a [`Class::Read`] and `git push` a [`Class::Write`], and
/// they are the same binary (ADR 0004 §2).
///
/// [`Class::Edit`] is outside the permissions model entirely: no grant, no
/// scope and no [`Rung`] can cover it, and it is not expressible in a
/// persisted entry — `Class` is `Deserialize` only for `read` and `write`,
/// so a hand-written `edit` in a YAML file is a load error rather than a
/// rule that silently does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Read,
    Write,
    /// Never deserialized from config — see the type's doc comment.
    #[serde(skip_deserializing)]
    Edit,
}

impl Class {
    /// Whether a grant of `self` covers a call of `other`. `write` covers
    /// `read`; nothing covers `edit`.
    pub fn covers(self, other: Class) -> bool {
        match (self, other) {
            (_, Class::Edit) | (Class::Edit, _) => false,
            _ => self >= other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Class::Read => "read",
            Class::Write => "write",
            Class::Edit => "edit",
        }
    }
}

/// A scope's standing answer for any call no entry covers (ADR 0004 §6).
/// Widening: `ask` prompts for everything, `read` runs read-classified calls,
/// `write` runs both. `edit` asks under all three — it is the floor, not a
/// rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    #[default]
    Ask,
    Read,
    Write,
}

impl Rung {
    pub const ORDER: [Rung; 3] = [Rung::Ask, Rung::Read, Rung::Write];

    /// Whether this rung runs a call of `class` without asking.
    pub fn covers(self, class: Class) -> bool {
        match (self, class) {
            (_, Class::Edit) | (Rung::Ask, _) => false,
            (Rung::Read, Class::Read) => true,
            (Rung::Read, Class::Write) => false,
            (Rung::Write, _) => true,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rung::Ask => "ask",
            Rung::Read => "read",
            Rung::Write => "write",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Rung::Ask => "every call asks, every time",
            Rung::Read => "reads run; writes and edits ask",
            Rung::Write => "reads and writes run; edits ask",
        }
    }
}

/// One line of an allow or deny list: a program, and optionally the class it
/// is qualified by.
///
/// On disk it is either a bare program name or a single-key map, which is
/// what makes the file readable as prose:
///
/// ```yaml
/// allow:
///   - git: read       # class-qualified
///   - cargo: write
/// deny:
///   - curl            # the whole program, whatever the class
/// ```
///
/// A `None` class means *every* class. In an allow list that is the widest
/// grant expressible; in a deny list it is ADR 0004 §8's row 8, the lock on
/// a whole program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantEntry {
    pub program: String,
    pub class:   Option<Class>,
}

impl GrantEntry {
    pub fn program(program: impl Into<String>) -> Self {
        Self { program: program.into(), class: None }
    }

    pub fn classed(program: impl Into<String>, class: Class) -> Self {
        Self { program: program.into(), class: Some(class) }
    }

    /// Whether this *allow* entry lets a call of `class` on `program` run.
    pub fn allows(&self, program: &str, class: Class) -> bool {
        self.program == program && self.class.map_or(class != Class::Edit, |c| c.covers(class))
    }

    /// Whether this *deny* entry blocks a call of `class` on `program`.
    ///
    /// Asymmetric with [`allows`](Self::allows) on purpose: denying a
    /// program's reads denies its writes too, because a rule that forbade
    /// reading while permitting writing would describe no coherent posture.
    /// So a deny of class `C` blocks every call at or above `C`.
    pub fn denies(&self, program: &str, class: Class) -> bool {
        self.program == program && self.class.is_none_or(|c| class >= c)
    }
}

impl std::fmt::Display for GrantEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.class {
            Some(c) => write!(f, "{}: {}", self.program, c.label()),
            None => write!(f, "{}", self.program),
        }
    }
}

/// The on-disk spelling of a [`GrantEntry`] — a bare string or a one-key map.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Program(String),
    Classed(BTreeMap<String, Class>),
}

impl Serialize for GrantEntry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.class {
            None => RawEntry::Program(self.program.clone()).serialize(s),
            Some(c) => RawEntry::Classed(BTreeMap::from([(self.program.clone(), c)])).serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for GrantEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        match RawEntry::deserialize(d)? {
            RawEntry::Program(p) if p.is_empty() => Err(D::Error::custom("a grant entry needs a program name")),
            RawEntry::Program(p) => Ok(Self { program: p, class: None }),
            RawEntry::Classed(map) if map.len() == 1 => {
                let (program, class) = map.into_iter().next().expect("length checked");
                Ok(Self { program, class: Some(class) })
            }
            RawEntry::Classed(_) => Err(D::Error::custom("a grant entry names one program: `git: read`, not several")),
        }
    }
}

/// Permissions for one scope: the standing [`Rung`], and the two entry lists.
///
/// Allow and deny stay separate on disk and in memory because deny is a lock
/// (ADR 0004 §7) — collapsing them into one ordered list would turn that from
/// a structural property into a sort order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsConfig {
    pub version: u32,
    /// What runs without asking when no entry below covers the call.
    ///
    /// `None` means this file does not state one, which is not the same as
    /// stating `ask`: ADR 0004 §6 has the narrower file win outright, so a
    /// project that has never answered the question must fall through to the
    /// global file rather than silently overriding it with a default that
    /// serde invented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Rung>,
    #[serde(default)]
    pub allow: Vec<GrantEntry>,
    #[serde(default)]
    pub deny: Vec<GrantEntry>,
}

impl PermissionsConfig {
    pub fn empty() -> Self {
        Self { version: PERMISSIONS_VERSION, default: None, allow: vec![], deny: vec![] }
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

/// Field set owned by aldwin-tui; this crate only persists it. Kept as
/// plain, permissive types (rather than importing aldwin-tui's own types)
/// since aldwin-config is a leaf crate — `depends_on: []`.
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

/// Project-only. Path-keyed only, no content hash (see aldwin-permissions).
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
