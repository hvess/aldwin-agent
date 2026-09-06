use mjolnir_config::Config;
use mjolnir_core::{Command, Event};
use tokio::sync::mpsc;

/// What `intercept` decided to do with one incoming command.
enum Intercepted {
    /// Not a slash command (or not a `Submit` at all) — forward unchanged.
    Forward(Command),
    /// A known slash command ran (or an unknown one was rejected); nothing
    /// reaches the core.
    Handled,
    /// `/exit` — `run_interceptor` stops entirely rather than
    /// looping again, per mjolnir-cli.md's Decisions: "CLI owns the
    /// dispatch table so slash commands can trigger ... process
    /// operations that the core has no visibility into." Ending the
    /// interceptor task drops both its `forward` (core command) and
    /// `events` sender clones; the core's own command channel then closes
    /// too (dropping its `events` sender in turn), so the TUI's event
    /// channel closes once both are gone and it exits the same way it
    /// already does on `None` from `events.recv()` — no new `Event`
    /// variant needed, and no core changes at all.
    Quit,
}

/// Single source of truth for `/help`'s listing — keep in sync with the
/// `match` in `intercept` below by hand; six entries doesn't earn a
/// data-driven dispatch table yet.
const HELP_TEXT: &str = "commands: /help (this list), /clear (clear conversation context), /exit (end the session), \
     /model [provider/]model (pick where the model runs, and which one), /reload-config (reload config files from disk), \
     /theme light|dark (switch color theme)";

/// `/model`'s own usage line, quoted by every branch that rejects an
/// argument so the developer never has to go and find `/help`.
const MODEL_USAGE: &str = "usage: /model [provider/]model";

/// Valid `/theme` argument values — kept as the single source of truth for
/// both the accept-check and the error message's own listing, so the two
/// can't drift apart.
const VALID_THEMES: [&str; 2] = ["dark", "light"];

/// Intercepts `/`-prefixed `Submit` input before it would otherwise reach
/// the core, per mjolnir-cli.md: "the core's only input is Submit, Cancel,
/// ApproveTool — it has no slash-command semantics." Runs synchronously in
/// the interceptor's own recv loop (`run_interceptor`), before any forward
/// send — not a post-send hook, per the spec's explicit Pitfall.
async fn intercept(command: Command, config: &Config, session_model: &str, events: &mpsc::Sender<Event>) -> Intercepted {
    let Command::Submit { text } = &command else { return Intercepted::Forward(command) };
    let Some(rest) = text.trim_start().strip_prefix('/') else { return Intercepted::Forward(command) };

    let rest = rest.trim();
    match rest {
        "help" => {
            let _ = events.send(Event::Notice { message: HELP_TEXT.into() }).await;
            Intercepted::Handled
        }
        "reload-config" => {
            handle_reload_config(config, events).await;
            Intercepted::Handled
        }
        // Unlike /help and /reload-config, this one core needs to act on
        // (wipe ConversationLog) — so it's translated and forwarded rather
        // than handled locally; core acknowledges with Event::HistoryCleared
        // once done, which is what actually tells the TUI to wipe its own
        // rendered log (see mjolnir_tui::App::apply_event).
        "clear" => Intercepted::Forward(Command::ClearHistory),
        "exit" => Intercepted::Quit,
        // "theme" alone (no argument) reports the current setting rather
        // than erroring — same "tell the developer where they stand"
        // instinct as `/help`, since there's no running-TUI state this
        // process can peek at other than what's already on disk.
        "theme" => {
            handle_theme(None, config, events).await;
            Intercepted::Handled
        }
        other if other.starts_with("theme ") => {
            handle_theme(Some(other["theme ".len()..].trim()), config, events).await;
            Intercepted::Handled
        }
        // Same shape as `/theme`: bare reports where the developer stands,
        // an argument changes it.
        "model" => {
            handle_model(None, config, session_model, events).await;
            Intercepted::Handled
        }
        other if other.starts_with("model ") => {
            handle_model(Some(other["model ".len()..].trim()), config, session_model, events).await;
            Intercepted::Handled
        }
        other => {
            let _ = events.send(Event::Notice { message: format!("unknown slash command: /{other} (try /help)") }).await;
            Intercepted::Handled
        }
    }
}

