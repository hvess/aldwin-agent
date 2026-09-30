//! The design's rules, checked on `TestBackend` frames. Every cell of every
//! scene is pinned separately by `tests/render_snapshot.rs`.

use aldwin_core::{
    ChangedFile, Changeset, Event, PlanStep, Question, ReviewOutcome, StepId, StepState, ToolCall,
    ToolResult, TurnEndReason, TurnId,
};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::style::Modifier;
use ratatui::Terminal;

use super::grid::{Ctx, BODY_X, COMMAND_COL, GUTTER_LN, MARGIN_X, MARK_COL, NUMBER_COL, SIGN_COL};
use super::markdown::render_prose;
use super::question::{OPTION_INSET, PANEL_PAD};
use crate::app::{App, ModelChoice, ProviderChoice};
use crate::log::{Act, LogEntry, Took, Verb, WorkItem};
use crate::motion::Motion;
use crate::palette::Theme;
use crate::review::SelectionLabel;
use crate::tokens::{GAUGE_CELL, GROUP_GAP, MARK_COLS, MARK_ROWS};

fn app() -> App {
    App::new("claude-sonnet-5".into())
        .with_facts("gateway", Some("main"))
        .with_commands(crate::app::tests::commands())
        .with_catalogue(
            vec![ProviderChoice {
                id: "anthropic".into(),
                purpose: "claude models".into(),
                models: vec![ModelChoice {
                    id: "claude-sonnet-5".into(),
                    purpose: "balanced".into(),
                    context: 1_000_000,
                }],
                account: None,
            }],
            Some("anthropic".into()),
        )
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| super::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// The terminal cursor's position after a draw, if shown.
fn caret(app: &mut App, width: u16, height: u16) -> Option<(u16, u16)> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| super::draw(f, app)).unwrap();
    let backend = terminal.backend();
    let at = backend.cursor_position();
    backend.cursor_visible().then_some((at.x, at.y))
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect::<String>()
}

fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    (0..buf.area.height).find(|&y| row_text(buf, y).contains(needle))
}

/// The footer's row: the one the context bar is drawn on.
fn footer_row(buf: &Buffer) -> u16 {
    find_row(buf, &GAUGE_CELL.to_string()).expect("every screen draws the context bar")
}

