use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use aldwin_config::{Config, InitOutcome, McpServer, ProviderKind, Scope};
use aldwin_core::{Agent, LlmClient, LlmError, LlmEvent, LlmRequest};
use aldwin_permissions::Engine;
use aldwin_tools::{register_mcp_tools, Dispatcher, McpBridge};
use futures::{Stream, StreamExt};
use tokio::sync::mpsc;

use crate::context;
use crate::context_approval;
use crate::error::StartupError;
use crate::history::History;
use crate::slash;

/// Whichever client `provider_config` selects, built the same way at
/// startup and on every `/model` after it — so a model swapped into a
/// running session is reached exactly as one chosen at launch would be.
fn build_client(config: aldwin_llm::ProviderConfig) -> Result<Arc<dyn LlmClient>, aldwin_llm::LlmClientInitError> {
    Ok(match config.kind {
        ProviderKind::Anthropic => Arc::new(aldwin_llm::AnthropicClient::new(config)?),
        ProviderKind::OpenaiCompatible => Arc::new(aldwin_llm::OpenAiCompatibleClient::new(config)?),
    })
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
    fn stream<'a>(&'a self, request: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        // Resolved once, when the request starts, and held by the stream
        // for as long as it runs: a swap landing mid-turn cannot pull the
        // client out from under a request already in flight. That turn
        // finishes on the client it began on and the next one picks up the
        // new one.
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
    fn switch(&self, config: &aldwin_llm::ProviderConfig) -> Result<(), String> {
        self.store(build_client(config.clone()).map_err(|e| e.to_string())?);
        Ok(())
    }
}

const CHANNEL_CAPACITY: usize = 64;

/// The display halves of the whole catalogue, in catalogue order — what
/// first run's provider step and the session's `/model` picker both list.
/// aldwin-tui is handed ids and purposes and nothing else: it renders the
/// list, it does not know what an endpoint or a key variable is, and it
/// does not depend on this crate or on aldwin-llm to find out.
fn catalogue_choices() -> Vec<aldwin_tui::ProviderChoice> {
    aldwin_llm::PROVIDERS
        .iter()
        .map(|p| {
            aldwin_tui::ProviderChoice::new(
                p.id,
                p.purpose,
                p.models.iter().map(|m| aldwin_tui::ModelChoice::new(m.id, m.purpose)).collect(),
            )
        })
        .collect()
}

/// Whether the first-run screen carries the provider and model steps at
/// all, given that it is opening for one reason or the other.
///
/// Yes when there is nothing configured — the question has to be answered
/// before a client can be built — and yes when what is configured is a
/// catalogue row the lists can open on. No in the one remaining case: a
/// `provider.yaml` pointed at an endpoint the catalogue cannot name, where
/// every row on the screen is somewhere the developer is not, and pressing
/// through would move them off their own endpoint.
fn asks_for_a_provider(needs_provider: bool, configured: &aldwin_tui::Configured) -> bool {
    needs_provider || configured.provider.is_some()
}

/// Where the first-run screen's provider and model lists open: whatever
/// already supplies the setting, project file over global, reduced to the
/// two ids the screen can match against its own rows.
fn configured_for_first_run(config: &Config) -> aldwin_tui::Configured {
    let Some(current) = config.project_provider().or_else(|| config.global_provider().ok()) else {
        return aldwin_tui::Configured::default();
    };
    aldwin_tui::Configured {
        // `None` for an endpoint the catalogue does not know — there is no
        // row for it, so the lists open at the top rather than on a
        // neighbour that merely looks similar.
        provider: aldwin_llm::identify(&current).map(|p| p.id.to_string()),
        model:    Some(current.model.clone()),
    }
}

