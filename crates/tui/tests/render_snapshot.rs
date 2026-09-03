//! Full-fidelity render snapshots — the automated form of the
//! "render it before trusting your reading of it" discipline in this
//! project's `CLAUDE.md`.
//!
//! Every scene below is drawn against `ratatui::backend::TestBackend` at
//! four frame sizes in both themes, and the *entire* resulting buffer —
//! every cell's symbol, foreground, background and modifiers — is
//! serialized to `tests/snapshots/render.snap`. The unit tests in
//! `ui.rs` assert facts about individual rows; this asserts the whole
//! frame, colors included, which is what makes a layout refactor
//! provably output-preserving rather than merely test-passing.
//!
//! Build identity — the release version, the commit, the working directory
//! — is pinned per scene rather than inherited from the build (see
//! [`fixed_identity`]). The first cut of this file did inherit it, which
//! meant the snapshot encoded who generated it: it broke on the next
//! commit, on a dirty tree, and on any checkout at a different path.
//!
//! Regenerate deliberately, after eyeballing the diff:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p mjolnir-tui --test render_snapshot
//! ```

use std::fmt::Write as _;
use std::sync::Arc;

use mjolnir_config::Config;
use mjolnir_permissions::{Engine, PromptPayload};
use mjolnir_tui::{App, LogEntry, Theme, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/render.snap");

/// Frame sizes worth pinning: the design system's own 120×36 frame, a
/// conventional 80×24, a narrow terminal that forces wrapping and drops
/// the option-detail column, and a wide one.
const SIZES: [(u16, u16); 4] = [(120, 36), (80, 24), (52, 20), (160, 44)];

const SCENES: [&str; 11] = [
    "empty",
    "conversation",
    "markdown",
    "fenced_diff",
    "tools",
    "approval",
    "approval_large",
    "prompt",
    "prompt_path",
    "prompt_scoped",
    "long",
];

#[test]
fn every_scene_renders_exactly_as_recorded() {
    let mut out = String::new();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let _ = writeln!(out, "=== {theme:?} {scene_name} {width}x{height}");
                out.push_str(&serialize(&buffer));
            }
        }
    }

    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(SNAPSHOT, &out).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(SNAPSHOT).unwrap_or_else(|e| {
        panic!("missing snapshot {SNAPSHOT} ({e}) — regenerate with UPDATE_SNAPSHOTS=1");
    });
    if expected != out {
        let actual_path = std::env::temp_dir().join("mjolnir-render.actual.snap");
        let _ = std::fs::write(&actual_path, &out);
        panic!("{}\n\nfull output written to {}", first_difference(&expected, &out), actual_path.display());
    }
}

