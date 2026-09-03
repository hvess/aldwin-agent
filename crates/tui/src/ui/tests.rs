//! Frame-level rendering tests: an `App` seeded with representative state,
//! drawn against `ratatui::backend::TestBackend`, asserted against the
//! cells that actually came out.
//!
//! The companion `tests/render_snapshot.rs` pins every cell and colour of
//! every scene; these say *why* each fact matters.

use super::chrome::{highlight_command_tokens, input_height};
use super::decision::GRANT_RULE_MAX;
use super::draw;
use super::grid::{Ctx, CONTENT_INDENT, MARGIN_X};
use super::markdown::{parse_inline, render_line as render_markdown_line};
use super::transcript::intro_content;

use crate::app::{App, PermState, StatusInfo};
use crate::log::LogEntry;
use crate::palette::{self, DARK};
use mjolnir_config::Config;
use mjolnir_permissions::{Engine, PromptPayload};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;
use std::sync::Arc;

/// The dark palette at `width` columns — what every builder below is
/// handed in place of the old `(pal, width)` pair.
fn ctx(width: u16) -> Ctx<'static> {
    Ctx::new(&DARK, width)
}

fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
    App::new("claude-sonnet-5".into(), Arc::new(Engine::new(config)))
}

/// Regression test found during a rust-skills audit of this session's
/// changes: `clamp_panel`'s budget math assumed its own truncation
/// marker always cost exactly one row, but `card_line` wraps it — like
/// any other card row — once its ~70-column text is wider than the
/// panel, which is common, not exotic (any panel narrower than ~70-75
/// columns). The undercounted budget let the *tail* (the options list —
/// the one thing that must never be cut, per `clamp_panel`'s own doc
/// comment) get silently pushed past the panel's real row budget: an
/// unusually long permission-prompt target (an arbitrarily long shell
/// command is realistic user input, not contrived) forced its title to
/// wrap across many rows on a modest terminal, and the resulting
/// truncation lost part of the *options list itself* — not just part of
/// the title, which would at least be the intended trade-off. Confirmed
/// to fail against the pre-fix `clamp_panel` (options 7-8 absent) before
/// confirming it passes against the iterative budget-refit fix, which
/// correctly sacrifices more of the (already-abbreviated) title instead.
#[test]
fn a_long_prompt_title_can_be_abbreviated_but_the_full_options_list_must_survive() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "x".repeat(300), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    // Narrow (30 cols, so the target still wraps across many rows and
    // forces real abbreviation) but tall enough (34 rows) that the
    // panel's own chrome doesn't get compressed by the outer layout
    // before `clamp_panel`'s own "the tail always survives" guarantee
    // — which this test actually exercises — can be observed.
    let out = rendered(&mut app, 30, 34);
    assert!(out.contains("4  Always allow") && out.contains("5  Deny"), "every option must stay visible even when the title itself needs to be abbreviated: {out:?}");
}

/// Companion regression: the same budget bug also printed a nonsensical
/// "0 more lines not shown" marker whenever the panel's mandatory head
/// and tail already accounted for the whole panel with nothing left in
/// the middle to actually hide.
#[test]
fn no_truncation_marker_appears_when_nothing_was_actually_hidden() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 50, 14);
    assert!(!out.contains("0 more line"), "a degenerate all-head-and-tail panel must not claim to have hidden 0 lines: {out:?}");
}




fn rendered(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("")
}

/// The first screen row containing `needle`, scanning top to bottom.
/// Used instead of hand-derived coordinates wherever a test cares about
/// relative position (e.g. "does this row also carry that content")
/// rather than an exact row number — more robust to layout changes than
/// pinning down arithmetic that has to track every band's height by
/// hand.
fn find_row(buffer: &ratatui::buffer::Buffer, needle: &str) -> u16 {
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
        if row.contains(needle) {
            return y;
        }
    }
    panic!("row containing {needle:?} not found");
}

/// Per explicit developer feedback — "the status line is above the text
/// field input, but if I remember correctly it is below in the designs"
/// — and it is: `BottomBar.jsx` reads blank / composer / blank / status /
/// blank, and the design system's own prose calls the composer "a
/// three-row field with one quiet status line under it." This pins the
/// order itself, not either row's absolute coordinate, so a future
/// change to the bar's height can't quietly flip the two back.
#[test]
fn the_status_line_sits_below_the_composer_not_above_it() {
    let mut app = app();
    app.input = "drafting".into();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let composer_row = find_row(&buffer, "drafting");
    // "messages", not the model name — the model is named in the top
    // bar too, and `find_row` scans downward, so it would match there.
    let status_row = find_row(&buffer, "messages");
    assert!(
        status_row > composer_row,
        "the status line ({status_row}) must render below the composer ({composer_row}), not above it"
    );
    assert_eq!(status_row, composer_row + 2, "exactly one blank row parts them (BottomBar.jsx's blank/composer/blank/status/blank)");
}

/// The grid, straight off `tokens/cells.css`: every content row starts at
/// `--margin-x` (27px = 3 cells), the speaker label occupies
/// `--label-col` (108px = 12 cells) from there, and `--label-gutter`
/// (18px = 2 cells) parts it from the body column at `--body-col`
/// (153px = cell 17). Raised after developer feedback that "the chat
/// rows themselves appear misaligned and do not follow the cell/grid
/// system" — they now do, and this is what holds them there.
#[test]
fn speaker_rows_sit_on_the_grids_label_and_body_columns() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "question".into() });
    app.log.push(LogEntry::AssistantText { text: "answer".into() });
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let row_text = |y: u16| -> String { (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect() };
    for (label, body) in [("you", "question"), ("harness", "answer")] {
        let y = find_row(&buffer, body);
        let row = row_text(y);
        assert!(
            row[..MARGIN_X].chars().all(|c| c == ' '),
            "row {y:?} must start with the grid's {MARGIN_X}-cell left margin: {row:?}"
        );
        assert!(row[MARGIN_X..].starts_with(label), "the {label:?} label must start in cell {MARGIN_X}: {row:?}");
        assert!(row[CONTENT_INDENT..].starts_with(body), "{body:?} must start in the body column, cell {CONTENT_INDENT}: {row:?}");
    }
}

/// The transcript recedes to 35% while a decision is open — the
/// reference puts the whole conversation column at `opacity:.35` in both
/// of its panel scenes, so the panel is the one live surface. See
/// `fade_area`/`palette::PANEL_TRANSCRIPT_OPACITY`.
#[test]
fn the_transcript_dims_while_a_decision_panel_is_open() {
    let text = "an earlier answer";
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: text.into() });

    let undimmed = {
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer[(CONTENT_INDENT as u16, find_row(&buffer, text))].fg
    };

    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    let backend = TestBackend::new(100, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let dimmed = buffer[(CONTENT_INDENT as u16, find_row(&buffer, text))].fg;

    assert_ne!(dimmed, undimmed, "the transcript must recede while a decision panel is open");
    assert_eq!(
        dimmed,
        palette::fade(undimmed, DARK.ground, palette::PANEL_TRANSCRIPT_OPACITY),
        "it must recede by exactly the reference's 35%, composited onto the ground it sits on"
    );
}

/// Regression test for the bug the user actually hit: scroll math
/// compared `viewport_height` (rendered rows) against `app.log.len()`
/// (entry count) instead of `app.total_lines()` (rendered rows), so
/// `max_offset` stayed 0 for any conversation with fewer entries than
/// the viewport had rows — which is most of them, since a handful of
/// multi-line entries routinely outgrows a terminal's row count. The
/// old `ScrollState`-only unit tests couldn't catch this: they fed
/// `total_len` in whatever unit the test author chose, never
/// exercising the actual `App`/`ui::draw` wiring that picks that unit.
/// This one does, with a small viewport that can't possibly show all
/// ten 5-line entries at once.
#[test]
fn auto_follow_shows_the_tail_of_a_long_conversation_in_a_small_viewport() {
    let mut app = app();
    for i in 0..10 {
        app.log.push(LogEntry::AssistantText { text: format!("entry-{i}\nline2\nline3\nline4\nline5") });
    }

    // Log area gets a modest double-digit row count — the persistent
    // top bar, decision-panel band (zero-height here, nothing pending),
    // status line and input box eat the rest — nowhere near the ~80
    // rows ten 5-line entries (each now also carrying its own
    // `harness` speaker-label row) plus nine separators need.
    let out = rendered(&mut app, 100, 20);

    assert!(out.contains("entry-9"), "the latest entry must be visible under auto-follow");
    assert!(!out.contains("entry-0"), "the earliest entry must have scrolled out of view");
}

