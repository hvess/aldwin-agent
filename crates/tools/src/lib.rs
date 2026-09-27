//! The `ToolDispatcher` impl, the six built-in tools, the `sandbox`, the MCP
//! bridge and `staging`, where a turn's edits wait for the review (ADR 0009).
//! Spec: `docs/spec/aldwin-tools.md`.
//!
//! The workspace is the only boundary (ADR 0011): every tool resolves paths
//! through [`Workspace`], and every process a tool starts can write only
//! inside it.

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

use std::sync::Arc;

pub use dispatcher::Dispatcher;
pub use error::ToolError;
pub use mcp::{register_mcp_tools, McpBridge, McpRegistrationFailure};
pub use paths::Workspace;
pub use registry::Registry;
pub use staging::Staging;

use tools::{AskTool, EditTool, ExplainTool, PlanTool, ReadTool, RunTool};

/// Registers the six built-ins, contained by `workspace`; `edit` writes into
/// `staging` and `read` reads through it.
///
/// # Panics
///
/// Panics if two built-ins share a name, a bug here, never input.
pub fn builtin_registry(workspace: Workspace, staging: Arc<Staging>) -> Registry {
    let mut registry = Registry::new();
    registry
        .register(Arc::new(ReadTool::new(workspace.clone(), staging.clone())))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(EditTool::new(workspace.clone(), staging.clone())))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(RunTool::new(workspace.clone())))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(ExplainTool::new(workspace, staging)))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(PlanTool::new()))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(AskTool::new()))
        .expect("built-in names are unique");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_all_six_tools() {
        let workspace = Workspace::new(".");
        let registry = builtin_registry(workspace.clone(), Arc::new(Staging::new(workspace)));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(
            names,
            ["ask", "edit", "explain", "plan", "read", "run"].map(String::from)
        );
    }
}
