use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use aldwin_config::{Config, InitOutcome, McpServer, ProviderKind};
use aldwin_core::{Agent, LlmClient, LlmError, LlmEvent, LlmRequest};
use aldwin_tools::{register_mcp_tools, Dispatcher, McpBridge, Staging, Workspace};
use futures::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinError;

use crate::context;
use crate::error::StartupError;
use crate::history::History;
use crate::slash;

/// Whichever client the provider settings in force select, built the same
/// way at startup and on every `/model` after it — so a model swapped into a
/// running session is reached exactly as one chosen at launch would be.
///
/// The one place a `provider.yaml`'s settings become aldwin-llm's: that
/// crate knows nothing of files, scopes or overlays.
fn build_client(
    provider: &aldwin_config::ProviderConfig,
) -> Result<Arc<dyn LlmClient>, aldwin_llm::LlmClientInitError> {
    let config = aldwin_llm::ProviderConfig {
        kind: provider.provider,
        model: provider.model.clone(),
        api_key_env: provider.api_key_env.clone(),
        base_url: provider.base_url.clone(),
        extended_thinking_budget: provider.extended_thinking_budget,
    };
    Ok(match config.kind {
        ProviderKind::Anthropic => Arc::new(aldwin_llm::AnthropicClient::new(config)?),
        ProviderKind::OpenaiCompatible => {
            Arc::new(aldwin_llm::OpenAiCompatibleClient::new(config)?)
        }
    })
}

/// The client a session starts on when no provider is configured (ADR
/// 0009 §6: there is no first-run screen, so the launch card opens with
/// `Model  not set` and the first message asks). Any request through it
/// answers with the one thing that is true.
struct Unconfigured;

impl LlmClient for Unconfigured {
    fn stream<'a>(
        &'a self,
        _: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        Box::pin(futures::stream::iter([Err(LlmError::Terminal {
            attempts: 0,
            message: "no model is configured yet — pick one with /model".into(),
        })]))
    }
}

/// The session's client, behind a swap.
///
/// `Agent<C, D>` takes its client by value and owns it for the life of the
/// process, so `/model` cannot hand it a new one — but it can replace what
/// is *inside* the one it already has. That is this: the agent is handed the
/// handle, `/model` rebuilds the client in it, and core stays generic over
/// `C: LlmClient` without learning that providers exist (see
/// `slash::ModelSwitch`). A trait object, so both provider clients fit and
/// a test can put one of its own in there and stream through it.
#[derive(Clone)]
struct ClientHandle(Arc<std::sync::RwLock<Arc<dyn LlmClient>>>);

impl ClientHandle {
    fn new(client: Arc<dyn LlmClient>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(client)))
    }

    fn store(&self, client: Arc<dyn LlmClient>) {
        *self.0.write().expect("client lock poisoned") = client;
    }
}

