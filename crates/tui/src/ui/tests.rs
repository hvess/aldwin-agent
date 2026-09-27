//! The design's rules, checked on `TestBackend` frames. Every cell of every
//! scene is pinned separately by `tests/render_snapshot.rs`.

use aldwin_core::{
    ChangedFile, Changeset, Event, PlanStep, Question, ReviewOutcome, StepState, TurnEndReason,
    TurnId,
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
use crate::log::LogEntry;
use crate::palette::Theme;
use crate::tokens::{MARK_COLS, MARK_ROWS};

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
    assert!(footer.contains("/  Commands"));
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

/// Frames B, C and J.
#[test]
fn a_disclosure_glyph_is_in_the_rows_tone() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage { text: "go".into() });
    a.log.push(LogEntry::Work {
        items: vec![crate::log::WorkItem {
            call_id: "c".into(),
            verb: crate::log::Verb::Read,
            target: "src/x.rs".into(),
            fact: Some("6 lines".into()),
            failed: false,
        }],
        open: false,
    });
    let buf = render(&mut a, 100, 36);
    let pal = Theme::Dark.palette();
    let y = find_row(&buf, "Read 1 file").unwrap();
    let glyph = col_of(&buf, y, "›").unwrap() as u16;
    assert_eq!(buf[(glyph, y)].fg, pal.label2);
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
        items: vec![crate::log::WorkItem {
            call_id: "c".into(),
            verb: crate::log::Verb::Read,
            target: "src/x.rs".into(),
            fact: Some("6 lines".into()),
            failed: false,
        }],
        open: false,
    });
    let footer = |a: &mut App| {
        let buf = render(a, 100, 36);
        row_text(&buf, find_row(&buf, "Context").unwrap())
    };
    assert_eq!(
        footer(&mut a).split("Context").next().unwrap().trim(),
        "● Working…     esc  Stop"
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
        row_text(&buf, footer).contains("↑↓  Choose")
            && row_text(&buf, footer).contains("↩  Select"),
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
    let footer = row_text(&buf, field + 2);
    assert_eq!(
        footer.split("Context").next().unwrap().trim(),
        "↑↓  Choose     ↩  Run     esc  Close",
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

/// `--caret-period`.
#[test]
fn the_caret_blinks_by_hiding_the_cursor() {
    let mut a = app();
    assert!(caret(&mut a, 100, 36).is_some());
    for _ in 0..9 {
        a.tick();
    }
    assert_eq!(caret(&mut a, 100, 36), None);
}

#[test]
fn the_review_lays_out_tree_and_diff_on_the_grid() {
    let mut a = app();
    a.log.push(LogEntry::UserMessage {
        text: "Add rate limiting to the gateway.".into(),
    });
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
    assert!(row_text(&buf, field + 2).contains("?  Keys"));
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
    let y = find_row(&buf, "Context").unwrap();
    let start = col_of(&buf, y, "━").unwrap() as u16;
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

#[test]
fn the_idle_footer_sets_commands_beside_the_context_bar() {
    let mut a = app();
    let buf = render(&mut a, 100, 30);
    let pal = Theme::Dark.palette();
    let footer = find_row(&buf, "Context").unwrap();
    assert_eq!(col_of(&buf, footer, "Ready"), Some(BODY_X));
    let commands = col_of(&buf, footer, "/  Commands").unwrap();
    let context = col_of(&buf, footer, "Context").unwrap();
    assert_eq!(
        commands + "/  Commands".len() + crate::tokens::GROUP_GAP,
        context,
        "frame A: right-flush, one group gap before Context"
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
    let footer = find_row(&buf, "Context").unwrap();
    assert_eq!(
        row_text(&buf, footer).trim_start().split("  ").next(),
        Some("Context ━━━━━━━━━━ 0%"),
        "no status word, no keys"
    );
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
        row_text(&buf, find_row(&buf, "Context").unwrap()).contains("Ready"),
        "the next turn's footer is its own"
    );
}

#[test]
fn a_narrow_footer_drops_the_commands_before_the_context_bar() {
    let mut a = app();
    let buf = render(&mut a, 44, 20);
    let footer = row_text(&buf, find_row(&buf, "Context").unwrap());
    assert!(
        footer.contains("Ready") && footer.trim_end().ends_with("0%"),
        "{footer:?}"
    );
    assert!(!footer.contains("Commands"), "{footer:?}");
    let buf = render(&mut a, 100, 20);
    assert!(row_text(&buf, find_row(&buf, "Context").unwrap()).contains("/  Commands"));
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
        Some(("1 line".into(), "f.rs · 7".into()))
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
    let footer = |a: &mut App| {
        let buf = render(a, 200, 36);
        row_text(&buf, find_row(&buf, "Context").unwrap())
    };
    let keys = footer(&mut a);
    assert!(
        keys.contains("Space  Show All Lines")
            && keys.contains("Click, drag or Shift ↑↓  Select")
            && keys.contains("Tab  Next file"),
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
    let footer = row_text(&buf, find_row(&buf, "Context").unwrap());
    assert!(
        footer.contains("Waiting for you") && footer.contains("esc  Back"),
        "{footer:?}"
    );
    assert!(!footer.contains("Working"), "{footer:?}");
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
        Some((MARGIN_X as u16 + 1, draft)),
        "the caret after the edge"
    );
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
