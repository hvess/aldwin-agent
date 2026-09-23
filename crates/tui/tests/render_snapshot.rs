//! Full-fidelity render snapshots — the automated form of the
//! "render it before trusting your reading of it" discipline in this
//! project's `CLAUDE.md`.
//!
//! Every scene below is drawn against `ratatui::backend::TestBackend` at
//! three frame sizes in both themes, and the *entire* resulting buffer —
//! every cell's symbol, foreground, background and modifiers — is
//! serialized to `tests/snapshots/render.snap`. The unit tests in
//! `ui/tests.rs` assert facts about individual rows; this asserts the whole
//! frame, colors included, which is what makes a layout refactor provably
//! output-preserving rather than merely test-passing.
//!
//! Build identity — the release version, the commit, the project, the
//! branch — is pinned per scene rather than inherited from the build (see
//! [`fixed_identity`]), so the snapshot does not encode who generated it.
//!
//! Regenerate deliberately, after eyeballing the diff:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot
//! ```

use std::fmt::Write as _;

use aldwin_core::{ChangedFile, Changeset, Event, PlanStep, Question, ReviewOutcome, StepState};
use aldwin_tui::{App, LogEntry, ModelChoice, ProviderChoice, Theme, WorkItem};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/render.snap");

/// The three sizes the review loop uses — small for vertical pressure, a
/// frame the size of the design's own (its body is 28 rows of 24px in an
/// 880px window, roughly 104×32), and a maximized terminal whose job is to
/// catch a layout that sprawls rather than one that clips.
const SIZES: [(u16, u16); 3] = [(80, 24), (104, 32), (200, 50)];

/// The ten frames of `Aldwin Agent TUI.dc.html`, by their letters, plus
/// the states the design leaves to the product.
const SCENES: [&str; 13] = [
    "launch",      // A
    "working",     // B
    "details",     // C
    "running",     // D
    "question",    // E
    "commands",    // F
    "review",      // G
    "selecting",   // H
    "commented",   // I
    "saved",       // J
    "markdown",    // a table and a fence — ADR 0002
    "failure",     // ADR 0009 §5: a sentence, no red
    "long",        // an overflowing transcript
];

/// The catalogue the questions offer. Pinned, like the identity.
fn catalogue() -> Vec<ProviderChoice> {
    vec![
        ProviderChoice {
            id:      "anthropic".into(),
            purpose: "claude models · ANTHROPIC_API_KEY".into(),
            models:  vec![
                ModelChoice { id: "claude-sonnet-5".into(), purpose: "balanced; a good default".into(), context: 1_000_000 },
                ModelChoice { id: "claude-opus-5".into(), purpose: "slower, deeper".into(), context: 1_000_000 },
            ],
        },
        ProviderChoice {
            id:      "openai".into(),
            purpose: "gpt models · OPENAI_API_KEY".into(),
            models:  vec![ModelChoice { id: "gpt-5".into(), purpose: "balanced; a good default".into(), context: 400_000 }],
        },
    ]
}

fn fixed_identity(mut app: App) -> App {
    app.status.version = "1.0.0".into();
    app.status.commit = "0000000".into();
    app = app.with_facts("gateway", Some("main"));
    app.with_catalogue(catalogue(), Some("anthropic".into()))
}

fn app(theme: Theme) -> App {
    fixed_identity(App::new("claude-sonnet-5".into()).with_theme(theme))
}

#[test]
fn every_scene_renders_exactly_as_recorded() {
    let mut out = String::new();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(theme);
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
        let actual_path = std::env::temp_dir().join("aldwin-render.actual.snap");
        let _ = std::fs::write(&actual_path, &out);
        panic!("{}\n\nfull output written to {}", first_difference(&expected, &out), actual_path.display());
    }
}

