use std::collections::HashMap;
use std::sync::Arc;

use aldwin_config::{McpServer, McpTransport};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP server {server:?}: no such server is configured")]
    UnknownServer { server: String },
    #[error(
        "MCP server {server:?}: only stdio transport is supported (http is not yet implemented)"
    )]
    UnsupportedTransport { server: String },
    #[error("MCP server {server:?}: failed to spawn {command:?}: {source}")]
    Spawn {
        server: String,
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("MCP server {server:?}: {message}")]
    Rpc { server: String, message: String },
}

/// Subprocess host for MCP servers. Config entries are supplied once at
/// construction (a snapshot — matches how `builtin_registry` takes a fixed
/// `project_root`, not a live `Config` handle); each server's connection is
/// spawned lazily, on first enumeration or call of any of its tools, and
/// persists for the bridge's lifetime.
pub struct McpBridge {
    servers: HashMap<String, McpServer>,
    running: tokio::sync::Mutex<HashMap<String, Arc<RunningService<RoleClient, ()>>>>,
}

impl McpBridge {
    pub fn new(servers: Vec<McpServer>) -> Self {
        Self {
            servers: servers.into_iter().map(|s| (s.name.clone(), s)).collect(),
            running: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn server_names(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    async fn client_for(
        &self,
        server_name: &str,
    ) -> Result<Arc<RunningService<RoleClient, ()>>, McpError> {
        let mut running = self.running.lock().await;
        // A server whose transport has closed is spawned afresh rather than
        // handed out again — cached for good, one crash failed every call to
        // that server's tools for the rest of the session.
        if let Some(client) = running
            .get(server_name)
            .filter(|c| !c.is_transport_closed())
        {
            return Ok(client.clone());
        }

        let entry = self
            .servers
            .get(server_name)
            .ok_or_else(|| McpError::UnknownServer {
                server: server_name.to_string(),
            })?;
        let McpTransport::Stdio { command, args } = &entry.transport else {
            return Err(McpError::UnsupportedTransport {
                server: server_name.to_string(),
            });
        };

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args).envs(&entry.env);
        let transport = TokioChildProcess::new(cmd).map_err(|source| McpError::Spawn {
            server: server_name.to_string(),
            command: command.clone(),
            source,
        })?;
        let service = ().serve(transport).await.map_err(|e| McpError::Rpc {
            server: server_name.to_string(),
            message: e.to_string(),
        })?;

        let service = Arc::new(service);
        running.insert(server_name.to_string(), service.clone());
        Ok(service)
    }

    /// Enumerates every tool `server_name` advertises. Spawning happens here
    /// (or in `call_tool`, whichever runs first) — see the struct doc.
    pub async fn list_tools(&self, server_name: &str) -> Result<Vec<rmcp::model::Tool>, McpError> {
        let client = self.client_for(server_name).await?;
        client.list_all_tools().await.map_err(|e| McpError::Rpc {
            server: server_name.to_string(),
            message: e.to_string(),
        })
    }

    /// Returns `(content, is_error)` — the caller (McpTool) decides how to
    /// fold `is_error` into aldwin-tools' own `ToolError` convention.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(String, bool), McpError> {
        let client = self.client_for(server_name).await?;
        let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments);
        let result = client.call_tool(params).await.map_err(|e| McpError::Rpc {
            server: server_name.to_string(),
            message: e.to_string(),
        })?;

        let text: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();
        Ok((text.join("\n"), result.is_error.unwrap_or(false)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_config::McpTransport;
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

    #[tokio::test]
    async fn lists_tools_from_a_real_spawned_server() {
        let bridge = McpBridge::new(vec![fake_server()]);
        let tools = bridge.list_tools("fake").await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
    }

    #[tokio::test]
    async fn calls_a_tool_and_gets_its_text_content_back() {
        let bridge = McpBridge::new(vec![fake_server()]);
        let mut args = serde_json::Map::new();
        args.insert("text".into(), json!("hello from the test"));
        let (content, is_error) = bridge.call_tool("fake", "echo", args).await.unwrap();
        assert_eq!(content, "hello from the test");
        assert!(!is_error);
    }

    #[tokio::test]
    async fn calling_an_unknown_tool_name_surfaces_is_error() {
        let bridge = McpBridge::new(vec![fake_server()]);
        let (content, is_error) = bridge
            .call_tool("fake", "does-not-exist", serde_json::Map::new())
            .await
            .unwrap();
        assert!(is_error);
        assert!(content.contains("no such tool"));
    }

    #[tokio::test]
    async fn unconfigured_server_name_is_a_structured_error() {
        let bridge = McpBridge::new(vec![]);
        let err = bridge.list_tools("nope").await.unwrap_err();
        assert!(matches!(err, McpError::UnknownServer { .. }));
    }

    #[tokio::test]
    async fn http_transport_is_not_yet_supported() {
        let bridge = McpBridge::new(vec![McpServer {
            name: "web".into(),
            transport: McpTransport::Http {
                url: "http://localhost:1/".into(),
            },
            env: Default::default(),
        }]);
        let err = bridge.list_tools("web").await.unwrap_err();
        assert!(matches!(err, McpError::UnsupportedTransport { .. }));
    }

    #[tokio::test]
    async fn a_server_that_died_is_spawned_again_on_the_next_call() {
        let bridge = McpBridge::new(vec![fake_server()]);
        assert!(
            bridge
                .call_tool("fake", "die", serde_json::Map::new())
                .await
                .is_err(),
            "it exits without answering"
        );

        let revived = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            // The transport's closure is noticed by rmcp's own task, a beat
            // after the process goes.
            loop {
                if let Ok(tools) = bridge.list_tools("fake").await {
                    return tools;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("a dead server must not stay cached");
        assert_eq!(revived[0].name, "echo");
    }

    #[tokio::test]
    async fn the_second_call_reuses_the_already_spawned_server() {
        let bridge = McpBridge::new(vec![fake_server()]);
        bridge.list_tools("fake").await.unwrap();
        {
            let running = bridge.running.lock().await;
            assert_eq!(running.len(), 1);
        }
        bridge.list_tools("fake").await.unwrap();
        let running = bridge.running.lock().await;
        assert_eq!(
            running.len(),
            1,
            "second call must not spawn a second process"
        );
    }
}