/// Regression test for the bug the developer actually hit in a real
/// session: `total_lines`/`ScrollState` used to count one screen row
/// per *logical* source line (`log::line_count`), not per *wrapped*
/// screen row. A single line long enough to wrap at the render width —
/// a long tool-result summary, a long assistant line — then counted as
/// fewer rows than it actually occupied on screen, so a following
/// viewport's offset undershot where it needed to sit and the wrapped
/// tail got clipped below the log area instead of shown, right above
/// the status bar. `log_row_count` (via ratatui's own
/// `Paragraph::line_count`) fixes this by counting exactly what
/// `draw_log` renders, wrapping included.
#[test]
fn auto_follow_accounts_for_wrapped_rows_not_just_logical_lines() {
    let mut app = app();
    for i in 0..5 {
        app.log.push(LogEntry::AssistantText { text: format!("short-{i}") });
    }
    // One long single logical line — `log::line_count` used to count
    // this as exactly 1 row; at width 100 it actually wraps into
    // several.
    let tail = "END-OF-LONG-LINE";
    app.log.push(LogEntry::AssistantText { text: format!("{}{tail}", "word ".repeat(40)) });

    let out = rendered(&mut app, 100, 12);

    assert!(out.contains(tail), "the wrapped tail of the last entry must be visible under auto-follow, not clipped below the log area");
    assert!(!out.contains("short-0"), "earlier entries must have scrolled out of view to make room for the wrapped entry");
}

/// Regression test for the exact bug class the visual redesign risked
/// reintroducing: once the log panel got a real border, `render_width`/
/// `render_height` (and therefore `build_log_lines`/`log_row_count`)
/// must be sourced from the panel's *inner* rect, not the outer one —
/// see `draw`'s doc comment. A line here is sized to land exactly on
/// that 2-column boundary: at the true inner width (98, for a 100-wide
/// outer area) it wraps into 2 rows; at the outer width (100) it would
/// be miscounted as fitting in 1. The actual on-screen render always
/// wraps correctly (ratatui re-wraps against the real inner `Rect` at
/// render time, regardless of what width the *count* used) — so a
/// regression here doesn't clip anything directly, it desyncs
/// `ScrollState`'s offset math from what's really on screen by exactly
/// 1 row, same as the two historical incidents this file already
/// documents, and the tail ends up scrolled just out of view. Verified
/// against a deliberately reintroduced bug (sourcing `render_width`
/// from `log_area.width` instead of `log_inner.width` in `draw`) before
/// confirming this passes against the real code.
#[test]
fn log_row_count_uses_the_bordered_panels_inner_width_not_the_outer_width() {
    let mut app = app();
    for i in 0..8 {
        app.log.push(LogEntry::AssistantText { text: format!("short-{i}") });
    }
    let tail = "END-OF-LONG-LINE"; // 16 chars
    // `render_assistant_text` prepends a 2-char marker onto an entry's
    // first rendered line — accounted for here so the total (marker +
    // 80 'x's + " " + the 16-char tail = 99 chars) lands exactly on the
    // boundary: wraps at width 98 (inner), fits on one row at width 100
    // (outer).
    let filler = "x".repeat(80);
    app.log.push(LogEntry::AssistantText { text: format!("{filler} {tail}") });

    let out = rendered(&mut app, 100, 12);

    assert!(out.contains(tail), "the wrapped tail must be visible under auto-follow when scroll math is measured against the panel's inner width");
}

/// Regression test for the 2026-08-31 status-line correction: the old
/// header showed a read/shell/edit permission summary on every frame —
/// per explicit developer feedback, that's gone from the always-visible
/// status line now (it still shows once, in the welcome hero, before
/// the first real log entry — see `intro_banner_shows_...` below). Log
/// pushed first so the hero (which still shows permissions) isn't what
/// this test is accidentally reading from.
#[test]
fn status_line_shows_the_model_name_without_a_permission_summary() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("claude-sonnet-5"));
    assert!(!out.contains("read:deny"));
    assert!(!out.contains("shell:deny"));
    assert!(!out.contains("edit:deny"));
}

/// The status line replaces the removed sidebar as the place activity
/// (thinking/working), in-flight tools, and a running message count are
/// surfaced — per explicit developer direction that this information
/// belongs "right above the input field," not in a separate panel. The
/// leading activity word itself names the in-flight tool (see
/// `activity_label`) rather than a generic "working…" once one is
/// running — per a later developer request for more descriptive
/// progress feedback (`status_line_describes_the_running_tool_instead_
/// of_a_generic_working_label` below covers that specifically); the
/// tool's raw name still shows again in the trailing `tools:` list this
/// test also checks, since that list is the detailed record of exactly
/// what's running, not just the headline.
#[test]
fn status_line_shows_activity_running_tools_and_message_count() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.status.running_tools = vec![crate::app::RunningTool { call_id: "c1".into(), name: "shell".into() }];
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("running a shell command"), "an in-flight tool should describe itself in the status line: {out:?}");
    assert!(out.contains("shell"), "an in-flight tool's name should also show in the trailing tools list: {out:?}");
    assert!(out.contains("1 message"), "the status line should show a running message count: {out:?}");
}

/// Direct developer feedback: "the status shows working and thinking,
/// but I wonder if we can be more descriptive about what the model is
/// actually doing" — a bare "working…" for an entire turn gave no sense
/// of progress. `activity_label` now distinguishes three sub-phases of
/// an active, non-thinking turn: a named tool in flight, assistant text
/// already streaming for this step, or neither yet (still "working…",
/// the honest label for "waiting on the model's first token or tool
/// call of this step" — there's no more specific truthful thing to say
/// there).
#[test]
fn status_line_describes_the_running_tool_instead_of_a_generic_working_label() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.status.running_tools = vec![crate::app::RunningTool { call_id: "c1".into(), name: "read".into() }];
    assert!(rendered(&mut app, 100, 20).contains("reading a file"));
}

#[test]
fn status_line_says_running_n_tools_when_more_than_one_is_in_flight() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.status.running_tools = vec![
        crate::app::RunningTool { call_id: "c1".into(), name: "read".into() },
        crate::app::RunningTool { call_id: "c2".into(), name: "shell".into() },
    ];
    assert!(rendered(&mut app, 100, 20).contains("running 2 tools…"));
}

#[test]
fn status_line_shows_responding_once_assistant_text_is_streaming() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.log.push(LogEntry::AssistantText { text: "partial".into() });
    assert!(rendered(&mut app, 100, 20).contains("responding…"));
}

/// Regression test found during self-review of `activity_label`: a
/// `RunningTool` with an empty `name` (see `App::apply_event`'s doc
/// comment on `pending_tool_names` — the lookup this falls back from can
/// in principle miss) must fall back to its `call_id`, the same way the
/// trailing `tools:` list already did — not silently produce "using …"
/// with nothing after "using ". Both now share `running_tool_name`.
#[test]
fn status_line_falls_back_to_the_call_id_for_a_running_tool_with_no_name() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.status.running_tools = vec![crate::app::RunningTool { call_id: "call-42".into(), name: String::new() }];
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("using call-42"), "activity label must not show a blank tool name: {out:?}");
}