/// A human-readable pointer at the first differing line, so a failure says
/// *what* moved rather than only *that* something did.
fn first_difference(expected: &str, actual: &str) -> String {
    let mut section = "<start>";
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e.starts_with("=== ") {
            section = e;
        }
        if e != a {
            return format!("render changed in {section} (line {}):\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!("render changed in length: expected {} lines, got {}", expected.lines().count(), actual.lines().count())
}

/// Replaces the three facts that vary with the build and the machine — the
/// release version, the commit, and the working directory — with fixed
/// stand-ins, so the snapshot records layout and colour rather than the
/// identity of whoever regenerated it. Chosen to be representative widths:
/// a three-part version, an 8-character short hash, and the design
/// system's own example path.
fn fixed_identity(mut app: App) -> App {
    app.status.version = "0.0.0".into();
    app.status.commit = "0badc0de".into();
    app.status.cwd = Some("~/src/gateway".into());
    app
}

fn engine() -> Arc<Engine> {
    // Leaked rather than held in a `TempDir` guard: the engine only reads
    // the (empty) config it was opened with, and every scene wants the same
    // default-deny state, so keeping the directory alive for the whole test
    // is simpler than threading a guard through each scene.
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
    Arc::new(Engine::new(config))
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| mjolnir_tui::__preview_draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// One row per line: the row's symbols verbatim, then a run-length-encoded
/// style track (`fg/bg/modifier-bits×count`). Symbols stay readable so a
/// layout diff is legible at a glance; the style track makes a pure color
/// regression just as loud as a moved character.
fn serialize(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut symbols = String::new();
        let mut styles: Vec<(String, usize)> = Vec::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            symbols.push_str(cell.symbol());
            let key = format!("{:?}/{:?}/{}", cell.fg, cell.bg, cell.modifier.bits());
            match styles.last_mut() {
                Some((last, count)) if *last == key => *count += 1,
                _ => styles.push((key, 1)),
            }
        }
        let track = styles.iter().map(|(key, count)| format!("{key}x{count}")).collect::<Vec<_>>().join(" ");
        let _ = writeln!(out, "{y:>3}|{symbols}|{track}");
    }
    out
}

fn scene(name: &str, app: &mut App) {
    match name {
        "empty" => {}
        "conversation" => conversation(app),
        "markdown" => markdown(app),
        "fenced_diff" => fenced_diff(app),
        "tools" => tools(app),
        "approval" => approval(app, SMALL_DIFF),
        "approval_large" => approval_large(app),
        "prompt" => prompt(app),
        "prompt_path" => prompt_path(app),
        "prompt_scoped" => prompt_scoped(app),
        "long" => long(app),
        other => panic!("unknown scene {other:?}"),
    }
}

const SMALL_DIFF: &str = "--- a/src/page.rs\n+++ b/src/page.rs\n@@\n fn page(items: &[Item], size: usize, n: usize) -> &[Item] {\n     let start = n * size;\n-    let end = start + size;\n+    let end = (start + size).min(items.len());\n     &items[start..end]\n }\n";

fn conversation(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "Can you refactor the retry logic in llm/src/client.rs to use exponential backoff?".into() });
    app.log.push(LogEntry::AssistantText {
        text: "Sure — here's the plan:\n\n1. Add a `backoff_ms` helper\n2. Wire it into the retry loop\n3. Cap at **5** attempts\n\n```rust\nfn backoff_ms(attempt: u32) -> u64 {\n    100 * 2u64.pow(attempt)\n}\n```\n\nThat gives `100ms, 200ms, 400ms, ...`. Want me to apply it?".into(),
    });
    app.log.push(LogEntry::Notice { message: "context file AGENTS.md injected (project scope)".into() });
    app.status.turn = Some(3);
    app.status.step = Some(2);
    app.input = "/theme light and then keep going".into();
    app.cursor = app.input.chars().count();
}

/// Exercises every inline and block markdown branch in one entry — the
/// wrapper, the heading/bullet/ordered/blockquote/rule prefixes, and the
/// nested inline delimiters.
fn markdown(app: &mut App) {
    app.log.push(LogEntry::AssistantText {
        text: "# Heading one\n### Heading three\n\nProse with **bold**, *italic*, _also italic_, `inline code`, ~~struck~~ and a [link](https://example.com/very/long/path) in it, long enough that it has to wrap across more than one row on any reasonable frame width.\n\n> A blockquote line\n\n- first bullet\n- second bullet with enough words that it wraps too\n\n1. ordered one\n2) ordered two\n\n---\n\nsupercalifragilisticexpialidociousandthensomemoretomakeitunbreakablyloooooong"
            .into(),
    });
}

fn fenced_diff(app: &mut App) {
    app.log.push(LogEntry::AssistantText { text: format!("Here is the change:\n\n```diff\n{SMALL_DIFF}```\n\nApply it?") });
}

fn tools(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "grep for TODO across the repo and summarize".into() });
    app.log.push(LogEntry::ToolActivity {
        step_id: mjolnir_core::StepId(1),
        calls:   vec![
            ToolActivityEntry { call_id: "c1".into(), name: "shell".into(), status: ToolActivityStatus::Completed { is_error: false, summary: "42 matches across 17 files".into() } },
            ToolActivityEntry { call_id: "c2".into(), name: "read".into(), status: ToolActivityStatus::Running },
            ToolActivityEntry { call_id: "c4".into(), name: "".into(), status: ToolActivityStatus::Completed { is_error: true, summary: "exit status 1".into() } },
        ],
    });
    app.turn_active = true;
    app.status.running_tools =
        vec![mjolnir_tui::__PreviewRunningTool { call_id: "c2".into(), name: "read".into() }, mjolnir_tui::__PreviewRunningTool { call_id: "c3".into(), name: "shell".into() }];
    app.status.turn = Some(4);
    app.status.step = Some(1);
}

