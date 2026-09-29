//! The `ToolDispatcher` impl, the seven built-in tools, the `sandbox`, the MCP
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
pub use paths::{TakenRoots, Widening, Workspace};
pub use registry::Registry;
pub use staging::Staging;

use aldwin_config::Config;
use tools::{AskTool, EditTool, ExplainTool, PlanTool, ReadTool, ReloadTool, RunTool};

/// Registers the seven built-ins, contained by `workspace`; `edit` writes
/// into `staging`, `read` reads through it, and `reload` re-reads `config`.
///
/// # Panics
///
/// Panics if two built-ins share a name, a bug here, never input.
pub fn builtin_registry(config: Config, workspace: Workspace, staging: Arc<Staging>) -> Registry {
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
        .register(Arc::new(ReloadTool::new(
            config,
            workspace.clone(),
            staging.clone(),
        )))
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

    use tempfile::tempdir;

    #[test]
    fn builtin_registry_has_all_seven_tools() {
        let (project, global) = (tempdir().unwrap(), tempdir().unwrap());
        let config = Config::open_at(project.path(), global.path()).unwrap();
        let workspace = Workspace::new(".");
        let registry =
            builtin_registry(config, workspace.clone(), Arc::new(Staging::new(workspace)));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(
            names,
            ["ask", "edit", "explain", "plan", "read", "reload", "run"].map(String::from)
        );
    }
}