/// First run, per ADR 0001. What opens the screen is either question being
/// unanswered:
///
/// * no provider config resolves anywhere — where the model runs is unknown,
///   and it has to be answered before the LLM client can be constructed;
/// * this project has no `.aldwin/permissions.yaml` — a directory the agent
///   has never been pointed at, whose access posture is therefore undeclared.
///
/// The file's *existence* is the test, not whether it parses to an empty
/// allow list: a developer who has deliberately allowed nothing has answered
/// the question, and must not be asked again on every start.
///
/// Once the screen is open the provider and model are on it even when a
/// global `provider.yaml` already answers them (see `asks_for_a_provider`
/// for the one exception): entering a new directory is the moment a
/// developer decides what this project runs on. The lists open on what is
/// already configured, so confirming changes nothing.
///
/// Returns `false` when the developer quit without answering.
async fn first_run(config: &Config, cwd: &Path, theme: aldwin_tui::Theme) -> Result<bool, StartupError> {
    let needs_provider = config.global_provider().is_err();
    let needs_access = !cwd.join(".aldwin").join("permissions.yaml").exists();
    if !needs_provider && !needs_access {
        return Ok(true);
    }

    let configured = configured_for_first_run(config);
    let ask_provider = asks_for_a_provider(needs_provider, &configured);
    let Some(answers) = aldwin_tui::run_first_run(theme, catalogue_choices(), aldwin_llm::CURATED, ask_provider, needs_access, configured)
        .await
        .map_err(StartupError::FirstRun)?
    else {
        return Ok(false);
    };
    if let Some(id) = answers.provider.as_deref() {
        let picked = aldwin_llm::provider(id).ok_or_else(|| StartupError::UnknownProvider { id: id.to_string() })?;
        let current = config.project_provider().or_else(|| config.global_provider().ok());
        let next = slash::catalogue_provider_config(picked, answers.model.as_deref(), current.as_ref());
        match current {
            // Nothing to write: the developer confirmed the lists on the
            // rows they opened on. Writing anyway would create a project
            // file that only restates the global one, and then goes stale
            // the first time the global one changes.
            Some(current) if current == next => {}
            // A provider is already configured, so this answer is about
            // *this directory*. The developer's global default is left alone.
            Some(_) => config.set_provider(Scope::Project, next).map_err(StartupError::FirstRunWrite)?,
            // A true first run has no global default yet, so the answer
            // becomes one: a project-scope file would leave every other
            // directory unconfigured and ask again in each.
            None => config.set_provider(Scope::Global, next).map_err(StartupError::FirstRunWrite)?,
        }
    }
    // Answered only when it was asked. Writing an unasked answer into a
    // directory that already has a `permissions.yaml` would overwrite a
    // standing rung the developer had already settled — and could only
    // widen it, the one direction a default-deny agent must never move on
    // its own.
    //
    // Written even when the tier grants nothing: the file's existence is
    // what records that this directory's question has been answered.
    if let Some(rung) = answers.access {
        config.ensure_permissions(Scope::Project).map_err(StartupError::FirstRunWrite)?;
        config.set_default_rung(Scope::Project, rung).map_err(StartupError::FirstRunWrite)?;
    }
    Ok(true)
}