/// Regression test for explicit developer feedback: "the first item in
/// this row should say what the LLM is currently doing" — activity must
/// lead the status line, not trail after the model name/turn counter.
#[test]
fn status_line_shows_activity_before_the_model_name() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Anchored on "working", not "claude-sonnet-5" — the model name now
    // also appears in the persistent top bar (`draw_top_bar`), so
    // `find_row` would otherwise land on that row instead of the status
    // line; "working" only ever appears on the status line.
    let row = find_row(&buffer, "working");
    let row_text: String = (0..100).map(|x| buffer[(x, row)].symbol().to_string()).collect();
    let working_pos = row_text.find("working").expect("activity label present");
    let model_pos = row_text.find("claude-sonnet-5").expect("model name present");
    assert!(working_pos < model_pos, "activity should lead the status line, ahead of the model name: {row_text:?}");
}

#[test]
fn user_message_appears_in_the_rendered_log() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hello world".into() });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("hello world"));
}

#[test]
fn thinking_indicator_renders_only_while_active() {
    let mut app = app();
    // The spinner only ever renders below real log entries — in real
    // usage the log is never empty by the time `thinking`/`turn_active`
    // can be true, since `submit()` pushes the `UserMessage` before core
    // even has a chance to send `TurnStarted`/`ThinkingStart` back (see
    // `build_log_lines`'s hero-vs-entries branch). Push one here so this
    // exercises the same reachable state, not an empty-log + active-turn
    // combination that can't actually happen.
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.thinking = true;
    assert!(rendered(&mut app, 100, 20).contains("thinking…"));
    app.thinking = false;
    assert!(!rendered(&mut app, 100, 20).contains("thinking…"));
}

#[test]
fn working_spinner_shows_during_an_active_turn_with_no_thinking_block() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() }); // see thinking_indicator_renders_only_while_active
    app.turn_active = true;
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("working…"), "an active turn with no other feedback should still show loading progress: {out:?}");

    app.turn_active = false;
    assert!(!rendered(&mut app, 100, 20).contains("working…"), "no active turn means no spinner");
}

#[test]
fn thinking_takes_priority_over_the_working_spinner() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() }); // see thinking_indicator_renders_only_while_active
    app.turn_active = true;
    app.thinking = true;
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("thinking…"));
    assert!(!out.contains("working…"), "only one spinner label should show at a time");
}

#[test]
fn the_spinner_animates_across_ticks() {
    // `spinner_line` (the log's own copy of this animation) was removed
    // along with the log's duplicate working/thinking row — see
    // `build_log_lines`'s doc comment — so this now exercises the one
    // remaining spinner, `draw_status_line`'s own glyph indexing.
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    app.tick = 0;
    let frame0 = rendered(&mut app, 100, 20);
    app.tick = 1;
    let frame1 = rendered(&mut app, 100, 20);
    assert_ne!(frame0, frame1, "advancing the tick should change the spinner glyph shown in the status line");
}

#[test]
fn decision_panel_shows_labeled_keys_and_the_diff() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    // Tall enough for the panel's full chrome (top bar, title band,
    // footer) alongside the diff body without clamping it away.
    let out = rendered(&mut app, 100, 28);
    assert!(out.contains("Approve"));
    assert!(out.contains("Deny"));
    assert!(out.contains("old"));
    assert!(out.contains("new"));
}

/// Regression test for the actual developer complaint that prompted this
/// panel: an approval/prompt used to render as "a temporary row" mixed
/// into the scrolling chat log — described as "ugly, not clear,
/// disjointed." A pending card must not appear in the log at all any
/// more; `decision_panel_shows_labeled_keys_and_the_diff` above covers
/// that it does appear, in the fixed panel, via `App::pending_approvals`
/// instead.
#[test]
fn a_pending_approval_does_not_render_inline_in_the_conversation_log() {
    let mut app = app();
    // Deliberately not added to `pending_approvals` — this exercises
    // only `render_entry`'s own handling of an unresolved log entry.
    app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
    let out = rendered(&mut app, 100, 20);
    assert!(!out.contains("The agent wants to edit this file."), "a pending card must not render inline in the log any more — see the decision panel instead: {out:?}");
}

/// Once resolved, the full card (diff included) still leaves a
/// permanent record inline in the log, unchanged from before this
/// panel existed — only the *live* interaction moved, not the history.
#[test]
fn a_resolved_approval_still_leaves_a_full_record_in_the_conversation_log() {
    let mut app = app();
    app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: Some(true) });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("The agent wants to edit this file.") && out.contains("old") && out.contains("new"), "a resolved card should keep its full historical record: {out:?}");
    assert!(out.contains("resolved: approved"));
}

#[test]
fn a_pending_permission_prompt_does_not_render_inline_in_the_conversation_log() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "git status".into(), path_like: false };
    app.log.push(LogEntry::PermissionPrompt { call_id: "c1".into(), payload, resolution: None });
    let out = rendered(&mut app, 100, 20);
    assert!(!out.contains("Allow shell: git status?"), "a pending prompt must not render inline in the log — see the decision panel instead: {out:?}");
}

/// Regression test for the class of bug the "disjointed" complaint
/// described: an inline card was part of the scrolling log, so scrolling
/// away from the bottom could carry it out of view entirely. The fixed
/// panel doesn't participate in log scroll at all — it must stay visible
/// regardless of where the log's own scroll position sits.
#[test]
fn pending_approval_stays_visible_even_when_the_log_is_scrolled_away_from_the_bottom() {
    let mut app = app();
    for i in 0..30 {
        app.log.push(LogEntry::AssistantText { text: format!("entry-{i}") });
    }
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    app.scroll.line_up(); // disengage auto-follow, away from the bottom
    // Tall enough for the panel's full chrome (top bar, title band,
    // footer) alongside a real (if short) log viewport.
    let out = rendered(&mut app, 100, 28);
    assert!(out.contains("The agent wants to edit this file."), "the pending decision must stay visible in its own fixed panel regardless of log scroll position: {out:?}");
}

/// The decision panel shows a real numbered list, not keybinding hints —
/// per explicit developer request: "make sure the approval options
/// appear as a list and not some weird keyboard shortcuts." "Approve"/
/// "Deny" must each appear as a distinctly numbered row; selection is
/// now color-only (`OptionRow.jsx`: the accent `▌` mark plus the `band`
/// field together, never a distinct cursor glyph — see
/// `render_decision_options`), so the first (default-selected) option's
/// own `▌` must render in `mark`, not `mark_idle`. Option rows are the
/// one deliberate exception to the grid's 3-cell `MARGIN_X`: the mark is
/// flush to the frame edge in cell 0 (number in cell 3, label in cell
/// 6), which is why these probes read column 0 and not `MARGIN_X`.
#[test]
fn the_decision_panel_shows_a_numbered_approve_deny_list() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");
    assert!(out.contains("1  Approve"), "the first option must be numbered: {out:?}");
    assert!(out.contains("2  Deny"), "the second option must be numbered: {out:?}");

    let approve_row = find_row(&buffer, "1  Approve");
    let deny_row = find_row(&buffer, "2  Deny");
    assert_eq!(buffer[(0, approve_row)].fg, DARK.mark, "the selected (first) option's mark must be the accent color: {out:?}");
    assert_eq!(buffer[(0, deny_row)].fg, DARK.mark_idle, "an unselected option's mark must not be the accent color: {out:?}");
}

#[test]
fn the_decision_panel_shows_a_pending_permission_prompts_numbered_options() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "git status".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    // Tall enough for the panel's chrome (top bar, title band, footer)
    // plus a full 8-tier options list without the outer layout
    // squeezing any of it off-screen.
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("1  Allow once"), "the first option must be numbered: {out:?}");
    assert!(out.contains("3  Allow for this project"), "later options must be numbered too: {out:?}");
    assert!(out.contains("5  Deny"), "the single deny option closes the list: {out:?}");
    assert!(!out.contains("Always deny"), "the persistent deny tiers are no longer offered here: {out:?}");
}

