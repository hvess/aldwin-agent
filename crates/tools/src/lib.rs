//! `ToolDispatcher` impl, built-in tool set, and the staged changeset the
//! review is opened over. See `.claude/spec/aldwin-tools.md`.
//!
//! Covers the registry, dispatch, all six built-ins (Read, Edit, Run,
//! Explain, Plan, Ask), the `sandbox` every spawned process runs in, the LSP
//! client Explain uses, the MCP bridge (`mcp`), and `staging` — where every
//! edit of a turn waits for the review (ADR 0009).
//!
//! The workspace is the only boundary (ADR 0011): every tool resolves its
//! paths through [`Workspace`], and every process a tool starts — a `run`,
//! the language server, an MCP server — can write only inside it.

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

/// Registers the six built-ins over `workspace` — every one that touches a
/// file, `run` included, contained by it — and over `staging`, which `edit`
/// writes into and `read` reads through.
pub fn builtin_registry(workspace: Workspace, staging: Arc<Staging>) -> Registry {
    let mut registry = Registry::new();
    registry
        .register(Arc::new(ReadTool::new(workspace.clone(), staging.clone())))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(EditTool::new(workspace.clone(), staging)))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(RunTool::new(workspace.clone())))
        .expect("built-in names are unique");
    registry
        .register(Arc::new(ExplainTool::new(workspace)))
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
