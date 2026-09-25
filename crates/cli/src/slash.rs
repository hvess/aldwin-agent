use std::sync::Arc;

use aldwin_config::Config;
use aldwin_core::{Command, Event, SessionId};
use tokio::sync::mpsc;

use crate::history::History;

/// What `intercept` decided to do with one incoming command.
enum Intercepted {
    /// Not a slash command (or not a `Submit` at all) — forward unchanged.
    Forward(Command),
    /// A known slash command ran (or an unknown one was rejected); nothing
    /// reaches the core.
    Handled,
    /// `/exit` — `run_interceptor` stops entirely rather than
    /// looping again, per aldwin-cli.md's Decisions: "CLI owns the
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

/// One slash command, as the developer sees it: the name typed after the
/// `/`, what follows it when it takes an argument, and what it does.
pub(crate) struct SlashCommand {
    name: &'static str,
    argument: &'static str,
    summary: &'static str,
    /// Offered by the `/` menu. The rest are typed, and `/help` names them
    /// so they are not secret.
    in_menu: bool,
}

/// Every command `intercept` answers — the one table the `/` menu and
/// `/help` are both drawn from. The menu offers the developer's five, in
/// this order (`crates/review/baseline.json`,
/// `frame-command-list-is-not-the-products`). `/exit` is `/quit` under the
/// name it always had, and is not listed twice.
const COMMANDS: [SlashCommand; 7] = [
    SlashCommand {
        name: "resume",
        argument: "",
        summary: "Pick up an earlier conversation",
        in_menu: true,
    },
    SlashCommand {
        name: "model",
        argument: "",
        summary: "Change the model",
        in_menu: true,
    },
    SlashCommand {
        name: "quit",
        argument: "",
        summary: "Leave Aldwin",
        in_menu: true,
    },
    SlashCommand {
        name: "clear",
        argument: "",
        summary: "Start a fresh conversation in this project",
        in_menu: true,
    },
    SlashCommand {
        name: "theme",
        argument: "",
        summary: "Switch between the light and dark theme",
        in_menu: true,
    },
    SlashCommand {
        name: "reload-config",
        argument: "",
        summary: "Read the settings files again",
        in_menu: false,
    },
    SlashCommand {
        name: "help",
        argument: "",
        summary: "List these commands",
        in_menu: false,
    },
];

/// The `/` menu's rows, for aldwin-tui.
pub(crate) fn menu() -> Vec<aldwin_tui::CommandChoice> {
    COMMANDS
        .iter()
        .filter(|c| c.in_menu)
        .map(|c| aldwin_tui::CommandChoice {
            name: c.name.into(),
            summary: c.summary.into(),
        })
        .collect()
}

/// What `/help` says: every command, the menu's first.
fn help_text() -> String {
    let commands: Vec<String> = COMMANDS
        .iter()
        .map(|c| format!("/{}{} — {}", c.name, c.argument, c.summary.to_lowercase()))
        .collect();
    format!("The commands are {}.", commands.join(" · "))
}

/// What `/clear` and `/resume` say while a turn runs — see
/// `History::turn_in_flight`. `esc` because it is the key the footer offers
/// for stopping.
const TURN_IN_FLIGHT: &str = "A turn is running. Stop it with esc first, then try again.";

/// How to resume, quoted by the branches that cannot act — same "never
/// make them go and find /help" rule as `MODEL_USAGE`.
const RESUME_USAGE: &str = "Type /resume on its own to pick a conversation from a list.";

/// How to change the model, quoted by every branch that rejects an
/// argument so the developer never has to go and find `/help`.
const MODEL_USAGE: &str = "Change it with /model provider/model.";

/// Valid `/theme` argument values — kept as the single source of truth for
/// both the accept-check and the error message's own listing, so the two
/// can't drift apart.
const VALID_THEMES: [&str; 2] = ["dark", "light"];

/// How `/model` moves a *running* session onto another model.
///
/// `Agent<C, D>` takes its client by value for the life of the process, and
/// core is generic over `C: LlmClient` — it has no notion of a provider, let
/// alone of one being replaced. So the client the agent was handed is a
/// holder that can be rebuilt behind the trait, and this is the one thing
/// the interceptor needs to know about it (see `bootstrap::ClientHandle`).
///
/// Rebuilding can fail the same way startup can — the new provider's
/// `api_key_env` may not be exported — which is why this returns a result
/// rather than swapping blind. A failed swap leaves the session on the
/// client it already had.
pub trait ModelSwitch: Send + Sync {
    /// `config` is the provider settings in force — the file being written
    /// over any layer below it (`ProviderConfig::over`).
    fn switch(
        &self,
        config: &aldwin_config::ProviderConfig,
    ) -> Result<(), aldwin_llm::LlmClientInitError>;
}

/// The running session, as `/model` has to see it: what it is on right now,
/// and how to move it.
///
/// The two belong together because they change together: a successful swap
/// is precisely what makes `model` stale, so whatever performs the swap has
/// to be holding the field it invalidates.
pub struct Session {
    /// `provider/model`, as [`qualified`] renders it — what the next turn
    /// will actually run on.
    model: String,
    switch: Box<dyn ModelSwitch>,
    /// Run after a successful `/reload-config`, for state that does not share
    /// the `Config` handle and so does not see the reload on its own — today,
    /// the workspace roots (ADR 0007). Returns a line for the developer, or
    /// `None` when there is nothing to say.
    after_reload: Option<AfterReload>,
}

pub type AfterReload = Box<dyn Fn() -> Option<String> + Send + Sync>;

impl Session {
    pub fn new(model: String, switch: Box<dyn ModelSwitch>) -> Self {
        Self {
            model,
            switch,
            after_reload: None,
        }
    }

    pub fn with_after_reload(mut self, hook: AfterReload) -> Self {
        self.after_reload = Some(hook);
        self
    }
}

/// Intercepts `/`-prefixed `Submit` input before it would otherwise reach
/// the core, per aldwin-cli.md: core has no slash-command semantics, and a
/// `/` command reaches it only as the `Command` it translates to (`/clear`
/// as `ClearHistory`, `/resume` as `Resume`). Runs synchronously in
/// the interceptor's own recv loop (`run_interceptor`), before any forward
/// send — not a post-send hook, per the spec's explicit Pitfall.
async fn intercept(
    command: Command,
    config: &Config,
    session: &mut Session,
    history: Option<&Arc<History>>,
    events: &mpsc::Sender<Event>,
) -> Intercepted {
    let Command::Submit { text } = &command else {
        return Intercepted::Forward(command);
    };
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        if let Some(history) = history {
            history.turn_submitted();
        }
        return Intercepted::Forward(command);
    };

    // The command name, and whatever follows it. Only the commands that take
    // an argument match `Some`, so `/help me` is as unknown as `/nope`.
    let rest = rest.trim();
    let (name, arg) = match rest.split_once(char::is_whitespace) {
        Some((name, arg)) => (name, Some(arg.trim())),
        None => (rest, None),
    };
    match (name, arg) {
        ("help", None) => {
            let _ = events
                .send(Event::Notice {
                    message: help_text(),
                })
                .await;
            Intercepted::Handled
        }
        ("reload-config", None) => {
            handle_reload_config(config, session, events).await;
            Intercepted::Handled
        }
        // Unlike /help and /reload-config, this one core needs to act on
        // (wipe ConversationLog) — so it's translated and forwarded rather
        // than handled locally; core acknowledges with Event::HistoryCleared
        // once done, which is what actually tells the TUI to wipe its own
        // rendered log (see aldwin_tui::App::apply_event).
        //
        // The transcript is sealed when core acts, not here: core tells its
        // sink (`History`), and refuses the command while a turn runs.
        // "Forget everything" is about the model's context, and the record
        // of what was said stays on disk and stays resumable.
        ("clear", None) => {
            if history.is_some_and(|h| h.turn_in_flight()) {
                let _ = events
                    .send(Event::Notice {
                        message: TURN_IN_FLIGHT.into(),
                    })
                    .await;
                return Intercepted::Handled;
            }
            Intercepted::Forward(Command::ClearHistory)
        }
        // `/quit` is the menu's word; `/exit` the one the interceptor has
        // always known. Both leave.
        ("exit", None) | ("quit", None) => Intercepted::Quit,
        // Bare `/model` reports where the developer stands; an argument
        // changes it. Bare `/theme` switches, since that is what picking it
        // from the menu asks for.
        ("theme", arg) => {
            handle_theme(arg, config, events).await;
            Intercepted::Handled
        }
        ("model", arg) => {
            handle_model(arg, config, session, events).await;
            Intercepted::Handled
        }
        // Bare `/resume` normally never reaches here: aldwin-tui reads it
        // first and opens the picker, which answers by submitting
        // `/resume <id>`. What arrives is the bare form on a session with no
        // list to show — so `handle_resume` says why.
        ("resume", arg) => match handle_resume(arg, history, events).await {
            Some(command) => Intercepted::Forward(command),
            None => Intercepted::Handled,
        },
        _ => {
            let _ = events
                .send(Event::Notice {
                    message: format!("There is no /{rest} command. /help lists them all."),
                })
                .await;
            Intercepted::Handled
        }
    }
}

