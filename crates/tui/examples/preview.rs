//! Design-iteration harness — NOT part of the shipped app. Builds an `App`
//! seeded with representative `LogEntry`s (no real core/LLM wiring) and
//! draws it once to the real terminal so the rendered output can be
//! captured (tmux `capture-pane` + a screenshot pipeline) for visual review
//! while reworking `ui.rs`. Pass a scene name as argv[1]; see `scene()`
//! below for the list. Exits immediately after drawing (no input loop) so a
//! driving script can capture-pane right after launch.
use std::io;
use std::sync::Arc;

use mjolnir_config::Config;
use mjolnir_permissions::{Engine, PromptPayload};
use mjolnir_tui::{App, LogEntry, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;

fn main() -> io::Result<()> {
    let scene_name = std::env::args().nth(1).unwrap_or_else(|| "empty".into());

    let dir = tempfile::tempdir().unwrap();
    let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
    let engine = Arc::new(Engine::new(config));
    let mut app = App::new("claude-sonnet-5".into(), engine);

    scene(&scene_name, &mut app);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|f| mjolnir_tui::__preview_draw(f, &mut app))?;

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
        step_id: mjolnir_core::StepId(1),
        calls: vec![
            ToolActivityEntry { call_id: "c1".into(), name: "shell".into(), status: ToolActivityStatus::Completed { is_error: false, summary: "42 matches across 17 files".into() } },
            ToolActivityEntry { call_id: "c2".into(), name: "read".into(), status: ToolActivityStatus::Running },
        ],
    });
    app.turn_active = true;
    app.status.running_tools = vec![mjolnir_tui::__PreviewRunningTool { call_id: "c2".into(), name: "read".into() }];
    app.status.turn = Some(4);
    app.status.step = Some(1);
}

fn approval(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "fix the off-by-one in the pagination helper".into() });
    let diff = "--- a/src/page.rs\n+++ b/src/page.rs\n@@\n fn page(items: &[Item], size: usize, n: usize) -> &[Item] {\n     let start = n * size;\n-    let end = start + size;\n+    let end = (start + size).min(items.len());\n     &items[start..end]\n }\n";
    app.log.push(LogEntry::ApprovalCard { call_id: "call-1".into(), diff: diff.into(), resolution: None });
    app.pending_approval = Some(mjolnir_tui::__PreviewPendingApproval { call_id: "call-1".into() });
}

fn prompt(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "run the test suite".into() });
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test --workspace".into() };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-2".into(), payload: payload.clone(), resolution: None });
    app.pending_prompt = Some(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-2".into(), payload });
}

fn long(app: &mut App) {
    for i in 0..8 {
        app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
        app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
    }
    app.log.push(LogEntry::TurnEnded { reason: TurnEndReasonKind::EndTurn });
    app.log.push(LogEntry::RetryAttempt {
        info: mjolnir_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded, retrying".into(), attempt: 1 },
    });
    app.log.push(LogEntry::Error { message: "provider returned 529 overloaded".into() });
    app.status.turn = Some(9);
    app.status.step = Some(3);
}