impl LlmClient for ClientHandle {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        // Resolved once, when the request starts, and held by the stream
        // for as long as it runs: a swap landing mid-turn cannot pull the
        // client out from under a request already in flight.
        let client = self.0.read().expect("client lock poisoned").clone();
        Box::pin(async_stream::stream! {
            let mut inner = client.stream(request);
            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

impl slash::ModelSwitch for ClientHandle {
    /// Builds first and stores second, so a client that cannot be
    /// constructed — the new provider's `api_key_env` is not exported —
    /// leaves the session on the one it has.
    fn switch(
        &self,
        config: &aldwin_config::ProviderConfig,
    ) -> Result<(), aldwin_llm::LlmClientInitError> {
        self.store(build_client(config)?);
        Ok(())
    }
}

const CHANNEL_CAPACITY: usize = 64;

/// The display halves of the whole catalogue, in catalogue order — what
/// the `/model` question and the first message's two questions list.
/// aldwin-tui is handed ids, purposes and context sizes and nothing else:
/// it renders the list, it does not know what an endpoint or a key
/// variable is, and it does not depend on this crate or on aldwin-llm to
/// find out.
fn catalogue_choices() -> Vec<aldwin_tui::ProviderChoice> {
    aldwin_llm::PROVIDERS
        .iter()
        .map(|p| aldwin_tui::ProviderChoice {
            id: p.id.to_string(),
            purpose: p.purpose.to_string(),
            models: p
                .models
                .iter()
                .map(|m| aldwin_tui::ModelChoice {
                    id: m.id.to_string(),
                    purpose: m.purpose.to_string(),
                    context: m.context,
                })
                .collect(),
        })
        .collect()
}

/// The context files the session carries — `CLAUDE.md` and `AGENTS.md`
/// at the project root, whichever exist. Reading them is a read, and reads
/// need no permission (ADR 0009 §6); the old stdin prompt for each one is
/// gone with the rest of the asking.
fn context_files(cwd: &Path) -> Vec<PathBuf> {
    ["CLAUDE.md", "AGENTS.md"]
        .iter()
        .map(|f| cwd.join(f))
        .filter(|p| p.is_file())
        .collect()
}

/// The startup sequence from aldwin-cli.md, in order:
/// 1. init_global_if_empty — refuse to start on PartiallyPresent.
/// 2. Load all config layers (`Config::open` — refuses to start on any
///    parse failure, schema error, unknown major, or missing env var).
/// 3. Build the additional-context string.
/// 4. Instantiate the client (or the unconfigured stand-in), the
///    workspace, the staging area and the dispatcher.
/// 5. Create the agent loop.
/// 6. Launch the TUI.
/// 7. Block on TUI exit; drop channels; wait for the agent to drain.
///
/// There is no first-run screen: every launch opens straight to the field
/// under the launch card (ADR 0009 §6).
pub async fn run() -> Result<(), StartupError> {
    let cwd = std::env::current_dir().map_err(StartupError::Cwd)?;

    let config = Config::open(&cwd)?;
    match config.init_global_if_empty()? {
        InitOutcome::Created | InitOutcome::AlreadyPresent => {}
        InitOutcome::PartiallyPresent { missing } => {
            return Err(StartupError::PartiallyPresentGlobalConfig { missing })
        }
    }

    // `theme` is global-only — resolved once, before anything draws.
    let theme = aldwin_tui::Theme::from_config(config.global_tui().theme.as_deref());

    // What the session boots on. Nothing configured is not an error any
    // more: the launch card says `Model  not set` and the first message
    // asks, and `/model` moves the session onto the answer.
    let effective_provider = config.effective_provider();
    let (client, model_name, session_model) = match &effective_provider {
        Some(effective) => (
            ClientHandle::new(build_client(effective)?),
            effective.model.clone(),
            slash::qualified(effective, slash::identify(effective)),
        ),
        None => (
            ClientHandle::new(Arc::new(Unconfigured)),
            String::new(),
            String::new(),
        ),
    };

    // The workspace: the project root, plus whatever `.aldwin/permissions.yaml`
    // declares (ADR 0007) — the one boundary (ADR 0011). Project scope only,
    // and stated rather than inferred.
    let workspace = Workspace::new(cwd.clone());
    let reach_notice = apply_roots(&config, &cwd, &workspace);
    let additional_context = context::build(&cwd, &workspace.roots(), &context_files(&cwd));

    // TUI -> interceptor -> core, so slash commands never reach Submit;
    // core -> TUI directly for events (no interception needed there).
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    // Every edit of a turn waits here for the review (ADR 0009 §4).
    let staging = Arc::new(Staging::new(workspace.clone()));
    let mut registry = aldwin_tools::builtin_registry(workspace.clone(), staging.clone());
    let mcp_bridge = Arc::new(McpBridge::new(
        merged_mcp_servers(&config),
        workspace.clone(),
    ));
    // Best-effort per server/tool — one broken server must not prevent the
    // session from starting, or stop any other server's tools registering.
    // Each failure is said, because a server the developer configured and
    // cannot use is something they will otherwise go looking for.
    for failure in register_mcp_tools(mcp_bridge, &mut registry).await {
        let message = match failure.tool {
            Some(tool) => format!(
                "The MCP tool {tool} from {} could not be registered: {}",
                failure.server, failure.error
            ),
            None => format!(
                "The MCP server {} could not be started: {}",
                failure.server, failure.error
            ),
        };
        let _ = event_tx.try_send(aldwin_core::Event::Notice { message });
    }
    let dispatcher = Dispatcher::new(registry, staging).with_notices(event_tx.clone());

    // This session's transcript. `None` when the history directory cannot be
    // written — the session then runs without one, having said so once.
    // History must never be able to stop a session starting.
    let history = match History::open(
        config.history_dir(),
        &cwd,
        model_name.clone(),
        event_tx.clone(),
    ) {
        Ok(history) => Some(history),
        Err(e) => {
            let message = format!("history is off for this session: {e}");
            let _ = event_tx.try_send(aldwin_core::Event::Notice { message });
            None
        }
    };
    let sessions = history.as_ref().map(|h| h.resumable()).unwrap_or_default();

    let agent = Agent::new(
        client.clone(),
        dispatcher,
        Some(additional_context.as_str()),
    );
    let agent = match history.clone() {
        Some(history) => agent.with_sink(history),
        None => agent,
    };
    let session_state = {
        let (config, cwd, workspace) = (config.clone(), cwd.clone(), workspace.clone());
        slash::Session::new(session_model, Box::new(client))
            .with_after_reload(Box::new(move || apply_roots(&config, &cwd, &workspace)))
    };

    // Said once, at the top of the session: a workspace wider than the
    // project, a `permissions.yaml` still carrying keys from an earlier
    // model, and a system where nothing Aldwin starts can be confined.
    let notices = [
        reach_notice,
        stale_keys_notice(&config),
        unconfined_notice(aldwin_tools::sandbox::unavailable()),
    ];
    for message in notices.into_iter().flatten() {
        let _ = event_tx.try_send(aldwin_core::Event::Notice { message });
    }

    let interceptor = tokio::spawn(slash::run_interceptor(
        tui_cmd_rx,
        agent_cmd_tx,
        config.clone(),
        session_state,
        history,
        event_tx.clone(),
    ));
    let agent_task = tokio::spawn(agent.run(agent_cmd_rx, event_tx));

    let session = aldwin_tui::SessionProvider {
        catalogue: catalogue_choices(),
        current_provider: effective_provider
            .as_ref()
            .and_then(slash::identify)
            .map(|p| p.id.to_string()),
        sessions,
    };
    let tui_result = aldwin_tui::run(event_rx, tui_cmd_tx, model_name, theme, session).await;

    let interceptor = interceptor.await;
    let agent = agent_task.await;

    tui_result?;
    finished("interceptor", interceptor)?;
    finished("agent", agent)
}

/// How a session task ended, once the terminal is the developer's again. A
/// task that panicked is an error the process exits on, said after the
/// TUI has restored the screen — not a quiet exit 0 over a session that
/// had stopped answering.
fn finished(task: &'static str, joined: Result<(), JoinError>) -> Result<(), StartupError> {
    joined.map_err(|source| StartupError::TaskFailed { task, source })
}

/// A project-scope server entry replaces a global one of the same name
/// entirely (see aldwin-config's annotated mcp.yaml).
fn merged_mcp_servers(config: &Config) -> Vec<McpServer> {
    let mut by_name: BTreeMap<String, McpServer> = config
        .global_mcp()
        .servers
        .into_iter()
        .map(|s| (s.name.clone(), s))
        .collect();
    for server in config.project_mcp().servers {
        by_name.insert(server.name.clone(), server);
    }
    by_name.into_values().collect()
}

/// `allow:`, `default:` and `deny:` in a `permissions.yaml` do nothing now
/// (ADR 0011); a file that still says something through one is said out
/// loud once rather than silently honoured or silently ignored.
fn stale_keys_notice(config: &Config) -> Option<String> {
    let stale = config.stale_permissions();
    if stale.is_empty() {
        return None;
    }
    let files: Vec<String> = stale.iter().map(|p| p.display().to_string()).collect();
    Some(format!(
        "{} still has `allow:`, `default:` or `deny:` from an earlier permission model. None of them does anything now: the workspace is the only boundary, and every edit is reviewed. Only `roots:` is read.",
        files.join(" and ")
    ))
}

/// Where processes cannot be confined, everything Aldwin starts runs
/// unconfined — and the developer hears that once, here, never silently
/// (ADR 0011).
fn unconfined_notice(reason: Option<&str>) -> Option<String> {
    reason
        .map(|reason| format!("Commands can write outside the workspace on this system: {reason}."))
}

/// Points `workspace` at the roots the project's `permissions.yaml` declares
/// now, and returns what the developer should be told about it: the roots in
/// force beyond the project, and any that were written down but do not exist.
/// `None` when there is nothing beyond the project root and nothing dropped.
///
/// Called at startup and again after `/reload-config`. A relative root
/// resolves against the project root.
fn apply_roots(config: &Config, cwd: &Path, workspace: &Workspace) -> Option<String> {
    let declared: Vec<PathBuf> = config
        .project_permissions()
        .roots
        .iter()
        .map(|r| {
            if r.is_absolute() {
                r.clone()
            } else {
                cwd.join(r)
            }
        })
        .collect();
    let dropped = workspace.set_extra_roots(declared);
    let extra: Vec<String> = workspace
        .roots()
        .iter()
        .skip(1)
        .map(|r| r.display().to_string())
        .collect();

    let mut parts = Vec::new();
    if !extra.is_empty() {
        parts.push(format!(
            "The workspace also takes in {} (roots in .aldwin/permissions.yaml).",
            extra.join(", ")
        ));
    }
    if !dropped.is_empty() {
        let names: Vec<String> = dropped.iter().map(|r| r.display().to_string()).collect();
        parts.push(format!(
            "Ignored roots that do not exist: {}.",
            names.join(", ")
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_config::McpTransport;

    #[test]
    fn project_scope_server_replaces_a_global_one_of_the_same_name() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let mcp = |command: &str, other: &str| {
            format!(
                "version: 1\nservers:\n  - name: fs\n    kind: stdio\n    command: {command}\n{other}"
            )
        };
        std::fs::write(
            global.path().join("mcp.yaml"),
            mcp(
                "global-fs-server",
                "  - name: other\n    kind: stdio\n    command: other-server\n",
            ),
        )
        .unwrap();
        std::fs::create_dir(project.path().join(".aldwin")).unwrap();
        std::fs::write(
            project.path().join(".aldwin/mcp.yaml"),
            mcp("project-fs-server", ""),
        )
        .unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();

        let merged = merged_mcp_servers(&config);
        assert_eq!(merged.len(), 2);
        let fs = merged.iter().find(|s| s.name == "fs").unwrap();
        assert!(
            matches!(&fs.transport, McpTransport::Stdio { command, .. } if command == "project-fs-server")
        );
    }

    struct NamedClient(&'static str);

    impl LlmClient for NamedClient {
        fn stream<'a>(
            &'a self,
            _: LlmRequest<'a>,
        ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            Box::pin(futures::stream::iter([Ok(LlmEvent::TextDelta {
                text: self.0.into(),
            })]))
        }
    }

    async fn stream_text(handle: &ClientHandle) -> String {
        let request = LlmRequest {
            system: "s",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let mut text = String::new();
        let mut stream = handle.stream(request);
        while let Some(Ok(LlmEvent::TextDelta { text: delta })) = stream.next().await {
            text.push_str(&delta);
        }
        text
    }

    #[tokio::test]
    async fn the_handle_streams_through_the_client_currently_in_it() {
        let handle = ClientHandle::new(Arc::new(NamedClient("first")));
        assert_eq!(stream_text(&handle).await, "first");
        handle.store(Arc::new(NamedClient("second")));
        assert_eq!(stream_text(&handle).await, "second");
    }

    #[tokio::test]
    async fn a_swap_does_not_reach_a_request_already_in_flight() {
        let handle = ClientHandle::new(Arc::new(NamedClient("first")));
        let request = LlmRequest {
            system: "s",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let mut in_flight = handle.stream(request);
        handle.store(Arc::new(NamedClient("second")));
        let mut text = String::new();
        while let Some(Ok(LlmEvent::TextDelta { text: delta })) = in_flight.next().await {
            text.push_str(&delta);
        }
        assert_eq!(text, "first");
    }

    /// ADR 0009 §6: a session with nothing configured starts, and says the
    /// one true thing when asked to answer.
    #[tokio::test]
    async fn an_unconfigured_session_answers_with_how_to_configure_it() {
        let handle = ClientHandle::new(Arc::new(Unconfigured));
        let request = LlmRequest {
            system: "s",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let mut stream = handle.stream(request);
        match stream.next().await {
            Some(Err(LlmError::Terminal { message, .. })) => {
                assert!(message.contains("/model"), "{message}")
            }
            other => panic!("expected the unconfigured error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_swap_that_cannot_build_a_client_leaves_the_running_one_in_place() {
        const KEY: &str = "ALDWIN_SWAP_TEST_KEY";
        const ABSENT: &str = "ALDWIN_SWAP_TEST_KEY_NEVER_SET";
        std::env::set_var(KEY, "not-a-real-key");
        let config = |key: &str| aldwin_config::ProviderConfig {
            version: aldwin_config::PROVIDER_VERSION,
            provider: ProviderKind::Anthropic,
            model: "a-model".into(),
            api_key_env: key.to_string(),
            base_url: None,
            extended_thinking_budget: Some(1_000),
        };
        let handle = ClientHandle::new(Arc::new(NamedClient("the session's own")));
        let error = slash::ModelSwitch::switch(&handle, &config(ABSENT))
            .expect_err("no key is exported for this one");
        assert!(error.to_string().contains(ABSENT), "{error}");
        assert_eq!(stream_text(&handle).await, "the session's own");
        slash::ModelSwitch::switch(&handle, &config(KEY))
            .expect("a client that builds replaces the one in place");
    }

    /// Every catalogue row reaches the TUI with its models and their context
    /// sizes — the question would otherwise open on an empty second list.
    #[test]
    fn every_catalogue_row_carries_its_models_to_the_frontend() {
        let choices = catalogue_choices();
        assert_eq!(choices.len(), aldwin_llm::PROVIDERS.len());
        for (choice, provider) in choices.iter().zip(aldwin_llm::PROVIDERS) {
            assert_eq!(choice.id, provider.id);
            assert_eq!(
                choice
                    .models
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                provider.models.iter().map(|m| m.id).collect::<Vec<_>>()
            );
            assert!(choice.models.iter().all(|m| m.context > 0));
        }
    }

    /// A panicking agent task used to be awaited with `let _ =`, and the
    /// process exited 0 over a session that had stopped answering.
    #[tokio::test]
    async fn a_task_that_panicked_is_an_error_not_a_clean_exit() {
        let panicked = tokio::spawn(async { panic!("the agent fell over") }).await;
        let error = finished("agent", panicked).expect_err("a panic is not a clean exit");
        assert!(
            matches!(error, StartupError::TaskFailed { task: "agent", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("the agent fell over"), "{error}");

        let clean = tokio::spawn(async {}).await;
        assert!(finished("agent", clean).is_ok());
    }

    #[test]
    fn context_files_are_the_two_names_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        assert!(context_files(dir.path()).is_empty());
        std::fs::write(dir.path().join("AGENTS.md"), "notes").unwrap();
        assert_eq!(
            context_files(dir.path()),
            vec![dir.path().join("AGENTS.md")]
        );
    }

    #[test]
    fn a_stale_permissions_file_is_reported_once_and_a_clean_one_is_not() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        assert_eq!(stale_keys_notice(&config), None);

        std::fs::create_dir(project.path().join(".aldwin")).unwrap();
        std::fs::write(
            project.path().join(".aldwin/permissions.yaml"),
            "version: 2\ndeny:\n  - curl\n",
        )
        .unwrap();
        config.reload_all().unwrap();
        let notice = stale_keys_notice(&config).expect("a notice");
        assert!(
            notice.contains("permissions.yaml") && notice.contains("deny:"),
            "{notice}"
        );
    }

    #[test]
    fn an_unconfined_system_is_said_once_and_a_confined_one_is_not() {
        assert_eq!(unconfined_notice(None), None);
        let notice = unconfined_notice(Some("this kernel has no Landlock support")).unwrap();
        assert!(notice.contains("outside the workspace"), "{notice}");
        assert!(notice.contains("Landlock"), "{notice}");
    }

    #[test]
    fn roots_are_applied_reported_and_re_read_on_reload() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let sibling = tempfile::tempdir().unwrap();
        let aldwin = project.path().join(".aldwin");
        std::fs::create_dir_all(&aldwin).unwrap();
        let file = aldwin.join("permissions.yaml");
        std::fs::write(&file, "version: 2\n").unwrap();

        let config = Config::open_at(project.path(), global.path()).unwrap();
        let workspace = Workspace::new(project.path());
        assert_eq!(apply_roots(&config, project.path(), &workspace), None);

        std::fs::write(
            &file,
            format!(
                "version: 2\nroots:\n- {}\n- ../no-such-dir\n",
                sibling.path().display()
            ),
        )
        .unwrap();
        config.reload_all().unwrap();
        let notice = apply_roots(&config, project.path(), &workspace).expect("a notice");
        assert_eq!(workspace.roots().len(), 2);
        assert!(
            notice.contains(&sibling.path().canonicalize().unwrap().display().to_string()),
            "{notice}"
        );
        assert!(notice.contains("no-such-dir"), "{notice}");
    }
}
