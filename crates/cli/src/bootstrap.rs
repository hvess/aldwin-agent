use std::collections::BTreeMap;
use std::sync::Arc;

use amundsen_config::{Config, InitOutcome, McpServer};
use amundsen_core::Agent;
use amundsen_permissions::Engine;
use amundsen_tools::{register_mcp_tools, Dispatcher, McpBridge};
use tokio::sync::mpsc;

use crate::context;
use crate::error::StartupError;
use crate::slash;

const CHANNEL_CAPACITY: usize = 64;

/// The startup sequence from amundsen-cli.md, in order:
/// 1. init_global_if_empty — refuse to start on PartiallyPresent.
/// 2. Load all config layers (`Config::open` — refuses to start on any
///    parse failure, schema error, unknown major, or missing env var).
/// 3. Build the additional-context string.
/// 4. Instantiate AnthropicClient, PermissionsEngine, ToolDispatcher.
/// 5. Create the agent loop.
/// 6. Launch the TUI.
/// 7. Block on TUI exit; drop channels; wait for the agent to drain.
pub async fn run() -> Result<(), StartupError> {
    let cwd = std::env::current_dir().expect("current working directory must be readable");

    let config = Config::open(&cwd)?;
    match config.init_global_if_empty()? {
        InitOutcome::Created | InitOutcome::AlreadyPresent => {}
        InitOutcome::PartiallyPresent { missing } => return Err(StartupError::PartiallyPresentGlobalConfig { missing }),
    }

    let additional_context = context::build(&cwd, &config);

    let project_provider = config.project_provider();
    let global_provider = config.global_provider().map_err(StartupError::NoProvider)?;
    let provider_config = amundsen_llm::resolve(project_provider.as_ref(), &global_provider);
    let model_name = provider_config.model.clone();
    let client = amundsen_llm::AnthropicClient::new(provider_config)?;

    let permissions = Arc::new(Engine::new(config.clone()));

    let mut registry = amundsen_tools::builtin_registry(cwd.clone());
    let mcp_bridge = Arc::new(McpBridge::new(merged_mcp_servers(&config)));
    // A broken MCP server must not prevent the session from starting at
    // all — built-ins and every other server's tools should still work.
    if let Err(e) = register_mcp_tools(mcp_bridge, &mut registry).await {
        tracing::warn!("MCP tool registration failed, continuing without it: {e}");
    }
    let dispatcher = Dispatcher::new(registry, permissions.clone());

    let agent = Agent::new(client, dispatcher, model_name.clone(), Some(additional_context.as_str()));

    // TUI -> interceptor -> core, so slash commands never reach Submit;
    // core -> TUI directly for events (no interception needed there).
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    let interceptor = tokio::spawn(slash::run_interceptor(tui_cmd_rx, agent_cmd_tx, config.clone(), event_tx.clone()));
    let agent_task = tokio::spawn(agent.run(agent_cmd_rx, event_tx));

    let tui_result = amundsen_tui::run(event_rx, tui_cmd_tx, model_name, permissions).await;

    // The TUI dropped its command sender on return, closing tui_cmd_rx;
    // the interceptor then drops agent_cmd_tx, closing the core's command
    // channel — both drain to completion on their own from here.
    let _ = interceptor.await;
    let _ = agent_task.await;

    tui_result.map_err(StartupError::Io)
}

/// A project-scope server entry replaces a global one of the same name
/// entirely (see amundsen-config's annotated mcp.yaml) — this is that same
/// rule applied across the two already-loaded snapshots.
fn merged_mcp_servers(config: &Config) -> Vec<McpServer> {
    let mut by_name: BTreeMap<String, McpServer> = config.global_mcp().servers.into_iter().map(|s| (s.name.clone(), s)).collect();
    for server in config.project_mcp().servers {
        by_name.insert(server.name.clone(), server);
    }
    by_name.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use amundsen_config::McpTransport;

    fn server(name: &str, command: &str) -> McpServer {
        McpServer { name: name.into(), transport: McpTransport::Stdio { command: command.into(), args: vec![] }, env: Default::default() }
    }

    #[test]
    fn project_scope_server_replaces_a_global_one_of_the_same_name() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();

        config.add_mcp_server(amundsen_config::Scope::Global, server("fs", "global-fs-server")).unwrap();
        config.add_mcp_server(amundsen_config::Scope::Project, server("fs", "project-fs-server")).unwrap();
        config.add_mcp_server(amundsen_config::Scope::Global, server("other", "other-server")).unwrap();

        let merged = merged_mcp_servers(&config);
        assert_eq!(merged.len(), 2);
        let fs = merged.iter().find(|s| s.name == "fs").unwrap();
        assert!(matches!(&fs.transport, McpTransport::Stdio { command, .. } if command == "project-fs-server"));
    }
}