/// Every cell the app paints carries a colour from the design system —
/// `tokens.rs` is generated from `.claude/design/tokens/`, so this is the
/// design checked against the frame, cell by cell.
#[test]
fn every_cell_carries_a_colour_from_the_design_system() {
    for theme in [Theme::Dark, Theme::Light] {
        let allowed: Vec<ratatui::style::Color> = aldwin_tui::__design_palette(theme).to_vec();
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(theme);
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                for y in 0..height {
                    for x in 0..width {
                        let cell = &buffer[(x, y)];
                        assert!(allowed.contains(&cell.bg), "{theme:?} {scene_name} {width}x{height} at ({x},{y}): bg {:?} is not a design token", cell.bg);
                        if !cell.symbol().trim().is_empty() {
                            assert!(allowed.contains(&cell.fg), "{theme:?} {scene_name} {width}x{height} at ({x},{y}): fg {:?} of {:?} is not a design token", cell.fg, cell.symbol());
                        }
                    }
                }
            }
        }
    }
}

/// Every non-ASCII glyph comes from the design's closed table, or from the
/// one exception a recorded contradiction licenses (ADR 0002's table).
#[test]
fn every_glyph_comes_from_the_closed_table() {
    let (marks, by_exception) = aldwin_tui::__design_glyphs();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(theme);
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                for y in 0..height {
                    for x in 0..width {
                        for ch in buffer[(x, y)].symbol().chars() {
                            if ch.is_ascii() || marks.contains(&ch) || by_exception.contains(&ch) {
                                continue;
                            }
                            let licensed = by_exception.contains(&ch) && scene_name == "markdown";
                            assert!(licensed, "{theme:?} {scene_name} {width}x{height} at ({x},{y}): {ch:?} is not in the design's glyph table");
                        }
                    }
                }
            }
        }
    }
}

/// The one thing the transcript's text must never do is take the accent
/// — blue means you — and the one thing a failure must never be is red.
#[test]
fn the_agents_prose_is_never_blue_and_nothing_outside_a_diff_is_red() {
    for theme in [Theme::Dark, Theme::Light] {
        let pal = aldwin_tui::__design_palette(theme);
        let (accent, del, add) = (pal[0], pal[5], pal[1]); // alphabetical: accent, add, addcode, addrow, amber, del …
        for scene_name in ["working", "details", "running", "failure", "saved", "long"] {
            let mut app = app(theme);
            scene(scene_name, &mut app);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32 {
                for x in 0..104 {
                    let cell = &buffer[(x, y)];
                    if cell.symbol().trim().is_empty() {
                        continue;
                    }
                    let glyph = cell.symbol().chars().next().unwrap();
                    if cell.fg == accent {
                        assert!(!glyph.is_alphanumeric() || y >= 30, "{theme:?}/{scene_name} at ({x},{y}): {glyph:?} is prose in the accent");
                    }
                    assert_ne!(cell.fg, del, "{theme:?}/{scene_name} at ({x},{y}): red outside a diff");
                    assert_ne!(cell.fg, add, "{theme:?}/{scene_name} at ({x},{y}): green outside a diff");
                }
            }
        }
    }
}

/// Nothing is drawn inside the 3-cell left margin, and nothing inside the
/// 3-cell right margin — except the review, whose tree runs from the
/// frame's edge and whose selected row's `▎` sits in the diff pane's first
/// cell.
#[test]
fn every_conversation_scene_respects_the_three_cell_margins() {
    const MARGIN: usize = 3;
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES.iter().filter(|s| !matches!(**s, "review" | "selecting" | "commented")) {
            let mut app = app(theme);
            scene(scene_name, &mut app);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32u16 {
                let first = (0..104u16).find(|x| !buffer[(*x, y)].symbol().trim().is_empty());
                let Some(first) = first else { continue };
                assert!(first as usize >= MARGIN, "{theme:?}/{scene_name} row {y}: content starts in cell {first}, inside the margin");
                let last = (0..104u16).rev().find(|x| !buffer[(*x, y)].symbol().trim().is_empty()).unwrap();
                assert!(last as usize <= 103 - MARGIN, "{theme:?}/{scene_name} row {y}: content reaches cell {last}, inside the right margin");
            }
        }
    }
}

