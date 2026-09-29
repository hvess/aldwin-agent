use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use aldwin_config::{Config, InitOutcome, McpServer, ProviderKind};
use aldwin_core::{Agent, Event, LlmClient, LlmError, LlmEvent, LlmRequest};
use aldwin_llm::LlmClientInitError;
use aldwin_tools::{register_mcp_tools, Dispatcher, McpBridge, Staging, Workspace};
use futures::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinError;

use crate::connect::{self, Reach};
use crate::context;
use crate::error::{ShimError, StartupError};
use crate::history::History;
use crate::slash;

/// The client `provider` selects. The only place `provider.yaml` settings
/// become aldwin-llm's; used at startup and by every `/model`, so both reach
/// a provider the same way (ADR 0012, `connect::reach`). `notices` receives
/// a rotated token that could not be saved.
fn build_client(
    provider: &aldwin_config::ProviderConfig,
    config: &Config,
    notices: &mpsc::Sender<Event>,
) -> Result<Arc<dyn LlmClient>, LlmClientInitError> {
    let account = slash::identify(provider).and_then(|row| row.account);
    let auth = match connect::reach(&provider.api_key_env, account, config, notices)? {
        Reach::Through(auth) => auth,
        Reach::Neither(sentence) => return Ok(Arc::new(Said(sentence))),
    };
    let config = aldwin_llm::ProviderConfig {
        kind: provider.provider,
        model: provider.model.clone(),
        auth,
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

/// The client when no model can be reached yet: none configured (ADR 0009
/// §6, no first run) or neither account nor key (ADR 0012). Every request
/// fails with the sentence it holds.
struct Said(String);

impl LlmClient for Said {
    fn stream<'a>(
        &'a self,
        _: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        Box::pin(futures::stream::iter([Err(LlmError::NotSent(
            self.0.clone(),
        ))]))
    }
}

const NO_MODEL: &str = "No model is configured yet. Pick one with /model.";

/// The session's client, swappable in place.
///
/// `Agent<C, D>` owns its client for the process's life, so `/model`
/// replaces the client inside this handle instead (`slash::ModelSwitch`);
/// core never learns of providers. `config` and `notices` serve a rebuild's
/// connected account.
#[derive(Clone)]
struct ClientHandle {
    client: Arc<std::sync::RwLock<Arc<dyn LlmClient>>>,
    config: Config,
    notices: mpsc::Sender<Event>,
}

impl ClientHandle {
    fn new(client: Arc<dyn LlmClient>, config: Config, notices: mpsc::Sender<Event>) -> Self {
        Self {
            client: Arc::new(std::sync::RwLock::new(client)),
            config,
            notices,
        }
    }

    fn store(&self, client: Arc<dyn LlmClient>) {
        *self.client.write().expect("client lock poisoned") = client;
    }
}

impl LlmClient for ClientHandle {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        // Resolved once per request: a swap must not reach a request in
        // flight.
        let client = self.client.read().expect("client lock poisoned").clone();
        Box::pin(async_stream::stream! {
            let mut inner = client.stream(request);
            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

impl slash::ModelSwitch for ClientHandle {
    /// Builds before storing, so a failed build (e.g. `api_key_env` not
    /// exported) leaves the current client in place.
    fn switch(&self, config: &aldwin_config::ProviderConfig) -> Result<(), LlmClientInitError> {
        self.store(build_client(config, &self.config, &self.notices)?);
        Ok(())
    }
}

const CHANNEL_CAPACITY: usize = 64;

/// The catalogue's display fields, in catalogue order, for `/model`, the
/// first message's questions and `/connect`. aldwin-tui gets display fields
/// only; it depends on neither this crate nor aldwin-llm.
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
            account: p.account.map(|a| a.subscription().to_string()),
        })
        .collect()
}

/// The launch card's `Branch`: the nearest checkout's `HEAD`, read without
/// running git. A detached head is its first 8 hex digits. Never falls back
/// to an outer checkout: its branch is not this one's.
fn git_branch(dir: &Path) -> Option<String> {
    let git = dir
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|git| git.exists())?;
    // In a worktree or submodule `.git` is a file naming the real one.
    let git = match std::fs::read_to_string(&git) {
        Ok(link) => git
            .parent()?
            .join(link.trim().strip_prefix("gitdir:")?.trim()),
        Err(_) => git,
    };
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    Some(
        head.strip_prefix("ref: refs/heads/")
            .map_or_else(|| head.chars().take(8).collect(), str::to_string),
    )
}

/// `CLAUDE.md` and `AGENTS.md` at the project root, whichever exist. Read
/// without asking (ADR 0009 §6).
fn context_files(cwd: &Path) -> Vec<PathBuf> {
    ["CLAUDE.md", "AGENTS.md"]
        .iter()
        .map(|f| cwd.join(f))
        .filter(|p| p.is_file())
        .collect()
}