/// `/theme [light|dark]`. Unlike `/clear`, this never needs core at all —
/// it's a config write (`Config::set_tui`, persisting the choice so it
/// survives the developer's next launch, not just this session) plus an
/// `Event::ThemeChanged` sent directly into the same channel the TUI reads
/// from (see that event's own doc comment in mjolnir-core for why the
/// interceptor can reach the TUI this way without core's involvement).
/// `App::theme` is read fresh by `ui::draw` on every frame, so the change
/// is visible on the very next redraw — no restart needed.
async fn handle_theme(arg: Option<&str>, config: &Config, events: &mpsc::Sender<Event>) {
    let Some(arg) = arg else {
        let current = config.global_tui().theme.unwrap_or_else(|| "dark".into());
        let _ = events.send(Event::Notice { message: format!("current theme: {current} (usage: /theme light|dark)") }).await;
        return;
    };
    let normalized = arg.to_ascii_lowercase();
    if !VALID_THEMES.contains(&normalized.as_str()) {
        let _ = events.send(Event::Notice { message: format!("unknown theme {arg:?} (usage: /theme light|dark)") }).await;
        return;
    }

    let mut tui = config.global_tui();
    tui.theme = Some(normalized.clone());
    match config.set_tui(tui) {
        Ok(()) => {
            let _ = events.send(Event::Notice { message: format!("theme set to {normalized}") }).await;
            let _ = events.send(Event::ThemeChanged { theme: normalized }).await;
        }
        Err(e) => {
            let _ = events.send(Event::Notice { message: format!("failed to save theme: {e}") }).await;
        }
    }
}

