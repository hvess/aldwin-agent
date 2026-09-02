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
/// `match` in `intercept` below by hand; five entries doesn't earn a
/// data-driven dispatch table yet.
const HELP_TEXT: &str =
    "commands: /help (this list), /clear (clear conversation context), /exit (end the session), /reload-config (reload config files from disk), /theme light|dark (switch color theme)";

/// Valid `/theme` argument values — kept as the single source of truth for
/// both the accept-check and the error message's own listing, so the two
/// can't drift apart.
const VALID_THEMES: [&str; 2] = ["dark", "light"];

/// Intercepts `/`-prefixed `Submit` input before it would otherwise reach
/// the core, per mjolnir-cli.md: "the core's only input is Submit, Cancel,
/// ApproveTool — it has no slash-command semantics." Runs synchronously in
/// the interceptor's own recv loop (`run_interceptor`), before any forward
/// send — not a post-send hook, per the spec's explicit Pitfall.
async fn intercept(command: Command, config: &Config, events: &mpsc::Sender<Event>) -> Intercepted {
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
pub async fn run_interceptor(mut incoming: mpsc::Receiver<Command>, forward: mpsc::Sender<Command>, config: Config, events: mpsc::Sender<Event>) {
    while let Some(command) = incoming.recv().await {
        match intercept(command, &config, &events).await {
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
        let result = intercept(cmd, &cfg, &tx).await;
        assert!(matches!(result, Intercepted::Forward(Command::Submit { text }) if text == "hello"));
    }

    #[tokio::test]
    async fn non_submit_commands_pass_through_unchanged() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let cmd = Command::PromptResponse { call_id: "call-1".into(), payload: serde_json::Value::Null };
        let result = intercept(cmd, &cfg, &tx).await;
        assert!(matches!(result, Intercepted::Forward(Command::PromptResponse { .. })));
    }

    #[tokio::test]
    async fn unknown_slash_command_is_rejected_and_never_forwarded() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Command::Submit { text: "/nope".into() };
        let result = intercept(cmd, &cfg, &tx).await;
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
        let result = intercept(Command::Submit { text: "/help".into() }, &cfg, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => {
                for command in ["/help", "/clear", "/exit", "/reload-config", "/theme"] {
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
        let result = intercept(Command::Submit { text: "/exit".into() }, &cfg, &tx).await;
        assert!(matches!(result, Intercepted::Quit));
    }

    #[tokio::test]
    async fn clear_is_translated_and_forwarded_to_core_not_handled_locally() {
        let (_project, _global, cfg) = config();
        let (tx, _rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/clear".into() }, &cfg, &tx).await;
        assert!(
            matches!(result, Intercepted::Forward(Command::ClearHistory)),
            "core owns ConversationLog, so /clear must reach it as ClearHistory rather than being swallowed like /help"
        );
    }

    #[tokio::test]
    async fn quit_is_not_recognised_only_exit_is() {
        let (_project, _global, cfg) = config();
        let (tx, mut rx) = mpsc::channel(8);
        let result = intercept(Command::Submit { text: "/quit".into() }, &cfg, &tx).await;
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
        let result = intercept(cmd, &cfg, &tx).await;
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
        intercept(cmd, &config, &tx).await;

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
        let result = intercept(Command::Submit { text: "/theme".into() }, &cfg, &tx).await;
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
        let result = intercept(Command::Submit { text: "/theme light".into() }, &cfg, &tx).await;
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
        intercept(Command::Submit { text: "/theme LIGHT".into() }, &cfg, &tx).await;
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
        intercept(Command::Submit { text: "/theme light".into() }, &cfg, &tx).await;
        let _ = rx.recv().await;
        let _ = rx.recv().await;

        intercept(Command::Submit { text: "/theme dark".into() }, &cfg, &tx).await;
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
        let result = intercept(Command::Submit { text: "/theme neon".into() }, &cfg, &tx).await;
        assert!(matches!(result, Intercepted::Handled));
        match rx.recv().await {
            Some(Event::Notice { message }) => assert!(message.contains("neon"), "message was: {message}"),
            other => panic!("expected a Notice, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "an invalid theme must not emit ThemeChanged");
        assert_eq!(cfg.global_tui().theme, None, "an invalid theme must not be persisted");
    }

    #[tokio::test]
    async fn run_interceptor_forwards_normal_input_and_stops_others_reaching_it() {
        let (_project, _global, cfg) = config();
        let (tui_tx, tui_rx) = mpsc::channel(8);
        let (forward_tx, mut forward_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);

        let handle = tokio::spawn(run_interceptor(tui_rx, forward_tx, cfg, event_tx));

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

        let handle = tokio::spawn(run_interceptor(tui_rx, forward_tx, cfg, event_tx));

        tui_tx.send(Command::Submit { text: "/exit".into() }).await.unwrap();

        // The interceptor task ends on its own — no need to drop tui_tx.
        handle.await.unwrap();
        assert!(forward_rx.recv().await.is_none(), "forward must be dropped so the core's command channel closes");
        assert!(event_rx.recv().await.is_none(), "events must be dropped, not left open, on quit");
    }
}
