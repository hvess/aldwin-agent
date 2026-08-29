//! `ToolDispatcher` impl, built-in tool set, and the Edit approval gate. See
//! `.claude/spec/amundsen-tools.md`.
//!
//! Covers the registry, dispatch flow, permissions wiring, all four V0
//! built-ins (Read, Edit, shell, Explain), the LSP client Explain uses, and
//! the MCP bridge (`mcp`). One MCP sub-feature is deliberately not built
//! yet: the first-invocation edit-shape follow-up that lets an MCP tool
//! graduate to Edit's binary approval gate — see `mcp::tool`'s doc comment.
//! Every MCP tool goes through the standard four-tier prompt for now.

mod diff;
mod dispatcher;
mod error;
mod gate;
mod lsp;
mod mcp;
mod paths;
mod registry;
mod tools;

#[cfg(test)]
mod test_support;

pub use dispatcher::Dispatcher;
pub use error::ToolError;
pub use gate::ApprovalGate;
pub use mcp::{register_mcp_tools, McpBridge, McpError, McpTool};
pub use registry::{Registry, Tool, ToolDescriptor, ToolSource};
pub use tools::{EditTool, ExplainTool, ReadTool, ShellTool};

use std::path::PathBuf;

/// Registers the four V0 built-ins (Read, Edit, shell, Explain) rooted at
/// `project_root`.
pub fn builtin_registry(project_root: PathBuf) -> Registry {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(ReadTool::new(project_root.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(EditTool::new(project_root.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(ShellTool::new(project_root.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(ExplainTool::new(project_root))).expect("built-in names are unique");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_all_four_v0_tools() {
        let registry = builtin_registry(PathBuf::from("."));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(names, vec!["edit".to_string(), "explain".to_string(), "read".to_string(), "shell".to_string()]);
    }
}