/// The panel must say what each answer concretely does, not just name a
/// tier — the direct answer to "permissions are not clear ... what are
/// we concretely doing". Two halves: the rule a saved answer would add
/// (in the same `kind:pattern` form it takes in `permissions.yaml`), and
/// per-option details saying how long each answer lasts and where, if
/// anywhere, it is written.
#[test]
fn a_tool_prompt_states_the_rule_it_would_save_and_what_each_option_does() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("adds the rule  shell:cargo test"), "the exact rule must be named — allow means this command, not the shell tool: {out:?}");
    assert!(out.contains("this call only; nothing is saved"), "the once tier must say it saves nothing: {out:?}");
    assert!(out.contains("saved to .mjolnir/permissions.yaml"), "the project tier must name where it writes: {out:?}");
    assert!(out.contains("saved to ~/.mjolnir/permissions.yaml"), "the always tier must name the *global* file, not the project one: {out:?}");
}

/// The old footer claimed "saved to .mjolnir/permissions.yaml" under
/// every prompt, which was true of exactly one of the tiers on offer —
/// a standing, unconditional falsehood about where a decision lands.
/// Provenance is per-option now, so the footer must not restate it.
#[test]
fn the_panel_footer_makes_no_blanket_claim_about_where_answers_are_saved() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let backend = TestBackend::new(100, 34);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let footer_row = find_row(&buffer, "to confirm");
    let footer: String = (0..buffer.area.width).map(|x| buffer[(x, footer_row)].symbol().to_string()).collect();
    assert!(!footer.contains("permissions.yaml"), "the key-hint row must not carry a where-it-saves claim of its own: {footer:?}");
}

/// On a frame too narrow to seat the detail column, the labels alone
/// still have to resolve the list — details are dropped wholesale
/// rather than wrapping every row into an unreadable ladder.
#[test]
fn the_option_detail_column_is_dropped_rather_than_wrapped_on_a_narrow_frame() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 46, 34);
    assert!(out.contains("1  Allow once") && out.contains("5  Deny"), "the numbered list must survive intact: {out:?}");
    assert!(!out.contains("nothing is saved"), "the detail column must not wrap into the narrow list: {out:?}");
}

/// Per explicit developer feedback that it wasn't clear what a tool
/// prompt was actually asking for: the panel must lead with a
/// plain-English sentence, not just `kind: target`, while still showing
/// the literal wire call underneath for anyone who wants to verify it.
#[test]
fn a_tool_prompt_shows_a_humanized_title_and_the_raw_call_underneath() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("The agent wants to read a file"), "the title must be a human-readable explanation: {out:?}");
    assert!(out.contains("read: ./crates/tui/src/ui.rs"), "the literal tool call must still be shown: {out:?}");
}

/// The raw call line must be visually secondary (dim) to the humanized
/// title (accent/bold) — the whole point of the split is that the
/// sentence is what a developer reads first. Uses a non-shell kind —
/// `command_block_lines` gives an actual shell command
/// `CommandBlock.jsx`'s own treatment instead (see
/// `a_shell_prompt_shows_a_command_block_instead_of_a_raw_line` below).
#[test]
fn the_raw_call_line_is_dimmer_than_the_humanized_title() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "explain".into(), target: "src/gateway/router.rs".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let backend = TestBackend::new(100, 34);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let title_row = find_row(&buffer, "The agent wants to inspect code");
    let raw_row = find_row(&buffer, "explain: src/gateway/router.rs");
    assert_ne!(title_row, raw_row, "the title and the raw call must be on separate rows");
    assert_eq!(buffer[(MARGIN_X as u16, raw_row)].fg, DARK.label, "the raw call row must use the muted label color");
    assert_ne!(buffer[(MARGIN_X as u16, title_row)].fg, DARK.label, "the humanized title must not itself be the muted label color");
}

/// `CommandBlock.jsx`: a shell command gets a `ground`-colored field
/// with an accent `$` prompt, not the plain dim `shell: {command}` line
/// every other prompt kind still uses (see `command_block_lines`).
#[test]
fn a_shell_prompt_shows_a_command_block_instead_of_a_raw_line() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test --workspace".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(!out.contains("shell: cargo test --workspace"), "a shell command must not show the old raw `kind: target` line: {out:?}");
    assert!(out.contains("$ cargo test --workspace"), "a shell command should render as a `$ ` command block: {out:?}");
}

/// A path-like Tool prompt whose target has an enclosing directory must
/// show the scope-toggle hint, naming both the current (exact-file)
/// scope and what Tab would broaden it to — this is the actual
/// discoverability path for the "approve this whole directory" feature.
#[test]
fn a_path_like_prompt_shows_the_directory_scope_hint() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("adds the rule  read:./crates/tui/src/ui.rs"), "must name the rule the current exact-file scope would save: {out:?}");
    assert!(out.contains("Tab  widen it to this whole directory  read:./crates/tui/src/**"), "must show the directory glob Tab would switch to, and that Tab is how: {out:?}");
}

/// After toggling, the stated rule becomes the directory glob and Tab
/// becomes the way back to the exact file — otherwise the panel would
/// name a rule other than the one it is about to persist.
#[test]
fn toggling_scope_flips_which_pattern_the_panel_calls_current() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    app.decision_pattern_scope = crate::app::PatternScope::Directory;
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("adds the rule  read:./crates/tui/src/**"), "the directory glob must now be the rule on the table: {out:?}");
    assert!(out.contains("Tab  narrow it back to this one file  read:./crates/tui/src/ui.rs"), "the exact file must still be shown as what Tab switches back to: {out:?}");
}

/// A non-path-like prompt (shell, an MCP tool's JSON blob) has nothing
/// to broaden — it still states its rule, but must not offer a Tab
/// press that would be a no-op.
#[test]
fn a_non_path_like_prompt_offers_no_scope_toggle() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("adds the rule  shell:cargo test"), "the rule itself must still be stated: {out:?}");
    assert!(!out.contains("Tab "), "a shell target has no directory to broaden to, so no toggle should be offered: {out:?}");
}

/// Moving `App::decision_selected` (as Down would via `App::handle_decision_key`
/// — exercised directly here since `ui.rs`'s own tests only touch
/// render-relevant state, not key handling, which `app.rs`'s tests
/// already cover) must move the accent-colored `▌` mark in the rendered
/// list, not just the underlying index silently — selection is
/// color-only now (`OptionRow.jsx`), not a distinct cursor glyph.
#[test]
fn moving_the_decision_cursor_moves_the_selection_marker_in_the_rendered_list() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    app.decision_selected = 1;
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");
    assert!(out.contains("1  Approve") && out.contains("2  Deny"), "both options must still render: {out:?}");

    let approve_row = find_row(&buffer, "1  Approve");
    let deny_row = find_row(&buffer, "2  Deny");
    assert_eq!(buffer[(0, approve_row)].fg, DARK.mark_idle, "the cursor must have left the first option: {out:?}");
    assert_eq!(buffer[(0, deny_row)].fg, DARK.mark, "the cursor must now be on the second option: {out:?}");
}

/// A single pending approval must not claim there's more behind it — a
/// bare "+0 more pending" or similar would be worse than no count at
/// all. Companion to `two_pending_approvals_are_queued_not_overwritten_
/// and_resolve_in_order` in `app.rs` (which covers that a second request
/// actually queues); this covers the queue depth becoming visible to the
/// developer once it does.
#[test]
fn decision_panel_shows_no_queue_count_for_a_single_pending_approval() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    let out = rendered(&mut app, 100, 20);
    assert!(!out.contains("more pending"), "one pending approval must not claim there's another queued: {out:?}");
}

/// A second queued approval — the actual scenario the queueing fix
/// covers — must surface as a visible count in the decision panel, not
/// just be silently resolvable one at a time with no warning that
/// another card is about to demand input right after this one.
#[test]
fn decision_panel_shows_a_count_of_additional_pending_approvals() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "".into() });
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c2".into(), diff: "".into() });
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c3".into(), diff: "".into() });
    let out = rendered(&mut app, 100, 24);
    assert!(out.contains("+2 more pending"), "expected the decision panel to show 2 more queued beyond the front card, got: {out:?}");
}

