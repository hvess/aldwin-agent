use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::bridge::McpBridge;
use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::registry::{Tool, ToolDescriptor, ToolSource};

/// One remote MCP tool, proxied through `McpBridge`. `edit_class` is always
/// `false` at registration — per mjolnir-tools.md, MCP tools only ever
/// become edit-shaped via a first-invocation follow-up, never upfront. That
/// follow-up (and the config persistence it needs) isn't implemented in
/// this pass; every MCP tool goes through the standard four-tier prompt.
pub struct McpTool {
    descriptor:  ToolDescriptor,
    bridge:      Arc<McpBridge>,
    server:      String,
    remote_name: String,
}

impl McpTool {
    pub fn new(bridge: Arc<McpBridge>, server: String, registered_name: String, remote: &rmcp::model::Tool) -> Self {
        let input_schema = serde_json::Value::Object((*remote.input_schema).clone());
        Self {
            descriptor: ToolDescriptor {
                name: registered_name,
                description: remote.description.clone().unwrap_or_default().into_owned(),
                input_schema,
                edit_class: false,
                source: ToolSource::Mcp { server: server.clone() },
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

    /// MCP tool arguments vary arbitrarily by tool, unlike Read/shell's
    /// single clear string field — the whole serialised argument object is
    /// the coarsest-but-workable match target; a developer can still grant
    /// broadly ("always") rather than needing a precise glob over it.
    fn permission_target(&self, input: &Value) -> Result<String, ToolError> {
        Ok(serde_json::to_string(input).unwrap_or_default())
    }

    async fn call(&self, _call_id: &str, input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let arguments = match input {
            Value::Object(map) => map,
            Value::Null => serde_json::Map::new(),
            other => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), other);
                map
            }
        };

        let (content, is_error) = self.bridge.call_tool(&self.server, &self.remote_name, arguments).await?;
        if is_error {
            Err(ToolError::McpToolError { server: self.server.clone(), tool: self.remote_name.clone(), message: content })
        } else {
            Ok(content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_config::{McpServer, McpTransport};
    use serde_json::json;
    use std::sync::Arc as StdArc;

    fn fake_server() -> McpServer {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_mcp_server.py");
        McpServer { name: "fake".into(), transport: McpTransport::Stdio { command: "python3".into(), args: vec![script.into()] }, env: Default::default() }
    }

    fn remote_echo_tool() -> rmcp::model::Tool {
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), json!("object"));
        rmcp::model::Tool::new("echo", "Echoes the given text back.", std::sync::Arc::new(schema))
    }

    #[tokio::test]
    async fn call_proxies_through_the_bridge_and_returns_text() {
        let bridge = StdArc::new(McpBridge::new(vec![fake_server()]));
        let tool = McpTool::new(bridge, "fake".into(), "fake:echo".into(), &remote_echo_tool());

        let out = tool.call("c1", json!({"text": "hi"}), &crate::test_support::ALWAYS_APPROVE).await.unwrap();
        assert_eq!(out, "hi");
    }

    #[tokio::test]
    async fn is_error_result_becomes_a_structured_tool_error() {
        let bridge = StdArc::new(McpBridge::new(vec![fake_server()]));
        // Registered under the name "echo" but proxy to a remote name that
        // doesn't exist server-side, to force an isError result.
        let mut broken = remote_echo_tool();
        broken.name = "missing".into();
        let tool = McpTool::new(bridge, "fake".into(), "fake:missing".into(), &broken);

        let err = tool.call("c1", json!({}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::McpToolError { .. }));
    }

    #[test]
    fn descriptor_is_never_edit_class_at_registration() {
        let bridge = StdArc::new(McpBridge::new(vec![]));
        let tool = McpTool::new(bridge, "fake".into(), "fake:echo".into(), &remote_echo_tool());
        assert!(!tool.descriptor().edit_class);
        assert_eq!(tool.descriptor().source, ToolSource::Mcp { server: "fake".into() });
    }

    #[test]
    fn permission_target_is_the_serialised_arguments() {
        let bridge = StdArc::new(McpBridge::new(vec![]));
        let tool = McpTool::new(bridge, "fake".into(), "fake:echo".into(), &remote_echo_tool());
        let target = tool.permission_target(&json!({"text": "hi"})).unwrap();
        assert_eq!(target, r#"{"text":"hi"}"#);
    }
}
