use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use mjolnir_config::{Config, GrantList, InitOutcome, McpServer, ProviderConfig, ProviderKind, Scope, PROVIDER_VERSION};
use mjolnir_core::{Agent, LlmClient, LlmError, LlmEvent, LlmRequest};
use mjolnir_permissions::Engine;
use mjolnir_tools::{register_mcp_tools, Dispatcher, McpBridge};
use futures::Stream;
use tokio::sync::mpsc;

use crate::context;
use crate::context_approval;
use crate::error::StartupError;
use crate::slash;

/// Dispatches to whichever client `provider.yaml`'s `kind` selects.
/// `Agent<C, D>` is generic over `C: LlmClient` (monomorphized, not a trait
/// object), so this small enum exists to give `run()` a single concrete
/// type to build an `Agent` with — the alternative would be duplicating the
/// whole channel/task/TUI wiring below in two near-identical branches.
enum AnyLlmClient {
    Anthropic(mjolnir_llm::AnthropicClient),
    OpenAi(mjolnir_llm::OpenAiCompatibleClient),
}

impl LlmClient for AnyLlmClient {
    fn stream<'a>(&'a self, request: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        match self {
            Self::Anthropic(c) => c.stream(request),
            Self::OpenAi(c) => c.stream(request),
        }
    }
}

const CHANNEL_CAPACITY: usize = 64;

/// The `provider.yaml` a first-run answer writes: everything but the model
/// comes straight off the catalogue row the developer picked, and the model
/// is that provider's own default (`/model` changes it afterwards).
///
/// `provider.yaml` deliberately has no field a plaintext key could go in
/// (see `ProviderConfig`), so this writes the key variable's *name* and the
/// developer exports the key themselves.
fn first_run_provider_config(provider: &mjolnir_llm::Provider) -> ProviderConfig {
    ProviderConfig {
        version:                  PROVIDER_VERSION,
        provider:                 provider.kind,
        model:                    provider.default_model().to_string(),
        base_url:                 provider.base_url.map(String::from),
        api_key_env:              provider.api_key_env.to_string(),
        extended_thinking_budget: None,
    }
}

