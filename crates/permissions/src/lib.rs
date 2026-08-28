//! Default-deny permission engine. See `.claude/spec/amundsen-permissions.md`.
//!
//! `Engine` owns scope precedence (session > project > global, deny-wins
//! within a scope), pattern matching, and the in-memory shape of the tiered
//! prompt round trip. It has no dependency on amundsen-core: `PromptPayload`
//! / `PromptResponse` are the plain-data shapes that cross core's opaque
//! `serde_json::Value` boundary, and it is the caller's job (amundsen-tools,
//! amundsen-cli) to wire `Engine::check_tool` / `check_context_file` into
//! core's actual event/command channels.

mod engine;
mod error;
mod glob;
mod grant;
mod prompt;

pub use engine::{CheckOutcome, EffectiveContextFile, EffectiveGrant, EffectiveView, Engine, GrantScope};
pub use error::PermissionError;
pub use grant::{Decision, GrantKey};
pub use prompt::{ContextFileTier, PromptPayload, PromptResponse, ToolTier};