/// Regression test for a very large diff (e.g. a big added block — see
/// `clamp_panel`'s own doc comment on why the existing context-collapsing
/// doesn't bound this): the panel must truncate the body rather than
/// pushing the approve/deny keys off-frame, since those are the one
/// thing a developer absolutely must still be able to reach.
#[test]
fn a_very_large_diff_is_truncated_in_the_panel_but_the_buttons_stay_visible() {
    let mut app = app();
    let big_diff: String = (0..200).map(|i| format!("+line-{i}\n")).collect();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: big_diff });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("1  Approve") && out.contains("2  Deny"), "the numbered options must still be visible even when the diff is too large to show in full: {out:?}");
    assert!(out.contains("not shown"), "a truncated panel should say how much was hidden: {out:?}");
}

/// Regression test: `draw_decision_panel`'s `Paragraph` initially had no
/// `Wrap` at all — ratatui truncates rather than wraps an un-wrapped
/// `Paragraph`, so a permission prompt's title/keys (built from
/// arbitrary tool-call data, e.g. a long shell command in
/// `PromptPayload::Tool`'s `target`) could silently lose content past
/// the frame's right edge instead of the log panel's own established
/// wrap-and-recount behavior (`log_row_count`/`draw_log`). Caught before
/// this landed by visually inspecting a real render, not by an
/// automated check first — this test exists so a future regression is.
#[test]
fn a_long_permission_prompt_wraps_in_the_panel_instead_of_being_clipped() {
    let mut app = app();
    // 'q' rather than 'x': the status line's own "^c to exit"/
    // "^c to cancel" hint (`draw_status_line`) contains an 'x', which
    // would otherwise inflate this count by one independent of the
    // panel content this test actually cares about.
    let long_target = "q".repeat(200);
    let payload = PromptPayload::Tool { kind: "shell".into(), target: long_target.clone(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 60, 44);
    // Not a single contiguous run: each wrapped row now gets its own
    // fresh `BOX_PAD_H` left inset (the fix for the follow-up "known
    // limitation" complaint below), which breaks up the run of 'q's with
    // one inset space per wrapped row — counting characters, not
    // matching a literal substring, is what actually proves nothing was
    // dropped.
    //
    // The grant line (`grant_lines`) restates the target as part of the
    // rule it would save, elided at `GRANT_RULE_MAX` — so the expected
    // count is the command block's own full 200 plus whatever of the
    // rule survives elision past its "shell:" prefix. Derived from the
    // constant rather than written out, so tuning the elision width
    // can't silently turn this into a test of nothing.
    let in_grant_line = GRANT_RULE_MAX - "shell:".len();
    assert_eq!(out.matches('q').count(), 200 + in_grant_line, "all 200 characters of a long prompt target must be shown, wrapped rather than clipped: {out:?}");
}

/// Regression test for the actual reported defect, not just the
/// clipping symptom above: a wrapped continuation row of a filled
/// card/diff line used to fall back to the frame's plain background past
/// whatever content ratatui's own `Wrap` happened to draw on it, since
/// `filled_line` only ever padded/filled the *first* row it built. Checks
/// the command's last wrapped row still carries `CommandBlock.jsx`'s own
/// `ground` fill all the way to the panel's right edge — not the older
/// dim raw-line row (a shell target now gets the real `$ command` block
/// treatment; see `command_block_lines`).
#[test]
fn a_wrapped_card_row_keeps_its_full_width_background_fill() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "y".repeat(200), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    // Tall enough that the wrapped command block survives `clamp_panel`
    // alongside the panel's own chrome (band, options rule, footer).
    let backend = TestBackend::new(60, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // A run of ten consecutive `y`s only ever occurs inside the wrapped
    // `$ yyy...` command line (200 `y`s, hard-broken mid-run since it
    // has no whitespace to wrap at) — unlike a single "y", which the
    // input box's placeholder text also contains. The search stops
    // above the grant line, which restates a (differently-filled) slice
    // of the same target as the rule it would save (`grant_lines`), so
    // the row found is unambiguously the command block's own final
    // wrapped row, whose trailing padding is what this test checks.
    let needle = "y".repeat(10);
    let grant_row = find_row(&buffer, "adds the rule");
    let last_title_row = (0..grant_row)
        .rev()
        .find(|&y| {
            let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            row.contains(&needle)
        })
        .expect("row containing a run of y's not found");
    let last_col = buffer.area.width - 1;
    assert_eq!(
        buffer[(last_col, last_title_row)].bg,
        DARK.ground,
        "a wrapped command block row's trailing padding must keep its own background fill, not fall back to the frame background"
    );
}

#[test]
fn approval_card_colors_added_and_removed_lines_distinctly() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "-old\n+new".into() });
    let backend = TestBackend::new(100, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let removed_row = find_row(&buffer, "old");
    let added_row = find_row(&buffer, "new");
    // Column 3 lands inside "-old"/"+new" itself (past the box's own
    // left `│` border), so any column here works; picked to also land
    // on real text rather than the row's trailing padding.
    assert_eq!(buffer[(3, removed_row)].bg, DARK.del_bg, "a removed line should carry the removed-line background across the row");
    assert_eq!(buffer[(3, added_row)].bg, DARK.add_bg, "an added line should carry the added-line background across the row");
    assert_ne!(buffer[(3, removed_row)].bg, buffer[(3, added_row)].bg, "added and removed lines must be visually distinct");
}

#[test]
fn approval_card_collapses_unchanged_context_beyond_the_radius() {
    let diff = "--- f.rs\n+++ f.rs\n far\n context\n a\n b\n-old\n+new\n c\n d\n near\n";
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: diff.into() });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("unchanged line"), "a long run of unmodified context should collapse to an elision marker: {out:?}");
    assert!(!out.contains("far"), "context far from any change should be elided");
    assert!(out.contains("old") && out.contains("new"), "the change itself must still be shown");
    assert!(out.contains("a") && out.contains("b"), "the 2 lines of context immediately before a change must be kept");
    assert!(out.contains("c") && out.contains("d"), "the 2 lines of context immediately after a change must be kept");
}

/// Regression test for explicit developer feedback that diffs rendered
/// with no line numbers at all. `number_diff_lines` numbers each side
/// relative to the shown diff (no absolute file offset is available —
/// see its own doc comment); a context line carries the same number on
/// both sides, a removed line only its old-file number, an added line
/// only its new-file number.
#[test]
fn diff_lines_show_old_and_new_line_numbers() {
    let diff = "--- f.rs\n+++ f.rs\n one\n-old\n+new\n three\n";
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: diff.into() });
    // This diff carries a path line too (the "--- f.rs" header), one
    // more row of chrome than a bare hunk — tall enough that all 4 body
    // lines (context/removed/added/context) survive unclamped.
    let backend = TestBackend::new(100, 34);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row_text = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol().to_string()).collect() };

    let context_row = row_text(find_row(&buffer, "one"));
    let removed_row = row_text(find_row(&buffer, "old"));
    let added_row = row_text(find_row(&buffer, "new"));
    // One space after the sign — it belongs to the `add`/`del` sign
    // token, which the reference colors as `+ ` / `- `, not to the code.
    assert!(context_row.contains("1    1 │   one"), "a context line should show the same line number on both sides: {context_row:?}");
    assert!(removed_row.contains("2      │ - old"), "a removed line should show only its old-file line number: {removed_row:?}");
    assert!(added_row.contains("2 │ + new"), "an added line should show only its new-file line number: {added_row:?}");
}

/// Regression test for explicit developer feedback that posting a chat
/// message triggered a duplicate "working…" status row: one in the log
/// itself (`build_log_lines`, now removed) and one in the status line
/// (`draw_status_line`, which already showed the same thing right above
/// the input). The status line is now the only place it shows.
#[test]
fn active_turn_activity_shows_once_not_duplicated_between_log_and_status_line() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.turn_active = true;
    let out = rendered(&mut app, 100, 20);
    assert_eq!(out.matches("working…").count(), 1, "the working indicator must show exactly once (in the status line), not duplicated in the log: {out:?}");
}

