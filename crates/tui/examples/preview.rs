//! Design-iteration harness — NOT part of the shipped app. Builds an `App`
//! seeded with representative `LogEntry`s (no real core/LLM wiring) and
//! draws it once to the real terminal so the rendered output can be
//! captured (tmux `capture-pane` + a screenshot pipeline) for visual review
//! while reworking `ui.rs`. Pass a scene name as argv[1]; see `scene()`
//! below for the list. Pass a theme name as argv[2] — `light` or `dark`
//! (default) — added alongside `Palette`/`Theme` (2026-09-02) so a light-
//! theme change can be screenshotted the same way every dark-theme one
//! already was, rather than shipped on the RGB values alone. Exits
//! immediately after drawing (no input loop) so a driving script can
//! capture-pane right after launch.
use std::io;
use std::sync::Arc;

use aldwin_config::Config;
use aldwin_permissions::{Class, Engine, PromptPayload};
use aldwin_tui::{App, LogEntry, Theme, ToolActivityEntry, ToolActivityStatus};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;

fn main() -> io::Result<()> {
    let scene_name = std::env::args().nth(1).unwrap_or_else(|| "empty".into());
    let theme = Theme::from_config(std::env::args().nth(2).as_deref());

    let dir = tempfile::tempdir().unwrap();
    let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
    let engine = Arc::new(Engine::new(config));
    let mut app = App::new("claude-sonnet-5".into(), engine).with_theme(theme);

    scene(&scene_name, &mut app);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|f| aldwin_tui::__preview_draw(f, &mut app))?;

    // Hold the alternate screen open until the driving script has captured
    // it; it sends a keypress (or kills the process) to release us.
    let mut buf = [0u8; 1];
    let _ = io::Read::read(&mut io::stdin(), &mut buf);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn scene(name: &str, app: &mut App) {
    match name {
        "empty" => {}
        "conversation" => conversation(app),
        "tools" => tools(app),
        "approval" => approval(app),
        "prompt" => prompt(app),
        "prompt_path" => prompt_path(app),
        "long" => long(app),
        other => panic!("unknown scene {other:?}"),
    }
}

fn conversation(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "Can you refactor the retry logic in llm/src/client.rs to use exponential backoff?".into() });
    app.log.push(LogEntry::AssistantText {
        text: "Sure — here's the plan:\n\n1. Add a `backoff_ms` helper\n2. Wire it into the retry loop\n3. Cap at **5** attempts\n\n```rust\nfn backoff_ms(attempt: u32) -> u64 {\n    100 * 2u64.pow(attempt)\n}\n```\n\nThat gives `100ms, 200ms, 400ms, ...`. Want me to apply it?".into(),
    });
    app.log.push(LogEntry::Notice { message: "context file AGENTS.md injected (project scope)".into() });
    app.status.turn = Some(3);
    app.status.step = Some(2);
}

fn tools(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "grep for TODO across the repo and summarize".into() });
    app.log.push(LogEntry::ToolActivity {
        step_id: aldwin_core::StepId(1),
        calls: vec![
            ToolActivityEntry { call_id: "c1".into(), name: "shell".into(), status: ToolActivityStatus::Completed { is_error: false, summary: "42 matches across 17 files".into() } },
            ToolActivityEntry { call_id: "c2".into(), name: "read".into(), status: ToolActivityStatus::Running },
        ],
    });
    app.turn_active = true;
    // Two differently-named tools running at once — visualizes the status
    // line's "running N tools…" activity label and its trailing tools list.
    app.status.running_tools = vec![
        aldwin_tui::__PreviewRunningTool { call_id: "c2".into(), name: "read".into() },
        aldwin_tui::__PreviewRunningTool { call_id: "c3".into(), name: "shell".into() },
    ];
    app.status.turn = Some(4);
    app.status.step = Some(1);
}

fn approval(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "fix the off-by-one in the pagination helper".into() });
    let diff = "--- a/src/page.rs\n+++ b/src/page.rs\n@@\n fn page(items: &[Item], size: usize, n: usize) -> &[Item] {\n     let start = n * size;\n-    let end = start + size;\n+    let end = (start + size).min(items.len());\n     &items[start..end]\n }\n";
    app.log.push(LogEntry::ApprovalCard { call_id: "call-1".into(), diff: diff.into(), resolution: None });
    app.pending_approvals.push_back(aldwin_tui::__PreviewPendingApproval { call_id: "call-1".into(), diff: diff.into() });
}

fn prompt(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "run the test suite".into() });
    let payload = PromptPayload::Tool { program: "cargo".into(), argv: vec!["test".into(), "--workspace".into()], declared: Class::Write };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-2".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(aldwin_tui::__PreviewPendingPrompt { call_id: "call-2".into(), payload });
}

/// A path-like Tool prompt (`read`) — shows the humanized title/dim raw-call
/// split and the directory-scope hint (`ui::scope_hint_line`), neither of
/// which the plain `"prompt"` scene above exercises (its `shell` target has
/// no directory to broaden to).
fn prompt_path(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "what does the dispatcher do on a deny-by-absence?".into() });
    let payload = PromptPayload::Tool { program: "read".into(), argv: vec!["./crates/tools/src/dispatcher.rs".into()], declared: Class::Read };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-3".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(aldwin_tui::__PreviewPendingPrompt { call_id: "call-3".into(), payload });
}

fn long(app: &mut App) {
    for i in 0..8 {
        app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
        app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
    }
    app.log.push(LogEntry::TurnEnded { reason: aldwin_core::TurnEndReason::EndTurn });
    app.log.push(LogEntry::RetryAttempt {
        info: aldwin_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded, retrying".into(), attempt: 1 },
    });
    app.log.push(LogEntry::Error { message: "provider returned 529 overloaded".into() });
    app.status.turn = Some(9);
    app.status.step = Some(3);
}