/// `/model [provider/]model` — the command first run's `provider` step
/// promises. It picks both halves of "where the model runs, and which one",
/// which is why it is one command rather than two: a model id is meaningless
/// without the provider whose catalogue it comes from, and picking a provider
/// with no model would leave `provider.yaml` incomplete.
///
/// **Argument grammar.** The argument is split on its *first* `/` only.
///
/// * A provider's name, alone or with a trailing `/` — that provider. On its
///   default model, unless it is already the configured provider, in which
///   case the model you are on is kept: naming where you already are is not
///   a request to be moved.
/// * `provider/model` — both halves at once. Everything after the first
///   slash is the model, so a model id that itself contains slashes is
///   reachable as `openrouter/qwen/qwen3-coder`.
/// * Anything else — a model id on the provider already configured.
///
/// A name the catalogue knows is a provider in *either* form, which is the
/// rule the first cut got wrong: it validated the slashed form and read the
/// bare form as a model id, so `/model openai` wrote `model: openai` onto
/// whatever provider was set and reported success.
///
/// A slashed argument whose first segment is *not* a provider is rejected
/// rather than read as a model id containing a slash. Both readings are
/// available, and the rejected one is what a mistyped provider name looks
/// like: `/model gogle/gemini-2.5-pro` would otherwise quietly write
/// `gogle/gemini-2.5-pro` as a model on whatever provider was already set.
///
/// The provider half is validated against the catalogue — it decides an
/// endpoint, a wire dialect and a key variable, none of which can be guessed
/// from a name. The model half is not: a provider's real catalogue is a
/// network call away and changes without us, so `provider.yaml` takes any
/// model id and so does this. `mjolnir_llm::PROVIDERS`' model lists are
/// suggestions, and the notice says so by listing them as "known".
///
/// **It does not take effect now.** The `LlmClient` was constructed at
/// startup and handed to the agent loop, which owns it for the life of the
/// process; there is no way to swap it under a running turn. So this
/// persists the choice and says plainly that the running session keeps the
/// model it started with — unlike `/theme`, which really does apply on the
/// next redraw.
async fn handle_model(arg: Option<&str>, config: &Config, session_model: &str, events: &mpsc::Sender<Event>) {
    // Whichever scope actually supplies the setting is the one that gets
    // written: writing global while a project `provider.yaml` shadows it
    // would report a change the next start would ignore.
    let (scope, current) = match config.project_provider() {
        Some(project) => (mjolnir_config::Scope::Project, project),
        None => match config.global_provider() {
            Ok(global) => (mjolnir_config::Scope::Global, global),
            Err(e) => {
                let _ = events.send(Event::Notice { message: format!("no provider is configured: {e}") }).await;
                return;
            }
        },
    };
    let known = mjolnir_llm::identify(&current);

    let Some(arg) = arg.filter(|a| !a.is_empty()) else {
        let _ = events.send(Event::Notice { message: describe(&current, known) }).await;
        return;
    };

    // Split on the first `/` only — see this function's own doc comment for
    // why the left half is a provider only when it names one.
    //
    // A *bare* provider name means the same as `provider/`: that provider, on
    // its default model. Reading it as a model id was the first cut and was
    // silently destructive — `/model openai` wrote `model: openai` onto
    // whatever provider was already configured and reported success, and the
    // next start failed at the host with a model it had never heard of. The
    // slashed form was validated against the catalogue and the bare form was
    // not, which is the same name treated two different ways.
    let (provider, model) = match arg.split_once('/') {
        Some((head, tail)) => match named_provider(head) {
            Some(p) => (Some(p), tail.trim()),
            None => (None, arg),
        },
        None => match named_provider(arg) {
            Some(p) => (Some(p), ""),
            None => (None, arg),
        },
    };

    // A leading `/` from `/model /foo`, or a trailing one from
    // `/model anthropic/`, both land here as an empty model half.
    let mut next = match provider {
        Some(p) => {
            let mut next = mjolnir_config::ProviderConfig {
                version:                  mjolnir_config::PROVIDER_VERSION,
                provider:                 p.kind,
                model:                    p.default_model().to_string(),
                base_url:                 p.base_url.map(String::from),
                api_key_env:              p.api_key_env.to_string(),
                // A thinking budget is the developer's preference, not the
                // host's, so it survives a move between providers.
                extended_thinking_budget: current.extended_thinking_budget,
            };
            if !model.is_empty() {
                next.model = model.to_string();
            } else if known.map(|c| c.id) == Some(p.id) {
                // Naming the provider you are already on is not a request to
                // be moved off the model you are already using. Without this,
                // `/model anthropic` on `anthropic/claude-opus-5` would
                // quietly drop you back to the catalogue's default.
                next.model = current.model.clone();
            }
            next
        }
        None => {
            if model.contains('/') {
                let ids = mjolnir_llm::provider_ids().join(", ");
                let message = format!("unknown provider {:?} (known: {ids}; {MODEL_USAGE})", model.split('/').next().unwrap_or(model));
                let _ = events.send(Event::Notice { message }).await;
                return;
            }
            mjolnir_config::ProviderConfig { model: model.to_string(), ..current.clone() }
        }
    };
    next.version = mjolnir_config::PROVIDER_VERSION;

    if next == current {
        let _ = events.send(Event::Notice { message: format!("already on {}", qualified(&current, known)) }).await;
        return;
    }

    match config.set_provider(scope, next.clone()) {
        Ok(()) => {
            let where_ = match scope {
                mjolnir_config::Scope::Project => "this project's provider.yaml",
                mjolnir_config::Scope::Global => "the global provider.yaml",
            };
            let now = qualified(&next, mjolnir_llm::identify(&next));
            let message = format!(
                "{now} saved to {where_} — this session keeps {session_model}, since the client it started with cannot be \
                 swapped mid-run. Restart to use it."
            );
            let _ = events.send(Event::Notice { message }).await;
        }
        Err(e) => {
            let _ = events.send(Event::Notice { message: format!("failed to save the model: {e}") }).await;
        }
    }
}

/// The catalogue row `name` names, case-insensitively.
///
/// Only the *provider* half is folded: catalogue ids are lowercase by
/// construction (a test pins it) and `/theme` already accepts `LIGHT`, so
/// rejecting `/model Anthropic` would be the odd one out. Model ids are left
/// exactly as typed — they are opaque strings a host compares byte for byte,
/// and some really are mixed-case.
fn named_provider(name: &str) -> Option<&'static mjolnir_llm::Provider> {
    mjolnir_llm::provider(&name.trim().to_ascii_lowercase())
}