#[test]
fn user_and_assistant_messages_are_visually_distinct() {
    let mut app = app();
    // Distinct text per speaker (not both "hi") so `find_row` can locate
    // each one independently — needed since a user message now renders
    // as a multi-row padded bubble (see `render_entry`'s `UserMessage`
    // arm), so the two entries' exact row offsets aren't worth pinning
    // down by hand here (see `find_row`'s own doc comment).
    app.log.push(LogEntry::UserMessage { text: "user-hi".into() });
    app.log.push(LogEntry::AssistantText { text: "assistant-hi".into() });

    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let user_row = find_row(&buffer, "user-hi");
    let assistant_row = find_row(&buffer, "assistant-hi");
    // Both speakers' prose starts in the grid's body column
    // (`CONTENT_INDENT`), past the `MARGIN_X` margin and the label
    // column — see `with_label_column`.
    let user_cell = &buffer[(CONTENT_INDENT as u16, user_row)];
    let assistant_cell = &buffer[(CONTENT_INDENT as u16, assistant_row)];
    assert_ne!(
        (user_cell.fg, user_cell.modifier),
        (assistant_cell.fg, assistant_cell.modifier),
        "user and assistant text must use different styles"
    );
}

#[test]
fn a_slash_command_renders_differently_from_a_plain_user_message() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.log.push(LogEntry::UserMessage { text: "/exit".into() });

    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // `find_row`, not a hand-derived offset — the plain message is now
    // a multi-row padded bubble (blank pad row, content, blank pad
    // row), so "the next entry starts 2 rows down" no longer holds; see
    // `render_entry`'s `UserMessage` arm.
    let plain_row = find_row(&buffer, "hi");
    let command_row = find_row(&buffer, "/exit");
    let plain_cell = &buffer[(2, plain_row)]; // inside "hi"'s filled bubble
    let command_cell = &buffer[(2, command_row)]; // "> /exit"
    assert_ne!(
        (plain_cell.fg, plain_cell.modifier),
        (command_cell.fg, command_cell.modifier),
        "a slash command must not use the same style as a plain user message"
    );
}

#[test]
fn input_text_is_rendered_in_the_input_box() {
    let mut app = app();
    app.input = "draft text".into();
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("draft text"));
}

#[test]
fn highlight_command_tokens_dims_a_leading_command_word() {
    let line = highlight_command_tokens("/clear now", ctx(80));
    let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
    assert_eq!(styled, vec![("/clear", Some(DARK.dim)), (" ", None), ("now", Some(DARK.text))]);
}

/// The bug report this responds to: dimming only checked the input's
/// very first character, so a recognized command word typed anywhere
/// past position 0 never got flagged even though it's the same word.
#[test]
fn highlight_command_tokens_dims_a_command_word_mid_message() {
    let line = highlight_command_tokens("please run /exit for me", ctx(80));
    let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
    assert_eq!(
        styled,
        vec![
            ("please", Some(DARK.text)),
            (" ", None),
            ("run", Some(DARK.text)),
            (" ", None),
            ("/exit", Some(DARK.dim)),
            (" ", None),
            ("for", Some(DARK.text)),
            (" ", None),
            ("me", Some(DARK.text)),
        ]
    );
}

/// Regression test: an ordinary (non-command) word must carry an
/// explicit `DARK.text` foreground, not bare `Style::default()` — the
/// latter inherits the terminal's own default text color, which reads
/// fine on a dark-themed terminal by coincidence but renders dark-on-
/// dark against `draw_input`'s always-dark `DARK.bar_bottom` fill on a
/// light-themed one. Reported directly: "text is dark on light mode and
/// it clashes with the dark background."
#[test]
fn highlight_command_tokens_gives_plain_words_an_explicit_bright_fg() {
    let line = highlight_command_tokens("hello world", ctx(80));
    let fgs: Vec<Option<Color>> = line.spans.iter().map(|s| s.style.fg).collect();
    assert_eq!(fgs, vec![Some(DARK.text), None, Some(DARK.text)], "every word must set an explicit fg; only the whitespace between them may leave it unset");
}

#[test]
fn highlight_command_tokens_requires_an_exact_word_match() {
    // "/exiting" isn't the recognized "/exit" word, and "cleared" isn't
    // "/clear" — a substring match would false-positive on either, i.e.
    // dim them like a real command word instead of leaving them DARK.text.
    let line = highlight_command_tokens("/exiting cleared", ctx(80));
    assert!(line.spans.iter().all(|s| s.style.fg != Some(DARK.dim)));
}

/// Live counterpart to `a_slash_command_renders_differently_from_a_plain_user_message`
/// above: a slash command must read as dim the moment it's typed, not
/// only after Enter moves it into the log — otherwise the developer gets
/// no signal it's headed for the harness rather than the model until
/// it's too late to reconsider.
#[test]
fn command_token_is_dimmed_live_in_the_input_box() {
    let mut app = app();
    app.input = "/clear now".into();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // `BottomBar.jsx`'s five rows at the bottom of a 20-row frame —
    // blank(15) / composer(16) / blank(17) / status(18) / blank(19) —
    // so a single-line draft sits on row 16. Its content starts at the
    // grid's 3-cell `MARGIN_X`, plus 3 more for the accent `▶  ` prompt
    // prefix (the glyph and the two spaces after it): `/` lands in cell 6.
    let slash_cell = &buffer[(6, 16)]; // '/'
    let arg_cell = &buffer[(13, 16)]; // 'n' of "now"
    assert_eq!(slash_cell.symbol(), "/");
    assert_eq!(arg_cell.symbol(), "n");
    assert_ne!(
        (slash_cell.fg, slash_cell.modifier),
        (arg_cell.fg, arg_cell.modifier),
        "the command token must render differently from the rest of the typed line"
    );
}

/// Regression test for the reported bug: highlighting only ever
/// checked whether the input's very first character was `/`, so a
/// command word typed anywhere past position 0 in the same message
/// went unstyled even though it's the identical word.
#[test]
fn command_word_is_dimmed_live_even_mid_message() {
    let mut app = app();
    app.input = "hi /exit there".into();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // Composer row 16, content from cell 6 (3-cell `MARGIN_X` + the
    // 3-cell `▶  ` prompt prefix) — see
    // `command_token_is_dimmed_live_in_the_input_box` above.
    let leading_cell = &buffer[(6, 16)]; // 'h' of "hi"
    let slash_cell = &buffer[(9, 16)]; // '/' of "/exit"
    let trailing_cell = &buffer[(15, 16)]; // 't' of "there"
    assert_eq!(leading_cell.symbol(), "h");
    assert_eq!(slash_cell.symbol(), "/");
    assert_eq!(trailing_cell.symbol(), "t");
    assert_ne!((leading_cell.fg, leading_cell.modifier), (slash_cell.fg, slash_cell.modifier), "a mid-message command word must still be dimmed");
    assert_ne!((trailing_cell.fg, trailing_cell.modifier), (slash_cell.fg, slash_cell.modifier), "text after a mid-message command word must not also be dimmed");
}

/// `KNOWN_COMMAND_WORDS` is a hand-kept duplicate of `cli::slash::
/// intercept`'s real dispatch table (see that constant's own doc
/// comment on why, and its warning that `/theme` was added there
/// without any compiler or test forcing the two files to agree) — this
/// guards specifically against that one entry silently going stale
/// again, the same way `command_word_is_dimmed_live_even_mid_message`
/// guards `/exit`.
#[test]
fn theme_command_word_is_dimmed_live_like_every_other_known_command() {
    let line = highlight_command_tokens("/theme light", ctx(80));
    let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
    assert_eq!(styled, vec![("/theme", Some(DARK.dim)), (" ", None), ("light", Some(DARK.text))]);
}

