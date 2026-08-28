//! `ToolDispatcher` impl, built-in tool set, and the Edit approval gate. See
//! `.claude/spec/amundsen-tools.md`.
//!
//! This pass covers the registry, dispatch flow, permissions wiring, and the
//! three built-ins that don't need an external protocol client: Read, Edit,
//! shell. Explain (LSP-backed) and the MCP bridge (rmcp) are deferred to a
//! follow-up pass — each is a substantial protocol implementation in its own
//! right, and the spec itself flags LSP as a risk of outgrowing this crate.

mod diff;
mod dispatcher;
mod error;
mod gate;
mod registry;
mod tools;

#[cfg(test)]
mod test_support;

pub use dispatcher::Dispatcher;
pub use error::ToolError;
pub use gate::ApprovalGate;
pub use registry::{Registry, Tool, ToolDescriptor, ToolSource};
pub use tools::{EditTool, ReadTool, ShellTool};

use std::path::PathBuf;

/// Registers the three built-ins implemented so far (Read, Edit, shell)
/// rooted at `project_root`. Explain is not yet registered — see module doc.
pub fn builtin_registry(project_root: PathBuf) -> Registry {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(ReadTool::new(project_root.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(EditTool::new(project_root.clone()))).expect("built-in names are unique");
    registry.register(std::sync::Arc::new(ShellTool::new(project_root))).expect("built-in names are unique");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_the_three_implemented_tools() {
        let registry = builtin_registry(PathBuf::from("."));
        let mut names: Vec<String> = registry.definitions().into_iter().map(|d| d.name).collect();
        names.sort();
        assert_eq!(names, vec!["edit".to_string(), "read".to_string(), "shell".to_string()]);
    }
}