/// The startup sequence from aldwin-cli.md, in order:
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
/// after: per aldwin-permissions.md, "the session initializer tests each
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

    // `theme` is global-only (see aldwin-config's annotated tui.yaml) —
    // resolved once, before anything draws, and never revisited for the rest
    // of the session (aldwin_tui::palette's own doc comment explains why
    // this is a one-time explicit choice, not a live setting). Read here
    // rather than just before the session TUI because first run draws first.
    let theme = aldwin_tui::Theme::from_config(config.global_tui().theme.as_deref());

    // `false` means the developer quit the first-run screen without
    // answering. Nothing was written and no session opens — a first run that
    // was dismissed must not fall back to defaults, least of all for the
    // access question.
    if !first_run(&config, &cwd, theme).await? {
        return Ok(());
    }

    let permissions = Arc::new(Engine::new(config.clone()));

    let approved_context_files = context_approval::resolve(&cwd, &permissions);

    let project_provider = config.project_provider();
    let global_provider = config.global_provider().map_err(StartupError::NoProvider)?;
    let provider_config = aldwin_llm::resolve(project_provider.as_ref(), &global_provider);
    let model_name = provider_config.model.clone();
    // What this process boots on. `/model` moves the session off it and
    // updates `slash::Session` in step, so this is a starting point rather
    // than a fact about the whole run.
    let effective_provider = project_provider.clone().unwrap_or_else(|| global_provider.clone());
    let session_model = slash::qualified(&effective_provider, aldwin_llm::identify(&effective_provider));
    let client = ClientHandle::new(build_client(provider_config)?);

    // Reach: the project root, plus whatever `.aldwin/permissions.yaml`
    // declares (ADR 0007). Project scope only, and stated rather than
    // inferred — nothing here goes looking for sibling checkouts.
    let workspace = aldwin_tools::Workspace::new(cwd.clone());
    let reach_notice = apply_roots(&config, &cwd, &workspace);
    let additional_context = context::build(&cwd, &workspace.roots(), &approved_context_files);
    let mut registry = aldwin_tools::builtin_registry(workspace.clone());
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

    // TUI -> interceptor -> core, so slash commands never reach Submit;
    // core -> TUI directly for events (no interception needed there).
    //
    // Built before the agent rather than after it because the transcript
    // reports a failed write as an `Event::Notice`, so it needs the event
    // sender in hand at the moment it is opened.
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    // This session's transcript. `None` when the history directory cannot be
    // written — the session then runs without one, having said so once.
    // History must never be able to stop a session starting.
    let history = match History::open(config.history_dir(), model_name.clone(), event_tx.clone()) {
        Ok(history) => Some(history),
        Err(message) => {
            let _ = event_tx.try_send(aldwin_core::Event::Notice { message });
            None
        }
    };
    // Every transcript but this session's own — `resumable` is what excludes
    // it, so the picker never offers the session the developer is sitting in.
    let sessions = history.as_ref().map(|h| h.resumable()).unwrap_or_default();

    // The agent takes the handle; the interceptor keeps a clone of the same
    // one, which is what makes `/model` a live swap rather than a note for
    // the next start. The transcript is held the same way and for the same
    // reason: `/clear` seals it and `/resume` moves it onto another file,
    // both while the agent that owns it keeps running.
    let agent = Agent::new(client.clone(), dispatcher, model_name.clone(), Some(additional_context.as_str()));
    let agent = match history.clone() {
        Some(history) => agent.with_sink(history),
        None => agent,
    };
    let session_state = {
        let (config, cwd, workspace) = (config.clone(), cwd.clone(), workspace.clone());
        slash::Session::new(session_model, Box::new(client))
            .with_after_reload(Box::new(move || apply_roots(&config, &cwd, &workspace)))
    };
    // Reach wider than the project is said out loud, once, at the top of the
    // session. `.aldwin/permissions.yaml` can arrive with a clone, and a
    // `roots:` in it widens where every tool may point; the developer should
    // not have to open the file to find that out.
    if let Some(message) = reach_notice {
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

    // The picker opens on the row the session is actually running on, which
    // is `effective_provider` — the file that supplies the setting, not the
    // global one it may be shadowing.
    //
    // The label is not `current_provider` with a default: the picker must
    // open on a row that exists, so an unrecognised endpoint leaves that
    // `None`, while the resting screen still has a true name to print — the
    // kind the file itself declares.
    let identified = aldwin_llm::identify(&effective_provider).map(|p| p.id.to_string());
    let kind = match effective_provider.provider {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenaiCompatible => "openai-compatible",
    };
    let session = aldwin_tui::SessionProvider {
        provider_label:   Some(identified.clone().unwrap_or_else(|| kind.to_string())),
        catalogue:        catalogue_choices(),
        current_provider: identified,
        sessions,
    };
    let tui_result = aldwin_tui::run(event_rx, tui_cmd_tx, model_name, permissions, theme, session).await;

    // The TUI dropped its command sender on return, closing tui_cmd_rx;
    // the interceptor then drops agent_cmd_tx, closing the core's command
    // channel — both drain to completion on their own from here.
    let _ = interceptor.await;
    let _ = agent_task.await;

    tui_result.map_err(StartupError::Io)
}

/// A project-scope server entry replaces a global one of the same name
/// entirely (see aldwin-config's annotated mcp.yaml) — this is that same
/// rule applied across the two already-loaded snapshots.
fn merged_mcp_servers(config: &Config) -> Vec<McpServer> {
    let mut by_name: BTreeMap<String, McpServer> = config.global_mcp().servers.into_iter().map(|s| (s.name.clone(), s)).collect();
    for server in config.project_mcp().servers {
        by_name.insert(server.name.clone(), server);
    }
    by_name.into_values().collect()
}

