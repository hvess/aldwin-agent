use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use mjolnir_config::{Config, GrantList, InitOutcome, McpServer, ProviderConfig, ProviderKind, Scope, PROVIDER_VERSION};
use mjolnir_core::{Agent, LlmClient, LlmError, LlmEvent, LlmRequest};
use mjolnir_permissions::Engine;
use mjolnir_tools::{register_mcp_tools, Dispatcher, McpBridge};
use futures::{Stream, StreamExt};
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

/// Whichever client `provider_config` selects, built the same way at
/// startup and on every `/model` after it — so a model swapped into a
/// running session is reached exactly as one chosen at launch would be.
fn build_client(config: mjolnir_llm::ProviderConfig) -> Result<AnyLlmClient, mjolnir_llm::LlmClientInitError> {
    Ok(match config.kind {
        ProviderKind::Anthropic => AnyLlmClient::Anthropic(mjolnir_llm::AnthropicClient::new(config)?),
        ProviderKind::OpenaiCompatible => AnyLlmClient::OpenAi(mjolnir_llm::OpenAiCompatibleClient::new(config)?),
    })
}

/// The session's client, behind a swap.
///
/// `Agent<C, D>` takes its client by value and owns it for the life of the
/// process, so `/model` cannot hand it a new one — but it can replace what
/// is *inside* the one it already has. That is this: the agent is handed the
/// handle, `/model` rebuilds the client in it, and core stays generic over
/// `C: LlmClient` without learning that providers exist (see
/// `slash::ModelSwitch`).
/// Held as a trait object rather than an `AnyLlmClient` so what is inside
/// the handle is exactly what core sees through the trait — and so a test
/// can put a client of its own in there and stream through it, which is the
/// one thing about this indirection that has to be proved rather than read.
#[derive(Clone)]
struct ClientHandle(Arc<std::sync::RwLock<Arc<dyn LlmClient>>>);

impl ClientHandle {
    fn new(client: impl LlmClient + 'static) -> Self {
        Self(Arc::new(std::sync::RwLock::new(Arc::new(client))))
    }
}

