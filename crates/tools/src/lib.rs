//! `ToolDispatcher` impl, built-in tool set, and the staged changeset the
//! review is opened over. See `.claude/spec/aldwin-tools.md`.
//!
//! Covers the registry, dispatch flow, the lock, all six built-ins (Read,
//! Edit, Run, Explain, Plan, Ask), the read-enforcing `sandbox`, the LSP
//! client Explain uses, the MCP bridge (`mcp`), and `staging` — where every
//! edit of a turn waits for the review (ADR 0009).
//!
//! One piece of ADR 0004 is deliberately not built yet: letting the
//! developer classify an MCP tool, with the server's own claim shown as a
//! claim. Until it is, every MCP tool is a write — see `mcp::tool`'s
//! `permission` for why that is the only reading that cannot quietly be
//! wrong — and, because it runs in its own process over the real tree, it
//! opens the review the way `run` does.

mod dispatcher;
mod error;
mod lsp;
mod mcp;
mod paths;
mod registry;
pub mod sandbox;
mod staging;
mod tools;

#[cfg(test)]
mod test_support;

pub use dispatcher::Dispatcher;
pub use error::ToolError;
pub use mcp::{register_mcp_tools, McpBridge, McpError, McpRegistrationFailure, McpTool};
pub use paths::Workspace;
pub use registry::{PermissionRequest, Registry, Tool, ToolDescriptor, ToolSource};
pub use staging::{Staged, Staging, Written};
pub use tools::{AskTool, EditTool, ExplainTool, PlanTool, ReadTool, RunTool, CHAT_ABOUT_THIS};

/// Registers the six built-ins over `workspace` — every one that touches a
/// file, `run` included, contained by it — and over `staging`, which `edit`
/// writes into and `read` reads through.
pub fn builtin_registry(workspace: Workspace, staging: std::sync::Arc<Staging>) -> Registry {
    let mut registry = Registry::new();
    registry
        .register(std::sync::Arc::new(ReadTool::new(
            workspace.clone(),
            staging.clone(),
        )))
        .expect("built-in names are unique");
    registry
        .register(std::sync::Arc::new(EditTool::new(
            workspace.clone(),
            staging,
        )))
        .expect("built-in names are unique");
    registry
        .register(std::sync::Arc::new(RunTool::new(workspace.clone())))
        .expect("built-in names are unique");
    registry
        .register(std::sync::Arc::new(ExplainTool::new(workspace)))
        .expect("built-in names are unique");
    registry
        .register(std::sync::Arc::new(PlanTool::new()))
        .expect("built-in names are unique");
    registry
        .register(std::sync::Arc::new(AskTool::new()))
        .expect("built-in names are unique");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_all_six_tools() {
        let registry = builtin_registry(Workspace::new("."), std::sync::Arc::new(Staging::new()));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(
            names,
            ["ask", "edit", "explain", "plan", "read", "run"].map(String::from)
        );
    }
}