fn approval(app: &mut App, diff: &str) {
    app.log.push(LogEntry::UserMessage { text: "fix the off-by-one in the pagination helper".into() });
    app.log.push(LogEntry::ApprovalCard { call_id: "call-1".into(), diff: diff.into(), resolution: None });
    app.pending_approvals.push_back(mjolnir_tui::__PreviewPendingApproval { call_id: "call-1".into(), diff: diff.into() });
    app.decision_selected = 1;
}

/// A diff far too tall for any frame — drives `clamp_panel`'s truncation
/// path and the elided-context marker, neither of which the small diff
/// reaches.
fn approval_large(app: &mut App) {
    let mut diff = String::from("--- a/src/big.rs\n+++ b/src/big.rs\n@@\n");
    for i in 0..40 {
        diff.push_str(&format!("+    let value_{i} = compute_something_reasonably_long({i});\n"));
    }
    for i in 0..10 {
        diff.push_str(&format!(" // untouched context line {i}\n"));
    }
    diff.push_str("-    let gone = 1;\n");
    approval(app, &diff);
}

fn prompt(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "run the test suite".into() });
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test --workspace".into(), path_like: false };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-2".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-2".into(), payload });
}

fn prompt_path(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "what does the dispatcher do on a deny-by-absence?".into() });
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tools/src/dispatcher.rs".into(), path_like: true };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-3".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-3".into(), payload });
}

/// The same path-like prompt after Tab has widened the grant to the whole
/// directory, with a second request queued behind it — covers the
/// alternate-scope hint line and the "(+N more pending)" note.
fn prompt_scoped(app: &mut App) {
    prompt_path(app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.decision_selected = 3;
    let queued = PromptPayload::Tool { kind: "read".into(), target: "./crates/core/src/lib.rs".into(), path_like: true };
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-4".into(), payload: queued });
}

fn long(app: &mut App) {
    for i in 0..8 {
        app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
        app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
    }
    app.log.push(LogEntry::TurnEnded { reason: TurnEndReasonKind::EndTurn });
    app.log.push(LogEntry::TurnEnded { reason: TurnEndReasonKind::Cancelled });
    app.log.push(LogEntry::RetryAttempt { info: mjolnir_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded, retrying".into(), attempt: 1 } });
    app.log.push(LogEntry::Error { message: "provider returned 529 overloaded".into() });
    app.log.push(LogEntry::ApprovalCard { call_id: "done-1".into(), diff: SMALL_DIFF.into(), resolution: Some(true) });
    app.log.push(LogEntry::PermissionPrompt {
        call_id:    "done-2".into(),
        payload:    PromptPayload::Tool { kind: "shell".into(), target: "ls -la".into(), path_like: false },
        resolution: Some(mjolnir_tui::PromptResolution { allowed: true, label: "allowed once".into() }),
    });
    app.status.turn = Some(9);
    app.status.step = Some(3);
}

/// Every box a frame opens, it must close.
///
/// The defect this guards: the decision panel used to be trimmed to fit by
/// a blind row budget, which could cut a diff box in half — leaving a
/// `┌───┐` on screen with no `└───┘` under it and no indication anything
/// had been dropped. Boxes are now sized against their budget before they
/// are drawn, so the count of top and bottom edges always matches.
///
/// Scoped to the scenes whose only box is the decision panel's: the panel
/// is a fixed band and cannot scroll, so a half-drawn box there is always a
/// defect. In the conversation log a box legitimately straddles the
/// viewport edge — a `└` with its `┌` scrolled off the top is what
/// scrolling looks like, not a bug.
#[test]
fn no_frame_leaves_a_bordered_box_unclosed() {
    const PANEL_SCENES: [&str; 5] = ["approval", "approval_large", "prompt", "prompt_path", "prompt_scoped"];
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in PANEL_SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let (mut opened, mut closed) = (0, 0);
                for y in 0..height {
                    for x in 0..width {
                        match buffer[(x, y)].symbol() {
                            "┌" => opened += 1,
                            "└" => closed += 1,
                            _ => {}
                        }
                    }
                }
                assert_eq!(opened, closed, "{theme:?} {scene_name} {width}x{height} left {opened} box(es) open but closed {closed}");
            }
        }
    }
}