/// The footer before its context bar, from the mark column: `○ Ready`.
fn footer_lead(buf: &Buffer) -> String {
    let row = row_text(buf, footer_row(buf));
    row.split(GAUGE_CELL)
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn col_of(buf: &Buffer, y: u16, needle: &str) -> Option<usize> {
    row_text(buf, y)
        .find(needle)
        .map(|byte| row_text(buf, y)[..byte].chars().count())
}

#[test]
fn the_launch_card_leads_with_the_mark_and_the_four_facts() {
    let mut a = app();
    let buf = render(&mut a, 100, 36);
    // Row 0 is the body's top padding, rows 1–2 the card's own blanks.
    for y in 0..3 {
        assert!(row_text(&buf, y).trim().is_empty(), "row {y} is blank");
    }
    let name = find_row(&buf, "Aldwin").expect("the name row");
    assert_eq!(
        name,
        3 + (MARK_ROWS as u16 - 4) / 2,
        "the four facts centred against the mark"
    );
    assert!(buf[(col_of(&buf, name, "Aldwin").unwrap() as u16, name)]
        .modifier
        .contains(Modifier::BOLD));
    assert_eq!(
        col_of(&buf, name, "Aldwin"),
        Some(BODY_X + MARK_COLS + BODY_X),
        "mark at 5ch, a 5ch gap"
    );
    assert_eq!(
        col_of(&buf, name + 1, "Project"),
        Some(BODY_X + MARK_COLS + BODY_X)
    );
    assert_eq!(
        col_of(&buf, name + 1, "gateway"),
        Some(BODY_X + MARK_COLS + BODY_X + 10),
        "the value at --fact-col"
    );
    assert!(
        row_text(&buf, name + 2).contains("Branch") && row_text(&buf, name + 2).contains("main")
    );
    assert!(
        row_text(&buf, name + 3).contains("Model")
            && row_text(&buf, name + 3).contains("claude-sonnet-5")
    );
    // The mark is half-block cells, not a glyph.
    assert_eq!(buf[(BODY_X as u16 + 6, 3)].symbol(), "▀");
}

#[test]
fn the_field_is_at_the_margin_with_the_prompt_in_the_mark_column() {
    let mut a = app();
    let buf = render(&mut a, 100, 36);
    let y = find_row(&buf, "›").expect("the prompt");
    assert_eq!(col_of(&buf, y, "›"), Some(MARGIN_X));
    assert_eq!(
        row_text(&buf, y).trim(),
        "›",
        "an empty field carries no placeholder"
    );
    let pal = Theme::Dark.palette();
    assert_eq!(
        caret(&mut a, 100, 36),
        Some((BODY_X as u16, y)),
        "the caret sits on the body column"
    );
    assert_eq!(
        buf[(BODY_X as u16, y)].bg,
        pal.field,
        "the caret is the cursor's bar, not a painted cell"
    );
    assert_eq!(buf[(MARGIN_X as u16, y)].fg, pal.accent, "blue means you");
    assert_eq!(buf[(MARGIN_X as u16, y)].bg, pal.field);
    assert_eq!(
        buf[(MARGIN_X as u16 - 1, y)].bg,
        pal.win,
        "the margin is the window ground"
    );
    // The footer, two rows below.
    let footer = row_text(&buf, y + 2);
    assert_eq!(col_of(&buf, y + 2, "Ready"), Some(BODY_X));
    assert!(footer.contains("/ Commands"));
    assert!(footer.trim_end().ends_with("0%"), "{footer:?}");
    assert!(
        row_text(&buf, y + 3).trim().is_empty(),
        "one blank row closes the window"
    );
}

#[test]
fn the_echoed_prompt_is_a_tint_band_and_prose_is_at_the_body_column() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage {
        text: "Add rate limiting to the gateway.".into(),
    });
    a.log.push(LogEntry::AssistantText {
        text: "Looking at how requests move through the gateway.".into(),
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let echo = find_row(&buf, "Add rate limiting").unwrap();
    assert_eq!(echo, 1, "the first entry sits under the top padding");
    assert_eq!(col_of(&buf, echo, "›"), Some(MARGIN_X));
    assert_eq!(
        buf[(MARGIN_X as u16, echo)].fg,
        pal.label3,
        "the echo's mark is label3, its words label2"
    );
    assert_eq!(buf[(BODY_X as u16, echo)].fg, pal.label2);
    assert_eq!(buf[(MARGIN_X as u16, echo)].bg, pal.tint);
    assert_eq!(
        buf[(96, echo)].bg,
        pal.tint,
        "the band runs to the right margin"
    );
    assert_eq!(buf[(97, echo)].bg, pal.win);
    let prose = find_row(&buf, "Looking at").unwrap();
    assert_eq!(prose, echo + 2, "one blank row between groups");
    assert_eq!(col_of(&buf, prose, "Looking"), Some(BODY_X));
    assert_eq!(buf[(BODY_X as u16, prose)].fg, pal.label);
}

#[test]
fn the_plan_marks_done_running_and_pending_in_their_three_tones() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.log.push(LogEntry::Plan {
        steps: vec![
            PlanStep {
                text: "Count requests per key".into(),
                state: StepState::Done,
                file: None,
                note: None,
            },
            PlanStep {
                text: "Turn away requests over the limit".into(),
                state: StepState::Running,
                file: None,
                note: None,
            },
            PlanStep {
                text: "Check that it works".into(),
                state: StepState::Pending,
                file: None,
                note: None,
            },
        ],
        docked: false,
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "Count requests").unwrap();
    assert_eq!(col_of(&buf, y, "✓"), Some(MARGIN_X));
    assert_eq!(col_of(&buf, y, "Count"), Some(BODY_X));
    assert_eq!(buf[(MARGIN_X as u16, y)].fg, pal.accent);
    assert_eq!(buf[(BODY_X as u16, y)].fg, pal.label2);
    assert_eq!(buf[(MARGIN_X as u16, y + 1)].symbol(), "●");
    assert_eq!(
        buf[(MARGIN_X as u16, y + 1)].fg,
        pal.amber,
        "amber means running"
    );
    assert_eq!(buf[(BODY_X as u16, y + 1)].fg, pal.label);
    assert_eq!(buf[(MARGIN_X as u16, y + 2)].symbol(), "○");
    assert_eq!(buf[(MARGIN_X as u16, y + 2)].fg, pal.label3);
    assert_eq!(
        buf[(BODY_X as u16, y + 2)].fg,
        pal.label2,
        "frame B: a pending step's text is label2"
    );
}

/// Frame P's turn: three steps with their files, the middle one running
/// with a note, and two files staged.
fn drafting() -> App {
    let mut a = app();
    for c in "Add rate limiting to the gateway.".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    let step = |text: &str, state, file: &str, note: Option<&str>| PlanStep {
        text: text.into(),
        state,
        file: Some(file.into()),
        note: note.map(Into::into),
    };
    a.apply_event(Event::PlanUpdated {
        turn_id: TurnId(1),
        steps: vec![
            step("Count requests per key", StepState::Done, "limit.rs", None),
            step(
                "Turn away requests over the limit",
                StepState::Running,
                "router.rs",
                Some("Adding the limiter to `build_stack`"),
            ),
            step(
                "Check that it works",
                StepState::Pending,
                "tests/limit.rs",
                None,
            ),
        ],
    });
    let limit = "x\n".repeat(48);
    for file in [
        ChangedFile {
            path: "src/gateway/limit.rs".into(),
            before: None,
            after: limit,
        },
        ChangedFile {
            path: "src/gateway/router.rs".into(),
            before: Some("a\nb\nc\n".into()),
            after: "a\nB\nc\nd\ne\nf\ng\nh\n".into(),
        },
    ] {
        a.apply_event(Event::Staged { file });
    }
    a
}

/// Frame P: the plan docked above the field on `tint`, its rows inset one
/// cell, each step's file and counts flush right with a mark column spare.
#[test]
fn the_plan_card_holds_the_plan_above_the_field_while_edits_are_staged() {
    let mut a = drafting();
    let (width, height) = (100, 36);
    let buf = render(&mut a, width, height);
    let pal = Theme::Dark.palette();

    let title = find_row(&buf, "Draft, nothing saved").expect("the card's title row");
    let glyph_x = MARGIN_X + OPTION_INSET;
    let text_x = glyph_x + MARK_COL;
    // The right column ends a mark column and the inset inside the card.
    let right_end = width as usize - MARGIN_X - OPTION_INSET - MARK_COL;
    assert_eq!(
        col_of(&buf, title, "Add rate limiting to the gateway"),
        Some(text_x)
    );
    assert_eq!(buf[(text_x as u16, title)].fg, pal.label);
    assert_eq!(
        col_of(&buf, title, "Draft, nothing saved"),
        Some(right_end - "Draft, nothing saved".len())
    );
    assert_eq!(buf[(MARGIN_X as u16, title)].bg, pal.tint);
    assert_eq!(buf[(MARGIN_X as u16 - 1, title)].bg, pal.win);
    assert_eq!(
        buf[(MARGIN_X as u16, title - 1)].bg,
        pal.tint,
        "a blank row pads the top"
    );

    let done = title + 2;
    assert_eq!(buf[(glyph_x as u16, done)].symbol(), "✓");
    assert_eq!(buf[(glyph_x as u16, done)].fg, pal.accent);
    assert_eq!(col_of(&buf, done, "Count requests per key"), Some(text_x));
    assert_eq!(
        buf[(text_x as u16, done)].fg,
        pal.label3,
        "frame P: done steps back to label3"
    );
    assert!(
        row_text(&buf, done).contains("limit.rs  +48 "),
        "a new file has no −0"
    );

    let running = done + 1;
    assert_eq!(buf[(glyph_x as u16, running)].fg, pal.amber);
    assert_eq!(buf[(text_x as u16, running)].fg, pal.label);
    let counts = "router.rs  +6 −1";
    let at = col_of(&buf, running, counts).expect("the running step's file and counts");
    assert_eq!(at + counts.chars().count(), right_end);
    assert_eq!(buf[(at as u16, running)].fg, pal.label2);
    assert_eq!(buf[((at + 11) as u16, running)].fg, pal.add);
    assert_eq!(buf[((at + 14) as u16, running)].fg, pal.del);

    let note = running + 1;
    assert_eq!(
        col_of(&buf, note, "Adding the limiter to build_stack"),
        Some(text_x)
    );
    assert_eq!(buf[(text_x as u16, note)].fg, pal.label2);
    let code = col_of(&buf, note, "build_stack").unwrap();
    assert_eq!(
        buf[(code as u16, note)].fg,
        pal.code,
        "a name in the note is code"
    );

    let pending = note + 1;
    assert_eq!(buf[(glyph_x as u16, pending)].symbol(), "○");
    assert_eq!(buf[(text_x as u16, pending)].fg, pal.label3);
    assert!(row_text(&buf, pending)
        .trim_end()
        .ends_with("tests/limit.rs"));

    // Its bottom pad, a blank row on the window, then the field.
    assert_eq!(buf[(MARGIN_X as u16, pending + 1)].bg, pal.tint);
    assert_eq!(buf[(MARGIN_X as u16, pending + 2)].bg, pal.win);
    assert_eq!(buf[(MARGIN_X as u16, pending + 3)].symbol(), "›");
    assert_eq!(
        (0..height)
            .filter(|&y| row_text(&buf, y).contains("Count requests per key"))
            .count(),
        1,
        "the docked plan is not drawn in the conversation too"
    );
}

#[test]
fn a_file_name_two_staged_files_answer_to_gets_no_counts() {
    let mut a = drafting();
    a.apply_event(Event::Staged {
        file: ChangedFile {
            path: "tests/limit.rs".into(),
            before: None,
            after: "x\n".into(),
        },
    });
    let buf = render(&mut a, 100, 36);
    let done = find_row(&buf, "Count requests per key").unwrap();
    assert!(
        row_text(&buf, done).trim_end().ends_with("limit.rs"),
        "`limit.rs` names both staged files"
    );
    let pending = find_row(&buf, "Check that it works").unwrap();
    assert!(row_text(&buf, pending).contains("tests/limit.rs  +1"));
}

/// The comments' round edits under the waiting review, which holds the
/// screen: its card is never drawn over the review.
#[test]
fn a_review_holds_the_screen_over_the_next_rounds_card() {
    let mut a = drafting();
    a.apply_event(Event::ReviewRequested {
        review_id: "r".into(),
        changeset: Changeset {
            files: vec![ChangedFile {
                path: "src/gateway/router.rs".into(),
                before: None,
                after: "x\n".into(),
            }],
        },
    });
    a.apply_event(Event::Staged {
        file: ChangedFile {
            path: "src/gateway/router.rs".into(),
            before: None,
            after: "y\n".into(),
        },
    });
    let buf = render(&mut a, 100, 36);
    assert!(find_row(&buf, "Nothing is saved until you approve").is_some());
    assert!(find_row(&buf, "Draft, nothing saved").is_none());
}

#[test]
fn a_question_from_the_agent_takes_the_cards_place() {
    let mut a = drafting();
    a.apply_event(Event::QuestionAsked {
        call_id: "q1".into(),
        question: Question {
            question: "Limit requests without a key too?".into(),
            detail: String::new(),
            options: vec!["Yes".into(), "No".into(), "Chat about this".into()],
        },
    });
    let buf = render(&mut a, 100, 36);
    assert!(find_row(&buf, "Limit requests without a key too?").is_some());
    assert!(find_row(&buf, "Draft, nothing saved").is_none());
}

/// Frames B, C and J.
#[test]
fn a_disclosure_glyph_is_in_the_rows_tone() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.log.push(LogEntry::Work {
        acts: vec![read_call()],
        open: false,
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "Read 1 file").unwrap();
    let glyph = col_of(&buf, y, "›").unwrap() as u16;
    assert_eq!(buf[(glyph, y)].fg, pal.label2);
}

fn read_call() -> Act {
    Act::Call(WorkItem {
        call_id: "c".into(),
        verb: Verb::Read,
        target: "src/x.rs".into(),
        fact: Some("6 lines".into()),
        failed: false,
    })
}

/// ADR 0015 and 0018: a turn's thoughts and calls are one `Disclosure` on
/// the prose column; opened, each thought's reasoning sits under its row,
/// in `label2`, wrapped, not shortened, in the order it happened.
#[test]
fn a_turns_thoughts_and_calls_are_one_disclosure() {
    let mut a = app();
    let reasoning = "The limit belongs beside auth, where the key is already known. ".repeat(3);
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.log.push(LogEntry::Work {
        acts: vec![
            Act::Thought {
                text: format!("{reasoning}\nThen a test."),
                took: Took::Seconds(12),
            },
            read_call(),
        ],
        open: false,
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "Thought for 12s · Read 1 file  ›").expect("the summary");
    assert_eq!(col_of(&buf, y, "Thought"), Some(BODY_X));
    assert_eq!(buf[(BODY_X as u16, y)].fg, pal.label2);
    assert!(find_row(&buf, "Then a test.").is_none(), "closed");

    if let Some(LogEntry::Work { open, .. }) = a.log.last_mut() {
        *open = true;
    }
    let buf = render(&mut a, 100, 36);
    assert!(find_row(&buf, "Thought for 12s · Read 1 file  ⌄").is_some());
    let first = find_row(&buf, "The limit belongs").unwrap();
    assert_eq!(col_of(&buf, first, "The limit"), Some(BODY_X));
    assert_eq!(buf[(BODY_X as u16, first)].fg, pal.label2);
    let last = find_row(&buf, "Then a test.").expect("every line, unabridged");
    assert!(last > first + 1, "the long line wraps");
    let call = find_row(&buf, "src/x.rs").expect("the call's row");
    assert!(call > last, "in the order it happened");
}

/// Frame J: `Send  ↩` right-flush, one cell in, `label3` until there is a
/// draft.
#[test]
fn after_a_save_the_field_offers_send() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.apply_event(Event::ReviewClosed {
        outcome: ReviewOutcome::Saved {
            files: vec!["a".into()],
            comments_resolved: 0,
        },
    });
    a.apply_event(Event::TurnEnded {
        turn_id: TurnId(1),
        reason: TurnEndReason::EndTurn,
    });
    let pal = Theme::Dark.palette();
    let buf = render(&mut a, 100, 36);
    let y = find_row(&buf, "Send  ↩").expect("the action");
    let right = buf.area.width - MARGIN_X as u16;
    assert_eq!(buf[(right - 2, y)].symbol(), "↩");
    assert_eq!(buf[(right - 2, y)].fg, pal.label3, "grey while empty");
    a.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    assert_eq!(buf[(right - 2, y)].fg, pal.accent, "ready once typed");
}

/// Frames B and C: Space opens the work but is never named in the footer.
#[test]
fn no_footer_names_the_details_key() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.log.push(LogEntry::Work {
        acts: vec![read_call()],
        open: false,
    });
    let footer = |a: &mut App| footer_lead(&render(a, 100, 36));
    assert_eq!(
        footer(&mut a),
        "● Thi  0m 00s",
        "the working line names no key"
    );
    a.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(a.details_open, "Space still opens the work");
    assert!(!footer(&mut a).contains("Space"));
    a.apply_event(Event::TurnEnded {
        turn_id: TurnId(1),
        reason: TurnEndReason::EndTurn,
    });
    assert!(
        !footer(&mut a).contains("Space"),
        "nor once the turn is over"
    );
}

