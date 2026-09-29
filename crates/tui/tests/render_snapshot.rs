//! Every scene at three sizes in both themes, every cell's symbol, colours
//! and modifiers, pinned in `tests/snapshots/render.snap`; plus
//! design-conformance checks over the same scenes.
//!
//! Build identity is pinned by [`fixed_identity`], so the snapshot does not
//! depend on the build.
//!
//! Regenerate only after reading the diff:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot
//! ```

use std::fmt::Write as _;

use aldwin_core::{
    ChangedFile, Changeset, Event, PlanStep, Question, ReviewOutcome, StepId, StepState,
    TurnEndReason, TurnId,
};
use aldwin_tui::{
    App, CommandChoice, LogEntry, ModelChoice, ProviderChoice, SessionChoice, Theme, Verb, WorkItem,
};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/render.snap");

/// Small (vertical pressure), the design frame's own size (about 104×32),
/// and maximized (catches sprawl rather than clipping).
const SIZES: [(u16, u16); 3] = [(80, 24), (104, 32), (200, 50)];

/// Frames A–J of `Aldwin Agent TUI.dc.html`, with `sent` and `asked` (review
/// states the design leaves to the product) beside frame I so the review
/// scenes stay together, then the other states the design leaves to the
/// product.
const SCENES: [&str; 22] = [
    "launch",              // A
    "working",             // B
    "details",             // C
    "running",             // D
    "question",            // E
    "commands",            // F
    "review",              // G
    "selecting",           // H
    "commented",           // I
    "sent",                // the comments with the agent: the review waits, working
    "asked",               // the agent asks while the review waits: the question in its band
    "saved",               // J
    "markdown",            // a table, a fence, a list and a quote — ADR 0002
    "failure",             // ADR 0009 §5: a sentence, no red
    "long",                // an overflowing transcript
    "wrapped",             // a diff line wider than the pane — baseline long-diff-lines-wrap
    "stopping",            // esc mid-turn: stopped, and nothing else
    "answering",           // "Chat about this": the question stays, the turn waits on you
    "launch_unconfigured", // nothing configured: the card reads `Model  not set`
    "plan",                // a finished turn: its plan, its work folded, its prose
    "resume",              // bare `/resume` over two past sessions
    "thinking",            // a finished turn's thought, opened by Space — ADR 0015
];

/// Full-window review scenes; their tree runs from the frame's edge.
const REVIEW_SCENES: [&str; 6] = [
    "review",
    "selecting",
    "commented",
    "sent",
    "asked",
    "wrapped",
];

/// Pinned provider catalogue.
fn catalogue() -> Vec<ProviderChoice> {
    vec![
        ProviderChoice {
            id: "anthropic".into(),
            purpose: "claude models · ANTHROPIC_API_KEY".into(),
            models: vec![
                ModelChoice {
                    id: "claude-sonnet-5".into(),
                    purpose: "balanced; a good default".into(),
                    context: 1_000_000,
                },
                ModelChoice {
                    id: "claude-opus-5".into(),
                    purpose: "slower, deeper".into(),
                    context: 1_000_000,
                },
            ],
            account: None,
        },
        ProviderChoice {
            id: "openai".into(),
            purpose: "gpt models · OPENAI_API_KEY".into(),
            models: vec![ModelChoice {
                id: "gpt-5".into(),
                purpose: "balanced; a good default".into(),
                context: 400_000,
            }],
            account: None,
        },
    ]
}

/// Pinned `/` menu rows, as aldwin-cli hands them in.
fn commands() -> Vec<CommandChoice> {
    [
        ("resume", "Pick up an earlier conversation"),
        ("model", "Change the model"),
        ("quit", "Leave Aldwin"),
        ("exit", "Leave Aldwin"),
        ("clear", "Start a fresh conversation in this project"),
    ]
    .into_iter()
    .map(|(name, summary)| CommandChoice {
        name: name.into(),
        summary: summary.into(),
    })
    .collect()
}