/// The startup sequence from mjolnir-cli.md, in order:
/// 1. init_global_if_empty — refuse to start on PartiallyPresent.
/// 2. Load all config layers (`Config::open` — refuses to start on any
///    parse failure, schema error, unknown major, or missing env var).
/// 3. Build the additional-context string.
/// 4. Instantiate AnthropicClient, PermissionsEngine, ToolDispatcher.
/// 5. Create the agent loop.
/// 6. Launch the TUI.
/// 7. Block on TUI exit; drop channels; wait for the agent to drain.
///
/// Step 4's PermissionsEngine is actually built ahead of step 3 here, not
/// after: per mjolnir-permissions.md, "the session initializer tests each
/// candidate [context] file" through the engine's own check_context_file
/// before composing the additional-context string, which needs the engine
/// to already exist. The spec's numbered list is the right order to read
/// it in, not a claim that 3 has zero dependency on 4.
pub async fn run() -> Result<(), StartupError> {
    let cwd = std::env::current_dir().map_err(StartupError::Cwd)?;

    let config = Config::open(&cwd)?;
    match config.init_global_if_empty()? {
        InitOutcome::Created | InitOutcome::AlreadyPresent => {}
        InitOutcome::PartiallyPresent { missing } => return Err(StartupError::PartiallyPresentGlobalConfig { missing }),
    }

    // `theme` is global-only (see mjolnir-config's annotated tui.yaml) —
    // resolved once, before anything draws, and never revisited for the rest
    // of the session (mjolnir_tui::palette's own doc comment explains why
    // this is a one-time explicit choice, not a live setting). Read here
    // rather than just before the session TUI because first run draws first.
    let theme = mjolnir_tui::Theme::from_config(config.global_tui().theme.as_deref());

    // First run, per ADR 0001. Two independent questions, and only the
    // unanswered ones are asked:
    //
    // * no provider config resolves anywhere — where the model runs is
    //   unknown, and it has to be answered before the LLM client below can
    //   be constructed;
    // * this project has no `.mjolnir/permissions.yaml` — a directory the
    //   harness has never been pointed at, whose access posture is
    //   therefore undeclared.
    //
    // The file's *existence* is the test, not whether it parses to an empty
    // allow list: a developer who has deliberately allowed nothing has
    // answered the question, and must not be asked again on every start.
    let needs_provider = config.global_provider().is_err();
    let needs_access = !cwd.join(".mjolnir").join("permissions.yaml").exists();
    if needs_provider || needs_access {
        // mjolnir-tui is handed the display half of each catalogue row and
        // nothing else — it renders the list, it does not know what an
        // endpoint or a key variable is, and it does not depend on this
        // crate or on mjolnir-llm to find out.
        let choices = mjolnir_llm::PROVIDERS
            .iter()
            .map(|p| mjolnir_tui::ProviderChoice::new(p.id, p.purpose))
            .collect::<Vec<_>>();
        // `None` means the developer quit without answering. Nothing is
        // written and no session opens — a first run that was dismissed must
        // not fall back to defaults, least of all for the access question.
        let Some(answers) = mjolnir_tui::run_first_run(theme, choices, mjolnir_llm::CURATED, needs_provider, needs_access)
            .await
            .map_err(StartupError::FirstRun)?
        else {
            return Ok(());
        };
        // Answered only when it was asked — the access-only run leaves the
        // provider the developer already configured alone.
        if let Some(id) = answers.provider.as_deref() {
            let picked = mjolnir_llm::provider(id).ok_or_else(|| StartupError::UnknownProvider { id: id.to_string() })?;
            config.set_provider(Scope::Global, first_run_provider_config(picked)).map_err(StartupError::FirstRunWrite)?;
        }
        // Again, answered only when it was asked. `add_grant` only ever adds,
        // so writing an unasked answer into a directory that already has a
        // `permissions.yaml` could only widen an allow list the developer had
        // already settled — the one direction a default-deny harness must
        // never move on its own.
        //
        // Written even when the tier grants nothing: the file's existence is
        // what records that this directory's question has been answered, so
        // an `ask` answer has to leave one behind or it would be asked again
        // on the next start.
        if let Some(tier) = answers.access {
            config.ensure_permissions(Scope::Project).map_err(StartupError::FirstRunWrite)?;
            for entry in tier.grants() {
                config.add_grant(Scope::Project, GrantList::Allow, entry).map_err(StartupError::FirstRunWrite)?;
            }
        }
    }

    let permissions = Arc::new(Engine::new(config.clone()));

    let approved_context_files = context_approval::resolve(&cwd, &permissions);
    let additional_context = context::build(&cwd, &approved_context_files);

    let project_provider = config.project_provider();
    let global_provider = config.global_provider().map_err(StartupError::NoProvider)?;
    let provider_config = mjolnir_llm::resolve(project_provider.as_ref(), &global_provider);
    let model_name = provider_config.model.clone();
    // What this process actually booted on, captured before anything can
    // rewrite `provider.yaml` underneath it. `/model` persists a choice it
    // cannot apply to a running session, and has to name what the session
    // is still using — which stops being what is on disk the moment the
    // command is used once.
    let effective_provider = project_provider.clone().unwrap_or_else(|| global_provider.clone());
    let session_model = slash::qualified(&effective_provider, mjolnir_llm::identify(&effective_provider));
    let client = match provider_config.kind {
        ProviderKind::Anthropic => AnyLlmClient::Anthropic(mjolnir_llm::AnthropicClient::new(provider_config)?),
        ProviderKind::OpenaiCompatible => AnyLlmClient::OpenAi(mjolnir_llm::OpenAiCompatibleClient::new(provider_config)?),
    };

    let mut registry = mjolnir_tools::builtin_registry(cwd.clone());
    let mcp_bridge = Arc::new(McpBridge::new(merged_mcp_servers(&config)));
    // Best-effort per server/tool (see register_mcp_tools' own doc comment)
    // — one broken server must not prevent the session from starting, or
    // stop any other server's tools from registering.
    for failure in register_mcp_tools(mcp_bridge, &mut registry).await {
        match failure.tool {
            Some(tool) => tracing::warn!("MCP server {:?}: tool {tool:?} not registered: {}", failure.server, failure.error),
            None => tracing::warn!("MCP server {:?}: no tools registered: {}", failure.server, failure.error),
        }
    }
    let dispatcher = Dispatcher::new(registry, permissions.clone());

    let agent = Agent::new(client, dispatcher, model_name.clone(), Some(additional_context.as_str()));

    // TUI -> interceptor -> core, so slash commands never reach Submit;
    // core -> TUI directly for events (no interception needed there).
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    let interceptor = tokio::spawn(slash::run_interceptor(tui_cmd_rx, agent_cmd_tx, config.clone(), session_model, event_tx.clone()));
    let agent_task = tokio::spawn(agent.run(agent_cmd_rx, event_tx));

    let tui_result = mjolnir_tui::run(event_rx, tui_cmd_tx, model_name, permissions, theme).await;

    // The TUI dropped its command sender on return, closing tui_cmd_rx;
    // the interceptor then drops agent_cmd_tx, closing the core's command
    // channel — both drain to completion on their own from here.
    let _ = interceptor.await;
    let _ = agent_task.await;

    tui_result.map_err(StartupError::Io)
}

/// A project-scope server entry replaces a global one of the same name
/// entirely (see mjolnir-config's annotated mcp.yaml) — this is that same
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
    use mjolnir_config::McpTransport;

    fn server(name: &str, command: &str) -> McpServer {
        McpServer { name: name.into(), transport: McpTransport::Stdio { command: command.into(), args: vec![] }, env: Default::default() }
    }

    #[test]
    fn project_scope_server_replaces_a_global_one_of_the_same_name() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();

        config.add_mcp_server(mjolnir_config::Scope::Global, server("fs", "global-fs-server")).unwrap();
        config.add_mcp_server(mjolnir_config::Scope::Project, server("fs", "project-fs-server")).unwrap();
        config.add_mcp_server(mjolnir_config::Scope::Global, server("other", "other-server")).unwrap();

        let merged = merged_mcp_servers(&config);
        assert_eq!(merged.len(), 2);
        let fs = merged.iter().find(|s| s.name == "fs").unwrap();
        assert!(matches!(&fs.transport, McpTransport::Stdio { command, .. } if command == "project-fs-server"));
    }
}
