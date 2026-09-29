use std::path::PathBuf;
use std::sync::Arc;

use aldwin_config::Config;
use aldwin_core::{Command, Event, SessionId};
use aldwin_login::{Account, Login};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::connect;
use crate::history::History;
use crate::update::{self, Outcome};

/// What `intercept` decided to do with one incoming command.
enum Intercepted {
    /// Not a slash command, or not a `Submit`: forward unchanged.
    Forward(Command),
    /// A slash command ran or was rejected; nothing reaches core.
    Handled,
    /// `/exit`: `run_interceptor` returns (aldwin-cli.md's Decisions). That
    /// drops its `forward` and `events` senders, core's command channel
    /// closes and core drops its `events` sender, and the TUI exits on the
    /// closed event channel. No `Event` variant exists for quitting.
    Quit,
}

/// One slash command: its name after the `/`, its argument text (empty if
/// none), and its summary.
pub(crate) struct SlashCommand {
    name: &'static str,
    argument: &'static str,
    summary: &'static str,
    /// Offered by the `/` menu; `/help` lists every command.
    in_menu: bool,
}

/// Every command `intercept` answers; the `/` menu and `/help` both draw
/// from it. Menu entries and order: `crates/review/baseline.json`,
/// `frame-command-list-is-not-the-products`. `/quit` and `/exit` are one
/// command.
const COMMANDS: [SlashCommand; 10] = [
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
        name: "connect",
        argument: "",
        summary: "Connect a provider account",
        in_menu: true,
    },
    SlashCommand {
        name: "quit",
        argument: "",
        summary: "Leave Aldwin",
        in_menu: true,
    },
    SlashCommand {
        name: "exit",
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
        name: "update",
        argument: "",
        summary: "Install the latest release",
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

/// What `/help` says: every command, in `COMMANDS` order.
fn help_text() -> String {
    let commands: Vec<String> = COMMANDS
        .iter()
        .map(|c| format!("/{}{} — {}", c.name, c.argument, c.summary.to_lowercase()))
        .collect();
    format!("The commands are {}.", commands.join(" · "))
}

/// What `/clear` and `/resume` say while a turn runs
/// (`History::turn_in_flight`). `esc` is the footer's stop key.
const TURN_IN_FLIGHT: &str = "A turn is running. Stop it with esc first, then try again.";

/// How to resume, quoted by every branch that cannot act.
const RESUME_USAGE: &str = "Type /resume on its own to pick a conversation from a list.";

/// How to change the model, quoted by every branch that rejects an
/// argument.
const MODEL_USAGE: &str = "Change it with /model provider/model.";

/// Valid `/theme` arguments.
const VALID_THEMES: [&str; 2] = ["dark", "light"];

/// How `/model` moves a running session onto another model, by rebuilding
/// the client inside the agent's (`bootstrap::ClientHandle`).
///
/// A failed rebuild (e.g. `api_key_env` not exported) must leave the
/// current client in place.
pub trait ModelSwitch: Send + Sync {
    /// Rebuilds the client on `config`, the provider settings in force (the
    /// written file over the layer below, `ProviderConfig::over`).
    fn switch(
        &self,
        config: &aldwin_config::ProviderConfig,
    ) -> Result<(), aldwin_llm::LlmClientInitError>;
}

/// The running session's model and the means to change it, held together
/// because a swap invalidates `model`.
pub struct Session {
    /// `provider/model`, as [`qualified`] renders it: what the next turn
    /// runs on.
    model: String,
    /// Also used when a `/connect` completes, to move the session onto the
    /// account.
    switch: Arc<dyn ModelSwitch>,
    /// Run after a successful `/reload-config` for state outside `Config`
    /// (the workspace roots, ADR 0007). Returns a notice, or `None`.
    after_reload: Option<AfterReload>,
    /// The `/connect` waiting for approval. A new `/connect` aborts it (two
    /// waits would race to write one entry). Must be aborted on drop: it
    /// holds an event sender, and the TUI exits only once all are gone.
    connecting: Option<JoinHandle<()>>,
    /// A `/connect` sends its stored account here. The interceptor, not the
    /// wait, decides whether to move the session, since only it knows the
    /// current model.
    connected_tx: mpsc::Sender<Account>,
    connected_rx: Option<mpsc::Receiver<Account>>,
    /// The binary `/update` replaces, resolved at startup: once replaced,
    /// Linux reports the running one as `… (deleted)`.
    exe: Option<PathBuf>,
    /// The `/update` running, if any. Must be aborted on drop, as
    /// `connecting` is; an abort cannot stop a rename already under way,
    /// which leaves either binary whole.
    updating: Option<JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in [self.connecting.take(), self.updating.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

pub type AfterReload = Box<dyn Fn() -> Option<String> + Send + Sync>;

impl Session {
    pub fn new(model: String, switch: Arc<dyn ModelSwitch>) -> Self {
        let (connected_tx, connected_rx) = mpsc::channel(1);
        Self {
            model,
            switch,
            after_reload: None,
            connecting: None,
            connected_tx,
            connected_rx: Some(connected_rx),
            // Canonical, so a symlinked install replaces its target rather
            // than the link.
            exe: std::env::current_exe()
                .and_then(|exe| exe.canonicalize())
                .ok(),
            updating: None,
        }
    }

    pub fn with_after_reload(mut self, hook: AfterReload) -> Self {
        self.after_reload = Some(hook);
        self
    }
}

/// Handles `/`-prefixed `Submit` input before core sees it (aldwin-cli.md);
/// core gets only translated commands (`/clear` as `ClearHistory`, `/resume`
/// as `Resume`). Must run in `run_interceptor`'s loop before the forward
/// send, never as a post-send hook (the spec's Pitfall).
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

    // Only commands that take an argument match `Some`, so `/help me` is as
    // unknown as `/nope`.
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
        ("update", None) => {
            handle_update(session, events).await;
            Intercepted::Handled
        }
        // Forwarded: core wipes its `ConversationLog`, moves its sink
        // (`History`) to a new transcript, and answers with
        // `Event::HistoryCleared`, on which the TUI wipes its own log
        // (`aldwin_tui::App::apply_event`). Core refuses it mid-turn.
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
        // `/quit` and `/exit` are one command; the menu offers both.
        ("exit", None) | ("quit", None) => Intercepted::Quit,
        // Bare `/theme` toggles, as picking it from the menu asks.
        ("theme", arg) => {
            handle_theme(arg, config, events).await;
            Intercepted::Handled
        }
        ("model", arg) => {
            handle_model(arg, config, session, events).await;
            Intercepted::Handled
        }
        // Bare `/connect` is normally caught by aldwin-tui, which opens the
        // list and answers with `/connect <provider>`.
        ("connect", arg) => {
            handle_connect(arg, config, session, events).await;
            Intercepted::Handled
        }
        // Bare `/resume` is normally caught by aldwin-tui's picker, which
        // submits `/resume <id>`; it arrives here only with no list to show.
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

/// `/resume [id]`: the `Command::Resume` to forward, or `None` after a
/// notice.
///
/// Records are read here so a failed read changes nothing. Core, on taking
/// them, moves its sink (`History`) onto that transcript and answers with
/// `Event::HistoryLoaded`. Staged changes are deliberately not restored.
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

    // An empty load is a transcript whose first turn never finished
    // (`aldwin_config::load`); resuming it would amount to `/clear`.
    if records.is_empty() {
        let _ = events
            .send(Event::Notice {
                message: "That conversation has no finished turn to resume.".into(),
            })
            .await;
        return None;
    }

    // Core confirms when it acts; if a turn is running by then, it refuses.
    Some(Command::Resume {
        session: id,
        records,
    })
}

/// `/theme [light|dark]`: bare, toggles the theme in effect. Persists via
/// `Config::set_tui` and sends `Event::ThemeChanged` straight to the TUI,
/// bypassing core (see that event's doc in aldwin-core).
async fn handle_theme(arg: Option<&str>, config: &Config, events: &mpsc::Sender<Event>) {
    let normalized = match arg {
        Some(arg) => arg.to_ascii_lowercase(),
        // Read as startup reads it, so the toggle is from what is on screen.
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

/// `/connect <provider>`: connects the account a catalogue row offers (ADR
/// 0012).
///
/// The sign-in runs as its own task so the interceptor stays free for
/// `/quit`. The task stores the account and sends it on `connected_tx`; the
/// interceptor then decides via [`moved_onto`].
async fn handle_connect(
    arg: Option<&str>,
    config: &Config,
    session: &mut Session,
    events: &mpsc::Sender<Event>,
) {
    let offered: Vec<String> = aldwin_llm::PROVIDERS
        .iter()
        .filter(|p| p.account.is_some())
        .map(|p| format!("/connect {}", p.id))
        .collect();
    let usage = format!("Connect one with {}.", offered.join(" or "));
    let Some(provider) = arg.filter(|a| !a.is_empty()).map(str::to_ascii_lowercase) else {
        let message = format!("Say which account. {usage}");
        let _ = events.send(Event::Notice { message }).await;
        return;
    };
    let Some(account) = aldwin_llm::provider(&provider).and_then(|p| p.account) else {
        let message = format!("There is no account to connect for {provider}. {usage}");
        let _ = events.send(Event::Notice { message }).await;
        return;
    };

    if let Some(previous) = session.connecting.take() {
        previous.abort();
    }
    let (config, events, connected) =
        (config.clone(), events.clone(), session.connected_tx.clone());
    session.connecting = Some(tokio::spawn(async move {
        let name = account.name();
        let failed =
            |e: &dyn std::fmt::Display| format!("The {name} account could not be connected: {e}.");
        let (login, prompt) = match Login::start(account).await {
            Ok(started) => started,
            Err(e) => {
                let _ = events
                    .send(Event::Notice {
                        message: failed(&e),
                    })
                    .await;
                return;
            }
        };
        let message = format!(
            "Open {} and enter the code {}. It is good for {}.",
            prompt.url,
            prompt.code,
            minutes(prompt.expires_in)
        );
        let _ = events.send(Event::Notice { message }).await;
        let credentials = match login.wait().await {
            Ok(credentials) => credentials,
            Err(e) => {
                let _ = events
                    .send(Event::Notice {
                        message: failed(&e),
                    })
                    .await;
                return;
            }
        };
        if let Err(e) = config.set_connection(account.id(), connect::record(&credentials)) {
            let message = format!(
                "The {name} account was approved, but could not be saved: {e}. Connect it again once that is fixed."
            );
            let _ = events.send(Event::Notice { message }).await;
            return;
        }
        let _ = connected.send(account).await;
    }));
}

/// `/update`: installs the latest release over this binary
/// ([`update::update`]). Runs as its own task, as `/connect` does, so the
/// interceptor stays free.
async fn handle_update(session: &mut Session, events: &mpsc::Sender<Event>) {
    if session
        .updating
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        let message = "Aldwin is already looking for an update.".to_string();
        let _ = events.send(Event::Notice { message }).await;
        return;
    }
    let Some(exe) = session.exe.clone() else {
        let message = "Aldwin could not find its own binary, so it cannot replace it. Install the latest release with install.sh instead.".to_string();
        let _ = events.send(Event::Notice { message }).await;
        return;
    };
    let message = "Looking for a newer version of Aldwin.".to_string();
    let _ = events.send(Event::Notice { message }).await;

    let events = events.clone();
    session.updating = Some(tokio::spawn(async move {
        let message = match update::update(env!("CARGO_PKG_REPOSITORY"), &exe).await {
            Ok(Outcome::Current(version)) => {
                format!("Aldwin is already up to date: {version} is the latest version.")
            }
            Ok(Outcome::Installed(version)) => format!(
                "Aldwin {version} is installed. Quit and start it again to run it; /resume picks this conversation back up."
            ),
            Err(e) => format!("Aldwin could not be updated: {e}. Nothing was changed."),
        };
        let _ = events.send(Event::Notice { message }).await;
    }));
}

/// `12 minutes`, `1 minute`: a code's lifetime, rounded up, at least 1.
fn minutes(lifetime: std::time::Duration) -> String {
    match lifetime.as_secs().div_ceil(60).max(1) {
        1 => "1 minute".into(),
        n => format!("{n} minutes"),
    }
}

/// The notice for an approved, stored account. The session moves onto it
/// only if it runs that provider's model exactly as `provider.yaml` states
/// it, so a pending `/reload-config` edit or a `/model` during the wait is
/// never overridden.
fn moved_onto(account: Account, config: &Config, session: &Session) -> String {
    let name = account.name();
    let on_it = config.effective_provider().filter(|p| {
        let row = identify(p);
        row.map(|row| row.id) == Some(account.id()) && qualified(p, row) == session.model
    });
    match on_it {
        None => format!(
            "Connected to {name}. A model on {} now runs on your account.",
            account.id()
        ),
        Some(effective) => match session.switch.switch(&effective) {
            Ok(()) => format!("Connected to {name}. {} now runs on your account.", effective.model),
            Err(e) => format!(
                "Connected to {name}, but the session could not move onto it: {e}. Pick the model again with /model."
            ),
        },
    }
}

/// `/model [provider/]model`: sets provider and model together.
///
/// The argument splits on its first `/` only:
///
/// * A catalogue provider name, bare or with a trailing `/`: that provider on
///   its default model, or on the current model if already configured.
/// * `provider/model`: both; the model may contain slashes
///   (`openrouter/qwen/qwen3-coder`).
/// * A slashed argument whose head is not a provider: rejected, since it is
///   what a mistyped provider looks like (`gogle/gemini-2.5-pro`).
/// * Anything else: a model id on the configured provider.
///
/// A bare provider name must never be read as a model id. Only the
/// provider half is validated; any model id is accepted, as `provider.yaml`
/// does.
///
/// Bare `/model` is normally caught by aldwin-tui's picker; it arrives here
/// only with no catalogue to show, and reports the current model.
///
/// The client is rebuilt before anything is written, so a provider that
/// cannot be built (unexported `api_key_env`) persists nothing. A provider
/// offering an account builds even with neither account nor key (ADR 0012,
/// `connect::reach`). Sends `Event::ModelChanged` on success.
async fn handle_model(
    arg: Option<&str>,
    config: &Config,
    session: &mut Session,
    events: &mpsc::Sender<Event>,
) {
    // Write the scope that supplies the setting: a global write under a
    // project `provider.yaml` would be ignored at the next start. `global`
    // is kept regardless: the client is built from the overlay
    // (`ProviderConfig::over`), as startup builds it. With nothing
    // configured (ADR 0009 §6) the global file is written.
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

    // Grammar: see this function's doc.
    let (head, tail) = arg
        .split_once('/')
        .map_or((arg, ""), |(head, tail)| (head, tail.trim()));
    let next = match named_provider(head) {
        Some(p) => {
            // An empty model half on the configured provider keeps the
            // current model; otherwise `/model anthropic` on
            // `anthropic/claude-opus-5` would reset to the default.
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

    // "Already on" must hold for the session, not only the file: a
    // `/reload-config` can change the file without touching the client. When
    // they disagree, fall through and swap.
    if current.as_ref() == Some(&next) && now == session.model {
        // Points at the list: this is usually reached by naming the current
        // provider while looking for models.
        let message = format!("You are already on {now}. /model on its own opens the list.");
        let _ = events.send(Event::Notice { message }).await;
        return;
    }

    // Overlaid as startup does (`Config::effective_provider`).
    let below = match scope {
        aldwin_config::Scope::Project => global.as_ref(),
        aldwin_config::Scope::Global => None,
    };
    let resolved = next.clone().over(below);

    // Before the write: a provider that cannot be built must not be left on
    // disk for the next start to fail on.
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
        // The swap already happened; only the next start is affected.
        Err(e) => format!(
            "Now on {now}, but it could not be saved to {where_}: {e}. The next start will use {}.",
            current
                .as_ref()
                .map_or_else(|| "nothing".to_string(), |c| qualified(c, identify(c)))
        ),
    };
    let _ = events.send(Event::Notice { message }).await;
    session.model = now;
    // The bare model id, not the qualified name: it matches
    // `StatusInfo::model_name` at startup, and the picker matches the
    // provider separately.
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
/// Catalogue ids are lowercase (a test pins it). Never fold model ids: hosts
/// compare them byte for byte, and some are mixed-case.
fn named_provider(name: &str) -> Option<&'static aldwin_llm::Provider> {
    aldwin_llm::provider(&name.trim().to_ascii_lowercase())
}

/// The `provider.yaml` for catalogue row `provider` (used by `/model` and
/// the provider question): fields from the row, the model defaulting to the
/// row's default. `api_key_env` is a variable name; `provider.yaml` holds no
/// key. Account-vs-key is decided at client build (ADR 0012).
///
/// From `current` instead of the row:
///
/// * the thinking budget, always (a developer preference);
/// * `api_key_env`, only when `provider` is already the configured one, so a
///   custom variable name (`ANTHROPIC_KEY_WORK`) survives a model change.
///
/// This also makes re-confirming the current row compare equal to disk.
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

/// The catalogue row a `provider.yaml` points at ([`aldwin_llm::identify`]).
pub(crate) fn identify(
    config: &aldwin_config::ProviderConfig,
) -> Option<&'static aldwin_llm::Provider> {
    aldwin_llm::identify(config.provider, config.base_url.as_deref())
}

/// `provider/model` for a catalogue endpoint; the bare model id for any
/// other, where naming a provider would be a guess.
pub(crate) fn qualified(
    config: &aldwin_config::ProviderConfig,
    known: Option<&aldwin_llm::Provider>,
) -> String {
    match known {
        Some(p) => format!("{}/{}", p.id, config.model),
        None => config.model.clone(),
    }
}

/// What bare `/model` reports: the current model, the provider's other
/// models (or the endpoint), and every provider.
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
        // An unknown endpoint is named, since the bare model id alone is
        // ambiguous.
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
            // The permissions.yaml header promises its edits are picked up
            // here; `roots:` needs this hook for that.
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

/// Background task: passes every TUI command through `intercept` and
/// forwards the rest to core; also acts on completed `/connect`s. Ends when
/// `incoming` closes (the TUI returned), on `/quit`, or when core's channel
/// closes.
pub async fn run_interceptor(
    mut incoming: mpsc::Receiver<Command>,
    forward: mpsc::Sender<Command>,
    config: Config,
    mut session: Session,
    history: Option<Arc<History>>,
    events: mpsc::Sender<Event>,
) {
    let mut connected = session
        .connected_rx
        .take()
        .expect("a session's connection receiver is taken once, here");
    loop {
        tokio::select! {
            command = incoming.recv() => {
                let Some(command) = command else { break };
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
            Some(account) = connected.recv() => {
                let message = moved_onto(account, &config, &session);
                let _ = events.send(Event::Notice { message }).await;
            }
        }
    }
    // Dropping `forward`, `events` and `session` (which aborts a waiting
    // `/connect`) shuts the session down; see `Intercepted::Quit`.
}

#[cfg(test)]
mod tests {
    use super::*;

    use aldwin_core::RecordSink;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// The model every test's session starts on.
    const SESSION_MODEL: &str = "anthropic/claude-sonnet-5";

    /// A `ModelSwitch` that records the configs it is given instead of
    /// building a client.
    #[derive(Clone, Default)]
    struct FakeSwitch {
        seen: Arc<Mutex<Vec<aldwin_config::ProviderConfig>>>,
        /// Fails every switch as if this `api_key_env` were not exported.
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
        Session::new(SESSION_MODEL.into(), Arc::new(FakeSwitch::default()))
    }

    /// A session and the configs its switch was given.
    fn recording_session() -> (Session, Arc<Mutex<Vec<aldwin_config::ProviderConfig>>>) {
        let switch = FakeSwitch::default();
        let seen = switch.seen.clone();
        (Session::new(SESSION_MODEL.into(), Arc::new(switch)), seen)
    }

    fn config() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        (project, global, config)
    }

    /// A history with one finished turn in a past session, and both ends of
    /// the channel it reports on, so a test reads all notices from one place.
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
        // A new current session, as at launch; the recorded one is past.
        history.cleared();
        // Not the newest: `cleared` just made an empty one.
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

    /// aldwin-history.md's fork-free Decision. The writer moves only when
    /// core calls `resumed`, since core may still refuse.
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

    /// A transcript with no finished turn loads as nothing; resuming it
    /// would amount to `/clear`. Reached only by a typed id.
    #[tokio::test]
    async fn resuming_a_session_with_no_finished_turn_reports_rather_than_wiping() {
        use aldwin_core::{LogRecord, TurnId};

        let (_project, _global, cfg) = config();
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        // A crashed process's transcript. Its id comes from the handle, since
        // unfinished sessions are not listed.
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

    /// aldwin-history.md step 6: the writer moves only when core calls
    /// `cleared`.
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

    /// Core discards both mid-turn; a writer moved anyway would send the
    /// running turn into another session's file.
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

    /// Only reachable by a typed id; the picker never offers it.
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

    #[tokio::test]
    async fn the_current_session_is_not_offered_as_resumable() {
        let (_dir, history, _id, _events, _rx) = recorded_history("first");
        let current = history.current();
        assert!(
            !history.resumable().iter().any(|s| s.id == current.0),
            "the session you are in is not something to resume"
        );
    }

    /// Regression: a session with no finished turn was listed, then refused
    /// when picked.
    #[tokio::test]
    async fn an_unfinished_session_is_not_listed() {
        use aldwin_core::{LogRecord, TurnId};

        let (_dir, history, _id, _events, _rx) = recorded_history("said something");
        // A past session whose only turn never finished.
        for record in [
            LogRecord::TurnStarted { turn_id: TurnId(2) },
            LogRecord::UserMessage {
                turn_id: TurnId(2),
                text: "cut off".into(),
            },
        ] {
            history.append(&record);
        }
        history.cleared();
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
            ["resume", "model", "connect", "quit", "exit", "clear", "theme"],
            "the developer's seven, in order"
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
                    "/exit",
                    "/model",
                    "/reload-config",
                    "/theme",
                    "/update",
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

    /// `/exit` also stops the interceptor; that is pinned by
    /// `slash_exit_stops_the_interceptor_and_drops_its_senders`.
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
        // A malformed project permissions.yaml fails that one layer.
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

    /// Regression: a pasted tab made `/theme` an unknown command.
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

    /// Writes catalogue provider `id` on its default model into `scope`.
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

    /// The next `Notice`, skipping any `ModelChanged`.
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

    #[tokio::test]
    async fn a_client_that_cannot_be_built_leaves_the_session_and_the_file_alone() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        let switch = FakeSwitch {
            fails_with: Some("GOOGLE_API_KEY"),
            ..Default::default()
        };
        let mut session = Session::new(SESSION_MODEL.into(), Arc::new(switch));
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

    /// Re-confirming the current row must not write a project file
    /// restating the global one.
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

        // A different provider takes the catalogue's variable.
        let google = aldwin_llm::provider("google").expect("a catalogue provider");
        let moved = catalogue_provider_config(google, Some("gemini-2.5-flash"), Some(&current));
        assert_eq!(moved.api_key_env, google.api_key_env);
    }

    /// The picker's `provider/model` answer is the common path, so it must
    /// keep a custom key variable too.
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

    /// Regression: `/model openai` wrote `model: openai` onto the current
    /// provider.
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

    /// Otherwise `/model anthropic` would silently reset `claude-opus-5` to
    /// the default.
    #[tokio::test]
    async fn naming_the_current_provider_keeps_the_current_model() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        // One session across both calls: "already on" checks the session
        // too.
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

    /// `/reload-config` can change the file without rebuilding the client;
    /// "already on" must then not be reported.
    #[tokio::test]
    async fn what_the_file_already_says_is_still_a_swap_when_the_session_is_elsewhere() {
        let (_project, _global, cfg) = config();
        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        // The session runs a different model from the one on disk.
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

        // A failed third names the second as current.
        let switch = FakeSwitch {
            fails_with: Some("NO_KEY"),
            ..Default::default()
        };
        session = Session::new(session.model.clone(), Arc::new(switch));
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

    /// ADR 0009 §6.
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

    /// The TUI exits only once every event sender is gone; see
    /// `Intercepted::Quit`.
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

        // Ends without `tui_tx` being dropped.
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

    #[tokio::test]
    async fn connect_without_a_provider_or_with_one_that_offers_none_says_which_do() {
        let (_project, _global, cfg) = config();
        for (text, expected) in [
            (
                "/connect",
                "Say which account. Connect one with /connect xai.",
            ),
            (
                "/connect anthropic",
                "There is no account to connect for anthropic. Connect one with /connect xai.",
            ),
            (
                "/connect nope",
                "There is no account to connect for nope. Connect one with /connect xai.",
            ),
        ] {
            let (tx, mut rx) = mpsc::channel(8);
            let mut session = session();
            let result = intercept(
                Command::Submit { text: text.into() },
                &cfg,
                &mut session,
                None,
                &tx,
            )
            .await;
            assert!(matches!(result, Intercepted::Handled));
            assert_eq!(notice(&mut rx).await, expected, "{text}");
            assert!(session.connecting.is_none(), "{text}: nothing to wait for");
        }
    }

    #[test]
    fn an_approved_account_moves_the_session_only_when_it_runs_that_provider() {
        let (_project, _global, cfg) = config();
        let (mut session, seen) = recording_session();

        with_provider(&cfg, aldwin_config::Scope::Global, "anthropic");
        assert_eq!(
            moved_onto(Account::Xai, &cfg, &session),
            "Connected to x.ai. A model on xai now runs on your account."
        );

        // The file says xai (a pending hand edit); the session runs claude.
        with_provider(&cfg, aldwin_config::Scope::Global, "xai");
        assert_eq!(
            moved_onto(Account::Xai, &cfg, &session),
            "Connected to x.ai. A model on xai now runs on your account."
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "the session was left alone"
        );

        session.model = "xai/grok-4.7".into();
        assert_eq!(
            moved_onto(Account::Xai, &cfg, &session),
            "Connected to x.ai. grok-4.7 now runs on your account."
        );
        assert_eq!(seen.lock().unwrap().len(), 1, "rebuilt on the account");
    }

    #[test]
    fn a_codes_lifetime_reads_in_whole_minutes_rounded_up() {
        use std::time::Duration;
        assert_eq!(minutes(Duration::from_secs(1800)), "30 minutes");
        assert_eq!(minutes(Duration::from_secs(90)), "2 minutes");
        assert_eq!(minutes(Duration::from_secs(30)), "1 minute");
    }

    /// The wait holds an event sender; the TUI exits only once all are gone.
    #[tokio::test]
    async fn ending_the_session_aborts_a_connect_still_waiting() {
        let mut session = session();
        let wait = tokio::spawn(std::future::pending::<()>());
        let abort = wait.abort_handle();
        session.connecting = Some(wait);
        drop(session);
        tokio::task::yield_now().await;
        assert!(abort.is_finished(), "the wait was aborted with the session");
    }

    #[tokio::test]
    async fn ending_the_session_aborts_an_update_still_running() {
        let mut session = session();
        let update = tokio::spawn(std::future::pending::<()>());
        let abort = update.abort_handle();
        session.updating = Some(update);
        drop(session);
        tokio::task::yield_now().await;
        assert!(
            abort.is_finished(),
            "the update was aborted with the session"
        );
    }

    /// Two updates would race to replace one binary.
    #[tokio::test]
    async fn an_update_already_running_is_not_started_again() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let mut session = session();
        session.updating = Some(tokio::spawn(std::future::pending::<()>()));
        let submit = Command::Submit {
            text: "/update".into(),
        };
        let result = intercept(submit, &cfg, &mut session, None, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        assert_eq!(
            notice(&mut rx).await,
            "Aldwin is already looking for an update."
        );
        assert!(rx.try_recv().is_err(), "nothing else was said");
    }
}