/// `/resume [id]` — load a past transcript into the running session.
///
/// Returns the command core needs rather than sending it, because the caller
/// is the one holding `forward`; everything else here is reporting.
///
/// The records are read here, because a read that fails must change
/// nothing. Core is handed them with the session's id; when it takes them it
/// moves its sink (`History`) onto that transcript, so the continued
/// conversation lands in the file it came from rather than forking a new
/// one, and its acknowledgement (`Event::HistoryLoaded`) is what the TUI
/// redraws from.
///
/// What is deliberately *not* restored: anything staged. A resumed session
/// starts with an empty changeset; the review it left open is gone.
async fn handle_resume(
    arg: Option<&str>,
    history: Option<&Arc<History>>,
    events: &mpsc::Sender<Event>,
) -> Option<Command> {
    let Some(history) = history else {
        let _ = events
            .send(Event::Notice {
                message: "This session is not being recorded, so there is nothing to resume."
                    .into(),
            })
            .await;
        return None;
    };

    let Some(arg) = arg.filter(|a| !a.is_empty()) else {
        let count = history.resumable().len();
        let message = match count {
            0 => "There are no earlier conversations in this project to resume.".to_string(),
            1 => format!("There is 1 earlier conversation here. {RESUME_USAGE}"),
            n => format!("There are {n} earlier conversations here. {RESUME_USAGE}"),
        };
        let _ = events.send(Event::Notice { message }).await;
        return None;
    };

    if history.turn_in_flight() {
        let _ = events
            .send(Event::Notice {
                message: TURN_IN_FLIGHT.into(),
            })
            .await;
        return None;
    }

    let id = SessionId(arg.to_string());
    // A session cannot be resumed into itself: the records would be replaced
    // by the ones already in the log, and the writer would be pointed at the
    // file it is already writing.
    if history.is_current(&id) {
        let _ = events
            .send(Event::Notice {
                message: "That is the conversation you are in.".into(),
            })
            .await;
        return None;
    }

    let records = match aldwin_config::load_session(history.dir(), &id) {
        Ok(records) => records,
        Err(e) => {
            let _ = events
                .send(Event::Notice {
                    message: format!("That conversation could not be read: {e}. {RESUME_USAGE}"),
                })
                .await;
            return None;
        }
    };

    // An empty load is not a failure: it is a transcript whose first turn
    // never finished (see `aldwin_config::load`). Resuming it would replace
    // the session with nothing, which is `/clear` wearing a disguise.
    if records.is_empty() {
        let _ = events
            .send(Event::Notice {
                message: "That conversation has no finished turn to resume.".into(),
            })
            .await;
        return None;
    }

    // Core says "Resumed" when it acts on this: a turn can still be running
    // when it arrives, and then nothing is resumed.
    Some(Command::Resume {
        session: id,
        records,
    })
}

/// `/theme [light|dark]` — bare, the other theme from the one in effect;
/// with an argument, that one. Unlike `/clear`, this never needs core at all —
/// it's a config write (`Config::set_tui`, persisting the choice so it
/// survives the developer's next launch, not just this session) plus an
/// `Event::ThemeChanged` sent directly into the same channel the TUI reads
/// from (see that event's own doc comment in aldwin-core for why the
/// interceptor can reach the TUI this way without core's involvement).
/// `App::theme` is read fresh by `ui::draw` on every frame, so the change
/// is visible on the very next redraw — no restart needed.
async fn handle_theme(arg: Option<&str>, config: &Config, events: &mpsc::Sender<Event>) {
    let normalized = match arg {
        Some(arg) => arg.to_ascii_lowercase(),
        // Read the way startup reads it, so the switch is from what is on
        // screen.
        None => match aldwin_tui::Theme::from_config(config.global_tui().theme.as_deref()) {
            aldwin_tui::Theme::Light => "dark".into(),
            aldwin_tui::Theme::Dark => "light".into(),
        },
    };
    if !VALID_THEMES.contains(&normalized.as_str()) {
        let _ = events
            .send(Event::Notice {
                message: format!(
                    "There is no {normalized} theme. Use /theme light or /theme dark."
                ),
            })
            .await;
        return;
    }

    let mut tui = config.global_tui();
    tui.theme = Some(normalized.clone());
    match config.set_tui(tui) {
        Ok(()) => {
            let _ = events
                .send(Event::Notice {
                    message: format!("The theme is now {normalized}."),
                })
                .await;
            let _ = events.send(Event::ThemeChanged { theme: normalized }).await;
        }
        Err(e) => {
            let _ = events
                .send(Event::Notice {
                    message: format!("The theme could not be saved: {e}."),
                })
                .await;
        }
    }
}