/// Points `workspace` at the roots the project's `permissions.yaml` declares
/// now, and returns what the developer should be told about it: the roots in
/// force beyond the project, and any that were written down but do not exist
/// — a typo there silently narrows reach, which reads as the agent refusing
/// for no reason. `None` when there is nothing beyond the project root and
/// nothing was dropped.
///
/// Called at startup and again after `/reload-config`. A relative root
/// resolves against the project root, so `roots: [../proton-libs]` means
/// what it looks like.
fn apply_roots(config: &Config, cwd: &Path, workspace: &aldwin_tools::Workspace) -> Option<String> {
    let declared: Vec<PathBuf> =
        config.project_permissions().roots.iter().map(|r| if r.is_absolute() { r.clone() } else { cwd.join(r) }).collect();
    let dropped = workspace.set_extra_roots(declared);
    let extra: Vec<String> = workspace.roots().iter().skip(1).map(|r| r.display().to_string()).collect();

    let mut parts = Vec::new();
    if !extra.is_empty() {
        parts.push(format!("tools can also reach {} (roots in .aldwin/permissions.yaml)", extra.join(", ")));
    }
    if !dropped.is_empty() {
        let names: Vec<String> = dropped.iter().map(|r| r.display().to_string()).collect();
        parts.push(format!("ignored roots that do not exist: {}", names.join(", ")));
    }
    if parts.is_empty() { None } else { Some(parts.join(" · ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_config::McpTransport;

    fn server(name: &str, command: &str) -> McpServer {
        McpServer { name: name.into(), transport: McpTransport::Stdio { command: command.into(), args: vec![] }, env: Default::default() }
    }

    #[test]
    fn project_scope_server_replaces_a_global_one_of_the_same_name() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();

        config.add_mcp_server(aldwin_config::Scope::Global, server("fs", "global-fs-server")).unwrap();
        config.add_mcp_server(aldwin_config::Scope::Project, server("fs", "project-fs-server")).unwrap();
        config.add_mcp_server(aldwin_config::Scope::Global, server("other", "other-server")).unwrap();

        let merged = merged_mcp_servers(&config);
        assert_eq!(merged.len(), 2);
        let fs = merged.iter().find(|s| s.name == "fs").unwrap();
        assert!(matches!(&fs.transport, McpTransport::Stdio { command, .. } if command == "project-fs-server"));
    }

    /// The two steps are on every screen that opens — except when what is
    /// configured is an endpoint the catalogue cannot name, where no row
    /// represents where the developer already is.
    #[test]
    fn the_provider_steps_are_held_back_only_for_an_endpoint_with_no_row() {
        let known = aldwin_tui::Configured { provider: Some("anthropic".into()), model: Some("claude-opus-5".into()) };
        let unknown = aldwin_tui::Configured { provider: None, model: Some("qwen3-coder".into()) };

        assert!(asks_for_a_provider(true, &aldwin_tui::Configured::default()), "a true first run has to ask");
        assert!(asks_for_a_provider(false, &known), "a catalogue row is a row the lists can open on");
        assert!(!asks_for_a_provider(false, &unknown), "pressing through rows that are all somewhere else is not an answer");
        assert!(asks_for_a_provider(true, &unknown), "nothing configured still has to be asked, whatever else is on disk");
    }

    /// The lists open on whatever supplies the setting — project file over
    /// global, the same precedence the session itself runs on.
    #[test]
    fn the_first_run_lists_open_on_the_configured_provider() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        assert_eq!(configured_for_first_run(&config), aldwin_tui::Configured::default(), "a true first run has nothing to open on");

        let anthropic = aldwin_llm::provider("anthropic").unwrap();
        config.set_provider(Scope::Global, slash::catalogue_provider_config(anthropic, Some("claude-opus-5"), None)).unwrap();
        let configured = configured_for_first_run(&config);
        assert_eq!(configured.provider.as_deref(), Some("anthropic"));
        assert_eq!(configured.model.as_deref(), Some("claude-opus-5"));

        let google = aldwin_llm::provider("google").unwrap();
        config.set_provider(Scope::Project, slash::catalogue_provider_config(google, Some("gemini-2.5-flash"), None)).unwrap();
        let shadowed = configured_for_first_run(&config);
        assert_eq!(shadowed.provider.as_deref(), Some("google"), "the project file is what the session would run on");
        assert_eq!(shadowed.model.as_deref(), Some("gemini-2.5-flash"));
    }

    /// A client that answers with its own name, so a test can tell which
    /// one a stream actually ran through — the whole point of the handle is
    /// that the answer changes, and nothing else here can observe it.
    struct NamedClient(&'static str);

    impl LlmClient for NamedClient {
        fn stream<'a>(&'a self, _: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            Box::pin(futures::stream::iter([Ok(LlmEvent::TextDelta { text: self.0.into() })]))
        }
    }

    /// Drains one request through `handle`, returning the text it yielded.
    async fn stream_text(handle: &ClientHandle) -> String {
        let request = LlmRequest { model: "m", system: "s", tools: &[], messages: &[], cache_breakpoints: &[] };
        let mut text = String::new();
        let mut stream = handle.stream(request);
        while let Some(Ok(LlmEvent::TextDelta { text: delta })) = stream.next().await {
            text.push_str(&delta);
        }
        text
    }

    /// The indirection has to be transparent: what core streams through the
    /// handle is whatever client is in it, events and all. Nothing else in
    /// this crate proves that — the agent reaches its client through this
    /// `stream` on every single turn.
    #[tokio::test]
    async fn the_handle_streams_through_the_client_currently_in_it() {
        let handle = ClientHandle::new(Arc::new(NamedClient("first")));
        assert_eq!(stream_text(&handle).await, "first");

        handle.store(Arc::new(NamedClient("second")));
        assert_eq!(stream_text(&handle).await, "second", "the next request runs on the client that replaced it");
    }

    /// The guarantee that makes a live swap safe: a request already in
    /// flight finishes on the client it started on. The stream is built
    /// before the swap and drained after it.
    #[tokio::test]
    async fn a_swap_does_not_reach_a_request_already_in_flight() {
        let handle = ClientHandle::new(Arc::new(NamedClient("first")));
        let request = LlmRequest { model: "m", system: "s", tools: &[], messages: &[], cache_breakpoints: &[] };
        let mut in_flight = handle.stream(request);

        handle.store(Arc::new(NamedClient("second")));

        let mut text = String::new();
        while let Some(Ok(LlmEvent::TextDelta { text: delta })) = in_flight.next().await {
            text.push_str(&delta);
        }
        assert_eq!(text, "first", "a turn must not change model half way through");
    }

    /// A swap is the whole of `/model`'s live half, and its failure mode is
    /// the one startup has: a key variable that is not exported. The
    /// session has to be left on the client it already had when that
    /// happens, not on nothing.
    #[tokio::test]
    async fn a_swap_that_cannot_build_a_client_leaves_the_running_one_in_place() {
        // A name of this test's own, so a parallel test's environment can
        // neither satisfy nor break it.
        const KEY: &str = "ALDWIN_SWAP_TEST_KEY";
        const ABSENT: &str = "ALDWIN_SWAP_TEST_KEY_NEVER_SET";
        std::env::set_var(KEY, "not-a-real-key");

        let config = |key: &str| aldwin_llm::ProviderConfig {
            kind:                     ProviderKind::Anthropic,
            model:                    "a-model".into(),
            api_key_env:              key.to_string(),
            base_url:                 None,
            extended_thinking_budget: 1_000,
        };

        let handle = ClientHandle::new(Arc::new(NamedClient("the session's own")));
        let error = slash::ModelSwitch::switch(&handle, &config(ABSENT)).expect_err("no key is exported for this one");
        assert!(error.contains(ABSENT), "the missing variable's name has to reach the developer: {error}");
        assert_eq!(stream_text(&handle).await, "the session's own", "a failed build must not disturb the session's client");

        // Not streamed through: the client this one builds is the real
        // Anthropic client, and polling it would put a request on the wire.
        slash::ModelSwitch::switch(&handle, &config(KEY)).expect("a client that builds replaces the one in place");
    }

    /// Every catalogue row reaches the TUI with its models on it — a row
    /// handed over without them would open a picker with an empty second
    /// list, and first run would fall back to the default it exists to stop
    /// choosing silently.
    #[test]
    fn every_catalogue_row_carries_its_models_to_the_frontend() {
        let choices = catalogue_choices();
        assert_eq!(choices.len(), aldwin_llm::PROVIDERS.len());
        for (choice, provider) in choices.iter().zip(aldwin_llm::PROVIDERS) {
            assert_eq!(choice.id, provider.id);
            assert_eq!(
                choice.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                provider.models.iter().map(|m| m.id).collect::<Vec<_>>(),
                "{} must offer the same models the catalogue lists",
                provider.id
            );
        }
    }

    /// ADR 0007 / audit: reach beyond the project is said out loud, a root
    /// that does not exist is reported rather than silently dropped, and a
    /// reload re-reads the file — the header promises that for every key.
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
        let workspace = aldwin_tools::Workspace::new(project.path());
        assert_eq!(apply_roots(&config, project.path(), &workspace), None, "nothing to say about a plain project");

        std::fs::write(&file, format!("version: 2\nroots:\n- {}\n- ../no-such-dir\n", sibling.path().display())).unwrap();
        config.reload_all().unwrap();
        let notice = apply_roots(&config, project.path(), &workspace).expect("a notice");

        assert_eq!(workspace.roots().len(), 2);
        assert!(notice.contains(&sibling.path().canonicalize().unwrap().display().to_string()), "{notice}");
        assert!(notice.contains("no-such-dir"), "a dropped root must be named: {notice}");
    }
}