/// Every frame's prose `padding: 0 5ch`.
#[test]
fn prose_wraps_as_far_from_the_right_edge_as_it_starts_from_the_left() {
    let mut a = app();
    a.log.push(LogEntry::AssistantText {
        text: "word ".repeat(80),
    });
    let width = 80;
    let buf = render(&mut a, width, 36);
    let rows: Vec<String> = (0..buf.area.height)
        .map(|y| row_text(&buf, y))
        .filter(|r| r.contains("word"))
        .collect();
    assert!(rows.len() > 1, "the text wraps");
    for row in rows {
        assert!(
            row.trim_end().chars().count() <= (width as usize) - BODY_X,
            "{row:?} runs past the prose column"
        );
    }
}

#[test]
fn a_question_takes_the_band_on_the_panel_ground_with_its_current_row_on_field() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.apply_event(Event::QuestionAsked {
        call_id: "q".into(),
        question: Question {
            question: "Should requests without an API key be limited too?".into(),
            detail: "Right now they skip the limit.".into(),
            options: vec![
                "Yes, limit them by address".into(),
                "No, let them through".into(),
                "Chat about this".into(),
            ],
        },
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    // Frame E's insets: see `question::PANEL_PAD`.
    let right = buf.area.width - 1;
    let (panel_x, text_x, option_x) = (
        MARGIN_X as u16,
        (MARGIN_X + PANEL_PAD) as u16,
        (MARGIN_X + OPTION_INSET) as u16,
    );
    let (number_x, answer_x) = (
        option_x + MARK_COL as u16,
        option_x + (MARK_COL + NUMBER_COL) as u16,
    );
    let q = find_row(&buf, "Should requests").unwrap();
    assert!(buf[(text_x, q)].modifier.contains(Modifier::BOLD));
    assert_eq!(
        col_of(&buf, q, "Should"),
        Some(text_x as usize),
        "the question sits 3ch inside the panel"
    );
    assert_eq!(
        buf[(panel_x, q)].bg,
        pal.panel,
        "the panel starts at the margin"
    );
    assert_eq!(
        buf[(right - MARGIN_X as u16, q)].bg,
        pal.panel,
        "and ends at the right margin"
    );
    assert_eq!(buf[(0, q)].bg, pal.win, "the margin stays window ground");
    assert_eq!(buf[(right, q)].bg, pal.win);
    assert!(
        row_text(&buf, q - 1).trim().is_empty() && buf[(panel_x, q - 1)].bg == pal.panel,
        "a blank panel row above the question"
    );
    let opt = find_row(&buf, "Yes, limit").unwrap();
    assert_eq!(opt, q + 3, "question, detail, blank, options");
    assert_eq!(col_of(&buf, opt, "›"), Some(option_x as usize));
    assert_eq!(col_of(&buf, opt, "1"), Some(number_x as usize));
    assert_eq!(
        col_of(&buf, opt, "Yes"),
        Some(answer_x as usize),
        "the text after --number-col"
    );
    assert_eq!(
        buf[(number_x, opt)].fg,
        pal.label2,
        "the current option's number is label2"
    );
    assert_eq!(
        buf[(number_x, opt + 1)].fg,
        pal.label3,
        "the others' are label3"
    );
    assert_eq!(
        buf[(option_x, opt)].bg,
        pal.field,
        "the current row on --field"
    );
    assert_eq!(
        buf[(panel_x, opt)].bg,
        pal.panel,
        "inset 1ch inside the panel"
    );
    assert_eq!(buf[(option_x, opt + 1)].bg, pal.panel);
    let footer = find_row(&buf, "Waiting for you").unwrap();
    assert!(
        row_text(&buf, footer).contains("○ Waiting for you     ↑↓ Choose     ↩ Select"),
        "frame E's footer"
    );
    assert!(
        !row_text(&buf, footer).contains("›"),
        "no field while a question is open"
    );
}

/// Frame F.
#[test]
fn the_command_menu_is_a_panel_on_the_field() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "resume").unwrap();
    assert!(!row_text(&buf, y).contains('/'), "no slash on the names");
    assert_eq!(col_of(&buf, y, "›"), Some(MARGIN_X + OPTION_INSET));
    assert_eq!(col_of(&buf, y, "resume"), Some(BODY_X + OPTION_INSET));
    assert_eq!(
        col_of(&buf, y, "Pick up"),
        Some(BODY_X + OPTION_INSET + COMMAND_COL),
        "purpose at --command-col"
    );
    assert_eq!(
        buf[(MARGIN_X as u16 + OPTION_INSET as u16, y)].fg,
        pal.accent
    );
    assert_eq!(
        buf[(MARGIN_X as u16 + OPTION_INSET as u16, y)].bg,
        pal.field
    );
    let purpose = col_of(&buf, y, "Pick up").unwrap() as u16;
    assert_eq!(buf[(purpose, y)].fg, pal.label, "the current purpose");
    let right = buf.area.width - 1;
    assert_eq!(
        (
            buf[(MARGIN_X as u16, y)].bg,
            buf[(right - MARGIN_X as u16, y)].bg
        ),
        (pal.panel, pal.panel),
        "the current row is inset a cell into the panel"
    );
    assert_eq!(
        (buf[(0, y)].bg, buf[(right, y)].bg),
        (pal.win, pal.win),
        "between the margins, not edge to edge"
    );
    assert_eq!(
        buf[(MARGIN_X as u16, y - 1)].bg,
        pal.panel,
        "a blank row on the panel above"
    );
    assert_eq!(buf[(MARGIN_X as u16, y - 2)].bg, pal.win);
    let model = y + 1;
    assert!(row_text(&buf, model).contains("model"));
    assert_eq!(
        buf[(BODY_X as u16 + OPTION_INSET as u16, model)].fg,
        pal.label2
    );
    let at = col_of(&buf, model, "Change").unwrap() as u16;
    assert_eq!(buf[(at, model)].fg, pal.label2);
    assert_eq!(
        buf[(MARGIN_X as u16 + OPTION_INSET as u16, model)].bg,
        pal.panel
    );
    assert!(row_text(&buf, y + 2).contains("quit"));
    assert!(row_text(&buf, y + 3).contains("exit"));
    assert!(row_text(&buf, y + 4).contains("clear"));
    assert!(!row_text(&buf, y + 4).contains('⌃'), "no shortcut column");
    assert_eq!(buf[(MARGIN_X as u16, y + 5)].bg, pal.panel, "and one below");
    let field = y + 6;
    assert_eq!(
        buf[(MARGIN_X as u16, field)].bg,
        pal.field,
        "the panel sits on the field"
    );
    assert_eq!(
        col_of(&buf, field, "›"),
        Some(MARGIN_X),
        "the field keeps its prompt"
    );
    assert_eq!(col_of(&buf, field, "/"), Some(BODY_X));
    assert_eq!(
        caret(&mut a, 100, 36),
        Some((BODY_X as u16 + 1, field)),
        "the caret after the slash"
    );
    assert_eq!(footer_row(&buf), field + 2);
    assert_eq!(
        footer_lead(&buf),
        "○ ↑↓ Choose     ↩ Run     esc Close",
        "frame F's footer, with no status word"
    );
}