/// `/model [provider/]model` — the one way a session gets or changes its
/// model. It picks both halves of "where the model runs, and which one",
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
/// model id and so does this. `aldwin_llm::PROVIDERS`' model lists are
/// suggestions, and the notice says so by listing them as "known".
///
/// **The bare form opens a list.** aldwin-tui reads `/model` with no
/// argument before it reaches here and opens the picker, which answers by
/// submitting `/model <provider>/<model>` — this function still does every
/// write, and still decides which scope it lands in. What reaches the branch
/// below is the bare form on a session with no catalogue to show, which
/// reports where the developer stands instead.
///
/// **It takes effect now.** The client is rebuilt on the new provider
/// *before* anything is written, and the session moves onto it — the same
/// way `/theme` really does apply on the next redraw. Two things follow from
/// that order. A provider whose `api_key_env` is not exported fails here
/// rather than at the developer's next start, and nothing is persisted when
/// it does: a `provider.yaml` that cannot boot is not an improvement on
/// being told no. And the notice names what the *next turn* will run on,
/// which is now the same thing the top bar and status line show — they read
/// `Event::ModelChanged`, sent below.
async fn handle_model(
    arg: Option<&str>,
    config: &Config,
    session: &mut Session,
    events: &mpsc::Sender<Event>,
) {
    // Whichever scope actually supplies the setting is the one that gets
    // written: writing global while a project `provider.yaml` shadows it
    // would report a change the next start would ignore.
    //
    // The global layer is kept even when the project one shadows it, because
    // the settings the new client is built from overlay the two —
    // `base_url` and `extended_thinking_budget` fall back to global (see
    // `ProviderConfig::over`), so building from the project file alone would
    // hand the session a client the next start would not reproduce.
    // Nothing configured is a state the session can be in now (ADR 0009
    // §6): the answer then lands in the global file, since there is no
    // other default for every other directory to inherit.
    let global = config.global_provider().ok();
    let (scope, current) = match config.project_provider() {
        Some(project) => (aldwin_config::Scope::Project, Some(project)),
        None => (aldwin_config::Scope::Global, global.clone()),
    };
    let known = current.as_ref().and_then(identify);

    let Some(arg) = arg.filter(|a| !a.is_empty()) else {
        let message = match &current {
            Some(current) => describe(current, known),
            None => format!(
                "No provider is configured yet. Pick one with /model provider/model. The providers are {}.",
                aldwin_llm::provider_ids().join(", ")
            ),
        };
        let _ = events.send(Event::Notice { message }).await;
        return;
    };

    // Split on the first `/` only, and a bare provider name means the same
    // as `provider/` — see this function's own doc comment for both rules.
    let (head, tail) = arg
        .split_once('/')
        .map_or((arg, ""), |(head, tail)| (head, tail.trim()));
    let next = match named_provider(head) {
        Some(p) => {
            // A leading `/` cannot reach here (`head` would be empty and name
            // nothing); a trailing one, or none at all, is an empty model
            // half. Naming the provider you are already on is then not a
            // request to be moved off the model you are using — without this
            // `/model anthropic` on `anthropic/claude-opus-5` would quietly
            // drop you back to the catalogue's default.
            let model = match tail {
                "" if known.map(|c| c.id) == Some(p.id) => {
                    current.as_ref().map(|c| c.model.as_str())
                }
                "" => None,
                model => Some(model),
            };
            catalogue_provider_config(p, model, current.as_ref())
        }
        None if arg.contains('/') => {
            let ids = aldwin_llm::provider_ids().join(", ");
            let message = format!(
                "There is no provider called {head}. The providers are {ids}. {MODEL_USAGE}"
            );
            let _ = events.send(Event::Notice { message }).await;
            return;
        }
        None => match &current {
            Some(current) => aldwin_config::ProviderConfig {
                version: aldwin_config::PROVIDER_VERSION,
                model: arg.to_string(),
                ..current.clone()
            },
            None => {
                let ids = aldwin_llm::provider_ids().join(", ");
                let message = format!("No provider is configured, so say which one runs {arg}: /model provider/{arg}. The providers are {ids}.");
                let _ = events.send(Event::Notice { message }).await;
                return;
            }
        },
    };

    let now = qualified(&next, identify(&next));

    // "Already on" has to be true of the *session*, not only of the file.
    // The two can disagree — a hand-edited `provider.yaml` picked up by
    // `/reload-config` moves what is on disk without touching the client the
    // session holds — and reporting no change while the session runs
    // something else is exactly the "says the model is already selected when
    // it isn't" this command was fixed for once already. When they disagree
    // this falls through and swaps, which is what the developer asked for.
    if current.as_ref() == Some(&next) && now == session.model {
        // Never a dead end. Naming the provider you are already on is the
        // most likely way to reach this branch, and it is what a developer
        // types when they are reaching for a list of models — so the notice
        // says where the list is rather than stopping at "already on".
        let message = format!("You are already on {now}. /model on its own opens the list.");
        let _ = events.send(Event::Notice { message }).await;
        return;
    }

    // What the session would actually run on, overlaid the same way startup
    // overlays it (`Config::effective_provider`) — the file just chosen over
    // the layer below it.
    let below = match scope {
        aldwin_config::Scope::Project => global.as_ref(),
        aldwin_config::Scope::Global => None,
    };
    let resolved = next.clone().over(below);

    // Before the write, not after: a provider the session cannot actually
    // reach must not be left on disk for the next start to fail on.
    if let Err(e) = session.switch.switch(&resolved) {
        let message = format!(
            "Could not switch to {now}: {e}. You are still on {}, and nothing was saved.",
            session.model
        );
        let _ = events.send(Event::Notice { message }).await;
        return;
    }

    let where_ = match scope {
        aldwin_config::Scope::Project => "this project's provider.yaml",
        aldwin_config::Scope::Global => "the global provider.yaml",
    };
    let message = match config.set_provider(scope, next.clone()) {
        Ok(()) => format!("Now on {now}, saved to {where_}."),
        // The swap already happened, so the session really is on the new
        // model — it is only the next start that will not be.
        Err(e) => format!(
            "Now on {now}, but it could not be saved to {where_}: {e}. The next start will use {}.",
            current
                .as_ref()
                .map_or_else(|| "nothing".to_string(), |c| qualified(c, identify(c)))
        ),
    };
    let _ = events.send(Event::Notice { message }).await;
    session.model = now;
    // The bare model id, not the qualified name: it is what the session
    // started with in `StatusInfo::model_name`, and the picker matches the
    // provider half against catalogue ids separately.
    let identified = identify(&next);
    let context_window = identified
        .and_then(|p| p.models.iter().find(|m| m.id == next.model))
        .map(|m| m.context);
    let changed = Event::ModelChanged {
        provider: identified.map(|p| p.id.to_string()),
        model: next.model.clone(),
        context_window,
    };
    let _ = events.send(changed).await;
}

/// The catalogue row `name` names, case-insensitively.
///
/// Only the *provider* half is folded: catalogue ids are lowercase by
/// construction (a test pins it) and `/theme` already accepts `LIGHT`, so
/// rejecting `/model Anthropic` would be the odd one out. Model ids are left
/// exactly as typed — they are opaque strings a host compares byte for byte,
/// and some really are mixed-case.
fn named_provider(name: &str) -> Option<&'static aldwin_llm::Provider> {
    aldwin_llm::provider(&name.trim().to_ascii_lowercase())
}

/// The `provider.yaml` that naming catalogue row `provider` writes — by the
/// provider question and by `/model` alike: everything but the model comes
/// straight off the row, and the model is that provider's own default when
/// none was given.
///
/// `provider.yaml` deliberately has no field a plaintext key could go in
/// (see `ProviderConfig`), so this writes the key variable's *name* and the
/// developer exports the key themselves.
///
/// `current` is whatever already supplies the setting, when anything does.
/// Two fields come from it rather than from the catalogue row:
///
/// * the thinking budget, always — a developer's preference, not the
///   host's, so it survives a move between providers;
/// * the key variable, but only when `provider` is the one already
///   configured. A developer who exports their Anthropic key as
///   `ANTHROPIC_KEY_WORK` has said so in `provider.yaml`, and changing the
///   *model* on that provider is not a request to be moved back onto the
///   catalogue's default variable name — which would break their next
///   start. Naming a different provider is a different endpoint with a
///   different key, so there the catalogue's variable is the right one.
///
/// Carrying both is also what makes an unchanged answer compare equal to
/// what is on disk, so confirming the current row writes nothing at all.
pub(crate) fn catalogue_provider_config(
    provider: &aldwin_llm::Provider,
    model: Option<&str>,
    current: Option<&aldwin_config::ProviderConfig>,
) -> aldwin_config::ProviderConfig {
    let on_this_provider = current.filter(|c| identify(c).map(|p| p.id) == Some(provider.id));
    aldwin_config::ProviderConfig {
        version: aldwin_config::PROVIDER_VERSION,
        provider: provider.kind,
        model: model
            .unwrap_or_else(|| provider.default_model())
            .to_string(),
        base_url: provider.base_url.map(String::from),
        api_key_env: on_this_provider.map_or_else(
            || provider.api_key_env.to_string(),
            |c| c.api_key_env.clone(),
        ),
        extended_thinking_budget: current.and_then(|c| c.extended_thinking_budget),
    }
}

/// The catalogue row a `provider.yaml` points at — see
/// [`aldwin_llm::identify`].
pub(crate) fn identify(
    config: &aldwin_config::ProviderConfig,
) -> Option<&'static aldwin_llm::Provider> {
    aldwin_llm::identify(config.provider, config.base_url.as_deref())
}

/// `provider/model` when the endpoint is one the catalogue knows, and the
/// bare model id when the developer has pointed `provider.yaml` at an
/// endpoint of their own — naming a provider there would be a guess.
pub(crate) fn qualified(
    config: &aldwin_config::ProviderConfig,
    known: Option<&aldwin_llm::Provider>,
) -> String {
    match known {
        Some(p) => format!("{}/{}", p.id, config.model),
        None => config.model.clone(),
    }
}

/// What the bare `/model` reports: where the developer stands, what else
/// that provider offers, and every provider there is.
fn describe(
    current: &aldwin_config::ProviderConfig,
    known: Option<&aldwin_llm::Provider>,
) -> String {
    let mut out = format!("You are on {}.", qualified(current, known));
    if let Some(p) = known {
        let others: Vec<&str> = p
            .models
            .iter()
            .map(|m| m.id)
            .filter(|id| *id != current.model)
            .collect();
        if !others.is_empty() {
            out.push_str(&format!(" Other {} models: {}.", p.id, others.join(", ")));
        }
    } else {
        // An endpoint the catalogue has never seen — say so rather than
        // silently reporting a bare model id as though it were the whole
        // answer.
        out.push_str(&format!(
            " It runs at {}.",
            current
                .base_url
                .as_deref()
                .unwrap_or("the provider's default endpoint")
        ));
    }
    out.push_str(&format!(
        " The providers are {}.",
        aldwin_llm::provider_ids().join(", ")
    ));
    out.push_str(&format!(" {MODEL_USAGE}"));
    out
}