/// Pinned past sessions for bare `/resume`, newest first; `when` arrives
/// preformatted, so no clock is involved.
fn sessions() -> Vec<SessionChoice> {
    [
        ("which providers are set up?", "2026-09-19 13:00", 1),
        ("how should the retry loop back off?", "2026-09-18 13:00", 4),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (title, when, turns))| SessionChoice {
        id: format!("session-{i}"),
        title: title.into(),
        when: when.into(),
        turns,
    })
    .collect()
}

fn fixed_identity(mut app: App, provider: Option<&str>) -> App {
    app.status_mut().version = "1.0.0".into();
    app.status_mut().commit = "0000000".into();
    app.with_facts("gateway", Some("main"))
        .with_commands(commands())
        .with_sessions(sessions())
        .with_catalogue(catalogue(), provider.map(str::to_string))
}

/// The app in `scene_name`'s state. `launch_unconfigured` has no model and
/// no provider, as aldwin-cli starts it when nothing is configured.
fn app(scene_name: &str, theme: Theme) -> App {
    let (model, provider) = match scene_name {
        "launch_unconfigured" => ("", None),
        _ => ("claude-sonnet-5", Some("anthropic")),
    };
    let mut app = fixed_identity(App::new(model.into()).with_theme(theme), provider);
    scene(scene_name, &mut app);
    app
}

#[test]
fn every_scene_renders_exactly_as_recorded() {
    let mut out = String::new();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(scene_name, theme);
                let (buffer, caret) = render_with_caret(&mut app, width, height);
                let _ = writeln!(out, "=== {theme:?} {scene_name} {width}x{height}");
                let _ = writeln!(out, "caret {caret:?}");
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
        panic!(
            "{}\n\nfull output written to {}",
            first_difference(&expected, &out),
            actual_path.display()
        );
    }
}

/// Every painted colour is a token generated from `docs/design/`.
#[test]
fn every_cell_carries_a_colour_from_the_design_system() {
    for theme in [Theme::Dark, Theme::Light] {
        let allowed: Vec<ratatui::style::Color> = aldwin_tui::design_palette(theme).to_vec();
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(scene_name, theme);
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

/// Every non-ASCII glyph is in the closed table or licensed by a recorded
/// contradiction; `nothing_inside_a_frame_is_stroked` holds ADR 0002's
/// table glyphs to the `markdown` scene.
#[test]
fn every_glyph_comes_from_the_closed_table() {
    let (marks, by_exception) = aldwin_tui::design_glyphs();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = app(scene_name, theme);
                let buffer = render(&mut app, width, height);
                for y in 0..height {
                    for x in 0..width {
                        for ch in buffer[(x, y)].symbol().chars() {
                            if ch.is_ascii() || marks.contains(&ch) {
                                continue;
                            }
                            assert!(by_exception.contains(&ch), "{theme:?} {scene_name} {width}x{height} at ({x},{y}): {ch:?} is not in the design's glyph table");
                        }
                    }
                }
            }
        }
    }
}

/// Accent blue is never on prose above the footer rows, and diff red and
/// green never appear outside a diff (ADR 0009 §5).
#[test]
fn the_agents_prose_is_never_blue_and_nothing_outside_a_diff_is_red() {
    for theme in [Theme::Dark, Theme::Light] {
        let pal = aldwin_tui::design_palette(theme);
        let (accent, del, add) = (pal[0], pal[6], pal[1]); // alphabetical: accent, add, addcode, addrow, amber, code, del …
        for scene_name in [
            "working", "details", "running", "failure", "saved", "long", "markdown", "stopping",
            "plan",
        ] {
            let mut app = app(scene_name, theme);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32 {
                for x in 0..104 {
                    let cell = &buffer[(x, y)];
                    if cell.symbol().trim().is_empty() {
                        continue;
                    }
                    let glyph = cell.symbol().chars().next().unwrap();
                    if cell.fg == accent {
                        assert!(
                            !glyph.is_alphanumeric() || y >= 30,
                            "{theme:?}/{scene_name} at ({x},{y}): {glyph:?} is prose in the accent"
                        );
                    }
                    assert_ne!(
                        cell.fg, del,
                        "{theme:?}/{scene_name} at ({x},{y}): red outside a diff"
                    );
                    assert_ne!(
                        cell.fg, add,
                        "{theme:?}/{scene_name} at ({x},{y}): green outside a diff"
                    );
                }
            }
        }
    }
}