/// Also pins that the typed text turns accent once it spells a command.
#[test]
fn the_command_field_completes_the_current_command_in_grey() {
    let mut a = app();
    for c in ['/', 'c'] {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "clear").unwrap();
    assert!(find_row(&buf, "resume").is_none(), "narrowed to the match");
    let name = (BODY_X + OPTION_INSET) as u16;
    assert_eq!(buf[(name, y)].fg, pal.label, "what is typed of it");
    assert_eq!(buf[(name + 1, y)].fg, pal.label2, "and the rest");
    let field = y + 2;
    assert_eq!(row_text(&buf, field).trim(), "› /clear");
    assert_eq!(
        buf[(BODY_X as u16 + 1, field)].fg,
        pal.label,
        "not yet a command"
    );
    assert_eq!(
        buf[(BODY_X as u16 + 2, field)].fg,
        pal.label3,
        "the completion"
    );
    assert_eq!(caret(&mut a, 100, 36), Some((BODY_X as u16 + 2, field)));

    for c in "lear".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let buf = render(&mut a, 100, 36);
    let field = find_row(&buf, "› /clear").unwrap();
    assert_eq!(
        buf[(BODY_X as u16, field)].fg,
        pal.accent,
        "a real command is blue"
    );
    assert_eq!(buf[(BODY_X as u16 + 5, field)].fg, pal.accent);
}

#[test]
fn the_completion_follows_the_current_row() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    a.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    assert!(find_row(&buf, "› /model").is_some());
    a.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    assert!(
        find_row(&buf, "› /quit").is_some(),
        "typing goes back to the top match"
    );
}

/// Mid-text the caret is the cursor on the next character's cell, which
/// keeps its own paint.
#[test]
fn the_caret_stands_before_the_character_it_is_at() {
    let mut a = app();
    for c in "abc".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "› abc").unwrap();
    assert_eq!(caret(&mut a, 100, 36), Some((BODY_X as u16 + 2, y)));
    assert_eq!(buf[(BODY_X as u16 + 2, y)].bg, pal.field);
    assert_eq!(buf[(BODY_X as u16 + 2, y)].fg, pal.label);
}

/// motion.css: on for the first half of `--caret-period` (1.05s), off for
/// the second; 100ms ticks.
#[test]
fn the_caret_blinks_by_hiding_the_cursor() {
    let mut a = app();
    assert!(caret(&mut a, 100, 36).is_some());
    a.advance(4);
    assert!(caret(&mut a, 100, 36).is_some(), "still the shown half");
    a.advance(1);
    assert_eq!(caret(&mut a, 100, 36), None, "half a period in");
    a.advance(5);
    assert!(caret(&mut a, 100, 36).is_some(), "a whole period in");
}