/// `provider/model` when the endpoint is one the catalogue knows, and the
/// bare model id when the developer has pointed `provider.yaml` at an
/// endpoint of their own — naming a provider there would be a guess.
pub(crate) fn qualified(config: &mjolnir_config::ProviderConfig, known: Option<&mjolnir_llm::Provider>) -> String {
    match known {
        Some(p) => format!("{}/{}", p.id, config.model),
        None => config.model.clone(),
    }
}

/// What the bare `/model` reports: where the developer stands, what else
/// that provider offers, and every provider there is.
fn describe(current: &mjolnir_config::ProviderConfig, known: Option<&mjolnir_llm::Provider>) -> String {
    let mut out = format!("model: {}", qualified(current, known));
    if let Some(p) = known {
        let others: Vec<&str> = p.models.iter().map(|m| m.id).filter(|id| *id != current.model).collect();
        if !others.is_empty() {
            out.push_str(&format!(" · known {} models: {}", p.id, others.join(", ")));
        }
    } else {
        // An endpoint the catalogue has never seen — say so rather than
        // silently reporting a bare model id as though it were the whole
        // answer.
        out.push_str(&format!(" · at {}", current.base_url.as_deref().unwrap_or("the provider's default endpoint")));
    }
    out.push_str(&format!(" · providers: {}", mjolnir_llm::provider_ids().join(", ")));
    out.push_str(&format!(" ({MODEL_USAGE})"));
    out
}

async fn handle_reload_config(config: &Config, events: &mpsc::Sender<Event>) {
    match config.reload_all() {
        // No PermissionsEngine re-instantiation needed: Engine holds this
        // same (Arc-backed) Config handle, so reload_all()'s in-place
        // mutation is visible on Engine's very next check_tool call —
        // nothing to swap, and nothing to have swapped stale (see the
        // spec's Pitfall on this). PermissionsChanged just tells the TUI to
        // refresh its status-bar summary against what's already current.
        Ok(()) => {
            let _ = events.send(Event::Notice { message: "config reloaded".into() }).await;
            let _ = events.send(Event::PermissionsChanged { payload: serde_json::Value::Null }).await;
        }
        Err(failures) => {
            let detail = failures.iter().map(|f| format!("{}: {}", f.path.display(), f.error)).collect::<Vec<_>>().join("; ");
            let _ = events.send(Event::Notice { message: format!("reload failed ({detail}); previous config retained") }).await;
        }
    }
}