/// Regression test: no visible cursor at all was a standing complaint —
/// the input box rendered the draft text but never told the real
/// terminal where the cursor sat within it.
#[test]
fn the_terminal_cursor_is_placed_inside_the_input_box_at_the_draft_cursor() {
    let mut app = app();
    app.input = "hi".into();
    app.cursor = 2; // end of "hi"
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();

    assert!(terminal.backend().cursor_visible(), "the terminal cursor must be shown while the input is focused");
    let pos = terminal.backend().cursor_position();
    // `BottomBar.jsx` is five rows deep for a single-line draft —
    // blank / composer / blank / status / blank — so the composer's one
    // content row is the 4th row up from the bottom of the frame.
    assert_eq!(pos.y, 20 - 4, "cursor should sit on the composer's one content row");
    assert_eq!(
        pos.x,
        MARGIN_X as u16 + 3 + 2,
        "cursor should sit right after \"hi\" (3 for the grid's left margin, 3 for the accent `▶  ` prompt prefix on the first line, 2 for the two typed chars)"
    );
}

/// Regression test for the composer's `▶` prompt glyph (added to match
/// `Composer.jsx`): it only ever renders on the input's first source
/// line, so the cursor's own placement math must add its 2-column width
/// back in for line 0 specifically, not for every line — otherwise
/// either the first line's cursor lands 2 columns short of the real
/// caret, or every other line's cursor drifts 2 columns too far right
/// chasing a glyph that was never drawn there.
#[test]
fn moving_the_composer_cursor_accounts_for_the_prompt_glyph_on_the_first_line() {
    let mut app = app();
    app.input = "hi\nbye".into();
    app.cursor = app.input.len(); // end of "bye", on the second line
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let pos = terminal.backend().cursor_position();
    assert_eq!(pos.x, MARGIN_X as u16 + 3, "the second line carries no prompt glyph, so its cursor should sit right after \"bye\" with only the grid's left margin ahead of it");
}

#[test]
fn the_terminal_cursor_is_hidden_while_an_approval_card_is_pending() {
    let mut app = app();
    app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into(), diff: "diff".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    assert!(!terminal.backend().cursor_visible(), "input is blocked while a card is pending — no cursor should show");
}

/// `build_log_lines` inserts one blank separator row between every pair
/// of rendered entries — checked via `find_row` (not a hand-derived
/// offset) since each entry's own speaker label adds rows too.
#[test]
fn a_blank_line_separates_consecutive_log_entries() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "first".into() });
    app.log.push(LogEntry::UserMessage { text: "second".into() });

    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let first_row = find_row(&buffer, "first");
    let second_row = find_row(&buffer, "second");
    assert!(second_row > first_row + 1, "the two entries must not land on adjacent rows: {first_row} vs {second_row}");
    let blank_row_between = (first_row + 1..second_row).any(|y| (0..buffer.area.width).all(|x| buffer[(x, y)].symbol() == " "));
    assert!(blank_row_between, "there must be a genuinely blank row between the two entries");
}

/// Regression guard for the removed assistant-speaker marker — per
/// explicit developer feedback that the leading "●" needed to go, since
/// text color (bright assistant vs. muted-and-tinted user — see
/// `user_and_assistant_messages_are_visually_distinct`) already
/// separates the two speakers without it.
#[test]
fn no_chat_message_renders_the_old_assistant_marker() {
    let mut assistant_app = app();
    assistant_app.log.push(LogEntry::AssistantText { text: "hi".into() });
    assert!(!rendered(&mut assistant_app, 100, 20).contains('●'), "the assistant marker was removed and must not reappear");

    let mut user_app = app();
    user_app.log.push(LogEntry::UserMessage { text: "hi".into() });
    assert!(!rendered(&mut user_app, 100, 20).contains('●'));
}

#[test]
fn fenced_code_block_is_stripped_of_its_fences_and_syntax_highlighted() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "here:\n```rust\nfn main() {}\n```\ndone".into() });

    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

    assert!(!out.contains("```"), "the literal fence markers must not reach the screen");
    assert!(!out.contains('╭') && !out.contains('╰'), "the code block must not draw the old hand-drawn ASCII border any more");
    assert!(out.contains("rust"), "the language tag should appear in the block's header");
    assert!(out.contains("fn main"), "the code itself must still be shown");

    // `CONTENT_INDENT`, not column 0 — the turn's label column
    // (`with_label_column`) sits ahead of every row's real content now.
    let label_row = find_row(&buffer, "rust");
    assert_eq!(buffer[(CONTENT_INDENT as u16, label_row)].bg, DARK.diff_box, "the language label row should sit on `diff_box`, the design system's nested-quote surface");

    // At least two distinct foreground colors within the code line —
    // proof it went through the highlighter, not just plain dim text.
    // Restricted to a narrow column range so unstyled padding cells
    // past the printed text can't manufacture a spurious second color.
    let code_row = find_row(&buffer, "fn main");
    assert_eq!(buffer[(CONTENT_INDENT as u16, code_row)].bg, DARK.diff_box, "the code line should sit on `diff_box` too, so the block reads as one filled field — a real code block in a document");
    let colors: std::collections::HashSet<Color> = (CONTENT_INDENT as u16..CONTENT_INDENT as u16 + 20).map(|x| buffer[(x, code_row)].fg).collect();
    assert!(colors.len() > 1, "expected the highlighted code line to use more than one color, got {colors:?}");
}

/// A ```diff fence gets `InlineDiff.jsx`'s own real bordered box
/// (`boxed_diff_lines`/`diff_box_border`) — a deliberate return to a
/// drawn box, per the design system's own spec for a quoted diff,
/// superseding the older "no box, no label, full-width color only"
/// rule this test used to check for (the hand-drawn `╭─ diff`/`╰─`
/// generic code-block box that rule was reacting to is still gone —
/// this is `InlineDiff`'s own square, one-cell-thick border, not that).
#[test]
fn a_diff_fenced_code_block_renders_a_bordered_box_with_no_language_label() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "here's the change:\n```diff\n-old line\n+new line\n```".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

    assert!(!out.contains("diff"), "a diff fence must not label itself \"diff\": {out:?}");
    assert!(out.contains("old line") && out.contains("new line"), "the diff content itself must still be shown: {out:?}");
    assert!(out.contains('┌') && out.contains('└'), "a diff fence should draw InlineDiff's own real box border: {out:?}");

    // `CONTENT_INDENT`, not column 0 — the turn's label column sits
    // ahead of the box; the tint starts on the box's own left `│` edge.
    let removed_row = find_row(&buffer, "old line");
    let added_row = find_row(&buffer, "new line");
    assert_eq!(buffer[(CONTENT_INDENT as u16, removed_row)].bg, DARK.del_bg, "a removed line should carry the removed-line background starting at its box's left edge");
    assert_eq!(buffer[(CONTENT_INDENT as u16, added_row)].bg, DARK.add_bg, "an added line should carry the added-line background starting at its box's left edge");
}

/// A diff fence at the very start of an assistant message (no leading
/// prose) must still render its full box, indented under the `harness`
/// label column the same as any other content.
#[test]
fn a_diff_fence_as_the_very_first_thing_in_a_message_still_renders_its_box() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "```diff\n-old line\n```".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let removed_row = find_row(&buffer, "old line");
    assert_eq!(buffer[(CONTENT_INDENT as u16, removed_row)].bg, DARK.del_bg, "the diff row's background must reach its box's left edge even with no leading prose ahead of it");
}

/// Regression test for explicit developer feedback: an ordinary
/// end-of-turn used to print a "— answered —" log row; now that the
/// status line shows live idle/thinking/working activity, that row is
/// redundant and must not render at all. Cancelled/error still do.
#[test]
fn an_ordinary_turn_end_renders_no_log_row() {
    let mut end_app = app();
    end_app.log.push(LogEntry::UserMessage { text: "hi".into() });
    end_app.log.push(LogEntry::TurnEnded { reason: crate::log::TurnEndReasonKind::EndTurn });
    let out = rendered(&mut end_app, 100, 20);
    assert!(!out.contains("answered"), "an ordinary turn end must not render its own log row any more: {out:?}");

    let mut cancelled_app = app();
    cancelled_app.log.push(LogEntry::TurnEnded { reason: crate::log::TurnEndReasonKind::Cancelled });
    assert!(rendered(&mut cancelled_app, 100, 20).contains("cancelled"), "a cancelled turn must still render inline");
}