#[test]
fn under_reduced_motion_the_caret_holds() {
    let mut a = app().with_motion(Motion::Reduced);
    for _ in 0..10 {
        a.advance(1);
        assert!(caret(&mut a, 100, 36).is_some(), "tick {}", a.tick);
    }
}

#[test]
fn the_review_lays_out_tree_and_diff_on_the_grid() {
    let mut a = app();
    for c in "Add rate limiting to the gateway.".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    a.log.push(LogEntry::AssistantText {
        text: "Each key gets 100 requests a minute.".into(),
    });
    let before: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let after = before.replace("line 10\n", "line ten\n");
    a.apply_event(Event::ReviewRequested {
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
    let buf = render(&mut a, 110, 40);
    let pal = Theme::Dark.palette();
    assert_eq!(col_of(&buf, 1, "Add rate limiting"), Some(BODY_X));
    assert!(buf[(BODY_X as u16, 1)].modifier.contains(Modifier::BOLD));
    assert!(row_text(&buf, 1)
        .trim_end()
        .ends_with("Nothing is saved until you approve"));
    assert_eq!(col_of(&buf, 2, "Each key"), Some(BODY_X));
    // Tree: `TREE_W` (28) on tint.
    assert_eq!(buf[(0, 4)].bg, pal.tint);
    assert_eq!(buf[(27, 4)].bg, pal.tint);
    assert_eq!(buf[(28, 4)].bg, pal.win);
    assert_eq!(col_of(&buf, 5, "○○"), Some(MARGIN_X), "two unread dots");
    // The tree's folder row, not the diff header that also names the path.
    let folder = (0..buf.area.height)
        .find(|&y| row_text(&buf, y)[..28].contains("src/gateway"))
        .unwrap();
    assert_eq!(buf[(MARGIN_X as u16, folder)].fg, pal.label3);
    let current = folder + 1;
    assert_eq!(
        col_of(&buf, current, "›"),
        Some(1),
        "the current file's glyph centred in 3 cells"
    );
    assert_eq!(col_of(&buf, current, "router.rs"), Some(5));
    assert_eq!(buf[(0, current)].bg, pal.field);
    let added = find_row(&buf, "limit.rs +").unwrap();
    assert_eq!(
        buf[(col_of(&buf, added, "+").unwrap() as u16, added)].fg,
        pal.add
    );
    // Diff pane at `TREE_W + PANE_GAP` (32); a 5-cell gutter.
    let path = find_row(&buf, "src/gateway/router.rs").unwrap();
    assert_eq!(col_of(&buf, path, "src/gateway/router.rs"), Some(28 + 4));
    assert!(row_text(&buf, path).contains("+1 −1"));
    let fold = find_row(&buf, "⋯  8 lines").unwrap();
    assert_eq!(col_of(&buf, fold, "⋯"), Some(32 + 5 + 2));
    let del = find_row(&buf, "line 10").unwrap();
    assert_eq!(buf[(32, del)].bg, pal.delrow);
    assert_eq!(
        col_of(&buf, del, "−"),
        Some(32 + 5),
        "the sign centred in its 2 cells"
    );
    let add = find_row(&buf, "line ten").unwrap();
    assert_eq!(buf[(32, add)].bg, pal.addrow);
    assert_eq!(
        col_of(&buf, add, "10"),
        Some(32 + 3),
        "the line number right-aligned in 5"
    );
    // Not-ready approve: `label3`, and its words say what it waits for.
    let field = find_row(&buf, "Approve after reading 1 file  ⌃↩").unwrap();
    assert!(
        !row_text(&buf, field).contains("Ask"),
        "an empty field carries no placeholder"
    );
    assert_eq!(
        buf[(col_of(&buf, field, "Approve").unwrap() as u16, field)].fg,
        pal.label3
    );
    assert!(row_text(&buf, field + 2).contains("○ ? Keys"));
}

#[test]
fn a_saved_review_folds_into_one_accent_checked_row() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.apply_event(Event::ReviewClosed {
        outcome: ReviewOutcome::Saved {
            files: vec!["a".into(), "b".into(), "c".into()],
            comments_resolved: 1,
        },
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "Saved 3 files").unwrap();
    assert_eq!(col_of(&buf, y, "✓"), Some(MARGIN_X));
    assert_eq!(buf[(MARGIN_X as u16, y)].fg, pal.accent);
    assert!(row_text(&buf, y).contains("· 1 comment resolved"));
}

#[test]
fn a_failure_is_a_sentence_and_no_cell_is_red() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.log.push(LogEntry::Failure {
        message: "The tests failed, 2 of 6.".into(),
        detail: Some("thread 'x' panicked".into()),
        open: true,
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "The tests failed").unwrap();
    assert_eq!(buf[(BODY_X as u16, y)].fg, pal.label);
    assert_eq!(buf[(BODY_X as u16, y + 1)].fg, pal.label2);
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            assert_ne!(buf[(x, y)].fg, pal.del, "red appears only in a diff");
        }
    }
}

#[test]
fn the_context_bar_lights_round_percent_over_ten_segments() {
    let mut a = app();
    a.status.context_used = Some(410_000);
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = footer_row(&buf);
    let start = col_of(&buf, y, &GAUGE_CELL.to_string()).unwrap() as u16;
    assert!(row_text(&buf, y).trim_end().ends_with("41%"));
    assert_eq!(
        buf[(start + 3, y)].fg,
        pal.fill,
        "the fourth of four filled is full fill"
    );
    assert_eq!(buf[(start + 4, y)].fg, pal.track);
    assert_eq!(buf[(start + 9, y)].fg, pal.track);
}

#[test]
fn a_streaming_reply_rebuilds_one_block_not_the_conversation() {
    let mut a = app();
    for i in 0..20 {
        a.log.push(LogEntry::UserMessage {
            text: format!("q{i}"),
        });
        a.log.push(LogEntry::AssistantText {
            text: format!("a{i}"),
        });
    }
    let _ = render(&mut a, 100, 36);
    if let Some(LogEntry::AssistantText { text }) = a.log.last_mut() {
        text.push_str(" more");
    }
    let _ = render(&mut a, 100, 36);
    assert_eq!(a.blocks_rebuilt(), 1);
}