/// Nothing is drawn in the 3-cell left or right margin; review scenes are
/// exempt, their tree running from the frame's edge.
#[test]
fn every_conversation_scene_respects_the_three_cell_margins() {
    const MARGIN: usize = 3;
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES.iter().filter(|s| !REVIEW_SCENES.contains(s)) {
            let mut app = app(scene_name, theme);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32u16 {
                let first = (0..104u16).find(|x| !buffer[(*x, y)].symbol().trim().is_empty());
                let Some(first) = first else { continue };
                assert!(first as usize >= MARGIN, "{theme:?}/{scene_name} row {y}: content starts in cell {first}, inside the margin");
                let last = (0..104u16)
                    .rev()
                    .find(|x| !buffer[(*x, y)].symbol().trim().is_empty())
                    .unwrap();
                assert!(last as usize <= 103 - MARGIN, "{theme:?}/{scene_name} row {y}: content reaches cell {last}, inside the right margin");
            }
        }
    }
}

/// No box-drawing or edge glyph outside the `markdown` scene (ADR 0002),
/// at every size.
#[test]
fn nothing_inside_a_frame_is_stroked() {
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES.iter().filter(|s| **s != "markdown") {
            for (width, height) in SIZES {
                let mut app = app(scene_name, theme);
                let buffer = render(&mut app, width, height);
                for y in 0..height {
                    for x in 0..width {
                        let cell = &buffer[(x, y)];
                        assert!(
                            !"─│┌┐└┘├┤┬┴┼╭╮╰╯┃║╔╗╚╝▁▔".contains(cell.symbol()),
                            "{theme:?}/{scene_name} {width}x{height} at {x},{y}: {:?} is a stroke",
                            cell.symbol()
                        );
                    }
                }
            }
        }
    }
}

/// An underline is a stroke, so no cell in any scene is underlined,
/// `markdown`'s headings and links included.
#[test]
fn no_cell_is_underlined() {
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            let mut app = app(scene_name, theme);
            let buffer = render(&mut app, 104, 32);
            for y in 0..32u16 {
                for x in 0..104u16 {
                    assert!(
                        !buffer[(x, y)]
                            .modifier
                            .contains(ratatui::style::Modifier::UNDERLINED),
                        "{theme:?}/{scene_name} at {x},{y}: {:?} is underlined",
                        buffer[(x, y)].symbol()
                    );
                }
            }
        }
    }
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    render_with_caret(app, width, height).0
}

/// The buffer, and the terminal cursor as `(x, y)` when shown; no cell
/// carries it.
fn render_with_caret(app: &mut App, width: u16, height: u16) -> (Buffer, Option<(u16, u16)>) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|f| aldwin_tui::draw(f, app)).expect("draw");
    let backend = terminal.backend();
    let at = backend.cursor_position();
    let caret = backend.cursor_visible().then_some((at.x, at.y));
    (backend.buffer().clone(), caret)
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key(KeyEvent::new(code, modifiers));
}