/// Background task: pumps every command the TUI sends through `intercept`,
/// forwarding what survives to the core. Ends (and so drops `forward`,
/// closing the core's command channel) once `incoming` closes — which
/// happens when the TUI's own `run()` returns and drops its sender.
pub async fn run_interceptor(
    mut incoming: mpsc::Receiver<Command>,
    forward: mpsc::Sender<Command>,
    config: Config,
    session_model: String,
    events: mpsc::Sender<Event>,
) {
    while let Some(command) = incoming.recv().await {
        match intercept(command, &config, &session_model, &events).await {
            Intercepted::Forward(command) => {
                if forward.send(command).await.is_err() {
                    break;
                }
            }
            Intercepted::Handled => {}
            Intercepted::Quit => break,
        }
    }
    // `forward` and `events` drop here — see `Intercepted::Quit`'s doc
    // comment for why that's enough to shut the whole session down.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the process booted with. Every notice that says what the running
    /// session is still using must name *this*, not whatever the last
    /// `/model` call wrote — those are different once the command has been
    /// used twice in one session.
    const SESSION_MODEL: &str = "anthropic/claude-sonnet-5";

    fn config() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        (project, global, config)
    }

    #[tokio::test]
    async fn non_slash_input_passes_through_unchanged() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let cmd = Command::Submit { text: "hello".into() };
        let result = intercept(cmd, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Forward(Command::Submit { text }) if text == "hello"));
    }

    #[tokio::test]
    async fn non_submit_commands_pass_through_unchanged() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let cmd = Command::PromptResponse { call_id: "call-1".into(), payload: serde_json::Value::Null };
        let result = intercept(cmd, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Forward(Command::PromptResponse { .. })));
    }

    #[tokio::test]
    async fn unknown_slash_command_is_rejected_and_never_forwarded() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit { text: "/nope".into() };
        let result = intercept(cmd, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains("/nope")),
            other => panic!("expected a Notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn help_lists_every_known_command() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/help".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => {
                for command in ["/help", "/clear", "/exit", "/model", "/reload-config", "/theme"] {
                    assert!(message.contains(command), "help text missing {command}: {message}");
                }
            }
            other => panic!("expected a Notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exit_is_recognised_as_the_quit_command() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/exit".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Quit));
    }

    #[tokio::test]
    async fn clear_is_translated_and_forwarded_to_core_not_handled_locally() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/clear".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(
            matches!(result, Intercepted::Forward(Command::ClearHistory)),
            "core owns ConversationLog, so /clear must reach it as ClearHistory rather than being swallowed like /help"
        );
    }

    #[tokio::test]
    async fn quit_is_not_recognised_only_exit_is() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/quit".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled), "/quit must not be a recognised command");
        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains("/quit")),
            other => panic!("expected a Notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reload_config_success_emits_notice_and_permissions_changed() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit { text: "/reload-config".into() };
        let result = intercept(cmd, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        assert!(matches!(rx.recv().await, Some(Event::Notice { .. })));
        assert!(matches!(rx.recv().await, Some(Event::PermissionsChanged { .. })));
    }

    #[tokio::test]
    async fn reload_config_failure_surfaces_the_failing_path_verbatim() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        // Establish the project dir, then hand-corrupt permissions.yaml so
        // reload_all() fails on that one layer.
        config.add_grant(mjolnir_config::Scope::Project, mjolnir_config::GrantList::Allow, "read:**").unwrap();
        let bad_path = project.path().join(".mjolnir").join("permissions.yaml");
        std::fs::write(&bad_path, "not: [valid, yaml: at all").unwrap();

        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit { text: "/reload-config".into() };
        intercept(cmd, &config, SESSION_MODEL, &tx).await;

        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains(&bad_path.display().to_string()), "message was: {message}"),
            other => panic!("expected a Notice, got {other:?}"),
        }
        // No PermissionsChanged on failure — nothing changed.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn theme_with_no_argument_reports_the_current_default() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/theme".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains("current theme: dark"), "message was: {message}"),
            other => panic!("expected a Notice, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no-argument /theme must not persist or emit ThemeChanged");
    }

    #[tokio::test]
    async fn theme_light_persists_and_emits_theme_changed() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/theme light".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));

        assert!(matches!(rx.recv().await, Some(Event::Notice { .. })));
        match rx.recv().await {
            Some(Event::ThemeChanged { theme }) => assert_eq!(theme, "light"),
            other => panic!("expected ThemeChanged, got {other:?}"),
        }
        assert_eq!(cfg.global_tui().theme.as_deref(), Some("light"), "the choice must survive the next launch too, not just this session");
    }

    #[tokio::test]
    async fn theme_argument_is_case_insensitive() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/theme LIGHT".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = rx.recv().await; // Notice
        match rx.recv().await {
            Some(Event::ThemeChanged { theme }) => assert_eq!(theme, "light", "must normalize to lowercase"),
            other => panic!("expected ThemeChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn theme_back_to_dark_persists_and_emits_theme_changed() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/theme light".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = rx.recv().await;
        let _ = rx.recv().await;

        intercept(Command::Submit { text: "/theme dark".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = rx.recv().await; // Notice
        match rx.recv().await {
            Some(Event::ThemeChanged { theme }) => assert_eq!(theme, "dark"),
            other => panic!("expected ThemeChanged, got {other:?}"),
        }
        assert_eq!(cfg.global_tui().theme.as_deref(), Some("dark"));
    }

    #[tokio::test]
    async fn theme_invalid_value_is_rejected_not_persisted_and_no_theme_changed_sent() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/theme neon".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains("neon"), "message was: {message}"),
            other => panic!("expected a Notice, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "an invalid theme must not emit ThemeChanged");
        assert_eq!(cfg.global_tui().theme, None, "an invalid theme must not be persisted");
    }

    /// Every `/model` test needs a provider already on disk — the session
    /// this command runs in cannot exist without one.
    fn with_provider(config: &Config, scope: mjolnir_config::Scope, id: &str) {
        let p = mjolnir_llm::provider(id).expect("a catalogue provider");
        config
            .set_provider(
                scope,
                mjolnir_config::ProviderConfig {
                    version:                  mjolnir_config::PROVIDER_VERSION,
                    provider:                 p.kind,
                    model:                    p.default_model().into(),
                    base_url:                 p.base_url.map(String::from),
                    api_key_env:              p.api_key_env.into(),
                    extended_thinking_budget: None,
                },
            )
            .unwrap();
    }

    async fn notice(rx: &mut mpsc::Receiver<Event>) -> String {
        match rx.recv().await {
            Some(Event::Notice { message }) => message,
            other => panic!("expected a Notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn model_with_no_argument_reports_where_the_developer_stands() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/model".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(matches!(result, Intercepted::Handled));

        let message = notice(&mut rx).await;
        assert!(message.contains("model: anthropic/claude-sonnet-5"), "{message}");
        assert!(message.contains("providers: anthropic"), "the bare form has to say what it would accept: {message}");
        assert!(message.contains(MODEL_USAGE), "{message}");
        assert!(rx.try_recv().is_err(), "no-argument /model must not persist anything");
    }

    /// A bare model id keeps the provider — the common case, and the one
    /// where a provider name would be noise.
    #[tokio::test]
    async fn a_bare_model_id_changes_the_model_and_leaves_the_provider_alone() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model claude-opus-5".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("anthropic/claude-opus-5"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.model, "claude-opus-5");
        assert_eq!(saved.provider, mjolnir_config::ProviderKind::Anthropic, "the provider must be untouched");
    }

    /// The command has to say that nothing changed *now*, because nothing
    /// did: the client was built at startup and belongs to the agent loop.
    #[tokio::test]
    async fn changing_the_model_says_the_running_session_keeps_the_old_one() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model claude-opus-5".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("this session keeps anthropic/claude-sonnet-5"), "{message}");
        assert!(message.contains("Restart"), "{message}");
    }

    /// `provider/model` moves both halves — endpoint and key variable
    /// included, which is the whole reason the provider half is validated.
    #[tokio::test]
    async fn a_qualified_argument_moves_the_endpoint_and_the_key_variable_too() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model google/gemini-2.5-flash".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("google/gemini-2.5-flash"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.provider, mjolnir_config::ProviderKind::OpenaiCompatible);
        assert_eq!(saved.model, "gemini-2.5-flash");
        assert_eq!(saved.api_key_env, "GOOGLE_API_KEY");
        assert_eq!(saved.base_url, mjolnir_llm::provider("google").unwrap().base_url.map(String::from));
    }

    /// A provider named with no model takes that provider's default, so the
    /// file is never left half-written.
    #[tokio::test]
    async fn a_provider_with_no_model_takes_that_providers_default() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model google/".into() }, &cfg, SESSION_MODEL, &tx).await;

        let _ = notice(&mut rx).await;
        assert_eq!(cfg.global_provider().unwrap().model, mjolnir_llm::provider("google").unwrap().default_model());
    }

    /// Everything past the *first* slash is the model, so a model id that
    /// contains slashes is reachable by naming its provider.
    #[tokio::test]
    async fn only_the_first_slash_splits_so_a_slashed_model_id_survives() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model deepseek/vendor/some-model".into() }, &cfg, SESSION_MODEL, &tx).await;

        let _ = notice(&mut rx).await;
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.model, "vendor/some-model");
        assert_eq!(saved.api_key_env, "DEEPSEEK_API_KEY");
    }

    /// A bare provider name means that provider on its default model —
    /// the same as `provider/`. The first cut read it as a *model* id and
    /// wrote `model: openai` onto whatever provider was already set,
    /// reporting success; the next start then failed at the host with a
    /// model it had never heard of.
    #[tokio::test]
    async fn a_bare_provider_name_switches_provider_rather_than_becoming_a_model_id() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model openai".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("openai/gpt-5"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.model, mjolnir_llm::provider("openai").unwrap().default_model());
        assert_eq!(saved.api_key_env, "OPENAI_API_KEY", "the endpoint and key must move with the name");
    }

    /// Naming the provider you are already on keeps the model you are on.
    /// Taking the catalogue default instead would make `/model anthropic`
    /// a silent downgrade from `claude-opus-5`.
    #[tokio::test]
    async fn naming_the_current_provider_keeps_the_current_model() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model claude-opus-5".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = notice(&mut rx).await;

        intercept(Command::Submit { text: "/model anthropic".into() }, &cfg, SESSION_MODEL, &tx).await;
        let message = notice(&mut rx).await;
        assert!(message.contains("already on anthropic/claude-opus-5"), "{message}");
        assert_eq!(cfg.global_provider().unwrap().model, "claude-opus-5");
    }

    /// `/model openai` and `/model openai/` are the same instruction.
    #[tokio::test]
    async fn a_bare_provider_and_a_trailing_slash_mean_the_same_thing() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);

        intercept(Command::Submit { text: "/model openai".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = notice(&mut rx).await;
        let bare = cfg.global_provider().unwrap();

        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        intercept(Command::Submit { text: "/model openai/".into() }, &cfg, SESSION_MODEL, &tx).await;
        let _ = notice(&mut rx).await;
        assert_eq!(cfg.global_provider().unwrap(), bare);
    }

    /// The provider half folds case, like `/theme` does; the model half is
    /// left exactly as typed, because a host compares it byte for byte.
    #[tokio::test]
    async fn the_provider_half_is_case_insensitive_and_the_model_half_is_not() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model Google/Gemini-2.5-Flash".into() }, &cfg, SESSION_MODEL, &tx).await;

        let _ = notice(&mut rx).await;
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.api_key_env, "GOOGLE_API_KEY", "GOOGLE must resolve to the google row");
        assert_eq!(saved.model, "Gemini-2.5-Flash", "the model id must survive verbatim");
    }

    /// A mistyped provider is rejected rather than written as part of a
    /// model id on whatever provider happened to be set.
    #[tokio::test]
    async fn a_slashed_argument_with_an_unknown_provider_is_rejected_not_written() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model gogle/gemini-2.5-pro".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("unknown provider \"gogle\""), "{message}");
        assert!(message.contains("known: anthropic"), "{message}");
        assert_eq!(cfg.global_provider().unwrap().model, "claude-sonnet-5", "nothing may be written on a rejection");
    }

    /// Writing global while a project `provider.yaml` shadows it would
    /// report a change the next start ignores.
    #[tokio::test]
    async fn the_scope_written_is_the_one_that_actually_supplies_the_setting() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        with_provider(&cfg, mjolnir_config::Scope::Project, "google");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model gemini-2.5-flash".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("this project's provider.yaml"), "{message}");
        assert_eq!(cfg.project_provider().unwrap().model, "gemini-2.5-flash");
        assert_eq!(cfg.global_provider().unwrap().model, "claude-sonnet-5", "the shadowed scope must be left alone");
    }

    /// Setting what is already set says so instead of reporting a change
    /// and telling the developer to restart for it.
    #[tokio::test]
    async fn setting_the_current_model_reports_no_change_and_writes_nothing() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model anthropic/claude-sonnet-5".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("already on anthropic/claude-sonnet-5"), "{message}");
        assert!(!message.contains("Restart"), "nothing changed, so nothing needs restarting: {message}");
    }

    /// Two `/model` calls in one session: the second must still name what
    /// the *process* booted with, not what the first call wrote. Reading the
    /// current setting off disk for this would have been wrong the moment
    /// the command was used twice.
    #[tokio::test]
    async fn the_session_model_reported_is_the_one_the_process_started_with() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, mjolnir_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);

        intercept(Command::Submit { text: "/model claude-opus-5".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(notice(&mut rx).await.contains("this session keeps anthropic/claude-sonnet-5"));

        intercept(Command::Submit { text: "/model lumo/lumo-max".into() }, &cfg, SESSION_MODEL, &tx).await;
        let message = notice(&mut rx).await;
        assert!(message.contains("this session keeps anthropic/claude-sonnet-5"), "{message}");
        assert!(!message.contains("keeps anthropic/claude-opus-5"), "the first call's write is not what the session is running: {message}");
    }

    /// A hand-written endpoint is not a catalogue provider, and must not be
    /// reported as one.
    #[tokio::test]
    async fn an_endpoint_the_catalogue_does_not_know_is_reported_as_itself() {
        let (_project, _global, cfg) = config();
        cfg.set_provider(
            mjolnir_config::Scope::Global,
            mjolnir_config::ProviderConfig {
                version:                  mjolnir_config::PROVIDER_VERSION,
                provider:                 mjolnir_config::ProviderKind::OpenaiCompatible,
                model:                    "qwen3-coder".into(),
                base_url:                 Some("http://localhost:8000/v1/chat/completions".into()),
                api_key_env:              "VLLM_API_KEY".into(),
                extended_thinking_budget: None,
            },
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model".into() }, &cfg, SESSION_MODEL, &tx).await;

        let message = notice(&mut rx).await;
        assert!(message.contains("model: qwen3-coder"), "{message}");
        assert!(message.contains("http://localhost:8000/v1/chat/completions"), "{message}");
        assert!(!message.contains("model: lumo/"), "a local endpoint must not be labelled with someone else's name: {message}");
    }

    #[tokio::test]
    async fn model_with_no_provider_configured_says_so_rather_than_panicking() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(Command::Submit { text: "/model claude-opus-5".into() }, &cfg, SESSION_MODEL, &tx).await;
        assert!(notice(&mut rx).await.contains("no provider is configured"));
    }

    #[tokio::test]
    async fn run_interceptor_forwards_normal_input_and_stops_others_reaching_it() {
        let (_project, _global, cfg) = config();
        let (tui_tx, tui_rx) = mpsc::channel(8);
        let (forward_tx, mut forward_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);

        let handle = tokio::spawn(run_interceptor(tui_rx, forward_tx, cfg, SESSION_MODEL.into(), event_tx));

        tui_tx.send(Command::Submit { text: "/nope".into() }).await.unwrap();
        tui_tx.send(Command::Submit { text: "hi".into() }).await.unwrap();
        drop(tui_tx); // simulates the TUI exiting

        assert!(matches!(event_rx.recv().await, Some(Event::Notice { .. })));
        assert!(matches!(forward_rx.recv().await, Some(Command::Submit { text }) if text == "hi"));
        assert!(forward_rx.recv().await.is_none(), "forward sender must be dropped once incoming closes");
        handle.await.unwrap();
    }

    /// `/exit` must stop `run_interceptor` outright — not just skip
    /// forwarding this one command — dropping both its `forward` and
    /// `events` sender clones so the core's command channel closes (and,
    /// once the core drains, its own `events` sender), which is what
    /// eventually closes the TUI's event channel and lets it exit.
    #[tokio::test]
    async fn slash_exit_stops_the_interceptor_and_drops_its_senders() {
        let (_project, _global, cfg) = config();
        let (tui_tx, tui_rx) = mpsc::channel(8);
        let (forward_tx, mut forward_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);

        let handle = tokio::spawn(run_interceptor(tui_rx, forward_tx, cfg, SESSION_MODEL.into(), event_tx));

        tui_tx.send(Command::Submit { text: "/exit".into() }).await.unwrap();

        // The interceptor task ends on its own — no need to drop tui_tx.
        handle.await.unwrap();
        assert!(forward_rx.recv().await.is_none(), "forward must be dropped so the core's command channel closes");
        assert!(event_rx.recv().await.is_none(), "events must be dropped, not left open, on quit");
    }
}