/// Every row of the transcript, after a draw has set its width.
fn transcript_rows(a: &mut App) -> Vec<String> {
    let _ = render(a, 80, 24);
    a.transcript_view(usize::MAX / 2)
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn a_streamed_text_draws_as_the_whole_text_would_at_every_delta() {
    let reply = "Intro line.\n\n## Heading\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\n\
                 ```rust\nfn f() {\n\n    x\n}\n```\n\nAfter the fence, a long line \
                 that wraps past eighty cells because it keeps going and going on.\n\n\
                 - a bullet\n1. a step\n\nlast";
    for open_thought in [false, true] {
        let entry = |text: &str| {
            if open_thought {
                // Earlier acts make the stream's head more than one row.
                LogEntry::Work {
                    acts: vec![
                        read_call(),
                        Act::Thought {
                            text: "Done.".into(),
                            took: Took::Seconds(1),
                        },
                        Act::Thought {
                            text: text.into(),
                            took: Took::Running,
                        },
                    ],
                    open: true,
                }
            } else {
                LogEntry::AssistantText { text: text.into() }
            }
        };
        let mut streamed = app();
        streamed
            .log
            .push(LogEntry::UserMessage { text: "go".into() });
        streamed.log.push(entry(""));
        let mut at = 0;
        while at < reply.len() {
            at = (at + 7).min(reply.len());
            while !reply.is_char_boundary(at) {
                at += 1;
            }
            match streamed.log.last_mut() {
                Some(LogEntry::AssistantText { text }) => *text = reply[..at].to_string(),
                Some(LogEntry::Work { acts, .. }) => {
                    let Some(Act::Thought { text, .. }) = acts.last_mut() else {
                        unreachable!("the test's work ends in a thought");
                    };
                    *text = reply[..at].to_string();
                }
                _ => unreachable!("the test pushed a streaming entry last"),
            }
            let mut fresh = app();
            fresh.log.push(LogEntry::UserMessage { text: "go".into() });
            fresh.log.push(entry(&reply[..at]));
            assert_eq!(
                transcript_rows(&mut streamed),
                transcript_rows(&mut fresh),
                "after {at} bytes (thought: {open_thought})"
            );
        }
    }
}

/// A new thought in an open `Work` grows the stream's fixed head: the
/// cached rows of the last thought must not be spliced under it.
#[test]
fn a_second_thought_streams_as_the_whole_work_would_draw() {
    let work = |acts: Vec<Act>| LogEntry::Work { acts, open: true };
    let thought = |text: &str, took| Act::Thought {
        text: text.into(),
        took,
    };
    let mut streamed = app();
    streamed
        .log
        .push(LogEntry::UserMessage { text: "go".into() });
    streamed
        .log
        .push(work(vec![thought("One.", Took::Running)]));
    let _ = transcript_rows(&mut streamed);
    let last = work(vec![
        thought("One.", Took::Seconds(1)),
        thought("Two", Took::Running),
    ]);
    *streamed.log.last_mut().unwrap() = last.clone();
    let mut fresh = app();
    fresh.log.push(LogEntry::UserMessage { text: "go".into() });
    fresh.log.push(last);
    assert_eq!(transcript_rows(&mut streamed), transcript_rows(&mut fresh));
}

#[test]
fn a_frame_after_a_change_looks_only_from_the_changed_entry() {
    let mut a = app();
    for i in 0..20 {
        a.log.push(LogEntry::UserMessage {
            text: format!("q{i}"),
        });
    }
    let _ = render(&mut a, 100, 36);
    assert_eq!(a.log.take_changed(), a.log.len(), "the draw caught up");
    a.log[3] = LogEntry::UserMessage {
        text: "edited".into(),
    };
    let _ = render(&mut a, 100, 36);
    assert_eq!(a.blocks_rebuilt(), 1);
    assert!(transcript_rows(&mut a).iter().any(|r| r.contains("edited")));
}

#[test]
fn the_idle_footer_sets_commands_beside_the_context_bar() {
    let mut a = app();
    let buf = render(&mut a, 100, 30);
    let pal = Theme::Dark.palette();
    let footer = footer_row(&buf);
    assert_eq!(col_of(&buf, footer, "○"), Some(MARGIN_X));
    assert_eq!(
        buf[(MARGIN_X as u16, footer)].fg,
        pal.label3,
        "idle: a grey ○ in the mark column"
    );
    assert_eq!(col_of(&buf, footer, "Ready"), Some(BODY_X));
    let commands = col_of(&buf, footer, "/ Commands").unwrap();
    let bar = col_of(&buf, footer, &GAUGE_CELL.to_string()).unwrap();
    assert_eq!(
        commands + "/ Commands".len() + GROUP_GAP,
        bar,
        "frame A: right-flush, one group gap before the bar"
    );
    assert_eq!(
        buf[(commands as u16, footer)].fg,
        pal.label2,
        "a footer glyph is label2, never the accent"
    );
}

#[test]
fn after_a_turn_that_saved_the_footer_is_the_context_bar_alone() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.apply_event(Event::ReviewClosed {
        outcome: ReviewOutcome::Saved {
            files: vec!["a".into()],
            comments_resolved: 0,
        },
    });
    let buf = render(&mut a, 100, 30);
    assert_eq!(footer_lead(&buf), "○", "no status word, no keys");
    for c in "next".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    a.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
    a.apply_event(Event::TurnEnded {
        turn_id: TurnId(2),
        reason: TurnEndReason::EndTurn,
    });
    let buf = render(&mut a, 100, 30);
    assert!(
        footer_lead(&buf).contains("Ready"),
        "the next turn's footer is its own"
    );
}

#[test]
fn a_narrow_footer_drops_the_commands_before_the_context_bar() {
    let mut a = app();
    let buf = render(&mut a, 44, 20);
    let footer = row_text(&buf, footer_row(&buf));
    assert!(
        footer.contains("Ready") && footer.trim_end().ends_with("0%"),
        "{footer:?}"
    );
    assert!(!footer.contains("Commands"), "{footer:?}");
    let buf = render(&mut a, 100, 20);
    assert!(footer_lead(&buf).contains("/ Commands"));
}

/// Dates must line up at the panel's text column whatever the title length.
#[test]
fn a_session_date_is_a_right_flush_fact() {
    let session = |id: &str, title: &str| crate::resume::SessionChoice {
        id: id.into(),
        title: title.into(),
        when: "2026-09-19 13:00".into(),
        turns: 1,
    };
    let mut a = app().with_sessions(vec![
        session("a", "short"),
        session("b", "a much longer title than that"),
    ]);
    for c in "/resume".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let text_end = 100 - MARGIN_X - PANEL_PAD;
    for title in ["short", "a much longer"] {
        let y = find_row(&buf, title).unwrap();
        let at = col_of(&buf, y, "2026-09-19").unwrap();
        assert_eq!(
            at + "2026-09-19 13:00 · 1 turn".chars().count(),
            text_end,
            "{:?}",
            row_text(&buf, y)
        );
        assert_eq!(buf[(at as u16, y)].fg, pal.label2);
    }
}

/// The recorded `Pane` must match where rows are drawn (ADR 0010).
#[test]
fn a_click_on_a_drawn_diff_line_selects_that_line() {
    let mut a = app();
    let after = (1..=12).map(|i| format!("line {i}\n")).collect::<String>();
    a.apply_event(Event::ReviewRequested {
        review_id: "r".into(),
        changeset: Changeset {
            files: vec![ChangedFile {
                path: "src/f.rs".into(),
                before: None,
                after,
            }],
        },
    });
    render(&mut a, 100, 36);
    let buf = render(&mut a, 100, 36);
    let y = find_row(&buf, "line 7").unwrap();
    let x = col_of(&buf, y, "line 7").unwrap() as u16;
    let click = |kind| MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    a.handle_mouse(click(MouseEventKind::Down(MouseButton::Left)));
    a.handle_mouse(click(MouseEventKind::Up(MouseButton::Left)));
    assert_eq!(
        a.review().unwrap().selection_label(),
        Some(SelectionLabel {
            lines: "1 line".into(),
            file: "f.rs".into(),
            range: "7".into(),
        })
    );
    let buf = render(&mut a, 100, 36);
    assert_eq!(
        buf[(col_of(&buf, y, "7").unwrap() as u16 - 4, y)].symbol(),
        "▎",
        "the selection is drawn on the row that was clicked"
    );
}

