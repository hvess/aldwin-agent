//! `ToolDispatcher` impl, built-in tool set, and the Edit approval gate. See
//! `.claude/spec/aldwin-tools.md`.
//!
//! Covers the registry, dispatch flow, permissions wiring, all four
//! built-ins (Read, Edit, Run, Explain), the read-enforcing `sandbox`, the
//! LSP client Explain uses, and the MCP bridge (`mcp`).
//!
//! One piece of ADR 0004 is deliberately not built yet: letting the
//! developer classify an MCP tool, with the server's own claim shown as a
//! claim. Until it is, every MCP tool is a write — see `mcp::tool`'s
//! `permission` for why that is the only reading that cannot quietly be
//! wrong.

mod diff;
mod dispatcher;
mod error;
mod gate;
mod lsp;
mod mcp;
mod paths;
mod registry;
pub mod sandbox;
mod tools;

#[cfg(test)]
mod test_support;

pub use dispatcher::Dispatcher;
pub use error::ToolError;
pub use gate::ApprovalGate;
pub use mcp::{register_mcp_tools, McpBridge, McpError, McpRegistrationFailure, McpTool};
pub use registry::{PermissionRequest, Registry, Tool, ToolDescriptor, ToolSource};
pub use paths::Workspace;
pub use tools::{EditTool, ExplainTool, ReadTool, RunTool};

/// Registers the four V0 built-ins (Read, Edit, Run, Explain) over
/// `workspace` — every one of them, `run` included, contained by it.
pub fn builtin_registry(workspace: Workspace) -> Registry {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(ReadTool::new(workspace.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(EditTool::new(workspace.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(RunTool::new(workspace.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(ExplainTool::new(workspace))).expect("built-in names are unique");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_all_four_v0_tools() {
        let registry = builtin_registry(Workspace::new("."));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(names, vec!["edit".to_string(), "explain".to_string(), "read".to_string(), "run".to_string()]);
    }
}
