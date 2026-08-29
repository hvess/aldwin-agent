mod bridge;
mod tool;

pub use bridge::{McpBridge, McpError};
pub use tool::McpTool;

use std::sync::Arc;

use crate::error::ToolError;
use crate::registry::Registry;

/// Enumerates every configured server's tools and registers them.
/// Per amundsen-tools.md: MCP-supplied names that collide with a built-in
/// (or another already-registered MCP tool) are namespaced `<server>:<name>`;
/// otherwise the bare remote name is used. This is where each server
/// actually gets spawned (via `McpBridge::list_tools`) — see `McpBridge`'s
/// doc comment on why that's "lazy" in the sense the spec means, not
/// deferred all the way to a tool's first call.
///
/// A namespaced collision (two servers advertising the identical name) is a
/// genuine configuration conflict and returns `Err` — it isn't silently
/// dropped, and it isn't allowed to shadow whatever registered first.
pub async fn register_mcp_tools(bridge: Arc<McpBridge>, registry: &mut Registry) -> Result<(), ToolError> {
    for server in bridge.server_names() {
        for remote in bridge.list_tools(&server).await? {
            let bare_name = remote.name.to_string();
            let candidate = Arc::new(McpTool::new(bridge.clone(), server.clone(), bare_name.clone(), &remote));
            if registry.register(candidate).is_err() {
                let namespaced = format!("{server}:{bare_name}");
                let candidate = Arc::new(McpTool::new(bridge.clone(), server.clone(), namespaced, &remote));
                registry.register(candidate)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Tool, ToolDescriptor, ToolSource};
    use amundsen_config::{McpServer, McpTransport};

    fn fake_server(name: &str) -> McpServer {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_mcp_server.py");
        McpServer { name: name.into(), transport: McpTransport::Stdio { command: "python3".into(), args: vec![script.into()] }, env: Default::default() }
    }

    #[tokio::test]
    async fn registers_under_the_bare_name_when_there_is_no_collision() {
        let bridge = Arc::new(McpBridge::new(vec![fake_server("fake")]));
        let mut registry = Registry::new();
        register_mcp_tools(bridge, &mut registry).await.unwrap();
        assert!(registry.get("echo").is_some());
    }

    #[tokio::test]
    async fn namespaces_under_server_name_when_it_collides_with_a_built_in() {
        let bridge = Arc::new(McpBridge::new(vec![fake_server("fake")]));
        let mut registry = crate::builtin_registry(std::path::PathBuf::from("."));
        // Alias one built-in's registered name to "echo" indirectly isn't
        // possible without changing a built-in's name, so instead prove the
        // mechanism directly: pre-register something under "echo" the same
        // way a built-in would, then confirm the MCP tool falls back to the
        // namespaced form rather than erroring or overwriting it.
        struct Stub(ToolDescriptor);
        #[async_trait::async_trait]
        impl Tool for Stub {
            fn descriptor(&self) -> &ToolDescriptor { &self.0 }
            fn permission_target(&self, _input: &serde_json::Value) -> Result<String, ToolError> { Ok(String::new()) }
            async fn call(&self, _call_id: &str, _input: serde_json::Value, _gate: &dyn crate::gate::ApprovalGate) -> Result<String, ToolError> {
                Ok(String::new())
            }
        }
        registry
            .register(Arc::new(Stub(ToolDescriptor {
                name: "echo".into(),
                description: "pretend built-in".into(),
                input_schema: serde_json::json!({}),
                edit_class: false,
                source: ToolSource::Builtin,
            })))
            .unwrap();

        register_mcp_tools(bridge, &mut registry).await.unwrap();
        assert!(registry.get("fake:echo").is_some(), "should fall back to the namespaced name");
        // The pre-registered "echo" is untouched — built-ins win unprefixed.
        assert_eq!(registry.get("echo").unwrap().descriptor().source, ToolSource::Builtin);
    }
}