/// Footers name only keys that currently work.
#[test]
fn the_review_offers_space_only_while_there_is_a_fold() {
    let mut a = app();
    let before = (1..=30).map(|i| format!("line {i}\n")).collect::<String>();
    let after = before.replace("line 15\n", "line 15!\n");
    a.apply_event(Event::ReviewRequested {
        review_id: "r".into(),
        changeset: Changeset {
            files: vec![ChangedFile {
                path: "f.rs".into(),
                before: Some(before),
                after,
            }],
        },
    });
    a.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    let footer = |a: &mut App| footer_lead(&render(a, 200, 36));
    let keys = footer(&mut a);
    assert!(
        keys.contains("Space Show All Lines")
            && keys.contains("Click, drag or Shift ↑↓ Select")
            && keys.contains("Tab Next file"),
        "{keys:?}"
    );
    let marks = crate::tokens::MARKS;
    assert!(
        keys.chars().all(|c| c.is_ascii() || marks.contains(&c)),
        "every key glyph is in the closed table: {keys:?}"
    );
    a.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(!footer(&mut a).contains("Space"), "every fold is open");
}

fn one_file_review(a: &mut App, after: String) {
    a.apply_event(Event::ReviewRequested {
        review_id: "r".into(),
        changeset: Changeset {
            files: vec![ChangedFile {
                path: "src/f.rs".into(),
                before: None,
                after,
            }],
        },
    });
}

/// The words change with readiness, not only the colour (HIG
/// "Accessibility"); a typed draft counts as a comment.
#[test]
fn the_review_action_says_what_the_key_will_do() {
    let mut a = app();
    one_file_review(&mut a, (1..=80).map(|i| format!("line {i}\n")).collect());
    let buf = render(&mut a, 100, 30);
    assert!(
        find_row(&buf, "Approve after reading 1 file").is_some(),
        "not read yet"
    );
    for c in "rename it".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let buf = render(&mut a, 100, 30);
    let field = find_row(&buf, "Send 1 Comment  ⌃↩").expect("the typed line is a comment");
    assert_eq!(
        buf[(col_of(&buf, field, "Send").unwrap() as u16, field)].fg,
        Theme::Dark.palette().accent
    );
}

/// While the comments are with the agent the field offers no action and the
/// footer is the conversation's working one (HIG "Feedback").
#[test]
fn a_waiting_review_shows_the_turn_working_and_offers_no_action() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    one_file_review(&mut a, "line 1\n".into());
    for c in "rename it".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    a.apply_event(Event::ReviewClosed {
        outcome: ReviewOutcome::Commented { comments: 1 },
    });
    let buf = render(&mut a, 100, 30);
    assert!(
        find_row(&buf, "Nothing is saved until you approve").is_some(),
        "the review stays"
    );
    assert!(find_row(&buf, "⌃↩").is_none(), "no action to take");
    assert_eq!(
        footer_lead(&buf),
        "● Thi  0m 00s",
        "the working line, naming no key"
    );
    a.apply_event(Event::TurnEnded {
        turn_id: TurnId(1),
        reason: TurnEndReason::EndTurn,
    });
    let buf = render(&mut a, 100, 30);
    assert_eq!(
        footer_lead(&buf),
        "○ esc Close",
        "no turn running: the way out"
    );
}

/// Regression: a shortened comment dropped the blank cell frame I keeps
/// after it, its `…` landing on the pane's last cell.
#[test]
fn a_shortened_comment_keeps_one_blank_cell_after_it() {
    let mut a = app();
    one_file_review(&mut a, "let limit = 100;\n".into());
    a.review_for_tests().unwrap().select(0, 0);
    for c in "Read the limit from config rather than writing a number here".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let width = 80;
    let buf = render(&mut a, width, 24);
    let row = find_row(&buf, "◆ Read").expect("the comment is drawn");
    // The pane ends at the 3-cell right margin.
    let last = width - MARGIN_X as u16 - 1;
    assert_eq!(buf[(last, row)].symbol(), " ", "the trailing blank cell");
    assert_eq!(buf[(last - 1, row)].symbol(), "…", "shortened before it");
}

/// Baseline `long-diff-lines-wrap`: nothing is cut off, and continuation
/// rows keep the row's ground.
#[test]
fn a_long_diff_line_wraps_under_a_blank_gutter() {
    let mut a = app();
    let long = format!("let s = \"{}\";", "x".repeat(150));
    one_file_review(&mut a, format!("{long}\nshort\n"));
    let buf = render(&mut a, 100, 30);
    let pal = Theme::Dark.palette();
    let first = find_row(&buf, "let s").unwrap();
    let code_x = col_of(&buf, first, "let s").unwrap();
    let text: String = (first..first + 4)
        .map(|y| {
            let row: String = row_text(&buf, y).chars().skip(code_x).collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains(&"x".repeat(150)),
        "every character is on screen"
    );
    let next = first + 1;
    assert_ne!(buf[(code_x as u16, next)].symbol(), " ", "the code goes on");
    assert!(
        row_text(&buf, next)
            .chars()
            .skip(code_x - GUTTER_LN - SIGN_COL)
            .take(GUTTER_LN + SIGN_COL)
            .all(|c| c == ' '),
        "under a blank gutter and sign"
    );
    assert_eq!(buf[(code_x as u16 - 1, next)].bg, pal.addrow);
    assert!(find_row(&buf, "short").unwrap() > next);
}

#[test]
fn a_wrapped_file_is_read_only_when_its_last_row_is_seen() {
    let mut a = app();
    let lines: String = (1..=30)
        .map(|i| format!("{i} {}\n", "y".repeat(120)))
        .collect();
    one_file_review(&mut a, lines);
    render(&mut a, 100, 30);
    assert!(!a.review().unwrap().all_read());
    for _ in 0..10 {
        a.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        render(&mut a, 100, 30);
    }
    assert!(a.review().unwrap().all_read());
}

/// After "Chat about this": the footer shows `Waiting`, and `esc` returns
/// to the options.
#[test]
fn answering_in_words_keeps_the_question_on_screen() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.apply_event(Event::QuestionAsked {
        call_id: "q1".into(),
        question: Question {
            question: "Should requests without a key be limited?".into(),
            detail: "Right now they skip the limit.".into(),
            options: vec!["Yes".into(), Question::CHAT_ABOUT_THIS.into()],
        },
    });
    a.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    let buf = render(&mut a, 100, 30);
    let question = find_row(&buf, "Should requests without a key").expect("the question stays");
    let field = find_row(&buf, "›").unwrap();
    assert!(question < field, "above the field");
    assert!(
        find_row(&buf, "1  Yes").is_none(),
        "the options are not offered while you type"
    );
    let footer = footer_lead(&buf);
    assert!(
        footer.starts_with("○ Waiting for you") && footer.contains("esc Back"),
        "{footer:?}"
    );
}

/// The agent's question over a waiting review, answered in words: the
/// question alone over the review's field, and the review above both.
#[test]
fn answering_in_words_over_a_review_keeps_both_on_screen() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    one_file_review(&mut a, "a\n".into());
    for c in "rename it".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    a.apply_event(Event::ReviewClosed {
        outcome: ReviewOutcome::Commented { comments: 1 },
    });
    a.apply_event(Event::QuestionAsked {
        call_id: "q1".into(),
        question: Question {
            question: "Should requests without a key be limited?".into(),
            detail: "Right now they skip the limit.".into(),
            options: vec!["Yes".into(), Question::CHAT_ABOUT_THIS.into()],
        },
    });
    let buf = render(&mut a, 100, 30);
    assert!(
        find_row(&buf, "1  Yes").is_some(),
        "the options, in the band"
    );
    assert!(find_row(&buf, "src/f.rs").is_some(), "the review under it");

    a.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    for c in "only keyed".chars() {
        a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let buf = render(&mut a, 100, 30);
    let question = find_row(&buf, "Should requests without a key").expect("the question stays");
    let field = find_row(&buf, "› only keyed").expect("the words in the review's field");
    assert!(question < field, "above the field");
    assert!(find_row(&buf, "1  Yes").is_none());
    assert!(find_row(&buf, "src/f.rs").is_some(), "the review stays");
    let footer = footer_lead(&buf);
    assert!(
        footer.starts_with("○ Waiting for you") && footer.contains("esc Back"),
        "{footer:?}"
    );
}