/// Nothing is stroked: no box-drawing glyph outside a markdown table, no
/// underline standing in for a border, and the window ground under the
/// frame's first row.
#[test]
fn nothing_inside_a_frame_is_stroked() {
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES.iter().filter(|s| **s != "markdown") {
            let mut app = app(theme);
            scene(scene_name, &mut app);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32u16 {
                for x in 0..104u16 {
                    let cell = &buffer[(x, y)];
                    assert!(!"─│┌┐└┘├┤┬┴┼╭╮╰╯┃║╔╗╚╝▁▔".contains(cell.symbol()), "{theme:?}/{scene_name} at {x},{y}: {:?} is a stroke", cell.symbol());
                }
                let underlined_blanks = (0..104u16)
                    .filter(|x| buffer[(*x, y)].modifier.contains(ratatui::style::Modifier::UNDERLINED) && buffer[(*x, y)].symbol().trim().is_empty())
                    .count();
                assert!(underlined_blanks < 52, "{theme:?}/{scene_name} row {y}: a border drawn as an attribute");
            }
        }
    }
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|f| aldwin_tui::__preview_draw(f, app)).expect("draw");
    terminal.backend().buffer().clone()
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key(KeyEvent::new(code, modifiers));
}

fn echo(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "Add rate limiting to the gateway. 100 requests a minute per API key.".into() });
}

fn work(open: bool) -> LogEntry {
    let item = |verb: &str, target: &str, fact: &str| WorkItem { call_id: target.into(), verb: verb.into(), target: target.into(), fact: Some(fact.into()), failed: false };
    LogEntry::Work { items: vec![item("Read", "src/gateway/mod.rs", "412 lines"), item("Read", "src/gateway/router.rs", "188 lines"), item("Searched", "tower::limit", "7 matches")], open }
}

fn plan(states: [StepState; 3]) -> LogEntry {
    let texts = ["Count requests per key", "Turn away requests over the limit", "Check that it works"];
    LogEntry::Plan { steps: texts.iter().zip(states).map(|(t, s)| PlanStep { text: (*t).into(), state: s }).collect() }
}

fn changeset() -> Changeset {
    let before: String = (1..=160).map(|i| format!("        .route(\"/v{i}/chat\", post(chat))\n")).collect();
    let after = before.replacen(
        "        .route(\"/v144/chat\", post(chat))\n",
        "        .layer(RateLimitLayer::new(\n            Quota::per_minute(100),\n            cfg.limit_store.clone(),\n        ))\n",
        1,
    );
    Changeset {
        files: vec![
            ChangedFile { path: "src/gateway/limit.rs".into(), before: None, after: "pub struct Limit;\n".into() },
            ChangedFile { path: "src/gateway/router.rs".into(), before: Some(before), after },
            ChangedFile { path: "tests/limit.rs".into(), before: None, after: "#[test]\nfn limits() {}\n".into() },
        ],
    }
}

fn open_review(app: &mut App) {
    echo(app);
    app.log.push(LogEntry::AssistantText { text: "Each key gets 100 requests a minute; the rest are turned away before auth.".into() });
    app.status.context_used = Some(410_000);
    app.apply_event(Event::ReviewRequested { review_id: "review-1".into(), changeset: changeset() });
    // The frame opens on the second file, with the first read.
    press(app, KeyCode::Tab, KeyModifiers::NONE);
    if let Some(r) = app.review_for_tests() {
        r.files[0].read = true;
    }
}

