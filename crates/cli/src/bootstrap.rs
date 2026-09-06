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
#[derive(Debug)]
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
#[derive(Clone)]
struct ClientHandle(Arc<std::sync::RwLock<Arc<AnyLlmClient>>>);

impl ClientHandle {
    fn new(client: AnyLlmClient) -> Self {
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

impl slash::ModelSwitch for ClientHandle {
    /// Builds first and stores second, so a client that cannot be
    /// constructed — the new provider's `api_key_env` is not exported —
    /// leaves the session on the one it has.
    fn switch(&self, config: &mjolnir_llm::ProviderConfig) -> Result<(), String> {
        let client = build_client(config.clone()).map_err(|e| e.to_string())?;
        *self.0.write().expect("client lock poisoned") = Arc::new(client);
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
/// Only the thinking budget is taken from it — a developer's preference,
/// not the host's, so it survives a move between providers, the same rule
/// `/model` applies. It is also what makes an unchanged answer compare
/// equal to what is on disk, so confirming the lists writes nothing.
fn first_run_provider_config(provider: &mjolnir_llm::Provider, model: Option<&str>, current: Option<&ProviderConfig>) -> ProviderConfig {
    ProviderConfig {
        version:                  PROVIDER_VERSION,
        provider:                 provider.kind,
        // The developer's own answer, and the provider's default only when
        // the model step could not be asked (a catalogue row with no models
        // — which the real catalogue never has).
        model:                    model.unwrap_or_else(|| provider.default_model()).to_string(),
        base_url:                 provider.base_url.map(String::from),
        api_key_env:              provider.api_key_env.to_string(),
        extended_thinking_budget: current.and_then(|c| c.extended_thinking_budget),
    }
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
    let needs_provider = config.global_provider().is_err();
    let needs_access = !cwd.join(".mjolnir").join("permissions.yaml").exists();
    if needs_provider || needs_access {
        // mjolnir-tui is handed the display half of each catalogue row and
        // nothing else — it renders the list, it does not know what an
        // endpoint or a key variable is, and it does not depend on this
        // crate or on mjolnir-llm to find out.
        let choices = catalogue_choices();
        let configured = configured_for_first_run(&config);
        // `None` means the developer quit without answering. Nothing is
        // written and no session opens — a first run that was dismissed must
        // not fall back to defaults, least of all for the access question.
        let Some(answers) = mjolnir_tui::run_first_run(theme, choices, mjolnir_llm::CURATED, true, needs_access, configured)
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
    let session = mjolnir_tui::SessionProvider {
        catalogue:        catalogue_choices(),
        current_provider: mjolnir_llm::identify(&effective_provider).map(|p| p.id.to_string()),
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

    /// A swap is the whole of `/model`'s live half, and its failure mode is
    /// the one startup has: a key variable that is not exported. The
    /// session has to be left on the client it already had when that
    /// happens, not on nothing.
    #[test]
    fn a_swap_that_cannot_build_a_client_leaves_the_running_one_in_place() {
        // A name of this test's own, so a parallel test's environment can
        // neither satisfy nor break it.
        const KEY: &str = "MJOLNIR_SWAP_TEST_KEY";
        const ABSENT: &str = "MJOLNIR_SWAP_TEST_KEY_NEVER_SET";
        std::env::set_var(KEY, "not-a-real-key");

        let config = |key: &str, model: &str| mjolnir_llm::ProviderConfig {
            kind:                     ProviderKind::Anthropic,
            model:                    model.to_string(),
            api_key_env:              key.to_string(),
            base_url:                 None,
            extended_thinking_budget: 1_000,
        };
        let running = |handle: &ClientHandle| format!("{:?}", handle.0.read().unwrap());

        let handle = ClientHandle::new(build_client(config(KEY, "first-model")).expect("the key is exported"));
        assert!(running(&handle).contains("first-model"));

        let error = slash::ModelSwitch::switch(&handle, &config(ABSENT, "second-model")).expect_err("no key is exported for this one");
        assert!(error.contains(ABSENT), "the missing variable's name has to reach the developer: {error}");
        assert!(running(&handle).contains("first-model"), "a failed build must not disturb the session's client");

        slash::ModelSwitch::switch(&handle, &config(KEY, "second-model")).expect("a client that builds replaces the one in place");
        assert!(running(&handle).contains("second-model"));
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
