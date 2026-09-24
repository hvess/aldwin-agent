use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::bridge::McpBridge;
use crate::error::ToolError;
use crate::registry::{Tool, ToolDescriptor};
use aldwin_core::DispatchContext;

/// One remote MCP tool, proxied through `McpBridge`. It runs in its own
/// process over the real tree, so the dispatcher opens the review before it
/// the way it does before `run`; an MCP tool that edits files does so
/// without a diff (open-tasks 13), inside the workspace the sandbox allows.
pub struct McpTool {
    descriptor: ToolDescriptor,
    bridge: Arc<McpBridge>,
    server: String,
    remote_name: String,
}

impl McpTool {
    pub fn new(
        bridge: Arc<McpBridge>,
        server: String,
        registered_name: String,
        remote: &rmcp::model::Tool,
    ) -> Self {
        let input_schema = serde_json::Value::Object((*remote.input_schema).clone());
        Self {
            descriptor: ToolDescriptor {
                name: registered_name,
                description: remote.description.clone().unwrap_or_default().into_owned(),
                input_schema,
                // Its own process, over the real tree.
                observes_disk: true,
            },
            bridge,
            server,
            remote_name: remote.name.clone().into_owned(),
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let arguments = match input {
            Value::Object(map) => map,
            Value::Null => serde_json::Map::new(),
            other => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), other);
                map
            }
        };

        let (content, is_error) = self
            .bridge
            .call_tool(&self.server, &self.remote_name, arguments)
            .await?;
        if is_error {
            Err(ToolError::McpToolError {
                server: self.server.clone(),
                tool: self.remote_name.clone(),
                message: content,
            })
        } else {
            Ok(content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workspace;
    use aldwin_config::{McpServer, McpTransport};
    use serde_json::json;

    fn fake_server() -> McpServer {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fake_mcp_server.py"
        );
        McpServer {
            name: "fake".into(),
            transport: McpTransport::Stdio {
                command: "python3".into(),
                args: vec![script.into()],
            },
            env: Default::default(),
        }
    }

    fn remote_echo_tool() -> rmcp::model::Tool {
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), json!("object"));
        rmcp::model::Tool::new(
            "echo",
            "Echoes the given text back.",
            std::sync::Arc::new(schema),
        )
    }

    #[tokio::test]
    async fn call_proxies_through_the_bridge_and_returns_text() {
        let bridge = Arc::new(McpBridge::new(vec![fake_server()], Workspace::new(".")));
        let tool = McpTool::new(
            bridge,
            "fake".into(),
            "fake:echo".into(),
            &remote_echo_tool(),
        );

        let (ctx, _e, _p) = crate::test_support::dispatch_context();
        let out = tool.call("c1", json!({"text": "hi"}), &ctx).await.unwrap();
        assert_eq!(out, "hi");
    }

    #[tokio::test]
    async fn is_error_result_becomes_a_structured_tool_error() {
        let bridge = Arc::new(McpBridge::new(vec![fake_server()], Workspace::new(".")));
        // Registered under the name "echo" but proxy to a remote name that
        // doesn't exist server-side, to force an isError result.
        let mut broken = remote_echo_tool();
        broken.name = "missing".into();
        let tool = McpTool::new(bridge, "fake".into(), "fake:missing".into(), &broken);

        let (ctx, _e, _p) = crate::test_support::dispatch_context();
        let err = tool.call("c1", json!({}), &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::McpToolError { .. }));
    }

    /// An MCP server runs over the real tree, so the review opens before
    /// any of its tools the way it does before `run`.
    #[test]
    fn an_mcp_tool_observes_the_disk() {
        let bridge = Arc::new(McpBridge::new(vec![], Workspace::new(".")));
        let tool = McpTool::new(
            bridge,
            "fake".into(),
            "fake:echo".into(),
            &remote_echo_tool(),
        );
        assert!(tool.descriptor().observes_disk);
    }
}
