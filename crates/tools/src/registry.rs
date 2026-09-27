use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;

/// A registered (name, input schema) pair, plus the one thing the
/// dispatcher needs to know about a tool without knowing which tool it is.
/// Built-ins register through `builtin_registry`; MCP tools through
/// `register_mcp_tools`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether a call sees the files as they are on disk rather than through
    /// the staging overlay — a shell command, or an MCP server in its own
    /// process. Staged edits are reviewed before any such call (ADR 0009 §4).
    pub observes_disk: bool,
}

/// One concrete tool: what it looks like to the model, and what a call does.
#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> &ToolDescriptor;

    async fn call(
        &self,
        call_id: &str,
        input: Value,
        ctx: &DispatchContext,
    ) -> Result<String, ToolError>;
}

/// In-process map of name -> tool, behind core's `ToolDispatcher` impl.
#[derive(Default)]
pub struct Registry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

/// Lists the tools by name: a `dyn Tool` has no `Debug` of its own.
impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Duplicate names are rejected. Namespacing an MCP name that collides
    /// (`<server>:<name>`) is `register_mcp_tools`' job, not the registry's.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::DuplicateTool`] when a tool of the same name is
    /// already registered; the registry is left unchanged.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.descriptor().name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateTool { name });
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// The tool registered under `name`, if any.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// What core's `ToolDispatcher::definitions` needs — just the surface
    /// the model sees, stripped of dispatch-only metadata.
    pub fn definitions(&self) -> Vec<aldwin_core::ToolDefinition> {
        self.tools
            .values()
            .map(|t| {
                let d = t.descriptor();
                aldwin_core::ToolDefinition {
                    name: d.name.clone(),
                    description: d.description.clone(),
                    input_schema: d.input_schema.clone(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct StubTool(ToolDescriptor);

    #[async_trait]
    impl Tool for StubTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.0
        }
        async fn call(
            &self,
            _call_id: &str,
            _input: Value,
            _ctx: &DispatchContext,
        ) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    fn stub(name: &str) -> Arc<dyn Tool> {
        Arc::new(StubTool(ToolDescriptor {
            name: name.into(),
            description: "stub".into(),
            input_schema: json!({}),
            observes_disk: false,
        }))
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut registry = Registry::new();
        registry.register(stub("read")).unwrap();
        let err = registry.register(stub("read")).unwrap_err();
        assert!(matches!(err, ToolError::DuplicateTool { name } if name == "read"));
    }

    #[test]
    fn get_returns_the_registered_tool() {
        let mut registry = Registry::new();
        registry.register(stub("read")).unwrap();
        assert!(registry.get("read").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn definitions_strip_dispatch_only_metadata() {
        let mut registry = Registry::new();
        registry.register(stub("read")).unwrap();
        let defs = registry.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "read");
    }
}
