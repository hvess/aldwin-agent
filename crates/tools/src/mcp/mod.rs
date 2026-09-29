mod bridge;
mod tool;

pub use bridge::{McpBridge, McpError};
pub use tool::McpTool;

use std::sync::Arc;

use crate::error::ToolError;
use crate::registry::Registry;

/// A server or tool that `register_mcp_tools` could not register.
#[derive(Debug)]
pub struct McpRegistrationFailure {
    /// The server's name in `mcp.yaml`.
    pub server: String,
    /// The tool whose namespaced name also collided, or `None` when the
    /// server could not be enumerated.
    pub tool: Option<String>,
    /// What went wrong.
    pub error: ToolError,
}

/// Spawns each configured server (via [`McpBridge::list_tools`]) and
/// registers its tools under the bare name, or `<server>:<name>` on a
/// collision (aldwin-tools.md).
///
/// Best-effort: a failed server or a colliding namespaced name is returned,
/// never fatal, so a broken MCP server cannot stop the session starting.
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
    use aldwin_config::{Config, McpServer, McpTransport};
    use tempfile::tempdir;

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
        let bridge = Arc::new(McpBridge::new(vec![fake_server("fake")]));
        let mut registry = Registry::new();
        assert!(register_mcp_tools(bridge, &mut registry).await.is_empty());
        assert!(registry.get("echo").is_some());
    }

    #[tokio::test]
    async fn namespaces_under_server_name_when_it_collides_with_a_built_in() {
        let bridge = Arc::new(McpBridge::new(vec![fake_server("fake")]));
        let (project, global) = (tempdir().unwrap(), tempdir().unwrap());
        let config = Config::open_at(project.path(), global.path()).unwrap();
        let mut registry = crate::builtin_registry(
            config,
            Workspace::new("."),
            std::sync::Arc::new(crate::Staging::new(Workspace::new("."))),
        );
        // No built-in is named "echo", so a stub stands in for one.
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
        let bridge = Arc::new(McpBridge::new(vec![broken, fake_server("fake")]));
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
