use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;
use crate::gate::ApprovalGate;

/// Where a registered tool came from — surfaced by the Registry View
/// interface so the TUI can label built-ins vs. MCP-bridged tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    Builtin,
    Mcp { server: String },
}

/// A registered (name, input schema, edit_class, dispatch fn) tuple — see
/// amundsen-tools.md's Vocabulary. Built-ins register at crate init with a
/// static descriptor; MCP tools register lazily (not yet implemented in this
/// pass — see amundsen-tools.md's MCP Bridge / MCP Lifecycle sections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub name:         String,
    pub description:  String,
    pub input_schema: Value,
    /// When set, the permission engine refuses anything but the per-call
    /// binary approval gate — never allowlistable, at any tier.
    pub edit_class:   bool,
    pub source:       ToolSource,
}

/// One concrete tool. `permission_target` and `call` are split so the
/// dispatcher can run the generic four-tier permission check (for
/// `edit_class: false` tools) without each tool re-implementing that flow —
/// it only needs to say *what string* grant patterns should match against.
/// Approval-gated tools (`edit_class: true`) skip that generic check
/// entirely and drive `ApprovalGate::request_approval` from inside `call`
/// instead; `permission_target` is never invoked for them.
#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> &ToolDescriptor;

    fn permission_target(&self, input: &Value) -> Result<String, ToolError>;

    async fn call(&self, call_id: &str, input: Value, gate: &dyn ApprovalGate) -> Result<String, ToolError>;
}

/// In-process map of name -> tool. Single source for both core's
/// `ToolDispatcher` impl and TUI listing.
#[derive(Default)]
pub struct Registry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Duplicate names are rejected. MCP-supplied names that collide with a
    /// built-in are namespaced `<server>:<name>` by the (not yet
    /// implemented) MCP bridge before reaching this call — that is the
    /// bridge's job, not the registry's.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.descriptor().name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateTool { name });
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Read-only listing for the TUI's Registry View.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|t| t.descriptor().clone()).collect()
    }

    /// What core's `ToolDispatcher::definitions` needs — just the surface
    /// the model sees, stripped of dispatch-only metadata like `edit_class`.
    pub fn definitions(&self) -> Vec<amundsen_core::ToolDefinition> {
        self.tools
            .values()
            .map(|t| {
                let d = t.descriptor();
                amundsen_core::ToolDefinition {
                    name:         d.name.clone(),
                    description:  d.description.clone(),
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
        fn permission_target(&self, _input: &Value) -> Result<String, ToolError> {
            Ok(String::new())
        }
        async fn call(&self, _call_id: &str, _input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    fn stub(name: &str) -> Arc<dyn Tool> {
        Arc::new(StubTool(ToolDescriptor {
            name:         name.into(),
            description:  "stub".into(),
            input_schema: json!({}),
            edit_class:   false,
            source:       ToolSource::Builtin,
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
