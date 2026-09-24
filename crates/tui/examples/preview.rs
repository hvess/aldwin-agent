//! Design-iteration harness — NOT part of the shipped app. Builds an `App`
//! seeded with one of the snapshot scenes and draws it once to the real
//! terminal, so the rendered output can be captured for visual review.
//! Pass a scene name as argv[1] (`launch`, `working`, `question`,
//! `commands`, `review`, `saved`) and `light` or `dark` as argv[2]. Exits
//! on a keypress.

use std::io;

use aldwin_core::{
    ChangedFile, Changeset, Event, PlanStep, Question, ReviewOutcome, StepState, TurnId,
};
use aldwin_tui::{App, LogEntry, ModelChoice, ProviderChoice, Theme, Verb, WorkItem};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;

fn main() -> io::Result<()> {
    let scene_name = std::env::args().nth(1).unwrap_or_else(|| "launch".into());
    let theme = Theme::from_config(std::env::args().nth(2).as_deref());

    let catalogue = vec![ProviderChoice {
        id: "anthropic".into(),
        purpose: "claude models · ANTHROPIC_API_KEY".into(),
        models: vec![ModelChoice {
            id: "claude-sonnet-5".into(),
            purpose: "balanced; a good default".into(),
            context: 1_000_000,
        }],
    }];
    let mut app = App::new("claude-sonnet-5".into())
        .with_theme(theme)
        .with_catalogue(catalogue, Some("anthropic".into()));
    scene(&scene_name, &mut app);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.draw(|f| aldwin_tui::draw(f, &mut app))?;

    let mut buf = [0u8; 1];
    let _ = io::Read::read(&mut io::stdin(), &mut buf);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn echo(app: &mut App) {
    app.seed(LogEntry::UserMessage {
        text: "Add rate limiting to the gateway. 100 requests a minute per API key.".into(),
    });
}

fn scene(name: &str, app: &mut App) {
    match name {
        "launch" => {}
        "working" => {
            echo(app);
            app.seed(LogEntry::AssistantText {
                text: "Looking at how requests move through the gateway.".into(),
            });
            let item = |verb, target: &str, fact: &str| WorkItem {
                call_id: target.into(),
                verb,
                target: target.into(),
                fact: Some(fact.into()),
                failed: false,
            };
            app.seed(LogEntry::Work {
                items: vec![
                    item(Verb::Read, "src/gateway/mod.rs", "412 lines"),
                    item(Verb::Searched, "tower::limit", "7 matches"),
                ],
                open: true,
            });
            app.seed(LogEntry::Plan {
                steps: vec![
                    PlanStep {
                        text: "Count requests per key".into(),
                        state: StepState::Done,
                    },
                    PlanStep {
                        text: "Turn away requests over the limit".into(),
                        state: StepState::Running,
                    },
                    PlanStep {
                        text: "Check that it works".into(),
                        state: StepState::Pending,
                    },
                ],
            });
            app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            app.status_mut().context_used = Some(380_000);
        }
        "question" => {
            echo(app);
            app.apply_event(Event::QuestionAsked {
                call_id:  "q1".into(),
                question: Question {
                    question: "Should requests without an API key be limited too?".into(),
                    detail:   "Right now they skip the limit. Limiting them by address stops anonymous floods.".into(),
                    options:  vec!["Yes, limit them by address".into(), "No, let them through".into(), "Chat about this".into()],
                },
            });
        }
        "commands" => app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        "review" => {
            echo(app);
            let before: String = (1..=40).map(|i| format!("line {i}\n")).collect();
            let after = before.replace("line 20\n", "line twenty\nline twenty-one\n");
            app.apply_event(Event::ReviewRequested {
                review_id: "r".into(),
                changeset: Changeset {
                    files: vec![
                        ChangedFile {
                            path: "src/gateway/router.rs".into(),
                            before: Some(before),
                            after,
                        },
                        ChangedFile {
                            path: "tests/limit.rs".into(),
                            before: None,
                            after: "fn t() {}\n".into(),
                        },
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
        }
        other => panic!("unknown scene {other:?}"),
    }
}