impl LlmClient for ClientHandle {
    fn stream<'a>(&'a self, request: LlmRequest<'a>) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        // Resolved once, when the request starts, and held by the stream
        // for as long as it runs: a swap landing mid-turn cannot pull the
        // client out from under a request already in flight. That turn
        // finishes on the client it began on and the next one picks up the
        // new one — the same guarantee a turn had when the client could not
        // change at all.
        let client = self.0.read().expect("client lock poisoned").clone();
        Box::pin(async_stream::stream! {
            let mut inner = client.stream(request);
            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

impl ClientHandle {
    fn store(&self, client: Arc<dyn LlmClient>) {
        *self.0.write().expect("client lock poisoned") = client;
    }
}

impl slash::ModelSwitch for ClientHandle {
    /// Builds first and stores second, so a client that cannot be
    /// constructed — the new provider's `api_key_env` is not exported —
    /// leaves the session on the one it has.
    fn switch(&self, config: &mjolnir_llm::ProviderConfig) -> Result<(), String> {
        let client = build_client(config.clone()).map_err(|e| e.to_string())?;
        self.store(Arc::new(client));
        Ok(())
    }
}

const CHANNEL_CAPACITY: usize = 64;

/// The display halves of the whole catalogue, in catalogue order — what
/// first run's provider step and the session's `/model` picker both list.
/// mjolnir-tui is handed ids and purposes and nothing else: it renders the
/// list, it does not know what an endpoint or a key variable is, and it
/// does not depend on this crate or on mjolnir-llm to find out.
fn catalogue_choices() -> Vec<mjolnir_tui::ProviderChoice> {
    mjolnir_llm::PROVIDERS
        .iter()
        .map(|p| {
            mjolnir_tui::ProviderChoice::new(
                p.id,
                p.purpose,
                p.models.iter().map(|m| mjolnir_tui::ModelChoice::new(m.id, m.purpose)).collect(),
            )
        })
        .collect()
}

/// The `provider.yaml` a first-run answer writes: everything but the model
/// comes straight off the catalogue row the developer picked, and the model
/// is that provider's own default (`/model` changes it afterwards).
///
/// `provider.yaml` deliberately has no field a plaintext key could go in
/// (see `ProviderConfig`), so this writes the key variable's *name* and the
/// developer exports the key themselves.
///
/// `current` is whatever already supplies the setting, when anything does.
/// Two fields come from it rather than from the catalogue row:
///
/// * the thinking budget, always — a developer's preference, not the
///   host's, so it survives a move between providers, the same rule
///   `/model` applies;
/// * the key variable, but only when the answer names the provider that is
///   already configured. A developer who exports their Anthropic key as
///   `ANTHROPIC_KEY_WORK` has said so in `provider.yaml`, and changing the
///   *model* on that provider is not a request to be moved back onto the
///   catalogue's default variable name — which would break their next
///   start. Naming a different provider is a different endpoint with a
///   different key, so there the catalogue's variable is the right one.
///
/// Carrying both is also what makes an unchanged answer compare equal to
/// what is on disk, so confirming the lists writes nothing at all.
fn first_run_provider_config(provider: &mjolnir_llm::Provider, model: Option<&str>, current: Option<&ProviderConfig>) -> ProviderConfig {
    let on_this_provider = current.filter(|c| mjolnir_llm::identify(c).map(|p| p.id) == Some(provider.id));
    ProviderConfig {
        version:                  PROVIDER_VERSION,
        provider:                 provider.kind,
        // The developer's own answer, and the provider's default only when
        // the model step could not be asked (a catalogue row with no models
        // — which the real catalogue never has).
        model:                    model.unwrap_or_else(|| provider.default_model()).to_string(),
        base_url:                 provider.base_url.map(String::from),
        api_key_env:              on_this_provider.map_or_else(|| provider.api_key_env.to_string(), |c| c.api_key_env.clone()),
        extended_thinking_budget: current.and_then(|c| c.extended_thinking_budget),
    }
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
fn asks_for_a_provider(needs_provider: bool, configured: &mjolnir_tui::Configured) -> bool {
    needs_provider || configured.provider.is_some()
}

/// Where the first-run screen's provider and model lists open: whatever
/// already supplies the setting, project file over global, reduced to the
/// two ids the screen can match against its own rows.
fn configured_for_first_run(config: &Config) -> mjolnir_tui::Configured {
    let Some(current) = config.project_provider().or_else(|| config.global_provider().ok()) else {
        return mjolnir_tui::Configured::default();
    };
    mjolnir_tui::Configured {
        // `None` for an endpoint the catalogue does not know — there is no
        // row for it, so the lists open at the top rather than on a
        // neighbour that merely looks similar.
        provider: mjolnir_llm::identify(&current).map(|p| p.id.to_string()),
        model:    Some(current.model.clone()),
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

    // First run, per ADR 0001. What opens the screen is either question
    // being unanswered:
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
    //
    // Once the screen is open the provider and model are always on it, even
    // when a global `provider.yaml` already answers them. Entering a new
    // directory is the moment a developer decides what this project runs
    // on, and a screen that offers `access` alone made that undecidable
    // there: the lists were simply absent. They open on what is already
    // configured (`Configured`), so confirming costs three keystrokes and
    // changes nothing — the question is asked, not reopened.
    //
    // The exception is a `provider.yaml` pointed at an endpoint the
    // catalogue has never seen, which no row on that screen represents. The
    // lists would open at the top, on a provider the developer is not
    // using, and pressing through them would move the project off their own
    // endpoint — an answer given by inertia, which is the one thing this
    // screen exists to prevent. So a configured provider the catalogue
    // cannot name is left alone and only `access` is asked, exactly as
    // before.
    let needs_provider = config.global_provider().is_err();
    let needs_access = !cwd.join(".mjolnir").join("permissions.yaml").exists();
    if needs_provider || needs_access {
        // mjolnir-tui is handed the display half of each catalogue row and
        // nothing else — it renders the list, it does not know what an
        // endpoint or a key variable is, and it does not depend on this
        // crate or on mjolnir-llm to find out.
        let choices = catalogue_choices();
        let configured = configured_for_first_run(&config);
        let ask_provider = asks_for_a_provider(needs_provider, &configured);
        // `None` means the developer quit without answering. Nothing is
        // written and no session opens — a first run that was dismissed must
        // not fall back to defaults, least of all for the access question.
        let Some(answers) = mjolnir_tui::run_first_run(theme, choices, mjolnir_llm::CURATED, ask_provider, needs_access, configured)
            .await
            .map_err(StartupError::FirstRun)?
        else {
            return Ok(());
        };
        if let Some(id) = answers.provider.as_deref() {
            let picked = mjolnir_llm::provider(id).ok_or_else(|| StartupError::UnknownProvider { id: id.to_string() })?;
            let current = config.project_provider().or_else(|| config.global_provider().ok());
            let next = first_run_provider_config(picked, answers.model.as_deref(), current.as_ref());
            match current {
                // Nothing to write: the developer confirmed the lists on the
                // rows they opened on. Writing anyway would create a project
                // file that only restates the global one, and then goes stale
                // the first time the global one changes.
                Some(current) if current == next => {}
                // A provider is already configured, so this answer is about
                // *this directory* — the one the screen opened for, and the
                // one already getting a `.mjolnir/` written for its access
                // answer. The developer's global default is left alone.
                Some(_) => config.set_provider(Scope::Project, next).map_err(StartupError::FirstRunWrite)?,
                // A true first run has no global default yet, so the answer
                // becomes one: a project-scope file would leave every other
                // directory unconfigured and ask again in each.
                None => config.set_provider(Scope::Global, next).map_err(StartupError::FirstRunWrite)?,
            }
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
    // What this process boots on. `/model` moves the session off it and
    // updates `slash::Session` in step, so this is a starting point rather
    // than a fact about the whole run.
    let effective_provider = project_provider.clone().unwrap_or_else(|| global_provider.clone());
    let session_model = slash::qualified(&effective_provider, mjolnir_llm::identify(&effective_provider));
    let client = ClientHandle::new(build_client(provider_config)?);

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

    // The agent takes the handle; the interceptor keeps a clone of the same
    // one, which is what makes `/model` a live swap rather than a note for
    // the next start.
    let agent = Agent::new(client.clone(), dispatcher, model_name.clone(), Some(additional_context.as_str()));
    let session_state = slash::Session::new(session_model, Box::new(client));

    // TUI -> interceptor -> core, so slash commands never reach Submit;
    // core -> TUI directly for events (no interception needed there).
    let (tui_cmd_tx, tui_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (agent_cmd_tx, agent_cmd_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

    let interceptor = tokio::spawn(slash::run_interceptor(tui_cmd_rx, agent_cmd_tx, config.clone(), session_state, event_tx.clone()));
    let agent_task = tokio::spawn(agent.run(agent_cmd_rx, event_tx));

    // The picker opens on the row the session is actually running on, which
    // is `effective_provider` — the file that supplies the setting, not the
    // global one it may be shadowing.
    //
    // The label is not `current_provider` with a default: the picker must
    // open on a row that exists, so an unrecognised endpoint leaves that
    // `None`, while the resting screen still has a true name to print — the
    // kind the file itself declares.
    let identified = mjolnir_llm::identify(&effective_provider).map(|p| p.id.to_string());
    let kind = match effective_provider.provider {
        mjolnir_config::ProviderKind::Anthropic => "anthropic",
        mjolnir_config::ProviderKind::OpenaiCompatible => "openai-compatible",
    };
    let session = mjolnir_tui::SessionProvider {
        provider_label:   Some(identified.clone().unwrap_or_else(|| kind.to_string())),
        catalogue:        catalogue_choices(),
        current_provider: identified,
    };
    let tui_result = mjolnir_tui::run(event_rx, tui_cmd_tx, model_name, permissions, theme, session).await;

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

    /// The developer's own model answer is what gets written — the
    /// provider's catalogue default is the fallback for the case the model
    /// step could not be asked at all, not the normal path.
    #[test]
    fn first_run_writes_the_model_that_was_chosen() {
        let anthropic = mjolnir_llm::provider("anthropic").expect("a catalogue provider");
        let chosen = first_run_provider_config(anthropic, Some("claude-opus-5"), None);
        assert_eq!(chosen.model, "claude-opus-5");
        assert_eq!(chosen.api_key_env, anthropic.api_key_env, "the endpoint and key still come from the provider row");

        let unasked = first_run_provider_config(anthropic, None, None);
        assert_eq!(unasked.model, anthropic.default_model());
    }

    /// Confirming the lists on the rows they opened on is not a change, and
    /// must not leave a project file behind restating the global one.
    #[test]
    fn an_unchanged_answer_compares_equal_to_what_is_already_configured() {
        let google = mjolnir_llm::provider("google").expect("a catalogue provider");
        let current = ProviderConfig { extended_thinking_budget: Some(4_000), ..first_run_provider_config(google, Some("gemini-2.5-flash"), None) };
        let confirmed = first_run_provider_config(google, Some("gemini-2.5-flash"), Some(&current));
        assert_eq!(confirmed, current, "the thinking budget travels with it, so an unchanged answer is byte-identical");

        let moved = first_run_provider_config(google, Some("gemini-2.5-pro"), Some(&current));
        assert_ne!(moved, current);
        assert_eq!(moved.extended_thinking_budget, Some(4_000), "a preference of the developer's survives the move");
    }

    /// A key variable the developer chose is part of how they reach their
    /// provider, not part of which model they picked — changing the model
    /// on that provider must not quietly restore the catalogue's default
    /// variable name and break their next start.
    #[test]
    fn a_chosen_key_variable_survives_a_model_change_on_the_same_provider() {
        let anthropic = mjolnir_llm::provider("anthropic").expect("a catalogue provider");
        let current = ProviderConfig { api_key_env: "ANTHROPIC_KEY_WORK".into(), ..first_run_provider_config(anthropic, Some("claude-sonnet-5"), None) };

        let same_provider = first_run_provider_config(anthropic, Some("claude-opus-5"), Some(&current));
        assert_eq!(same_provider.api_key_env, "ANTHROPIC_KEY_WORK", "the developer's own variable is how they reach this provider");
        assert_eq!(same_provider.model, "claude-opus-5");

        // A different provider is a different endpoint with a different
        // key, so there the catalogue's variable is the right one.
        let google = mjolnir_llm::provider("google").expect("a catalogue provider");
        let moved = first_run_provider_config(google, Some("gemini-2.5-flash"), Some(&current));
        assert_eq!(moved.api_key_env, google.api_key_env);
    }

    /// The two steps are on every screen that opens — except when what is
    /// configured is an endpoint the catalogue cannot name, where no row
    /// represents where the developer already is.
    #[test]
    fn the_provider_steps_are_held_back_only_for_an_endpoint_with_no_row() {
        let known = mjolnir_tui::Configured { provider: Some("anthropic".into()), model: Some("claude-opus-5".into()) };
        let unknown = mjolnir_tui::Configured { provider: None, model: Some("qwen3-coder".into()) };

        assert!(asks_for_a_provider(true, &mjolnir_tui::Configured::default()), "a true first run has to ask");
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
        assert_eq!(configured_for_first_run(&config), mjolnir_tui::Configured::default(), "a true first run has nothing to open on");

        let anthropic = mjolnir_llm::provider("anthropic").unwrap();
        config.set_provider(Scope::Global, first_run_provider_config(anthropic, Some("claude-opus-5"), None)).unwrap();
        let configured = configured_for_first_run(&config);
        assert_eq!(configured.provider.as_deref(), Some("anthropic"));
        assert_eq!(configured.model.as_deref(), Some("claude-opus-5"));

        let google = mjolnir_llm::provider("google").unwrap();
        config.set_provider(Scope::Project, first_run_provider_config(google, Some("gemini-2.5-flash"), None)).unwrap();
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
        let handle = ClientHandle::new(NamedClient("first"));
        assert_eq!(stream_text(&handle).await, "first");

        handle.store(Arc::new(NamedClient("second")));
        assert_eq!(stream_text(&handle).await, "second", "the next request runs on the client that replaced it");
    }

    /// The guarantee that makes a live swap safe: a request already in
    /// flight finishes on the client it started on. The stream is built
    /// before the swap and drained after it.
    #[tokio::test]
    async fn a_swap_does_not_reach_a_request_already_in_flight() {
        let handle = ClientHandle::new(NamedClient("first"));
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
        const KEY: &str = "MJOLNIR_SWAP_TEST_KEY";
        const ABSENT: &str = "MJOLNIR_SWAP_TEST_KEY_NEVER_SET";
        std::env::set_var(KEY, "not-a-real-key");

        let config = |key: &str| mjolnir_llm::ProviderConfig {
            kind:                     ProviderKind::Anthropic,
            model:                    "a-model".into(),
            api_key_env:              key.to_string(),
            base_url:                 None,
            extended_thinking_budget: 1_000,
        };

        let handle = ClientHandle::new(NamedClient("the session's own"));
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
        assert_eq!(choices.len(), mjolnir_llm::PROVIDERS.len());
        for (choice, provider) in choices.iter().zip(mjolnir_llm::PROVIDERS) {
            assert_eq!(choice.id, provider.id);
            assert_eq!(
                choice.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                provider.models.iter().map(|m| m.id).collect::<Vec<_>>(),
                "{} must offer the same models the catalogue lists",
                provider.id
            );
        }
    }
}