/// Runs a session to completion (the startup sequence of aldwin-cli.md):
/// config, client, workspace and dispatcher, agent, TUI, then waits for the
/// tasks. No first-run screen (ADR 0009 §6).
///
/// `git_shim` is why the git shim is not installed, if it is not (ADR
/// 0013); the session starts anyway and says so once.
///
/// # Errors
///
/// [`StartupError`] when the working directory cannot be read, a config
/// layer fails to load, `~/.aldwin` is only partly present, the configured
/// client cannot be built, the terminal fails, or the agent or interceptor
/// task panicked.
pub async fn run(git_shim: Option<&ShimError>) -> Result<(), StartupError> {
    let cwd = std::env::current_dir().map_err(StartupError::Cwd)?;

    let config = Config::open(&cwd)?;
    match config.init_global_if_empty()? {
        InitOutcome::Created | InitOutcome::AlreadyPresent => {}
        InitOutcome::PartiallyPresent { missing } => {
            return Err(StartupError::PartiallyPresentGlobalConfig { missing })
        }
    }

    // `theme` is global-only, resolved once before anything draws.
    let theme = aldwin_tui::Theme::from_config(config.global_tui().theme.as_deref());

    // Commands: TUI -> interceptor -> core, so slash commands never reach
    // Submit. Events: core -> TUI directly.
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    // No provider configured is not an error: the session starts on `Said`
    // and `/model` moves it onto a real client.
    let effective_provider = config.effective_provider();
    let handle = |client| ClientHandle::new(client, config.clone(), event_tx.clone());
    let (client, model_name, session_model) = match &effective_provider {
        Some(effective) => (
            handle(build_client(effective, &config, &event_tx)?),
            effective.model.clone(),
            slash::qualified(effective, slash::identify(effective)),
        ),
        None => (
            handle(Arc::new(Said(NO_MODEL.into()))),
            String::new(),
            String::new(),
        ),
    };

    // The workspace, the only boundary (ADR 0007, ADR 0011): the project
    // root plus the project-scope `.aldwin/permissions.yaml` roots.
    let workspace = Workspace::new(cwd.clone());
    let reach_notice = apply_roots(&config, &cwd, &workspace);
    let roots = workspace.roots();
    let additional_context = context::build(
        &cwd,
        &roots,
        &context_files(&cwd),
        &context::skills(&cwd, &roots),
    );

    // Every edit of a turn waits here for the review (ADR 0009 §4).
    let staging = Arc::new(Staging::new(workspace.clone()));
    let mut registry = aldwin_tools::builtin_registry(workspace.clone(), staging.clone());
    let mcp_bridge = Arc::new(McpBridge::new(merged_mcp_servers(&config)));
    // Best effort per server and tool: a broken server must not stop the
    // session or other servers. Each failure is said.
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

    // `None` when the history directory cannot be written; said once.
    // History must never stop a session starting.
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
        slash::Session::new(session_model, Arc::new(client))
            .with_after_reload(Box::new(move || apply_roots(&config, &cwd, &workspace)))
    };

    // Said once at session start (ADR 0011 §3, ADR 0013).
    let notices = [
        reach_notice,
        stale_keys_notice(&config),
        unconfined_notice(aldwin_tools::sandbox::unavailable()),
        git_shim.map(unshimmed_notice),
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
        project: cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        branch: git_branch(&cwd),
        commands: slash::menu(),
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

/// A panicked session task as an error, so the process does not exit 0.
/// Called after the TUI has restored the terminal.
fn finished(task: &'static str, joined: Result<(), JoinError>) -> Result<(), StartupError> {
    joined.map_err(|source| StartupError::TaskFailed { task, source })
}

/// A project-scope server entry replaces a global one of the same name
/// entirely (aldwin-config's annotated mcp.yaml).
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

/// `allow:`, `default:` and `deny:` in a `permissions.yaml` are ignored
/// (ADR 0011); a file still using one is reported once.
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

/// Where the sandbox is unavailable, everything Aldwin starts runs
/// unconfined; this must be said once, never silently (ADR 0011 §3).
fn unconfined_notice(reason: Option<&str>) -> Option<String> {
    reason
        .map(|reason| format!("Commands can write outside the workspace on this system: {reason}."))
}

/// Without the git shim, commits lack the co-author trailer (ADR 0013);
/// said once.
fn unshimmed_notice(reason: &ShimError) -> String {
    format!("Commits made in this session will not name Aldwin as a co-author: {reason}.")
}

/// Sets `workspace`'s extra roots from the project's `permissions.yaml`
/// (relative to `cwd`) and returns the notice: roots beyond the project and
/// declared roots that do not exist, or `None` if neither. Called at startup
/// and after `/reload-config`.
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

    /// A handle over `client`, with a fresh config and an unread channel.
    fn handle(client: Arc<dyn LlmClient>) -> (ClientHandle, tempfile::TempDir, tempfile::TempDir) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        let (tx, _rx) = mpsc::channel(8);
        (ClientHandle::new(client, config, tx), project, global)
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
        let (handle, _project, _global) = handle(Arc::new(NamedClient("first")));
        assert_eq!(stream_text(&handle).await, "first");
        handle.store(Arc::new(NamedClient("second")));
        assert_eq!(stream_text(&handle).await, "second");
    }

    #[tokio::test]
    async fn a_swap_does_not_reach_a_request_already_in_flight() {
        let (handle, _project, _global) = handle(Arc::new(NamedClient("first")));
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

    /// ADR 0009 §6.
    #[tokio::test]
    async fn an_unconfigured_session_answers_with_how_to_configure_it() {
        let (handle, _project, _global) = handle(Arc::new(Said(NO_MODEL.into())));
        let request = LlmRequest {
            system: "s",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let mut stream = handle.stream(request);
        match stream.next().await {
            Some(Err(LlmError::NotSent(message))) => {
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
        let (handle, _project, _global) = handle(Arc::new(NamedClient("the session's own")));
        let error = slash::ModelSwitch::switch(&handle, &config(ABSENT))
            .expect_err("no key is exported for this one");
        assert!(error.to_string().contains(ABSENT), "{error}");
        assert_eq!(stream_text(&handle).await, "the session's own");
        slash::ModelSwitch::switch(&handle, &config(KEY))
            .expect("a client that builds replaces the one in place");
    }

    #[tokio::test]
    async fn a_swap_onto_a_provider_with_neither_account_nor_key_lands_on_the_sentence() {
        std::env::remove_var("XAI_API_KEY");
        let xai = aldwin_config::ProviderConfig {
            version: aldwin_config::PROVIDER_VERSION,
            provider: ProviderKind::OpenaiCompatible,
            model: "grok-4.7".into(),
            api_key_env: "XAI_API_KEY".into(),
            base_url: Some("https://api.x.ai/v1/chat/completions".into()),
            extended_thinking_budget: None,
        };
        let (handle, _project, _global) = handle(Arc::new(NamedClient("the session's own")));

        slash::ModelSwitch::switch(&handle, &xai).expect("the swap lands");
        let request = LlmRequest {
            system: "s",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let mut stream = handle.stream(request);
        match stream.next().await {
            Some(Err(LlmError::NotSent(message))) => {
                assert!(
                    message.starts_with("No x.ai account is connected"),
                    "{message}"
                );
                assert!(message.contains("/connect xai"), "{message}");
            }
            other => panic!("expected the sentence, got {other:?}"),
        }
        // The full account-then-key order is tested in `connect`.
    }

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

    /// Regression: a panicked agent task exited 0.
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
    fn a_missing_git_shim_is_said_with_its_reason() {
        let notice = unshimmed_notice(&ShimError::Install(std::io::Error::other("no space left")));
        assert!(notice.contains("co-author"), "{notice}");
        assert!(notice.contains("no space left"), "{notice}");
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

    #[test]
    fn the_branch_is_read_from_the_nearest_checkout() {
        let repo = tempfile::tempdir().unwrap();
        let git = repo.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        let nested = repo.path().join("crates/cli");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/audit-fixes\n").unwrap();
        assert_eq!(git_branch(&nested).as_deref(), Some("audit-fixes"));
        std::fs::write(git.join("HEAD"), "c82a5db0123456789\n").unwrap();
        assert_eq!(
            git_branch(repo.path()).as_deref(),
            Some("c82a5db0"),
            "detached, the short commit"
        );
    }

    /// Regression: inside a worktree nested in its main checkout, the main
    /// checkout's branch was shown.
    #[test]
    fn a_worktree_shows_its_own_branch() {
        let repo = tempfile::tempdir().unwrap();
        let git = repo.path().join(".git");
        let linked = git.join("worktrees/feature");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(linked.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        let tree = repo.path().join("worktrees/feature");
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", linked.display())).unwrap();
        assert_eq!(git_branch(&tree.join("src")).as_deref(), Some("feature"));

        std::fs::write(tree.join(".git"), "gitdir: ../../.git/worktrees/feature\n").unwrap();
        assert_eq!(
            git_branch(&tree).as_deref(),
            Some("feature"),
            "a relative gitdir is read from the worktree"
        );

        std::fs::write(tree.join(".git"), "not a link\n").unwrap();
        assert_eq!(git_branch(&tree), None, "never the main checkout's branch");
    }
}