async fn handle_reload_config(config: &Config, session: &Session, events: &mpsc::Sender<Event>) {
    match config.reload_all() {
        Ok(()) => {
            let _ = events
                .send(Event::Notice {
                    message: "Settings reloaded.".into(),
                })
                .await;
            // The permissions header promises an edit to the file is picked
            // up here. `roots:` was the one key for which that was not true.
            if let Some(message) = session.after_reload.as_ref().and_then(|hook| hook()) {
                let _ = events.send(Event::Notice { message }).await;
            }
        }
        Err(failures) => {
            let detail = failures
                .iter()
                .map(|f| format!("{}: {}", f.path.display(), f.error))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = events
                .send(Event::Notice {
                    message: format!(
                        "The settings could not be reloaded, so nothing changed: {detail}."
                    ),
                })
                .await;
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
    mut session: Session,
    history: Option<Arc<History>>,
    events: mpsc::Sender<Event>,
) {
    while let Some(command) = incoming.recv().await {
        match intercept(command, &config, &mut session, history.as_ref(), &events).await {
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

    use aldwin_core::RecordSink;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// What the process booted on — where every test's session starts.
    const SESSION_MODEL: &str = "anthropic/claude-sonnet-5";

    /// A `ModelSwitch` that records what it was asked to build rather than
    /// building it: no key variable to export, no HTTP client, and the
    /// resolved configs available to assert on afterwards.
    #[derive(Clone, Default)]
    struct FakeSwitch {
        seen: Arc<Mutex<Vec<aldwin_config::ProviderConfig>>>,
        /// Set to stand in for the one failure a real swap has: a provider
        /// whose `api_key_env` — this variable — is not exported.
        fails_with: Option<&'static str>,
    }

    impl ModelSwitch for FakeSwitch {
        fn switch(
            &self,
            config: &aldwin_config::ProviderConfig,
        ) -> Result<(), aldwin_llm::LlmClientInitError> {
            if let Some(var) = self.fails_with {
                return Err(aldwin_llm::LlmClientInitError::MissingApiKeyEnv { var: var.into() });
            }
            self.seen.lock().unwrap().push(config.clone());
            Ok(())
        }
    }

    fn session() -> Session {
        Session::new(SESSION_MODEL.into(), Box::new(FakeSwitch::default()))
    }

    /// A session whose switch records, and the record itself — for the tests
    /// that care about what the client was actually rebuilt on.
    fn recording_session() -> (Session, Arc<Mutex<Vec<aldwin_config::ProviderConfig>>>) {
        let switch = FakeSwitch::default();
        let seen = switch.seen.clone();
        (Session::new(SESSION_MODEL.into(), Box::new(switch)), seen)
    }

    fn config() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        (project, global, config)
    }

    // ── `/resume` ─────────────────────────────────────────────────────────

    /// A history with one finished turn already recorded, plus both ends of
    /// the channel its notices arrive on — the sender is the one the history
    /// reports failures on, so a test reads notices from both paths in one
    /// place.
    fn recorded_history(
        text: &str,
    ) -> (
        tempfile::TempDir,
        Arc<History>,
        SessionId,
        mpsc::Sender<Event>,
        mpsc::Receiver<Event>,
    ) {
        use aldwin_core::{LogRecord, TurnEndReason, TurnId};

        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel(8);
        let history = History::open(
            dir.path().to_path_buf(),
            Path::new("/p"),
            "m".into(),
            tx.clone(),
        )
        .expect("a store");
        for record in [
            LogRecord::TurnStarted { turn_id: TurnId(1) },
            LogRecord::UserMessage {
                turn_id: TurnId(1),
                text: text.into(),
            },
            LogRecord::AssistantMessage {
                turn_id: TurnId(1),
                step_id: aldwin_core::StepId(1),
                text: "sure".into(),
            },
            LogRecord::TurnEnded {
                turn_id: TurnId(1),
                reason: TurnEndReason::EndTurn,
            },
        ] {
            aldwin_core::RecordSink::append(history.as_ref(), &record);
        }
        // The session under test is a *new* one, as a fresh launch would be:
        // the recorded turn is now a past session to resume.
        history.cleared();
        // The recorded one, not whichever is newest — `cleared`
        // just made a newer, empty one.
        let id = SessionId(
            history
                .resumable()
                .into_iter()
                .find(|s| s.turns > 0)
                .expect("the recorded session")
                .id,
        );
        (dir, history, id, tx, rx)
    }

    /// Every notice already waiting on `rx`.
    fn drain(rx: &mut mpsc::Receiver<Event>) -> Vec<String> {
        std::iter::from_fn(|| match rx.try_recv() {
            Ok(Event::Notice { message }) => Some(message),
            _ => None,
        })
        .collect()
    }

    #[tokio::test]
    async fn resume_with_an_id_forwards_the_loaded_records_to_core() {
        let (_project, _global, cfg) = config();
        let (_dir, history, id, events, _rx) = recorded_history("the question I asked");

        let cmd = Command::Submit {
            text: format!("/resume {id}"),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &events).await;

        let Intercepted::Forward(Command::Resume { session, records }) = result else {
            panic!("a resume must reach core");
        };
        assert_eq!(session, id, "with the session it continues");
        assert!(
            records.iter().any(|r| matches!(r, aldwin_core::LogRecord::UserMessage { text, .. } if text == "the question I asked")),
            "the conversation came back"
        );
    }

    /// The fork-free Decision: the writer moves onto the resumed transcript,
    /// so the continued conversation lands in the file it came from. It
    /// moves when core takes the records and tells its sink — the
    /// interceptor only hands them over, since core may yet refuse them.
    #[tokio::test]
    async fn resuming_moves_the_writer_onto_the_resumed_transcript() {
        use aldwin_core::{LogRecord, TurnEndReason, TurnId};

        let (_project, _global, cfg) = config();
        let (dir, history, id, events, _rx) = recorded_history("first");
        let writing = history.current();

        let cmd = Command::Submit {
            text: format!("/resume {id}"),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &events).await;
        let Intercepted::Forward(Command::Resume { session, .. }) = result else {
            panic!("a resume must reach core");
        };
        assert!(
            history.is_current(&writing),
            "nothing moves before core acts"
        );
        history.resumed(&session); // what core does when it takes the records

        for record in [
            LogRecord::TurnStarted { turn_id: TurnId(2) },
            LogRecord::TurnEnded {
                turn_id: TurnId(2),
                reason: TurnEndReason::EndTurn,
            },
        ] {
            aldwin_core::RecordSink::append(history.as_ref(), &record);
        }

        let resumed = crate::history::session_choices(dir.path())
            .into_iter()
            .find(|s| s.id == id.0)
            .expect("still listed");
        assert_eq!(
            resumed.turns, 2,
            "the new turn landed in the resumed file, not a fork"
        );
    }

    #[tokio::test]
    async fn resuming_an_unknown_session_reports_and_sends_nothing() {
        let (_project, _global, cfg) = config();
        let (_dir, history, _id, events, mut rx) = recorded_history("first");

        let cmd = Command::Submit {
            text: "/resume 0000000000-0-0".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &events).await;

        assert!(
            matches!(result, Intercepted::Handled),
            "nothing reaches core"
        );
        let notices = drain(&mut rx);
        assert!(
            notices.iter().any(|m| m.contains("could not be read")),
            "{notices:?}"
        );
    }

    /// A transcript whose first turn never finished loads as nothing, and
    /// resuming it would be `/clear` wearing a disguise. The picker no longer
    /// offers such a session at all, so this is the typed-id path.
    #[tokio::test]
    async fn resuming_a_session_with_no_finished_turn_reports_rather_than_wiping() {
        use aldwin_core::{LogRecord, TurnId};

        let (_project, _global, cfg) = config();
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        // Another process's transcript, as a crash leaves it. Its id is taken
        // from the handle: an unfinished session is not listed, which is the
        // point of `an_unfinished_session_is_not_listed` below.
        let crashed = History::open(
            dir.path().to_path_buf(),
            Path::new("/p"),
            "m".into(),
            tx.clone(),
        )
        .expect("a store");
        aldwin_core::RecordSink::append(
            crashed.as_ref(),
            &LogRecord::TurnStarted { turn_id: TurnId(1) },
        );
        let id = crashed.current();
        let history = History::open(
            dir.path().to_path_buf(),
            Path::new("/p"),
            "m".into(),
            tx.clone(),
        )
        .expect("a store");

        let cmd = Command::Submit {
            text: format!("/resume {id}"),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &tx).await;

        assert!(matches!(result, Intercepted::Handled));
        let notices = drain(&mut rx);
        assert!(
            notices.iter().any(|m| m.contains("no finished turn")),
            "{notices:?}"
        );
    }

    #[tokio::test]
    async fn bare_resume_with_no_history_says_so() {
        let (_project, _global, cfg) = config();
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let history = History::open(
            dir.path().to_path_buf(),
            Path::new("/p"),
            "m".into(),
            tx.clone(),
        )
        .expect("a store");

        let cmd = Command::Submit {
            text: "/resume".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &tx).await;

        assert!(matches!(result, Intercepted::Handled));
        let notices = drain(&mut rx);
        assert!(
            notices
                .iter()
                .any(|m| m.contains("no earlier conversations")),
            "{notices:?}"
        );
    }

    /// History off entirely — the session runs, and `/resume` says why it
    /// cannot help rather than failing.
    #[tokio::test]
    async fn resume_without_a_transcript_reports_that_history_is_off() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);

        let cmd = Command::Submit {
            text: "/resume anything".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), None, &tx).await;

        assert!(matches!(result, Intercepted::Handled));
        let notices = drain(&mut rx);
        assert!(
            notices.iter().any(|m| m.contains("not being recorded")),
            "{notices:?}"
        );
    }

    /// Step 6: `/clear` reaches core to wipe the in-memory log, and the
    /// transcript is sealed when core does — through its sink, not here.
    #[tokio::test]
    async fn clear_reaches_core_and_leaves_the_sealing_to_it() {
        let (_project, _global, cfg) = config();
        let (dir, history, _id, events, _rx) = recorded_history("before");
        let before = crate::history::session_choices(dir.path()).len();
        let writing = history.current();

        let result = intercept(
            Command::Submit {
                text: "/clear".into(),
            },
            &cfg,
            &mut session(),
            Some(&history),
            &events,
        )
        .await;

        assert!(
            matches!(result, Intercepted::Forward(Command::ClearHistory)),
            "core still wipes its log"
        );
        assert!(
            history.is_current(&writing),
            "the writer moves when core acts, not on the way past"
        );
        history.cleared(); // what core does when it wipes the log
        assert!(!history.is_current(&writing));
        assert!(
            crate::history::session_choices(dir.path()).len() >= before,
            "clearing does not delete the record it seals"
        );
    }

    /// Core discards both mid-turn, so a writer moved on the way past would
    /// send the rest of the running conversation into another session's file.
    #[tokio::test]
    async fn clear_and_resume_leave_the_writer_alone_while_a_turn_runs() {
        let (_project, _global, cfg) = config();
        let (_dir, history, id, events, mut rx) = recorded_history("first");
        let writing = history.current();

        // Submitted, and core has logged nothing yet.
        let sent = intercept(
            Command::Submit {
                text: "hello".into(),
            },
            &cfg,
            &mut session(),
            Some(&history),
            &events,
        )
        .await;
        assert!(matches!(sent, Intercepted::Forward(Command::Submit { .. })));

        for text in ["/clear".to_string(), format!("/resume {id}")] {
            let result = intercept(
                Command::Submit { text: text.clone() },
                &cfg,
                &mut session(),
                Some(&history),
                &events,
            )
            .await;
            assert!(
                matches!(result, Intercepted::Handled),
                "{text} must not reach core mid-turn"
            );
            assert_eq!(drain(&mut rx), [TURN_IN_FLIGHT]);
            assert!(
                history.is_current(&writing),
                "{text} moved the writer under a running turn"
            );
        }

        let ended = aldwin_core::LogRecord::TurnEnded {
            turn_id: aldwin_core::TurnId(9),
            reason: aldwin_core::TurnEndReason::EndTurn,
        };
        aldwin_core::RecordSink::append(history.as_ref(), &ended);
        let result = intercept(
            Command::Submit {
                text: "/clear".into(),
            },
            &cfg,
            &mut session(),
            Some(&history),
            &events,
        )
        .await;
        assert!(
            matches!(result, Intercepted::Forward(Command::ClearHistory)),
            "and works again once the turn has ended"
        );
    }

    /// The picker never offers it, but a typed id can still name it.
    #[tokio::test]
    async fn resuming_the_session_you_are_in_is_refused() {
        let (_project, _global, cfg) = config();
        let (_dir, history, _id, events, mut rx) = recorded_history("first");
        let current = history.current();

        let cmd = Command::Submit {
            text: format!("/resume {current}"),
        };
        let result = intercept(cmd, &cfg, &mut session(), Some(&history), &events).await;

        assert!(matches!(result, Intercepted::Handled));
        let notices = drain(&mut rx);
        assert!(
            notices
                .iter()
                .any(|m| m.contains("the conversation you are in")),
            "{notices:?}"
        );
    }

    /// The session being written is not a row in its own list.
    #[tokio::test]
    async fn the_current_session_is_not_offered_as_resumable() {
        let (_dir, history, _id, _events, _rx) = recorded_history("first");
        let current = history.current();
        assert!(
            !history.resumable().iter().any(|s| s.id == current.0),
            "the session you are in is not something to resume"
        );
    }

    /// The end-to-end shape of the bug this fixed: launching and quitting
    /// without saying anything left a row in the picker that, when picked,
    /// was refused.
    #[tokio::test]
    async fn an_unfinished_session_is_not_listed() {
        let (_dir, history, _id, _events, _rx) = recorded_history("said something");
        // `recorded_history` sealed and opened a fresh session that has said
        // nothing — exactly the state a launch-and-quit leaves.
        let sessions = history.resumable();
        assert_eq!(
            sessions.len(),
            1,
            "only the session with a completed turn: {sessions:?}"
        );
        assert_eq!(sessions[0].title, "said something");
    }

    #[test]
    fn the_menu_and_help_are_drawn_from_one_table() {
        let menu: Vec<String> = menu().into_iter().map(|c| c.name).collect();
        assert_eq!(
            menu,
            ["resume", "model", "quit", "clear", "theme"],
            "the developer's five, in order"
        );
        let help = help_text();
        for command in COMMANDS {
            assert!(
                help.contains(&format!("/{}", command.name)),
                "{} missing from help",
                command.name
            );
        }
    }

    #[tokio::test]
    async fn non_slash_input_passes_through_unchanged() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let cmd = Command::Submit {
            text: "hello".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), None, &tx).await;
        assert!(
            matches!(result, Intercepted::Forward(Command::Submit { text }) if text == "hello")
        );
    }

    #[tokio::test]
    async fn non_submit_commands_pass_through_unchanged() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let cmd = Command::Answer {
            call_id: "call-1".into(),
            answer: aldwin_core::Answer::Chose { index: 0 },
        };
        let result = intercept(cmd, &cfg, &mut session(), None, &tx).await;
        assert!(matches!(
            result,
            Intercepted::Forward(Command::Answer { .. })
        ));
    }

    #[tokio::test]
    async fn unknown_slash_command_is_rejected_and_never_forwarded() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit {
            text: "/nope".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), None, &tx).await;
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
        let result = intercept(
            Command::Submit {
                text: "/help".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => {
                for command in [
                    "/help",
                    "/clear",
                    "/quit",
                    "/model",
                    "/reload-config",
                    "/theme",
                ] {
                    assert!(
                        message.contains(command),
                        "help text missing {command}: {message}"
                    );
                }
            }
            other => panic!("expected a Notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exit_is_recognised_as_the_quit_command() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let result = intercept(
            Command::Submit {
                text: "/exit".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(matches!(result, Intercepted::Quit));
    }

    #[tokio::test]
    async fn clear_is_translated_and_forwarded_to_core_not_handled_locally() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let result = intercept(
            Command::Submit {
                text: "/clear".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(
            matches!(result, Intercepted::Forward(Command::ClearHistory)),
            "core owns ConversationLog, so /clear must reach it as ClearHistory rather than being swallowed like /help"
        );
    }

    /// The menu says `/quit`; the interceptor always said `/exit`. Both leave.
    #[tokio::test]
    async fn quit_and_exit_both_leave() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        assert!(matches!(
            intercept(
                Command::Submit {
                    text: "/quit".into()
                },
                &cfg,
                &mut session(),
                None,
                &tx
            )
            .await,
            Intercepted::Quit
        ));
    }

    #[tokio::test]
    async fn reload_config_success_emits_a_notice() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit {
            text: "/reload-config".into(),
        };
        let result = intercept(cmd, &cfg, &mut session(), None, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        assert!(matches!(rx.recv().await, Some(Event::Notice { .. })));
        assert!(rx.try_recv().is_err(), "nothing else to say");
    }

    #[tokio::test]
    async fn reload_config_failure_surfaces_the_failing_path_verbatim() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        // Hand-corrupt the project's permissions.yaml so reload_all() fails
        // on that one layer.
        std::fs::create_dir(project.path().join(".aldwin")).unwrap();
        let bad_path = project.path().join(".aldwin").join("permissions.yaml");
        std::fs::write(&bad_path, "not: [valid, yaml: at all").unwrap();

        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit {
            text: "/reload-config".into(),
        };
        intercept(cmd, &config, &mut session(), None, &tx).await;

        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(
                message.contains(&bad_path.display().to_string()),
                "message was: {message}"
            ),
            other => panic!("expected a Notice, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn theme_with_no_argument_switches_to_the_other_and_persists_it() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        for expected in ["light", "dark"] {
            let result = intercept(
                Command::Submit {
                    text: "/theme".into(),
                },
                &cfg,
                &mut session(),
                None,
                &tx,
            )
            .await;
            assert!(matches!(result, Intercepted::Handled));
            assert!(notice(&mut rx)
                .await
                .contains(&format!("The theme is now {expected}")));
            match rx.recv().await {
                Some(Event::ThemeChanged { theme }) => assert_eq!(theme, expected),
                other => panic!("expected ThemeChanged, got {other:?}"),
            }
            assert_eq!(cfg.global_tui().theme.as_deref(), Some(expected));
        }
    }

    #[tokio::test]
    async fn theme_light_persists_and_emits_theme_changed() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(
            Command::Submit {
                text: "/theme light".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(matches!(result, Intercepted::Handled));

        assert!(matches!(rx.recv().await, Some(Event::Notice { .. })));
        match rx.recv().await {
            Some(Event::ThemeChanged { theme }) => assert_eq!(theme, "light"),
            other => panic!("expected ThemeChanged, got {other:?}"),
        }
        assert_eq!(
            cfg.global_tui().theme.as_deref(),
            Some("light"),
            "the choice must survive the next launch too, not just this session"
        );
    }

    #[tokio::test]
    async fn theme_argument_is_case_insensitive() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/theme LIGHT".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        let _ = rx.recv().await; // Notice
        match rx.recv().await {
            Some(Event::ThemeChanged { theme }) => {
                assert_eq!(theme, "light", "must normalize to lowercase")
            }
            other => panic!("expected ThemeChanged, got {other:?}"),
        }
    }

    /// Any whitespace separates a command from its argument, not only a
    /// space — a pasted tab used to make `/theme` an unknown command.
    #[tokio::test]
    async fn a_tab_separates_a_command_from_its_argument() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/theme\tlight".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(notice(&mut rx).await.contains("The theme is now light"));
    }

    #[tokio::test]
    async fn theme_back_to_dark_persists_and_emits_theme_changed() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/theme light".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        let _ = rx.recv().await;
        let _ = rx.recv().await;

        intercept(
            Command::Submit {
                text: "/theme dark".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
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
        let result = intercept(
            Command::Submit {
                text: "/theme neon".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => {
                assert!(message.contains("neon"), "message was: {message}")
            }
            other => panic!("expected a Notice, got {other:?}"),
        }
        assert!(
            rx.try_recv().is_err(),
            "an invalid theme must not emit ThemeChanged"
        );
        assert_eq!(
            cfg.global_tui().theme,
            None,
            "an invalid theme must not be persisted"
        );
    }

    /// Every `/model` test needs a provider already on disk — the session
    /// this command runs in cannot exist without one.
    fn with_provider(config: &Config, scope: aldwin_config::Scope, id: &str) {
        let p = aldwin_llm::provider(id).expect("a catalogue provider");
        config
            .set_provider(
                scope,
                aldwin_config::ProviderConfig {
                    version: aldwin_config::PROVIDER_VERSION,
                    provider: p.kind,
                    model: p.default_model().into(),
                    base_url: p.base_url.map(String::from),
                    api_key_env: p.api_key_env.into(),
                    extended_thinking_budget: None,
                },
            )
            .unwrap();
    }

    /// The next `Notice`, stepping over the `ModelChanged` a successful swap
    /// leaves behind — the tests that care about that event assert on it
    /// directly, and the rest are reading the message.
    async fn notice(rx: &mut mpsc::Receiver<Event>) -> String {
        loop {
            match rx.recv().await {
                Some(Event::Notice { message }) => return message,
                Some(Event::ModelChanged { .. }) => continue,
                other => panic!("expected a Notice, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn model_with_no_argument_reports_where_the_developer_stands() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(
            Command::Submit {
                text: "/model".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(matches!(result, Intercepted::Handled));

        let message = notice(&mut rx).await;
        assert!(
            message.contains("You are on anthropic/claude-sonnet-5"),
            "{message}"
        );
        assert!(
            message.contains("The providers are anthropic"),
            "the bare form has to say what it would accept: {message}"
        );
        assert!(message.contains(MODEL_USAGE), "{message}");
        assert!(
            rx.try_recv().is_err(),
            "no-argument /model must not persist anything"
        );
    }

    /// A bare model id keeps the provider — the common case, and the one
    /// where a provider name would be noise.
    #[tokio::test]
    async fn a_bare_model_id_changes_the_model_and_leaves_the_provider_alone() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model claude-opus-5".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(message.contains("anthropic/claude-opus-5"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.model, "claude-opus-5");
        assert_eq!(
            saved.provider,
            aldwin_config::ProviderKind::Anthropic,
            "the provider must be untouched"
        );
    }

    /// The change is what the session runs on from here — the client is
    /// rebuilt on it, and the bars are told so they stop naming the model
    /// the process happened to boot with.
    #[tokio::test]
    async fn changing_the_model_moves_the_running_session_onto_it() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (mut session, seen) = recording_session();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model claude-opus-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            message.contains("Now on anthropic/claude-opus-5"),
            "{message}"
        );
        assert!(
            !message.contains("Restart"),
            "nothing needs restarting any more: {message}"
        );

        let built = seen.lock().unwrap().clone();
        assert_eq!(built.len(), 1, "the client is rebuilt exactly once");
        assert_eq!(built[0].model, "claude-opus-5");
        assert_eq!(built[0].provider, aldwin_config::ProviderKind::Anthropic);

        match rx.recv().await {
            Some(Event::ModelChanged {
                provider,
                model,
                context_window,
            }) => {
                assert_eq!(
                    model, "claude-opus-5",
                    "the card shows the bare model id, as it did at startup"
                );
                assert_eq!(
                    provider.as_deref(),
                    Some("anthropic"),
                    "the question opens on the row it belongs to"
                );
                assert_eq!(
                    context_window,
                    Some(1_000_000),
                    "the context bar needs the window"
                );
            }
            other => panic!("expected ModelChanged, got {other:?}"),
        }
    }

    /// The one failure a swap has is the one startup has: the new provider's
    /// key variable is not exported. Nothing is written when it happens — a
    /// `provider.yaml` the next start cannot boot on is not an improvement
    /// on being told no.
    #[tokio::test]
    async fn a_client_that_cannot_be_built_leaves_the_session_and_the_file_alone() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let switch = FakeSwitch {
            fails_with: Some("GOOGLE_API_KEY"),
            ..Default::default()
        };
        let mut session = Session::new(SESSION_MODEL.into(), Box::new(switch));
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model google/gemini-2.5-flash".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            message.contains("GOOGLE_API_KEY"),
            "the reason has to survive verbatim: {message}"
        );
        assert!(
            message.contains("still on anthropic/claude-sonnet-5"),
            "{message}"
        );
        assert!(
            rx.try_recv().is_err(),
            "a failed swap must not tell the bars anything changed"
        );
        assert_eq!(
            cfg.global_provider().unwrap().model,
            "claude-sonnet-5",
            "nothing may be written on a failed swap"
        );
    }

    /// The developer's own model answer is what gets written — the
    /// provider's catalogue default is the fallback for the case the model
    /// step could not be asked at all, not the normal path.
    #[test]
    fn a_provider_row_writes_the_model_that_was_chosen() {
        let anthropic = aldwin_llm::provider("anthropic").expect("a catalogue provider");
        let chosen = catalogue_provider_config(anthropic, Some("claude-opus-5"), None);
        assert_eq!(chosen.model, "claude-opus-5");
        assert_eq!(
            chosen.api_key_env, anthropic.api_key_env,
            "the endpoint and key still come from the provider row"
        );

        let unasked = catalogue_provider_config(anthropic, None, None);
        assert_eq!(unasked.model, anthropic.default_model());
    }

    /// Confirming the lists on the rows they opened on is not a change, and
    /// must not leave a project file behind restating the global one.
    #[test]
    fn an_unchanged_answer_compares_equal_to_what_is_already_configured() {
        let google = aldwin_llm::provider("google").expect("a catalogue provider");
        let current = aldwin_config::ProviderConfig {
            extended_thinking_budget: Some(4_000),
            ..catalogue_provider_config(google, Some("gemini-2.5-flash"), None)
        };
        let confirmed = catalogue_provider_config(google, Some("gemini-2.5-flash"), Some(&current));
        assert_eq!(
            confirmed, current,
            "the thinking budget travels with it, so an unchanged answer is byte-identical"
        );

        let moved = catalogue_provider_config(google, Some("gemini-2.5-pro"), Some(&current));
        assert_ne!(moved, current);
        assert_eq!(
            moved.extended_thinking_budget,
            Some(4_000),
            "a preference of the developer's survives the move"
        );
    }

    /// A key variable the developer chose is part of how they reach their
    /// provider, not part of which model they picked — changing the model
    /// on that provider must not quietly restore the catalogue's default
    /// variable name and break their next start.
    #[test]
    fn a_chosen_key_variable_survives_a_model_change_on_the_same_provider() {
        let anthropic = aldwin_llm::provider("anthropic").expect("a catalogue provider");
        let current = aldwin_config::ProviderConfig {
            api_key_env: "ANTHROPIC_KEY_WORK".into(),
            ..catalogue_provider_config(anthropic, Some("claude-sonnet-5"), None)
        };

        let same_provider =
            catalogue_provider_config(anthropic, Some("claude-opus-5"), Some(&current));
        assert_eq!(
            same_provider.api_key_env, "ANTHROPIC_KEY_WORK",
            "the developer's own variable is how they reach this provider"
        );
        assert_eq!(same_provider.model, "claude-opus-5");

        // A different provider is a different endpoint with a different
        // key, so there the catalogue's variable is the right one.
        let google = aldwin_llm::provider("google").expect("a catalogue provider");
        let moved = catalogue_provider_config(google, Some("gemini-2.5-flash"), Some(&current));
        assert_eq!(moved.api_key_env, google.api_key_env);
    }

    /// The picker answers with `provider/model`, so the qualified form on the
    /// provider already configured is the common path — and it must keep the
    /// developer's own key variable exactly as the provider question does, not restore
    /// the catalogue's and fail the swap on a variable that is not exported.
    #[tokio::test]
    async fn a_chosen_key_variable_survives_a_qualified_model_change_on_the_same_provider() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let custom = aldwin_config::ProviderConfig {
            api_key_env: "ANTHROPIC_KEY_WORK".into(),
            ..cfg.global_provider().unwrap()
        };
        cfg.set_provider(aldwin_config::Scope::Global, custom)
            .unwrap();
        let (mut session, seen) = recording_session();
        let (tx, mut rx) = mpsc::channel(8);

        intercept(
            Command::Submit {
                text: "/model anthropic/claude-opus-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;

        let _ = notice(&mut rx).await;
        assert_eq!(
            seen.lock().unwrap()[0].api_key_env,
            "ANTHROPIC_KEY_WORK",
            "the client is rebuilt on the key the developer exports"
        );
        let saved = cfg.global_provider().unwrap();
        assert_eq!(saved.api_key_env, "ANTHROPIC_KEY_WORK");
        assert_eq!(saved.model, "claude-opus-5");
    }

    /// `provider/model` moves both halves — endpoint and key variable
    /// included, which is the whole reason the provider half is validated.
    #[tokio::test]
    async fn a_qualified_argument_moves_the_endpoint_and_the_key_variable_too() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model google/gemini-2.5-flash".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(message.contains("google/gemini-2.5-flash"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(
            saved.provider,
            aldwin_config::ProviderKind::OpenaiCompatible
        );
        assert_eq!(saved.model, "gemini-2.5-flash");
        assert_eq!(saved.api_key_env, "GOOGLE_API_KEY");
        assert_eq!(
            saved.base_url,
            aldwin_llm::provider("google")
                .unwrap()
                .base_url
                .map(String::from)
        );
    }

    /// A provider named with no model takes that provider's default, so the
    /// file is never left half-written.
    #[tokio::test]
    async fn a_provider_with_no_model_takes_that_providers_default() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model google/".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let _ = notice(&mut rx).await;
        assert_eq!(
            cfg.global_provider().unwrap().model,
            aldwin_llm::provider("google").unwrap().default_model()
        );
    }

    /// Everything past the *first* slash is the model, so a model id that
    /// contains slashes is reachable by naming its provider.
    #[tokio::test]
    async fn only_the_first_slash_splits_so_a_slashed_model_id_survives() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model deepseek/vendor/some-model".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

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
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model openai".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(message.contains("openai/gpt-5"), "{message}");
        let saved = cfg.global_provider().unwrap();
        assert_eq!(
            saved.model,
            aldwin_llm::provider("openai").unwrap().default_model()
        );
        assert_eq!(
            saved.api_key_env, "OPENAI_API_KEY",
            "the endpoint and key must move with the name"
        );
    }

    /// Naming the provider you are already on keeps the model you are on.
    /// Taking the catalogue default instead would make `/model anthropic`
    /// a silent downgrade from `claude-opus-5`.
    #[tokio::test]
    async fn naming_the_current_provider_keeps_the_current_model() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        // One session across both calls: the first moves it onto
        // `claude-opus-5`, and "already on" is now a statement about the
        // session as much as about the file.
        let mut session = session();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model claude-opus-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        let _ = notice(&mut rx).await;

        intercept(
            Command::Submit {
                text: "/model anthropic".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        let message = notice(&mut rx).await;
        assert!(
            message.contains("already on anthropic/claude-opus-5"),
            "{message}"
        );
        assert_eq!(cfg.global_provider().unwrap().model, "claude-opus-5");
    }

    /// `/model openai` and `/model openai/` are the same instruction.
    #[tokio::test]
    async fn a_bare_provider_and_a_trailing_slash_mean_the_same_thing() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);

        intercept(
            Command::Submit {
                text: "/model openai".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        let _ = notice(&mut rx).await;
        let bare = cfg.global_provider().unwrap();

        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        intercept(
            Command::Submit {
                text: "/model openai/".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        let _ = notice(&mut rx).await;
        assert_eq!(cfg.global_provider().unwrap(), bare);
    }

    /// The provider half folds case, like `/theme` does; the model half is
    /// left exactly as typed, because a host compares it byte for byte.
    #[tokio::test]
    async fn the_provider_half_is_case_insensitive_and_the_model_half_is_not() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model Google/Gemini-2.5-Flash".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let _ = notice(&mut rx).await;
        let saved = cfg.global_provider().unwrap();
        assert_eq!(
            saved.api_key_env, "GOOGLE_API_KEY",
            "GOOGLE must resolve to the google row"
        );
        assert_eq!(
            saved.model, "Gemini-2.5-Flash",
            "the model id must survive verbatim"
        );
    }

    /// A mistyped provider is rejected rather than written as part of a
    /// model id on whatever provider happened to be set.
    #[tokio::test]
    async fn a_slashed_argument_with_an_unknown_provider_is_rejected_not_written() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model gogle/gemini-2.5-pro".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            message.contains("There is no provider called gogle"),
            "{message}"
        );
        assert!(message.contains("The providers are anthropic"), "{message}");
        assert_eq!(
            cfg.global_provider().unwrap().model,
            "claude-sonnet-5",
            "nothing may be written on a rejection"
        );
    }

    /// Writing global while a project `provider.yaml` shadows it would
    /// report a change the next start ignores.
    #[tokio::test]
    async fn the_scope_written_is_the_one_that_actually_supplies_the_setting() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        with_provider(&cfg, aldwin_config::Scope::Project, "google");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model gemini-2.5-flash".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            message.contains("this project's provider.yaml"),
            "{message}"
        );
        assert_eq!(cfg.project_provider().unwrap().model, "gemini-2.5-flash");
        assert_eq!(
            cfg.global_provider().unwrap().model,
            "claude-sonnet-5",
            "the shadowed scope must be left alone"
        );
    }

    /// Setting what is already set says so instead of reporting a change
    /// and telling the developer to restart for it.
    #[tokio::test]
    async fn setting_the_current_model_reports_no_change_and_writes_nothing() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model anthropic/claude-sonnet-5".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            message.contains("already on anthropic/claude-sonnet-5"),
            "{message}"
        );
        assert!(
            !message.contains("Restart"),
            "nothing changed, so nothing needs restarting: {message}"
        );
    }

    /// The file and the session can disagree — `/reload-config` picks up a
    /// hand-edited `provider.yaml` without rebuilding the client the session
    /// holds. Asking for what the file already says must then still move the
    /// session, or the developer is told "already on" a model they are
    /// demonstrably not running.
    #[tokio::test]
    async fn what_the_file_already_says_is_still_a_swap_when_the_session_is_elsewhere() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        // The session booted on a different model from the one on disk.
        let (mut session, seen) = recording_session();
        session.model = "anthropic/claude-opus-5".into();
        let (tx, mut rx) = mpsc::channel(8);

        intercept(
            Command::Submit {
                text: "/model anthropic/claude-sonnet-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(
            !message.contains("already on"),
            "the session is not on it, whatever the file says: {message}"
        );
        assert!(
            message.contains("Now on anthropic/claude-sonnet-5"),
            "{message}"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "the client is rebuilt, which is the whole point of the command here"
        );
        assert_eq!(session.model, "anthropic/claude-sonnet-5");
    }

    /// Two `/model` calls in one session: the second moves off what the
    /// first one set, not off what the process booted with. The session
    /// model is state that each swap advances — a fixed startup string was
    /// only ever right while the session could not change model at all.
    #[tokio::test]
    async fn each_swap_advances_what_the_session_is_running() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let (mut session, seen) = recording_session();
        let (tx, mut rx) = mpsc::channel(8);

        intercept(
            Command::Submit {
                text: "/model claude-opus-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        assert!(notice(&mut rx)
            .await
            .contains("Now on anthropic/claude-opus-5"));
        let _ = rx.recv().await; // ModelChanged

        intercept(
            Command::Submit {
                text: "/model lumo/lumo-max".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        assert!(notice(&mut rx).await.contains("Now on lumo/lumo-max"));
        let _ = rx.recv().await; // ModelChanged

        let built = seen.lock().unwrap().clone();
        assert_eq!(
            built.iter().map(|c| c.model.as_str()).collect::<Vec<_>>(),
            ["claude-opus-5", "lumo-max"]
        );

        // And a third that cannot be built names the *second* as where the
        // session still is.
        let switch = FakeSwitch {
            fails_with: Some("NO_KEY"),
            ..Default::default()
        };
        session = Session::new(session.model.clone(), Box::new(switch));
        intercept(
            Command::Submit {
                text: "/model anthropic/claude-sonnet-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        let message = notice(&mut rx).await;
        assert!(message.contains("still on lumo/lumo-max"), "{message}");
    }

    /// A hand-written endpoint is not a catalogue provider, and must not be
    /// reported as one.
    #[tokio::test]
    async fn an_endpoint_the_catalogue_does_not_know_is_reported_as_itself() {
        let (_project, _global, cfg) = config();
        cfg.set_provider(
            aldwin_config::Scope::Global,
            aldwin_config::ProviderConfig {
                version: aldwin_config::PROVIDER_VERSION,
                provider: aldwin_config::ProviderKind::OpenaiCompatible,
                model: "qwen3-coder".into(),
                base_url: Some("http://localhost:8000/v1/chat/completions".into()),
                api_key_env: "VLLM_API_KEY".into(),
                extended_thinking_budget: None,
            },
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;

        let message = notice(&mut rx).await;
        assert!(message.contains("You are on qwen3-coder"), "{message}");
        assert!(
            message.contains("http://localhost:8000/v1/chat/completions"),
            "{message}"
        );
        assert!(
            !message.contains("on lumo/"),
            "a local endpoint must not be labelled with someone else's name: {message}"
        );
    }

    /// ADR 0009 §6: a session can start with nothing configured, and `/model
    /// provider/model` is how it gets a model — written globally, since
    /// there is no other default for every other directory to inherit.
    #[tokio::test]
    async fn with_nothing_configured_a_qualified_model_configures_the_global_file() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        intercept(
            Command::Submit {
                text: "/model claude-opus-5".into(),
            },
            &cfg,
            &mut session(),
            None,
            &tx,
        )
        .await;
        assert!(
            notice(&mut rx).await.contains("No provider is configured"),
            "a bare model id has nowhere to go"
        );
        assert!(cfg.global_provider().is_err());

        let (mut session, seen) = recording_session();
        intercept(
            Command::Submit {
                text: "/model anthropic/claude-opus-5".into(),
            },
            &cfg,
            &mut session,
            None,
            &tx,
        )
        .await;
        assert!(notice(&mut rx)
            .await
            .contains("Now on anthropic/claude-opus-5"));
        assert_eq!(cfg.global_provider().unwrap().model, "claude-opus-5");
        assert_eq!(seen.lock().unwrap().len(), 1, "the client is built on it");
    }

    #[tokio::test]
    async fn run_interceptor_forwards_normal_input_and_stops_others_reaching_it() {
        let (_project, _global, cfg) = config();
        let (tui_tx, tui_rx) = mpsc::channel(8);
        let (forward_tx, mut forward_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);

        let handle = tokio::spawn(run_interceptor(
            tui_rx,
            forward_tx,
            cfg,
            session(),
            None,
            event_tx,
        ));

        tui_tx
            .send(Command::Submit {
                text: "/nope".into(),
            })
            .await
            .unwrap();
        tui_tx
            .send(Command::Submit { text: "hi".into() })
            .await
            .unwrap();
        drop(tui_tx); // simulates the TUI exiting

        assert!(matches!(event_rx.recv().await, Some(Event::Notice { .. })));
        assert!(matches!(forward_rx.recv().await, Some(Command::Submit { text }) if text == "hi"));
        assert!(
            forward_rx.recv().await.is_none(),
            "forward sender must be dropped once incoming closes"
        );
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

        let handle = tokio::spawn(run_interceptor(
            tui_rx,
            forward_tx,
            cfg,
            session(),
            None,
            event_tx,
        ));

        tui_tx
            .send(Command::Submit {
                text: "/exit".into(),
            })
            .await
            .unwrap();

        // The interceptor task ends on its own — no need to drop tui_tx.
        handle.await.unwrap();
        assert!(
            forward_rx.recv().await.is_none(),
            "forward must be dropped so the core's command channel closes"
        );
        assert!(
            event_rx.recv().await.is_none(),
            "events must be dropped, not left open, on quit"
        );
    }
}
