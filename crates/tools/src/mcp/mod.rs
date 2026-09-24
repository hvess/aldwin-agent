mod bridge;
mod tool;

pub use bridge::{McpBridge, McpError};
pub use tool::McpTool;

use std::sync::Arc;

use crate::error::ToolError;
use crate::registry::Registry;

/// One server's or one tool's registration failure — see
/// `register_mcp_tools`. `tool: None` means the failure was at the
/// server-enumeration level (the whole server never got any tools
/// registered); `Some` means one specific tool within an otherwise-healthy
/// server failed (a namespaced double-collision).
#[derive(Debug)]
pub struct McpRegistrationFailure {
    pub server: String,
    pub tool: Option<String>,
    pub error: ToolError,
}

/// Enumerates every configured server's tools and registers them.
/// Per aldwin-tools.md: MCP-supplied names that collide with a built-in
/// (or another already-registered MCP tool) are namespaced `<server>:<name>`;
/// otherwise the bare remote name is used. This is where each server
/// actually gets spawned (via `McpBridge::list_tools`) — see `McpBridge`'s
/// doc comment on why that's "lazy" in the sense the spec means, not
/// deferred all the way to a tool's first call.
///
/// Best-effort across servers: one server failing to enumerate (spawn
/// failure, protocol error) does not stop any other server's tools from
/// registering — everything that went wrong comes back in the returned
/// list rather than aborting the whole call, so a caller can log it (or
/// not) without one broken server taking down every other one, per
/// bootstrap.rs's "a broken MCP server must not prevent the session from
/// starting at all." A namespaced double-collision (two servers advertising
/// the identical name) is likewise recorded and skipped, not fatal to the
/// rest of the batch.
pub async fn register_mcp_tools(
    bridge: Arc<McpBridge>,
    registry: &mut Registry,
) -> Vec<McpRegistrationFailure> {
    let mut failures = Vec::new();

    for server in bridge.server_names() {
        let tools = match bridge.list_tools(&server).await {
            Ok(tools) => tools,
            Err(error) => {
                failures.push(McpRegistrationFailure {
                    server,
                    tool: None,
                    error: error.into(),
                });
                continue;
            }
        };

        for remote in tools {
            let bare_name = remote.name.to_string();
            let candidate = Arc::new(McpTool::new(
                bridge.clone(),
                server.clone(),
                bare_name.clone(),
                &remote,
            ));
            if registry.register(candidate).is_err() {
                let namespaced = format!("{server}:{bare_name}");
                let candidate = Arc::new(McpTool::new(
                    bridge.clone(),
                    server.clone(),
                    namespaced,
                    &remote,
                ));
                if let Err(error) = registry.register(candidate) {
                    failures.push(McpRegistrationFailure {
                        server: server.clone(),
                        tool: Some(bare_name),
                        error,
                    });
                }
            }
        }
    }

    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Tool, ToolDescriptor};
    use crate::Workspace;
    use aldwin_config::{McpServer, McpTransport};

    fn fake_server(name: &str) -> McpServer {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fake_mcp_server.py"
        );
        McpServer {
            name: name.into(),
            transport: McpTransport::Stdio {
                command: "python3".into(),
                args: vec![script.into()],
            },
            env: Default::default(),
        }
    }

    #[tokio::test]
    async fn registers_under_the_bare_name_when_there_is_no_collision() {
        let bridge = Arc::new(McpBridge::new(
            vec![fake_server("fake")],
            Workspace::new("."),
        ));
        let mut registry = Registry::new();
        assert!(register_mcp_tools(bridge, &mut registry).await.is_empty());
        assert!(registry.get("echo").is_some());
    }

    #[tokio::test]
    async fn namespaces_under_server_name_when_it_collides_with_a_built_in() {
        let bridge = Arc::new(McpBridge::new(
            vec![fake_server("fake")],
            Workspace::new("."),
        ));
        let mut registry = crate::builtin_registry(
            Workspace::new("."),
            std::sync::Arc::new(crate::Staging::new(Workspace::new("."))),
        );
        // Alias one built-in's registered name to "echo" indirectly isn't
        // possible without changing a built-in's name, so instead prove the
        // mechanism directly: pre-register something under "echo" the same
        // way a built-in would, then confirm the MCP tool falls back to the
        // namespaced form rather than erroring or overwriting it.
        struct Stub(ToolDescriptor);
        #[async_trait::async_trait]
        impl Tool for Stub {
            fn descriptor(&self) -> &ToolDescriptor {
                &self.0
            }
            async fn call(
                &self,
                _call_id: &str,
                _input: serde_json::Value,
                _ctx: &aldwin_core::DispatchContext,
            ) -> Result<String, ToolError> {
                Ok(String::new())
            }
        }
        registry
            .register(Arc::new(Stub(ToolDescriptor {
                name: "echo".into(),
                description: "pretend built-in".into(),
                input_schema: serde_json::json!({}),
                observes_disk: false,
            })))
            .unwrap();

        assert!(register_mcp_tools(bridge, &mut registry).await.is_empty());
        assert!(
            registry.get("fake:echo").is_some(),
            "should fall back to the namespaced name"
        );
        // The pre-registered "echo" is untouched — built-ins win unprefixed.
        assert_eq!(
            registry.get("echo").unwrap().descriptor().description,
            "pretend built-in"
        );
    }

    #[tokio::test]
    async fn one_broken_server_does_not_stop_another_healthy_ones_tools_from_registering() {
        let broken = McpServer {
            name: "broken".into(),
            transport: McpTransport::Stdio {
                command: "does-not-exist-xyz".into(),
                args: vec![],
            },
            env: Default::default(),
        };
        let bridge = Arc::new(McpBridge::new(
            vec![broken, fake_server("fake")],
            Workspace::new("."),
        ));
        let mut registry = Registry::new();

        let failures = register_mcp_tools(bridge, &mut registry).await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].server, "broken");
        assert!(failures[0].tool.is_none());
        assert!(
            registry.get("echo").is_some(),
            "the healthy server's tool must still register"
        );
    }
}