/// Replaces the removed mascot-art tests — the hero no longer has any
/// art to check the shape/gradient of; see `intro_content`'s doc
/// comment on why (the Mjolnir Design System's explicit "no logo" rule).
#[test]
fn intro_banner_shows_the_active_model_and_is_exactly_intro_line_count_rows() {
    let status = StatusInfo {
        model_name:    "claude-sonnet-5".into(),
        turn:          None,
        step:          None,
        running_tools: vec![],
        read:          PermState::Denied,
        shell:         PermState::Denied,
        edit:          PermState::Denied,
    };
    assert_eq!(intro_content(&status, ctx(80)).len(), crate::log::INTRO_LINE_COUNT, "ui::intro_content must stay in sync with log::INTRO_LINE_COUNT");
    // Tall enough that the whole banner fits without auto-follow scroll
    // pushing its top rows out of view — see the sizing comment on
    // user_and_assistant_messages_are_visually_distinct.
    let out = rendered(&mut app(), 110, 40);
    assert!(out.contains("claude-sonnet-5"), "the active model should appear in the welcome banner");
    assert!(out.contains("every strike is yours to call."), "the tagline should appear in the welcome banner");
    assert!(out.contains(env!("MJOLNIR_GIT_HASH")), "the build's git commit should appear in the welcome banner, distinct from the static crate version");
    assert!(out.contains("read:deny") && out.contains("shell:deny") && out.contains("edit:deny"), "the banner should surface the current directory's permission model");
}

#[test]
fn a_fresh_session_shows_the_banner_before_any_log_entries() {
    let mut app = app();
    assert!(app.log.is_empty());
    let out = rendered(&mut app, 110, 40);
    assert!(out.contains("every strike is yours to call."));
}

/// Replaces the old `plain_user_messages_get_a_muted_background_but_
/// slash_commands_do_not` — the design system's `Prose`/`Turn`
/// components carry no filled background for chat content at all (see
/// `render_entry`'s `UserMessage` arm doc comment), so a plain message
/// is now distinguished from a slash command by its `you` speaker label
/// (absent for a command, which is directed at the harness, not
/// conversation) rather than a background tint.
#[test]
fn plain_user_messages_get_a_speaker_label_but_slash_commands_do_not() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.log.push(LogEntry::UserMessage { text: "/exit".into() });

    let backend = TestBackend::new(110, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let you_row = find_row(&buffer, "you");
    let command_row = find_row(&buffer, "/exit");
    assert!(you_row < command_row, "the plain message's own \"you\" speaker label must appear before the slash command");
    let command_cell = &buffer[(2, command_row)]; // "> /exit"
    assert_ne!(command_cell.fg, DARK.speaker_you, "a slash command must not be styled as a speaker-labeled chat message");
}

/// Replaces the old `the_welcome_banner_is_framed_by_a_border_spanning_
/// the_full_render_width` — the hero no longer draws its own border
/// (see `intro_content`'s doc comment); it's framed by the log panel's
/// own ratatui-drawn rounded border instead, which frames real log
/// content identically whether the hero or real entries are showing. No
/// dependency on the hero's row count, unlike the test this replaces.
#[test]
fn the_log_panel_has_no_drawn_border_but_is_still_opaque() {
    // Replaces the old `..._is_framed_by_a_rounded_border_...`: the
    // opaque-surfaces redesign's reference screenshot showed no box
    // anywhere around the conversation (see `draw`'s `log_block` doc
    // comment) — the log panel's own 4-sided border was dropped, but it
    // must still be a filled, opaque surface, not the terminal's own
    // background showing through at its edges.
    let mut app = app();
    let (width, height) = (110u16, 40u16);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let top = 4; // below the 3-row top bar + its 1-row rule
    // `BottomBar.jsx` is 5 rows for an empty draft (blank / composer /
    // blank / status / blank), with its own 1-row rule above it, so the
    // log's last row is 6 up from the frame's last row.
    let bottom = height - 1 - (1 + input_height("") + 1 + 1 + 1) - 1;
    for &(x, y) in &[(0, top), (width - 1, top), (0, bottom), (width - 1, bottom)] {
        let cell = &buffer[(x, y)];
        assert_ne!(cell.symbol(), "╭", "the log panel must not draw a border corner");
        assert_eq!(cell.bg, DARK.ground, "the log panel must still be opaque at its edges even without a drawn border");
    }
}

#[test]
fn bold_markdown_strips_asterisks_and_sets_the_bold_modifier() {
    let spans = parse_inline("say **hello** now", Style::default().fg(DARK.body), ctx(80));
    let bold = spans.iter().find(|s| s.content.as_ref() == "hello").expect("bold span present");
    assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    assert!(spans.iter().all(|s| !s.content.contains('*')), "literal asterisks must not reach the screen");
}

#[test]
fn italic_markdown_sets_the_italic_modifier() {
    let spans = parse_inline("that is *neat* stuff", Style::default().fg(DARK.body), ctx(80));
    let italic = spans.iter().find(|s| s.content.as_ref() == "neat").expect("italic span present");
    assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
}

#[test]
fn inline_code_strips_backticks_and_uses_a_distinct_color() {
    let spans = parse_inline("run `cargo test` first", Style::default().fg(DARK.body), ctx(80));
    let code = spans.iter().find(|s| s.content.as_ref() == "cargo test").expect("code span present");
    assert_eq!(code.style.fg, Some(DARK.code), "inline code should read as a distinct color, not a reversed-video block");
    assert!(!code.style.add_modifier.contains(Modifier::REVERSED), "inline code must not use reversed video");
    assert!(spans.iter().all(|s| !s.content.contains('`')), "literal backticks must not reach the screen");
}

#[test]
fn a_heading_line_drops_the_hashes_and_renders_bold() {
    let line = render_markdown_line("## Section Title", ctx(80));
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "Section Title");
    assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn a_bullet_line_replaces_the_dash_with_a_bullet_marker() {
    let line = render_markdown_line("- first item", ctx(80));
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "• first item");
}

#[test]
fn markdown_in_the_full_log_renders_without_literal_markup_characters() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "**bold** and `code` and *italic*".into() });
    let out = rendered(&mut app, 100, 20);
    assert!(!out.contains('*'), "literal asterisks must not reach the screen: {out:?}");
    assert!(!out.contains('`'), "literal backticks must not reach the screen: {out:?}");
    assert!(out.contains("bold"));
    assert!(out.contains("code"));
    assert!(out.contains("italic"));
}

/// Regression test: a long assistant prose line used to lose its left
/// inset on every wrapped row after the first — the manually-inserted
/// padding span only ever landed at the literal start of the logical
/// `Line`'s content, and ratatui's own `Wrap` (which actually splits it
/// across rows) has no concept of repeating that padding on the
/// continuation rows it produces. Reported as: "the first line of text
/// is correctly in line, but when the text wraps onto a second line, it
/// doesn't respect the padding."
#[test]
fn wrapped_assistant_prose_keeps_the_left_inset_on_every_row() {
    let mut app = app();
    // A single unbroken run, long enough to force at least one wrapped
    // continuation row regardless of the exact viewport width below —
    // no whitespace in it, so wrapping can only happen via the
    // hard-break path, keeping this independent of word-boundary logic.
    app.log.push(LogEntry::AssistantText { text: "x".repeat(300) });

    let backend = TestBackend::new(40, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let mut insets = Vec::new();
    for y in 0..buffer.area.height {
        let row: Vec<char> = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().chars().next().unwrap_or(' ')).collect();
        // At least 2 consecutive/any 'x's, not just one — the status
        // line's own "^c to exit"/"^c to cancel" hint (`draw_status_line`)
        // contains a lone 'x' too, which isn't part of the wrapped prose
        // this test cares about.
        if row.iter().filter(|&&c| c == 'x').count() > 1 {
            let inset = row.iter().position(|&c| c == 'x').unwrap();
            insets.push(inset);
        }
    }
    assert!(insets.len() > 1, "expected the long line to wrap onto multiple rows, got insets {insets:?}");
    assert!(insets.iter().all(|&i| i == insets[0]), "every wrapped row must share the same left inset, got {insets:?}");
}
