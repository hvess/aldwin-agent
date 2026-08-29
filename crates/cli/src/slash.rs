use amundsen_config::Config;
use amundsen_core::{Command, Event};
use tokio::sync::mpsc;

/// What `intercept` decided to do with one incoming command.
enum Intercepted {
    /// Not a slash command (or not a `Submit` at all) — forward unchanged.
    Forward(Command),
    /// A known slash command ran (or an unknown one was rejected); nothing
    /// reaches the core.
    Handled,
    /// `/exit` or `/quit` — `run_interceptor` stops entirely rather than
    /// looping again, per amundsen-cli.md's Decisions: "CLI owns the
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

/// Intercepts `/`-prefixed `Submit` input before it would otherwise reach
/// the core, per amundsen-cli.md: "the core's only input is Submit, Cancel,
/// ApproveTool — it has no slash-command semantics." Runs synchronously in
/// the interceptor's own recv loop (`run_interceptor`), before any forward
/// send — not a post-send hook, per the spec's explicit Pitfall.
async fn intercept(command: Command, config: &Config, events: &mpsc::Sender<Event>) -> Intercepted {
    let Command::Submit { text } = &command else { return Intercepted::Forward(command) };
    let Some(rest) = text.trim_start().strip_prefix('/') else { return Intercepted::Forward(command) };

    match rest.trim() {
        "reload-config" => {
            handle_reload_config(config, events).await;
            Intercepted::Handled
        }
        "exit" | "quit" => Intercepted::Quit,
        other => {
            let _ = events.send(Event::Notice { message: format!("unknown slash command: /{other}") }).await;
            Intercepted::Handled
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
    use amundsen_core::PromptId;

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
        let cmd = Command::PromptResponse { id: PromptId(1), payload: serde_json::Value::Null };
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
    async fn exit_and_quit_are_recognised_as_the_quit_command() {
        let (_project, _global, cfg) = config();
        for text in ["/exit", "/quit"] {
            let (tx, _rx) = mpsc::channel(8);
            let result = intercept(Command::Submit { text: text.into() }, &cfg, &tx).await;
            assert!(matches!(result, Intercepted::Quit), "{text} should be recognised as a quit command");
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
        config.add_grant(amundsen_config::Scope::Project, amundsen_config::GrantList::Allow, "read:**").unwrap();
        let bad_path = project.path().join(".amundsen").join("permissions.yaml");
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