/// Types the request and sends it, as the capture scenes do: a review is
/// titled by what was typed, never by a seeded message.
fn ask(app: &mut App) {
    for c in "Add rate limiting to the gateway. 100 requests a minute per API key.".chars() {
        press(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(app, KeyCode::Enter, KeyModifiers::NONE);
}

fn echo(app: &mut App) {
    app.seed(LogEntry::UserMessage {
        text: "Add rate limiting to the gateway. 100 requests a minute per API key.".into(),
    });
}

fn work(open: bool) -> LogEntry {
    let item = |verb, target: &str, fact: &str| WorkItem {
        call_id: target.into(),
        verb,
        target: target.into(),
        fact: Some(fact.into()),
        failed: false,
    };
    LogEntry::Work {
        items: vec![
            item(Verb::Read, "src/gateway/mod.rs", "412 lines"),
            item(Verb::Read, "src/gateway/router.rs", "188 lines"),
            item(Verb::Searched, "tower::limit", "7 matches"),
        ],
        open,
    }
}

/// `seconds` into the turn, doing what `event` says since its start and
/// heard from again just now, so the phrase is typed and not stalled: frame
/// B's `1m 02s`, frame D's `1m 40s`. 100ms ticks.
fn at_work(app: &mut App, seconds: u64, event: fn() -> Event) {
    app.apply_event(event());
    app.advance(seconds * 10);
    app.apply_event(event());
}

/// A reasoning delta that adds nothing to the log.
fn thought() -> Event {
    Event::ThinkingDelta {
        turn_id: TurnId(1),
        step_id: StepId(1),
        text: String::new(),
    }
}

/// A prose delta that adds nothing to the log.
fn prose() -> Event {
    Event::TextDelta {
        turn_id: TurnId(1),
        step_id: StepId(1),
        text: String::new(),
    }
}

fn plan(states: [StepState; 3]) -> LogEntry {
    let texts = [
        "Count requests per key",
        "Turn away requests over the limit",
        "Check that it works",
    ];
    LogEntry::Plan {
        steps: texts
            .iter()
            .zip(states)
            .map(|(t, s)| PlanStep {
                text: (*t).into(),
                state: s,
            })
            .collect(),
    }
}

fn changeset() -> Changeset {
    let before: String = (1..=160)
        .map(|i| format!("        .route(\"/v{i}/chat\", post(chat))\n"))
        .collect();
    let after = before.replacen(
        "        .route(\"/v144/chat\", post(chat))\n",
        "        .layer(RateLimitLayer::new(\n            Quota::per_minute(100),\n            cfg.limit_store.clone(),\n        ))\n",
        1,
    );
    Changeset {
        files: vec![
            ChangedFile {
                path: "src/gateway/limit.rs".into(),
                before: None,
                after: "pub struct Limit;\n".into(),
            },
            ChangedFile {
                path: "src/gateway/router.rs".into(),
                before: Some(before),
                after,
            },
            ChangedFile {
                path: "tests/limit.rs".into(),
                before: None,
                after: "#[test]\nfn limits() {}\n".into(),
            },
        ],
    }
}

fn open_review(app: &mut App) {
    ask(app);
    app.seed(LogEntry::AssistantText {
        text: "Each key gets 100 requests a minute; the rest are turned away before auth.".into(),
    });
    app.status_mut().context_used = Some(410_000);
    app.apply_event(Event::ReviewRequested {
        review_id: "review-1".into(),
        changeset: changeset(),
    });
    // The frame opens on the second file, with the first read.
    if let Some(r) = app.review_for_tests() {
        r.mark_read();
    }
    press(app, KeyCode::Tab, KeyModifiers::NONE);
}

fn scene(name: &str, app: &mut App) {
    match name {
        "launch" => {}
        "working" => {
            echo(app);
            app.seed(LogEntry::AssistantText {
                text: "Looking at how requests move through the gateway.".into(),
            });
            app.seed(work(false));
            app.seed(LogEntry::AssistantText {
                text: "Nothing limits requests yet. Adding a limit for each key.".into(),
            });
            app.seed(plan([
                StepState::Done,
                StepState::Running,
                StepState::Pending,
            ]));
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            at_work(app, 62, thought);
            app.status_mut().context_used = Some(380_000);
        }
        "details" => {
            scene("working", app);
            press(app, KeyCode::Char(' '), KeyModifiers::NONE);
        }
        "running" => {
            echo(app);
            app.seed(LogEntry::AssistantText {
                text: "The limit is in place. Checking that it works.".into(),
            });
            app.seed(plan([StepState::Done, StepState::Done, StepState::Running]));
            app.seed(LogEntry::AssistantText {
                text: "Running the tests. About ten seconds.".into(),
            });
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            // Capture's `running` holds the turn after its prose streams.
            at_work(app, 100, prose);
            app.status_mut().context_used = Some(410_000);
        }
        "question" => {
            echo(app);
            app.seed(LogEntry::AssistantText {
                text: "The limit works for every request that carries a key.".into(),
            });
            app.seed(work(false));
            app.status_mut().context_used = Some(440_000);
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
            app.seed(LogEntry::AssistantText {
                text: "Done. Each key now gets 100 requests a minute, read from settings.".into(),
            });
            app.seed(work(false));
            app.status_mut().context_used = Some(440_000);
            // Frame F types `/c`: narrowed, and completed in grey.
            press(app, KeyCode::Char('/'), KeyModifiers::NONE);
            press(app, KeyCode::Char('c'), KeyModifiers::NONE);
        }
        "review" => open_review(app),
        "selecting" => {
            open_review(app);
            // Added lines 144–145, as a drag leaves them; the mouse itself
            // is tested by `Review::handle_mouse`'s unit tests.
            if let Some(r) = app.review_for_tests() {
                r.select(3, 4);
            }
            for c in "Read the limit from config, not 100.".chars() {
                press(app, KeyCode::Char(c), KeyModifiers::NONE);
            }
        }
        "commented" => {
            scene("selecting", app);
            press(app, KeyCode::Enter, KeyModifiers::NONE);
        }
        "sent" => {
            // `⌃↩` sends the comment; core answers and starts the
            // follow-up turn, which the review waits inside.
            scene("commented", app);
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            press(app, KeyCode::Enter, KeyModifiers::CONTROL);
            app.apply_event(Event::ReviewClosed {
                outcome: ReviewOutcome::Commented { comments: 1 },
            });
            app.apply_event(Event::TurnEnded {
                turn_id: TurnId(1),
                reason: TurnEndReason::EndTurn,
            });
            app.apply_event(Event::FollowUp {
                turn_id: TurnId(2),
                text: "On src/gateway/router.rs, lines 4–5:\nRead the limit from config, not 100."
                    .into(),
            });
            app.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
            at_work(app, 4, thought);
        }
        "asked" => {
            scene("sent", app);
            app.apply_event(Event::QuestionAsked {
                call_id: "call-ask".into(),
                question: Question {
                    question: "Should requests without an API key be limited too?".into(),
                    detail: "Right now they skip the limit. Limiting them by address stops anonymous floods.".into(),
                    options: vec![
                        "Yes, limit them by address".into(),
                        "No, let them through".into(),
                        "Chat about this".into(),
                    ],
                },
            });
        }
        "saved" => {
            echo(app);
            app.apply_event(Event::ReviewClosed {
                outcome: ReviewOutcome::Saved {
                    files: vec!["a".into(), "b".into(), "c".into()],
                    comments_resolved: 1,
                },
            });
            app.seed(LogEntry::AssistantText {
                text: "Done. Each key now gets 100 requests a minute, read from settings.".into(),
            });
            app.seed(LogEntry::TurnBreak);
            app.status_mut().context_used = Some(460_000);
        }
        "markdown" => {
            app.seed(LogEntry::UserMessage {
                text: "which providers are set up?".into(),
            });
            app.seed(LogEntry::AssistantText {
                text: "## Providers\n\nThree, one per key — see [the docs](https://docs.example/providers):\n\n| provider | model | key |\n|---|---|---|\n| anthropic | claude-sonnet-5 | ANTHROPIC_API_KEY |\n| openai | gpt-5 | OPENAI_API_KEY |\n\nEach one needs:\n\n- its key exported\n- a model it offers\n\n> The key never goes in a file.\n\nThe default is set in `provider.yaml`:\n\n```yaml\nprovider: anthropic\nmodel: claude-sonnet-5\n```".into(),
            });
            app.seed(LogEntry::TurnBreak);
        }
        "failure" => {
            echo(app);
            app.seed(LogEntry::AssistantText {
                text: "Running the tests.".into(),
            });
            app.seed(LogEntry::Failure { message: "The tests failed, 2 of 6.".into(), detail: Some("---- limit::rejects_over_quota stdout ----\nthread panicked at src/gateway/limit.rs:44".into()), open: true });
            app.seed(LogEntry::TurnBreak);
        }
        "long" => {
            for i in 0..8 {
                app.seed(LogEntry::UserMessage {
                    text: format!("message {i}"),
                });
                app.seed(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
                app.seed(LogEntry::TurnBreak);
            }
        }
        "wrapped" => {
            ask(app);
            app.apply_event(Event::ReviewRequested {
                review_id: "review-1".into(),
                changeset: Changeset {
                    files: vec![ChangedFile {
                        path: "src/gateway/limit.rs".into(),
                        before: Some("pub struct Limit;\n".into()),
                        after: "pub struct Limit;\n\nconst MESSAGE: &str = \"This key has made more than its 100 requests this minute; wait for the next minute, or ask for a higher limit.\";\n".into(),
                    }],
                },
            });
        }
        "stopping" => {
            // The settled state: core ends the turn as cancelled at once
            // after `esc`, so the in-between frame is never seen.
            scene("running", app);
            press(app, KeyCode::Esc, KeyModifiers::NONE);
            app.apply_event(Event::TurnEnded {
                turn_id: TurnId(1),
                reason: TurnEndReason::Cancelled,
            });
        }
        "answering" => {
            scene("question", app);
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            press(app, KeyCode::Char('3'), KeyModifiers::NONE);
            for c in "Only the ones".chars() {
                press(app, KeyCode::Char(c), KeyModifiers::NONE);
            }
        }
        "launch_unconfigured" => {}
        "plan" => {
            // A finished turn: the work folds to its summary and the
            // still-running step goes back to pending.
            echo(app);
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            app.seed(LogEntry::Work {
                items: vec![WorkItem {
                    call_id: "call-read".into(),
                    verb: Verb::Read,
                    target: "src/gateway/router.rs".into(),
                    fact: Some("6 lines".into()),
                    failed: false,
                }],
                open: false,
            });
            app.seed(plan([
                StepState::Done,
                StepState::Running,
                StepState::Pending,
            ]));
            app.seed(LogEntry::AssistantText {
                text: "Looking at how requests move through the gateway. Every request passes auth and tracing and nothing counts them, so a limit belongs beside the auth layer where the key is already known.".into(),
            });
            app.status_mut().context_used = Some(380_000);
            app.apply_event(Event::TurnEnded {
                turn_id: TurnId(1),
                reason: TurnEndReason::EndTurn,
            });
        }
        "thinking" => {
            // Capture's fake answers at once: the block takes its 1s floor.
            echo(app);
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            let (turn_id, step_id) = (TurnId(1), StepId(1));
            app.apply_event(Event::ThinkingStart { turn_id, step_id });
            app.apply_event(Event::ThinkingDelta {
                turn_id,
                step_id,
                text: "The request is a limit per API key. The router adds auth and then tracing, and the key is only known after auth, so the limit goes right after it. The quota should come from config rather than a literal.".into(),
            });
            app.apply_event(Event::ThinkingEnd {
                turn_id,
                step_id,
                seconds: Some(1),
            });
            app.apply_event(Event::TextDelta {
                turn_id,
                step_id,
                text: "A limit fits beside the auth layer, where the key is already known.".into(),
            });
            app.apply_event(Event::TurnEnded {
                turn_id,
                reason: TurnEndReason::EndTurn,
            });
            press(app, KeyCode::Char(' '), KeyModifiers::NONE);
        }
        "resume" => {
            // Bare `/resume`, chosen from the menu: the session question.
            press(app, KeyCode::Char('/'), KeyModifiers::NONE);
            press(app, KeyCode::Char('r'), KeyModifiers::NONE);
            press(app, KeyCode::Enter, KeyModifiers::NONE);
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
    format!(
        "render differs from tests/snapshots/render.snap in length: expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

/// One line per row, cells as `symbol|fg|bg|modifiers` joined by tabs, so
/// a diff points at a cell.
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