#[test]
fn the_comment_field_names_escape_as_the_footers_do() {
    let mut a = app();
    one_file_review(&mut a, "a\nb\n".into());
    render(&mut a, 100, 30);
    a.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    let buf = render(&mut a, 100, 30);
    let label = find_row(&buf, "Commenting on").unwrap();
    assert!(row_text(&buf, label).trim_end().ends_with("esc"));
    let draft = label + 1;
    assert_eq!(
        caret(&mut a, 100, 30),
        Some(((MARGIN_X + MARK_COL) as u16, draft)),
        "the caret after the mark column"
    );
}

/// Frame H: `router.rs · 144–145`, the name in `--code`, the range not.
#[test]
fn the_selection_label_draws_the_file_name_as_code() {
    let mut a = app();
    one_file_review(&mut a, "a\nb\n".into());
    render(&mut a, 100, 30);
    a.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    let buf = render(&mut a, 100, 30);
    let label = find_row(&buf, "Commenting on").unwrap();
    let name = col_of(&buf, label, "f.rs · 1").unwrap() as u16;
    let pal = Theme::Dark.palette();
    assert_eq!(buf[(name, label)].fg, pal.code);
    assert_eq!(buf[(name + 5, label)].fg, pal.label2);
}

/// Regression: a table cut on a column's edge dropped the rest with no `…`.
#[test]
fn a_table_too_narrow_for_its_columns_ends_every_row_in_an_ellipsis() {
    let table = "| alpha | beta | gamma |\n|---|---|---|\n| a | b | c |";
    // Three columns need 4 × 3 + 1 cells; below that, rows are clipped.
    for width in 5..13 {
        let ctx = Ctx::new(Theme::Dark.palette(), width);
        for line in render_prose(table, ctx) {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if text.trim().is_empty() {
                continue;
            }
            assert!(text.ends_with('…'), "width {width}: {text:?}");
        }
    }
}

#[test]
fn an_underscore_inside_a_word_is_a_literal_not_italics() {
    let ctx = Ctx::new(Theme::Dark.palette(), 80);
    let line = &render_prose("Export ANTHROPIC_API_KEY first.", ctx)[0];
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "Export ANTHROPIC_API_KEY first.");
    assert!(line
        .spans
        .iter()
        .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)));
}

/// Frame B's prose: a `` `span` `` loses its backticks and takes `--code`
/// on the line's own ground, in both themes.
#[test]
fn inline_code_is_drawn_in_the_code_ink_without_its_backticks() {
    for theme in [Theme::Dark, Theme::Light] {
        let pal = theme.palette();
        let line = &render_prose("Adding a limit in `limit.rs`.", Ctx::new(pal, 80))[0];
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Adding a limit in limit.rs.");
        let code = line
            .spans
            .iter()
            .find(|s| s.content == "limit.rs")
            .expect("the span is its own run");
        assert_eq!(code.style.fg, Some(pal.code), "{theme:?}");
        assert_eq!(code.style.bg, None, "{theme:?}: no band behind it");
    }
}

/// The working line says what the running call does, in its own words, and
/// goes back to thinking when it ends (frame `W1`).
#[test]
fn the_working_line_names_the_running_call() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.apply_event(Event::ToolUseRequested {
        turn_id: TurnId(1),
        step_id: StepId(1),
        call: ToolCall {
            id: "c1".into(),
            name: "read".into(),
            input: serde_json::json!({ "path": "src/gateway/router.rs" }),
        },
    });
    a.apply_event(Event::ToolDispatched {
        turn_id: TurnId(1),
        step_id: StepId(1),
        call_id: "c1".into(),
    });
    a.advance(12);
    let buf = render(&mut a, 100, 30);
    assert_eq!(footer_lead(&buf), "● Reading router.rs  0m 01s");
    assert_eq!(
        buf[(MARGIN_X as u16, footer_row(&buf))].fg,
        Theme::Dark.palette().amber,
        "amber means running"
    );
    a.apply_event(Event::ToolCompleted {
        turn_id: TurnId(1),
        step_id: StepId(1),
        result: ToolResult {
            call_id: "c1".into(),
            content: "fn main() {}".into(),
            is_error: false,
        },
    });
    a.advance(5);
    assert_eq!(footer_lead(&render(&mut a, 100, 30)), "○ Thinking  0m 01s");
}

/// A stall reads differently from progress: a still grey `○` and "Still".
#[test]
fn thirty_quiet_seconds_read_as_a_stall() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.advance(310);
    let buf = render(&mut a, 100, 30);
    assert_eq!(footer_lead(&buf), "○ Still thinking  0m 31s");
    assert_eq!(
        buf[(MARGIN_X as u16, footer_row(&buf))].fg,
        Theme::Dark.palette().label3
    );
}

/// Time the developer spends on a question is theirs, not the turn's
/// silence.
#[test]
fn a_question_on_screen_does_not_count_toward_a_stall() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    a.apply_event(Event::QuestionAsked {
        call_id: "q1".into(),
        question: Question {
            question: "Should requests without a key be limited?".into(),
            detail: "Right now they skip the limit.".into(),
            options: vec!["Yes".into(), Question::CHAT_ABOUT_THIS.into()],
        },
    });
    a.advance(400);
    a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let lead = footer_lead(&render(&mut a, 100, 30));
    assert!(lead.starts_with("● Thinking  0m 40s"), "{lead:?}");
}

/// With calls in parallel the line names the latest still running, read
/// from the turn's work rows.
#[test]
fn a_finished_call_hands_the_line_to_the_latest_still_running() {
    let mut a = app();
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    for (id, path) in [("c1", "src/a.rs"), ("c2", "src/b.rs")] {
        a.apply_event(Event::ToolUseRequested {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call: ToolCall {
                id: id.into(),
                name: "read".into(),
                input: serde_json::json!({ "path": path }),
            },
        });
        a.apply_event(Event::ToolDispatched {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call_id: id.into(),
        });
    }
    a.advance(10);
    assert!(footer_lead(&render(&mut a, 100, 30)).contains("Reading b.rs"));
    a.apply_event(Event::ToolCompleted {
        turn_id: TurnId(1),
        step_id: StepId(1),
        result: ToolResult {
            call_id: "c2".into(),
            content: "b".into(),
            is_error: false,
        },
    });
    a.advance(10);
    assert!(footer_lead(&render(&mut a, 100, 30)).contains("Reading a.rs"));
}

/// Under reduced motion an idle screen asks for no redraws; a working one
/// does, for its timer.
#[test]
fn under_reduced_motion_only_a_working_turn_redraws() {
    let mut a = app().with_motion(Motion::Reduced);
    assert!(!a.is_animating());
    a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
    assert!(a.is_animating());
    assert!(app().is_animating(), "full motion: the caret blinks");
}