fn scene(name: &str, app: &mut App) {
    match name {
        "launch" => {}
        "working" => {
            echo(app);
            app.log.push(LogEntry::AssistantText { text: "Looking at how requests move through the gateway.".into() });
            app.log.push(work(false));
            app.log.push(LogEntry::AssistantText { text: "Nothing limits requests yet. Adding a limit for each key.".into() });
            app.log.push(plan([StepState::Done, StepState::Running, StepState::Pending]));
            app.turn_active = true;
            app.status.context_used = Some(380_000);
        }
        "details" => {
            scene("working", app);
            press(app, KeyCode::Char(' '), KeyModifiers::NONE);
        }
        "running" => {
            echo(app);
            app.log.push(LogEntry::AssistantText { text: "The limit is in place. Checking that it works.".into() });
            app.log.push(plan([StepState::Done, StepState::Done, StepState::Running]));
            app.log.push(LogEntry::AssistantText { text: "Running the tests. About ten seconds.".into() });
            app.turn_active = true;
            app.status.context_used = Some(410_000);
        }
        "question" => {
            echo(app);
            app.log.push(LogEntry::AssistantText { text: "The limit works for every request that carries a key.".into() });
            app.log.push(work(false));
            app.status.context_used = Some(440_000);
            app.apply_event(Event::QuestionAsked {
                call_id:  "q1".into(),
                question: Question {
                    question: "Should requests without an API key be limited too?".into(),
                    detail:   "Right now they skip the limit. Limiting them by address stops anonymous floods.".into(),
                    options:  vec!["Yes, limit them by address".into(), "No, let them through".into(), "Chat about this".into()],
                },
            });
        }
        "commands" => {
            app.log.push(LogEntry::AssistantText { text: "Done. Each key now gets 100 requests a minute, read from settings.".into() });
            app.log.push(work(false));
            app.status.context_used = Some(440_000);
            press(app, KeyCode::Char('/'), KeyModifiers::NONE);
        }
        "review" => open_review(app),
        "selecting" => {
            open_review(app);
            // Down to the first added row, then extend the selection by one.
            for _ in 0..3 {
                press(app, KeyCode::Down, KeyModifiers::NONE);
            }
            press(app, KeyCode::Down, KeyModifiers::SHIFT);
            press(app, KeyCode::Enter, KeyModifiers::NONE);
            for c in "Read the limit from config, not 100.".chars() {
                press(app, KeyCode::Char(c), KeyModifiers::NONE);
            }
        }
        "commented" => {
            scene("selecting", app);
            press(app, KeyCode::Enter, KeyModifiers::NONE);
        }
        "saved" => {
            echo(app);
            app.log.push(LogEntry::AssistantText { text: "Ready for you to review: rate limiting for the gateway.".into() });
            app.apply_event(Event::ReviewClosed { outcome: ReviewOutcome::Saved { files: vec!["a".into(), "b".into(), "c".into()], comments_resolved: 1 } });
            app.log.push(LogEntry::AssistantText { text: "Done. Each key now gets 100 requests a minute, read from settings.".into() });
            app.log.push(LogEntry::TurnBreak);
            app.status.context_used = Some(460_000);
        }
        "markdown" => {
            app.log.push(LogEntry::UserMessage { text: "which providers are set up?".into() });
            app.log.push(LogEntry::AssistantText {
                text: "Three, one per key:\n\n| provider | model | key |\n|---|---|---|\n| anthropic | claude-sonnet-5 | ANTHROPIC_API_KEY |\n| openai | gpt-5 | OPENAI_API_KEY |\n\nThe default is set in `provider.yaml`:\n\n```yaml\nprovider: anthropic\nmodel: claude-sonnet-5\n```".into(),
            });
            app.log.push(LogEntry::TurnBreak);
        }
        "failure" => {
            echo(app);
            app.log.push(LogEntry::AssistantText { text: "Running the tests.".into() });
            app.log.push(LogEntry::Failure { message: "The tests failed, 2 of 6.".into(), detail: Some("---- limit::rejects_over_quota stdout ----\nthread panicked at src/gateway/limit.rs:44".into()), open: true });
            app.log.push(LogEntry::TurnBreak);
        }
        "long" => {
            for i in 0..8 {
                app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
                app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
                app.log.push(LogEntry::TurnBreak);
            }
        }
        other => panic!("unknown scene {other:?}"),
    }
}

fn first_difference(expected: &str, actual: &str) -> String {
    let mut section = "<start>";
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e.starts_with("=== ") {
            section = e;
        }
        if e != a {
            return format!("render differs from tests/snapshots/render.snap at line {} (in {section}):\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!("render differs from tests/snapshots/render.snap in length: expected {} lines, got {}", expected.lines().count(), actual.lines().count())
}

/// One line per row: each cell as `symbol|fg|bg|modifiers`, cells joined
/// by tabs, so a diff points at a cell.
fn serialize(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let row: Vec<String> = (0..buffer.area.width)
            .map(|x| {
                let c = &buffer[(x, y)];
                format!("{}|{:?}|{:?}|{:?}", c.symbol(), c.fg, c.bg, c.modifier)
            })
            .collect();
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    out
}
