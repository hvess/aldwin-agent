//! Wire shapes for the prompt round trip described in mjolnir-permissions.md.
//! `mjolnir-core`'s `Event::PromptRequested` / `Command::PromptResponse`
//! carry an opaque `serde_json::Value` — these types are what that value
//! actually is. Callers (mjolnir-tools, mjolnir-tui) serialise
//! [`PromptPayload`] into the event and deserialise [`PromptResponse`] back
//! out of the command.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::grant::Decision;

/// Persistence tier for a tool four-tier prompt response — symmetric for
/// allow and deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    Once,
    Session,
    Project,
    Always,
}

/// Persistence tier for a context-file two-tier prompt response. No "once"
/// (injection is system-prompt-level, so "once" has no meaningful boundary)
/// and no "always" (context-file paths are intrinsically project-scoped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileTier {
    Session,
    Project,
}

/// What `Engine::check_tool` / `check_context_file` ask the developer to
/// decide, carried in `Event::PromptRequested`. `Edit` is included for a
/// complete Allow/Deny/PromptRequired contract over every guarded action,
/// but it is never routed through this generic round trip in practice —
/// callers use `DispatchContext::request_approval` (core's dedicated
/// per-call binary gate) instead; `Engine::record_tool_decision` refuses to
/// persist anything for an edit-class kind regardless.
///
/// `Tool`'s `path_like` says whether `target` is a project-relative file
/// path (set by the caller from the tool's own `permission_target_is_path`,
/// mjolnir-tools) rather than an argv/JSON blob — it drives the decision
/// panel's optional "approve this whole directory" scope toggle
/// (mjolnir-permissions.md's Pattern grammar already allows a path-glob
/// grant like `read:./**`; this is what lets a developer reach for one from
/// the prompt itself instead of hand-editing config).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum PromptPayload {
    Tool { kind: String, target: String, path_like: bool },
    ContextFile { path: PathBuf },
    Edit { kind: String },
}

/// The developer's answer to a [`PromptPayload`], carried in
/// `Command::PromptResponse`. `tier` on `ContextFile` is only meaningful
/// when `approve` is `true` — decline persists nothing at any tier.
///
/// `Tool`'s `pattern` is what actually gets persisted by
/// `Engine::record_tool_decision` — usually (but not necessarily) an
/// exact-match glob of the originating `PromptPayload::Tool`'s `target`;
/// when `path_like` offered a directory scope and the developer chose it,
/// this is the broader `<dir>/**` glob instead. The engine itself is
/// pattern-agnostic (see its own doc comment); the caller — here, the TUI —
/// decides how coarse the grant should be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum PromptResponse {
    Tool { decision: Decision, tier: ToolTier, pattern: String },
    ContextFile { approve: bool, #[serde(default, skip_serializing_if = "Option::is_none")] tier: Option<ContextFileTier> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_payload_round_trips_through_json_value() {
        let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["shape"], "tool");
        let back: PromptPayload = serde_json::from_value(value).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn context_file_payload_round_trips() {
        let payload = PromptPayload::ContextFile { path: PathBuf::from("./CLAUDE.md") };
        let value = serde_json::to_value(&payload).unwrap();
        let back: PromptPayload = serde_json::from_value(value).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn tool_response_round_trips() {
        let response = PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Project, pattern: "cargo test".into() };
        let value = serde_json::to_value(&response).unwrap();
        let back: PromptResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn decline_response_omits_tier() {
        let response = PromptResponse::ContextFile { approve: false, tier: None };
        let value = serde_json::to_value(&response).unwrap();
        assert!(value.get("tier").is_none());
        let back: PromptResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back, response);
    }
}
