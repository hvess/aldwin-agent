//! Frame-level rendering tests: an `App` seeded with representative state,
//! drawn against `ratatui::backend::TestBackend`, asserted against the
//! cells that actually came out.
//!
//! The companion `tests/render_snapshot.rs` pins every cell and colour of
//! every scene; these say *why* each fact matters.

use super::chrome::{self, highlight_command_tokens};
use super::decision::LABEL_COL;
use super::draw;
use super::grid::{Ctx, CONTENT_INDENT, GROUP_GAP, MARGIN_X};
use super::markdown::{parse_inline, render_line as render_markdown_line, render_prose};
use super::transcript::intro_content;

use crate::app::App;
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

/// Where the drawn caret is — the `▌` in `--tui-mark` the composer paints
/// at the draft's cursor. It used to be the terminal's own cursor, read off
/// `TestBackend::cursor_position`; `14d` draws it instead (see
/// `chrome::caret_row`), so the assertion is about a cell in the buffer now
/// rather than about a position the backend was told.
fn caret_at(buffer: &ratatui::buffer::Buffer) -> (u16, u16) {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if buffer[(x, y)].symbol() == "▌" && buffer[(x, y)].fg == DARK.mark {
                return (x, y);
            }
        }
    }
    panic!("no drawn caret in the frame");
}

/// Reported directly: "the chat doesn't have any top and bottom padding and
/// it means the text touches the top and bottom bars, the designs do not do
/// this". The transcript band used to hand its whole inner rect to the log,
/// so the first turn sat in the row immediately under the identity bar and
/// the last one in the row immediately above the composer band.
///
/// The fix is `ui::LOG_PAD_ROWS`: a blank row of the transcript's own ground
/// at each end of the band. Asserted as "the bars' neighbouring rows carry
/// nothing", with enough content in the log to fill the viewport several
/// times over — so a row left empty here is the padding doing its job, not
/// simply a short conversation not reaching that far.
#[test]
fn the_transcript_never_touches_the_bars() {
    let mut app = app();
    for i in 0..60 {
        app.log.push(LogEntry::AssistantText { text: format!("line-{i} of a conversation long enough to overflow the band") });
    }
    let (width, height) = (60u16, 20u16);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let row = |y: u16| -> String { (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect() };
    // The band runs from just under the top bar to just above the bottom
    // bar's own five rows.
    let (first, last) = (super::TOP_BAR_ROWS, height - 5 - 1);
    assert_eq!(row(first).trim(), "", "the row under the identity bar is the transcript's top padding: {:?}", row(first));
    assert_eq!(row(last).trim(), "", "and the row above the composer band is its bottom padding: {:?}", row(last));
    // One row, not a gap of unspecified size: the transcript still fills
    // everything between the two pad rows. (Which of those rows is blank
    // depends on where the turn separators fall, so the assertion is that
    // the band as a whole is still carrying content, not that any one row
    // is.)
    assert!((first + 1..last).any(|y| !row(y).trim().is_empty()), "the padding is one row at each end, not an empty band");
    assert_eq!(buffer[(0, first)].bg, DARK.ground, "padding is the transcript's own ground, not a third tone between the bands");
    assert_eq!(buffer[(0, last)].bg, DARK.ground);
}

/// The chrome bars are parted from the transcript by their *tone*, not by
/// anything drawn between them.
///
/// This is the end of a long argument with the medium. A 1px CSS border has
/// no literal rendering in a cell grid, and three shapes were tried and
/// rejected in turn — `─` on its own row (floats mid-cell, and costs a row
/// the grid doesn't have), `▁`/`▔` (right weight, rarely-exercised glyphs),
/// and `BorderType::QuadrantOutside` (well-supported, but half a cell
/// thick) — before settling on an `SGR 4` underline as a cell attribute.
/// The design system then removed borders altogether, which makes the whole
/// question moot: a band's step on the ground ladder is the boundary, and a
/// background colour is exact in a cell grid in a way a hairline never was.
///
/// So the assertion is the absence of any rule at all, plus the tonal step
/// that replaced it.
#[test]
fn the_top_bar_is_parted_from_the_transcript_by_tone_with_no_rule_drawn() {
    let mut app = app();
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let last = super::TOP_BAR_ROWS - 1;
    let cell = &buffer[(0, last)];
    assert_eq!(cell.symbol(), " ", "nothing is drawn into the bar's last row");
    assert!(!cell.modifier.contains(Modifier::UNDERLINED), "and nothing is ruled along it either — the boundary is tonal now");
    assert_eq!(cell.bg, DARK.bar, "the bar's last row is bar, all the way down");
    assert_eq!(buffer[(0, super::TOP_BAR_ROWS)].bg, DARK.ground, "and the transcript's ground begins in the very next cell");
    assert_ne!(DARK.bar, DARK.ground, "which only reads as a boundary because the two tones differ");
}

/// The mirror case. The bottom bar used to need the fiddliest part of the
/// underline scheme — an underline is always on the *bottom* of a cell and
/// ratatui has no overline modifier, so a `border-top` had to be drawn as
/// the underline of the row above, which then had to be painted `ground`
/// rather than `bar_bottom` to read correctly. All five of the bar's rows
/// are simply `bar_bottom` now.
#[test]
fn the_bottom_bar_is_parted_from_the_transcript_by_tone_with_no_rule_drawn() {
    let mut app = app();
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // The composer row, found by its prompt glyph, is the second of the
    // bar's five rows, so the bar's own first row is one above it.
    let first = find_row(&buffer, "▶") - 1;
    let cell = &buffer[(0, first)];
    assert!(!cell.modifier.contains(Modifier::UNDERLINED), "no rule above the bar — the step down from the transcript is the boundary");
    assert_eq!(cell.bg, DARK.bar_bottom, "the bar's own surface starts on its first row, not one row late");
    assert_eq!(buffer[(0, first - 1)].bg, DARK.ground, "with the transcript's ground immediately above it");
    assert_ne!(DARK.bar_bottom, DARK.ground, "which only reads as a boundary because the two tones differ");
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
    app.cursor = app.input.chars().count();
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
    let backend = TestBackend::new(56, 24);
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

/// The incremental cache must be invisible: what it serves after a mutation
/// has to equal what a cache built from scratch would serve.
///
/// This is the assertion that makes `Transcript::sync`'s "compare the entry
/// with `==`" safe to rely on. It walks the mutation shapes the session
/// actually performs — a streaming append to the last entry, a push, a
/// resolution back-filled onto an entry already in the log, a tool call
/// flipping from running to done, and a `/clear` that shortens the log —
/// and after each one compares the incrementally-synced rows against a
/// freshly-built `App` holding the identical log.
#[test]
fn an_incrementally_synced_transcript_equals_one_built_from_scratch() {
    use crate::log::{ToolActivityEntry, ToolActivityStatus};

    let (width, height) = (100u16, 24u16);
    let text = |rows: &[ratatui::text::Line<'static>]| -> Vec<String> {
        rows.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect()
    };
    // Rebuilds a second `App` from the same log, so its cache has never seen
    // any of the intermediate states the first one went through.
    let from_scratch = |log: &Vec<LogEntry>| -> Vec<String> {
        let mut fresh = app();
        fresh.log = log.clone();
        let _ = rendered(&mut fresh, width, height);
        text(&fresh.transcript_slice(0, usize::MAX))
    };

    let mut app = app();
    let _ = rendered(&mut app, width, height);

    let step = |app: &mut App, what: &str| {
        let _ = rendered(app, width, height);
        assert_eq!(text(&app.transcript_slice(0, usize::MAX)), from_scratch(&app.log), "incremental and from-scratch transcripts diverged after {what}");
    };

    app.log.push(LogEntry::UserMessage { text: "refactor the retry logic".into() });
    step(&mut app, "a first push");

    app.log.push(LogEntry::AssistantText { text: String::new() });
    for chunk in ["Here ", "is ", "the ", "plan.\n\n```rust\nfn f() {}\n```\n\nDone."] {
        let Some(LogEntry::AssistantText { text }) = app.log.last_mut() else { unreachable!() };
        text.push_str(chunk);
        step(&mut app, "a streamed delta");
    }

    app.log.push(LogEntry::ToolActivity {
        step_id: mjolnir_core::StepId(1),
        calls: vec![ToolActivityEntry { call_id: "c1".into(), name: "bash".into(), status: ToolActivityStatus::Running }],
    });
    step(&mut app, "a dispatched tool");

    let Some(LogEntry::ToolActivity { calls, .. }) = app.log.last_mut() else { unreachable!() };
    calls[0].status = ToolActivityStatus::Completed { is_error: false, summary: "412 lines".into() };
    step(&mut app, "a tool completing");

    app.log.push(LogEntry::PermissionPrompt {
        call_id: "c2".into(),
        payload: PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false },
        resolution: None,
    });
    step(&mut app, "a pending prompt");

    let Some(LogEntry::PermissionPrompt { resolution, .. }) = app.log.last_mut() else { unreachable!() };
    *resolution = Some(crate::log::PromptResolution { allowed: true, label: "allowed once".into() });
    step(&mut app, "that prompt resolving in place");

    app.log.truncate(2);
    step(&mut app, "a log that shrank");

    app.log.clear();
    step(&mut app, "/clear");
}

/// The property the incremental cache exists for: a streamed token must
/// re-render *its own entry*, not the conversation.
///
/// Measured rather than asserted structurally, because the structure is
/// exactly what a future refactor would break silently. Appending to the
/// last entry is timed against a transcript twenty times longer than the
/// same append on a short one; if invalidation ever goes back to being
/// whole-log, the ratio tracks the length. The bound is deliberately loose
/// (8x for a 20x transcript) — this must fail on a regression to O(log), not
/// on a slow CI box.
#[test]
fn a_streamed_delta_re_renders_one_entry_not_the_whole_transcript() {
    use std::time::Instant;

    let reply = format!("Here is the plan. {}\n\n```rust\nfn f(x: u32) -> u32 {{ x + 1 }}\n```\n", "prose ".repeat(40));
    let append = |app: &mut App| {
        let Some(LogEntry::AssistantText { text }) = app.log.last_mut() else { unreachable!() };
        text.push_str("token ");
    };

    let elapsed_for = |turns: usize| -> f64 {
        let mut app = app();
        for i in 0..turns {
            app.log.push(LogEntry::UserMessage { text: format!("question {i}") });
            app.log.push(LogEntry::AssistantText { text: reply.clone() });
        }
        let _ = rendered(&mut app, 100, 24);
        // Warm, then measure: the first sync after a resize renders every
        // entry by definition, which is not what this is about.
        for _ in 0..20 {
            append(&mut app);
            let _ = app.total_lines();
        }
        let t = Instant::now();
        for _ in 0..200 {
            append(&mut app);
            let _ = app.total_lines();
        }
        t.elapsed().as_secs_f64()
    };

    let short = elapsed_for(2);
    let long = elapsed_for(40);
    assert!(
        long < short * 8.0,
        "a delta on a 40-turn transcript took {long:.4}s against {short:.4}s on a 2-turn one — a streamed token is re-rendering the whole log again"
    );
}

/// The other half of the entry above, and a bug the multi-line composer
/// introduced before this was pinned.
///
/// `Transcript`'s cache used to be keyed on the log band's *height* as well
/// as its width, so anything that resized the band threw away every
/// rendered row in the session. The band is resized by the composer
/// growing — which used to mean an explicit newline, and now means any
/// keystroke that pushes the draft across a wrap column, i.e. ordinary
/// typing. Measured at 7.2x the steady-state frame cost on a 40-turn
/// transcript, rising with the session.
///
/// Height is not an input to `block_rows` — no arm of `render_entry` reads
/// it — so the fix was to drop it from the key. Shaped like its neighbour
/// above: the same work on a long transcript against a short one, with a
/// deliberately loose bound so this fails on a return to O(log), not on a
/// slow CI box.
#[test]
fn a_growing_composer_re_renders_nothing_in_the_transcript() {
    use std::time::Instant;

    let reply = format!("Here is the plan. {}\n\n```rust\nfn f(x: u32) -> u32 {{ x + 1 }}\n```\n", "prose ".repeat(40));
    let elapsed_for = |turns: usize| -> f64 {
        let mut app = app();
        for i in 0..turns {
            app.log.push(LogEntry::UserMessage { text: format!("question {i}") });
            app.log.push(LogEntry::AssistantText { text: reply.clone() });
        }
        let _ = rendered(&mut app, 100, 30);
        let t = Instant::now();
        for i in 0..60 {
            // One row of draft, then two, then one again — the band grows
            // and shrinks under the transcript on every pass.
            app.input = if i % 2 == 0 { "x".into() } else { "x\ny".into() };
            app.cursor = app.input.chars().count();
            let _ = rendered(&mut app, 100, 30);
        }
        t.elapsed().as_secs_f64()
    };

    let short = elapsed_for(2);
    let long = elapsed_for(40);
    assert!(
        long < short * 8.0,
        "a composer height change on a 40-turn transcript took {long:.4}s against {short:.4}s on a 2-turn one — resizing the log band is re-rendering the whole conversation again"
    );
}

/// The invariant the whole scroll path now rests on: a row of the
/// transcript is a row on screen, so `offset` indexes straight into
/// it and `total_lines()` is just its length.
///
/// Both facts used to be produced by a *second* pass — `Paragraph::wrap` at
/// render time and `Paragraph::line_count` for the count — which is what the
/// long doc comments in `scroll.rs` and `app.rs` were guarding, and what
/// made every frame re-wrap the whole conversation three times over. This
/// asserts the equivalence directly, on content deliberately full of the
/// things that used to need that second wrapper: long prose, a long notice,
/// a long error.
#[test]
fn a_transcript_row_is_a_screen_row_so_the_scroll_offset_indexes_straight_into_it() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "prose ".repeat(60) });
    app.log.push(LogEntry::Notice { message: "n".to_string() + &"otice ".repeat(30) });
    app.log.push(LogEntry::Error { message: "e".to_string() + &"rror ".repeat(30) });
    app.log.push(LogEntry::AssistantText { text: "MARKER-ROW".into() });

    let (width, height) = (60u16, 24u16);
    // One draw to settle `render_width`/`render_height` and the following
    // offset, which is what the count below is measured against.
    let _ = rendered(&mut app, width, height);
    let rows = app.transcript_slice(0, usize::MAX);
    assert_eq!(app.total_lines(), rows.len(), "the count is the row list's own length, not a second measurement of it");
    for (i, row) in rows.iter().enumerate() {
        let w: usize = row.spans.iter().map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref())).sum();
        assert!(w <= app.render_width as usize, "row {i} is {w} cells wide on a {}-cell column — nothing wraps it now, so it would be truncated", app.render_width);
    }

    // Scroll one row up from the bottom and read the frame back: the row
    // that appears at the top of the band must be exactly `rows[offset]`.
    app.scroll.line_up();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let top_of_band: String = (0..width).map(|x| buffer[(x, super::TOP_BAR_ROWS + 1)].symbol().to_string()).collect();
    let expected: String = rows[app.scroll.offset].spans.iter().map(|s| s.content.to_string()).collect();
    assert_eq!(top_of_band.trim_end(), expected.trim_end(), "the first drawn row must be rows[offset] exactly");
}

/// A `Notice` or an `Error` carries arbitrary text — a slash command's
/// answer, a provider's failure message — so neither was ever "short enough
/// not to wrap". They used to be handed to the log's `Paragraph` unwrapped
/// and broken by it, which is the failure `wrap.rs` exists to prevent: the
/// wrapper knows nothing about the label-column inset already applied, so
/// the continuation row came out flush against the frame's left edge. Now
/// they wrap to the body column first, like every other row does.
#[test]
fn a_long_notice_wraps_under_the_body_column_not_against_the_frame_edge() {
    let mut app = app();
    app.log.push(LogEntry::Notice { message: "wrapme ".repeat(30) });
    let (width, height) = (60u16, 20u16);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let first = find_row(&buffer, "notice:");
    let rows: Vec<String> = (first..height).map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect()).collect();
    let continuation: Vec<&String> = rows.iter().skip(1).take_while(|r| r.contains("wrapme")).collect();
    assert!(!continuation.is_empty(), "the notice must actually wrap at this width: {rows:?}");
    for row in continuation {
        assert_eq!(row.len() - row.trim_start().len(), CONTENT_INDENT, "every wrapped row keeps the body column's inset: {row:?}");
    }
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

/// Both bars name the model, and both are drawn from `status.model_name` on
/// every frame — so a `/model` swap mid-session has to change both of them
/// and leave neither showing what the process booted with.
#[test]
fn both_bars_follow_a_model_swap() {
    let mut app = app();
    app.log.push(LogEntry::UserMessage { text: "hi".into() });
    app.apply_event(mjolnir_core::Event::ModelChanged { provider: Some("google".into()), model: "gemini-2.5-flash".into() });

    let out = rendered(&mut app, 100, 20);
    assert_eq!(out.matches("gemini-2.5-flash").count(), 2, "the top bar and the status line both name it: {out}");
    assert!(!out.contains("claude-sonnet-5"), "nothing may still be showing the model the session left: {out}");
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

/// Once resolved, the edit still leaves a permanent record inline in the
/// log — only the *live* interaction moved, not the history. The record
/// is `ToolLine.jsx`'s own shape (glyph, tool name, path, right-flush
/// stat) over the diff box, not a second copy of the decision panel's
/// card: a resolved decision is a tool call that happened, and the
/// reference renders one of those as a line of turn content, on the same
/// body column as everything else in the turn.
#[test]
fn a_resolved_approval_still_leaves_a_full_record_in_the_conversation_log() {
    let mut app = app();
    app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "--- src/page.rs\n-old\n+new".into(), resolution: Some(true) });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("old") && out.contains("new"), "a resolved card should keep the diff it was answering: {out:?}");
    assert!(out.contains("edit") && out.contains("src/page.rs"), "the record should name the tool and the file it touched: {out:?}");
    assert!(out.contains("+1") && out.contains("-1"), "the record should carry the diff stat flush right, as ToolLine does: {out:?}");
    assert!(!out.contains("The agent wants to"), "the panel's own prompting sentence has no place in the historical record: {out:?}");
}

/// A *denied* edit records the refusal rather than a `+n -m` stat that
/// would describe a change which never happened, and shows no diff box:
/// nothing was written, so there is nothing to quote.
#[test]
fn a_denied_approval_records_the_refusal_and_no_diff_box() {
    let mut app = app();
    app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "--- src/page.rs\n-old\n+new".into(), resolution: Some(false) });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("denied"), "a denied edit should say so: {out:?}");
    assert!(!out.contains('┌'), "a denied edit wrote nothing, so it should quote no diff: {out:?}");
}

/// A resolved permission prompt records what the developer chose in
/// plain English. It used to record `format!("{response:?}")`, so the
/// conversation log carried a line of Rust — reported directly as one of
/// the "chat rows [that] do not match the designs at all".
#[test]
fn a_resolved_permission_prompt_records_a_human_phrase_not_a_debug_string() {
    let mut app = app();
    app.log.push(LogEntry::PermissionPrompt {
        call_id:    "c1".into(),
        payload:    PromptPayload::Tool { kind: "shell".into(), target: "touch hello.html".into(), path_like: false },
        resolution: Some(crate::log::PromptResolution { allowed: true, label: "allowed once".into() }),
    });
    let out = rendered(&mut app, 100, 20);
    assert!(out.contains("allowed once"), "the record should name the choice in the developer's own words: {out:?}");
    assert!(out.contains("shell") && out.contains("touch hello.html"), "the record should name the call it answered: {out:?}");
    assert!(!out.contains("decision:") && !out.contains("tier:"), "no wire-type debug formatting may reach the log: {out:?}");
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
    assert!(out.contains("3  Always allow git * in this project"), "later options must be numbered too: {out:?}");
    assert!(out.contains("5  Deny"), "the single deny option closes the list: {out:?}");
    assert!(!out.contains("Always deny"), "the persistent deny tiers are no longer offered here: {out:?}");
}

/// The panel must say what each answer concretely does, not just name a
/// tier — the direct answer to "permissions are not clear ... what are
/// we concretely doing". ADR 0003 moved that answer *into* each row: the
/// sentence names the rule in the same `kind:pattern` vocabulary it takes
/// in `permissions.yaml`, and names its own reach.
///
/// What it no longer names is the *file*. The old detail column said
/// "saved to ~/.mjolnir/permissions.yaml" where the sentence now says
/// "everywhere", so the two persisting tiers are distinguished by reach
/// rather than by path. That is `5a`'s own copy and a real loss of
/// provenance — recorded in ADR 0003 rather than quietly dropped, and
/// pinned here so it stays a decision.
#[test]
fn every_option_states_its_own_rule_and_its_reach() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("Always allow cargo * in this project"), "allow means every cargo command, not this one argv (ADR 0001): {out:?}");
    assert!(out.contains("Always allow cargo * everywhere"), "and the global tier must be distinguishable from the project one: {out:?}");
    assert!(out.contains("1  Allow once"), "the once tier saves nothing, so it quotes no rule: {out:?}");
    assert!(!out.contains(".mjolnir/permissions.yaml"), "the pair shape's provenance column is gone with the pair shape: {out:?}");
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
/// plain-English sentence, and still show the exact target underneath for
/// anyone who wants to verify it.
///
/// The target is now quoted bare, in the panel's field, rather than as a
/// `kind: target` line (conformance item 30). The kind has not been lost —
/// it is the title row's right-flush badge, which is where `5a` puts it and
/// the only place it appears in the frame.
#[test]
fn a_tool_prompt_shows_a_humanized_title_and_the_exact_target_underneath() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("The agent wants to read a file"), "the title must be a human-readable explanation: {out:?}");
    assert!(out.contains("./crates/tui/src/ui.rs"), "the exact target must still be shown: {out:?}");
    assert!(!out.contains("read: ./crates/tui/src/ui.rs"), "but not with a kind prefix the badge already carries: {out:?}");
}

/// Every prompt kind gets the field, not just a shell command — the one
/// slot `5a` gives the object under discussion. A path used to render as a
/// dim `kind: target` line at the margin, so on the majority of prompts the
/// panel had no field at all (conformance item 30, measured off the
/// handoff's own markup: `background:var(--t-ground)`, inset, a blank row
/// inside it above and below).
///
/// The `$` sigil stays shell-only. It is the one part of the field that
/// says something about *running*, and in front of a file path it would be
/// a lie.
#[test]
fn a_non_shell_target_gets_the_field_without_the_shell_sigil() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "explain".into(), target: "src/gateway/router.rs".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let backend = TestBackend::new(100, 34);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let title_row = find_row(&buffer, "The agent wants to inspect code");
    let target_row = find_row(&buffer, "src/gateway/router.rs");
    assert_ne!(title_row, target_row, "the sentence and the target must be on separate rows");

    // The field is inset by `MARGIN_X` and pads a further 2 cells, so the
    // target starts on cell 5 and the strip beside it reads as card.
    assert_eq!(buffer[(MARGIN_X as u16, target_row)].bg, DARK.ground, "the target must sit on the field's own ground");
    assert_eq!(buffer[((MARGIN_X - 1) as u16, target_row)].bg, DARK.bar, "with the card's surface showing beside it");
    assert_eq!(buffer[((MARGIN_X + 2) as u16, target_row)].fg, DARK.text, "and the target itself in primary text, not the old muted label");

    let out = rendered(&mut app, 100, 34);
    assert!(!out.contains("$ src/gateway/router.rs"), "a `$` in front of a path would claim it runs: {out:?}");
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

/// A path-like Tool prompt's persisting rows quote the enclosing
/// directory, and its session row quotes the file — so the developer can
/// see on each row that the grant it writes is broader (or not) than the
/// call that triggered it, without a separate summary row to cross-read
/// (ADR 0003).
#[test]
fn a_path_like_prompts_rows_quote_their_own_patterns() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("Always allow ./crates/tui/src/** in this project"), "the persisting row must quote the directory glob it would save: {out:?}");
    assert!(out.contains("Allow ./crates/tui/src/ui.rs for this session"), "and the session row the file it would allow: {out:?}");
    assert!(!out.contains("adds the rule"), "the separate grant-summary row is gone — each sentence states its own rule: {out:?}");
    assert!(!out.contains("Tab  "), "and so is the scope toggle it fed: {out:?}");
}

/// Both scopes are now on screen at once, one per row, where they used to
/// be one mutable rule plus a `Tab` hint naming the other. This is the
/// property that replaced the toggle, so it is worth pinning directly.
#[test]
fn both_grant_scopes_are_visible_at_once_without_a_toggle() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "./crates/tui/src/ui.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("./crates/tui/src/**"), "the broad unit: {out:?}");
    assert!(out.contains("./crates/tui/src/ui.rs for this session"), "and the exact target, on their own rows: {out:?}");
}

/// A shell prompt broadens to its *program*, not to a directory it does
/// not have. This inverts the old behaviour, which offered no toggle at
/// all on a non-path-like target and wrote the exact argv — the friction
/// ADR 0001 exists to remove.
#[test]
fn a_shell_prompts_persisting_rows_quote_the_program() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test -p gateway".into(), path_like: false };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("Always allow cargo * in this project"), "the design system's own permission copy, verbatim: {out:?}");
    assert!(out.contains("Allow cargo test -p gateway for this session"), "and the exact command on the session row: {out:?}");
}

/// The degenerate case the program unit still has to handle: a target
/// with no broader form than itself. Every row still quotes a rule — the
/// target itself — rather than one of them quietly widening to a glob the
/// developer cannot see on the row they are picking.
#[test]
fn a_target_with_no_broader_form_quotes_itself_on_every_row() {
    let mut app = app();
    let payload = PromptPayload::Tool { kind: "read".into(), target: "main.rs".into(), path_like: true };
    app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload });
    let out = rendered(&mut app, 100, 34);
    assert!(out.contains("Allow main.rs for this session"), "the session row states its rule: {out:?}");
    assert!(out.contains("Always allow main.rs in this project"), "and so does the project row, with no invented glob: {out:?}");
    assert!(!out.contains("Tab "), "there is no scope toggle to offer: {out:?}");
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
    // Since ADR 0003 the target is also quoted by each of the three
    // persisting/session sentences, elided to whatever room that row's
    // own words leave it — so the expected count is the command block's
    // full 200 plus what survives elision on each of those three rows.
    // Derived from the constants rather than written out, so tuning a
    // row's budget can't silently turn this into a test of nothing.
    //
    // `- 1` for the `…` itself: `grid::elide` bounds the *whole* result
    // to `max` cells, the trailing glyph included, since its callers are
    // hand-composed rows that have exactly that many cells to spend. The
    // quoted pattern leads with the target's own characters in every
    // case (the program glob is the 200 `q`s plus ` *`), so everything
    // that survives elision is a `q`.
    let quoted = |head: &str, tail: &str| 60 - (LABEL_COL + MARGIN_X + head.len() + tail.len()) - 1;
    let expected = 200
        + quoted("Allow ", " for this session")
        + quoted("Always allow ", " in this project")
        + quoted("Always allow ", " everywhere");
    assert_eq!(out.matches('q').count(), expected, "all 200 characters of a long prompt target must be shown, wrapped rather than clipped: {out:?}");
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
    // input box's placeholder text also contains. The search stops above
    // the options list, whose sentences quote elided slices of the same
    // target (ADR 0003 put the rule on the rows; it used to be a single
    // grant line in the same place), so the row found is unambiguously
    // the command block's own final wrapped row, whose trailing padding
    // is what this test checks.
    let needle = "y".repeat(10);
    let options_row = find_row(&buffer, "1  Allow once");
    let last_title_row = (0..options_row)
        .rev()
        .find(|&y| {
            let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            row.contains(&needle)
        })
        .expect("row containing a run of y's not found");
    // The block is inset by `MARGIN_X` (`CommandBlock.jsx` sits inside
    // the card's own `padding: 0 27px`), so the frame's last column is
    // the card's `bar` and the block's own last column is three in from
    // it. Both are checked: the fill has to reach the block's edge, and
    // the strip beyond it has to read as card rather than as more block.
    let last_col = buffer.area.width - 1;
    assert_eq!(
        buffer[(last_col - MARGIN_X as u16, last_title_row)].bg,
        DARK.ground,
        "a wrapped command block row's trailing padding must keep its own background fill, not fall back to the frame background"
    );
    assert_eq!(buffer[(last_col, last_title_row)].bg, DARK.bar, "the command block is inset from the card's edge, so the card's own surface shows beside it");
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
    // The field has no border, so its first cell past the card's own
    // margin is the row fill itself — there is no longer an edge column
    // that has to be held back off the tint.
    const FIELD: u16 = MARGIN_X as u16;
    assert_eq!(buffer[(FIELD, removed_row)].bg, DARK.del_row, "a removed line should carry the removed-line fill from the field's very first cell");
    assert_eq!(buffer[(FIELD, added_row)].bg, DARK.add_row, "an added line should carry the added-line fill from the field's very first cell");
    assert_ne!(buffer[(FIELD, removed_row)].bg, buffer[(FIELD, added_row)].bg, "added and removed lines must be visually distinct");
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
/// with no line numbers at all, re-pinned to the grid: the number sits
/// right-aligned in `--gutter-line-no-inline`'s 5 cells, then the sign,
/// then the code.
///
/// One number per row, not two. A two-column `old │ new` gutter took 11
/// cells before the sign, more than twice the grid's allowance, and
/// pushed every line of code out of the column the design puts it in
/// ("the diff ... appears to be very misaligned"). A unified-diff row
/// exists on exactly one side of the change, so the number that side
/// carries is the only one there is to show; a context row takes the
/// new-file number.
#[test]
fn diff_lines_show_their_line_number_in_the_grids_five_cell_gutter() {
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
    // Four cells of right-aligned number plus one of separation, then
    // the two-cell sign: code lands on the 8th cell of the box either
    // way, which is what makes added, removed and context rows line up.
    assert!(context_row.contains("   1   one"), "a context line takes its new-file number, in the 5-cell gutter: {context_row:?}");
    assert!(removed_row.contains("   2 - old"), "a removed line shows its old-file number: {removed_row:?}");
    assert!(added_row.contains("   2 + new"), "an added line shows its new-file number: {added_row:?}");
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
    app.cursor = app.input.chars().count();
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
    // Where typing leaves it. The caret is drawn now, so a cursor left at
    // 0 would be sitting on the very cell this test reads.
    app.cursor = app.input.chars().count();
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
    app.cursor = app.input.chars().count();
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
/// the input box rendered the draft text but never said where the cursor
/// sat within it. It said so with the *terminal's* cursor until the caret
/// became a drawn `▌` (`14d`: the composer is `▶  ▌`, both `--t-mark`),
/// which is what this now asserts — including that nothing asks the
/// terminal to paint a second one.
#[test]
fn the_caret_is_drawn_inside_the_input_box_at_the_draft_cursor() {
    let mut app = app();
    app.input = "hi".into();
    app.cursor = 2; // end of "hi"
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    assert!(!terminal.backend().cursor_visible(), "the caret is drawn, so the terminal's own cursor stays hidden");
    let (x, y) = caret_at(&buffer);
    // `BottomBar.jsx` is five rows deep for a single-line draft —
    // blank / composer / blank / status / blank — so the composer's one
    // content row is the 4th row up from the bottom of the frame.
    assert_eq!(y, 20 - 4, "the caret sits on the composer's one content row");
    assert_eq!(
        x,
        MARGIN_X as u16 + 3 + 2,
        "and right after \"hi\": the grid's left margin, the accent `▶  ` prompt prefix on the first line, then the two typed chars"
    );
}

/// The empty composer is `14d`'s own row, glyph for glyph: `▶`, two
/// spaces, the caret — both in `--t-mark`. The placeholder that follows it
/// is Mjolnir's own (the design draws none), and the one thing it may not
/// do is share a cell with the caret, which is exactly what it did while
/// the caret was the terminal's.
#[test]
fn the_empty_composer_draws_the_references_prompt_and_caret() {
    let mut app = app();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let row = 20 - 4;
    assert_eq!(buffer[(MARGIN_X as u16, row)].symbol(), "▶");
    assert_eq!(buffer[(MARGIN_X as u16, row)].fg, DARK.mark);
    assert_eq!(caret_at(&buffer), (MARGIN_X as u16 + 3, row), "the caret is two cells past the glyph, as the reference draws it");
    let text: String = (0..buffer.area.width).map(|x| buffer[(x, row)].symbol().to_string()).collect();
    assert!(text.starts_with("   ▶  ▌ Ask"), "and the placeholder starts past the caret rather than under it: {text:?}");
}

/// The composer's `▶` prompt (from `Composer.jsx`) is a *gutter*, not a
/// prefix on line 0: the glyph is drawn on the first row only, but all
/// three of its cells are reserved on every row, so a multi-line draft
/// keeps one left edge instead of stepping back three columns after its
/// first line. The caret has to be placed against that same gutter on
/// every row, or every row but the first drifts three columns left of
/// the text it is supposed to be sitting in.
#[test]
fn every_row_of_a_multiline_draft_shares_the_first_rows_left_edge() {
    let mut app = app();
    app.input = "alpha\nbravo".into();
    app.cursor = app.input.chars().count(); // end of "bravo", on the second line
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let first = find_row(&buffer, "alpha");
    let second = find_row(&buffer, "bravo");
    assert_eq!(second, first + 1, "the two source lines are two consecutive rows");
    // By cell, not by byte: the `▶` ahead of the first row is three bytes
    // wide and one cell wide, and `str::find` would report the difference
    // as a column offset that isn't there.
    let column_of = |y: u16, needle: &str| -> u16 {
        let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
        let byte = row.find(needle).expect("row carries the text");
        row[..byte].chars().count() as u16
    };
    assert_eq!(column_of(first, "alpha"), column_of(second, "bravo"), "both rows start on the same column");

    let (caret_x, caret_y) = caret_at(&buffer);
    assert_eq!(caret_y, second, "the caret is on the row its line was drawn on");
    assert_eq!(
        caret_x,
        MARGIN_X as u16 + 3 + 5,
        "and right after \"bravo\": the grid's left margin, the prompt gutter every row reserves, then the five typed chars"
    );
}

/// A single line long enough to wrap is several rows, and the caret has
/// to be on the row the text is actually on. It used to be placed from
/// the draft's *source* line and column while `Paragraph`'s own `Wrap`
/// decided the rows, so past the first wrap the two disagreed by a whole
/// row and grew further apart with every one after it.
#[test]
fn the_caret_follows_a_wrapped_draft_onto_its_continuation_row() {
    let mut app = app();
    app.input = "wrap ".repeat(30);
    app.cursor = app.input.chars().count();
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let first = find_row(&buffer, "wrap");
    let (_, caret_y) = caret_at(&buffer);
    assert!(caret_y > first, "a draft this long wraps, so its caret cannot still be on the first row");
    let caret_row: String = (0..buffer.area.width).map(|x| buffer[(x, caret_y)].symbol().to_string()).collect();
    assert!(caret_row.contains("wrap"), "and the row it is on has to be one of the draft's: {caret_row:?}");
}

/// The composer is capped rather than allowed to eat the frame: a pasted
/// file scrolls inside its band, with the caret kept in view.
#[test]
fn a_draft_taller_than_the_composer_scrolls_inside_it_instead_of_taking_the_frame() {
    let mut app = app();
    app.input = (0..40).map(|i| format!("line-{i}")).collect::<Vec<_>>().join("\n");
    app.cursor = app.input.chars().count();
    let (width, height) = (80u16, 30u16);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let cap = super::chrome::COMPOSER_MAX_ROWS;
    assert_eq!(chrome::Composer::new(&app.input, width).height(), cap, "the band stops growing at its cap");
    // The tail of the draft is what is on screen, since that is where the
    // caret is — and the head of it is not. Asserted row by row: the whole
    // buffer joined into one string has no row breaks in it, so a `contains`
    // over that can only ever answer about text that happens to sit on a
    // single row, which is not what "scrolled out of the band" means.
    let rows: Vec<String> =
        (0..height).map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>()).collect();
    let carrying = |needle: &str| rows.iter().filter(|r| r.contains(needle)).count();
    let last = find_row(&buffer, "line-39");
    assert_eq!(caret_at(&buffer).1, last, "the caret sits at the end of the last line it drew");
    assert_eq!(carrying("line-39"), 1, "the tail of the draft is on screen");
    for head in ["line-0 ", "line-1 ", "line-20"] {
        assert_eq!(carrying(head), 0, "the head of a long draft scrolls out of the band, but {head:?} is still on it");
    }
    // And the transcript still has most of the frame.
    assert!(find_row(&buffer, "Ask for a change") < height - cap, "the log keeps its band");
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

/// A ```diff fence renders as the design system's recessed field: the diff
/// rows on their own ground, no language label, and — since Turn 13 — no
/// outline of any kind. The hand-drawn `╭─ diff`/`╰─` generic code-block
/// box is long gone, and so now is the square `┌`/`└` box that briefly
/// replaced it; a quoted diff is parted from the prose around it by tone.
#[test]
fn a_diff_fenced_code_block_renders_an_unoutlined_field_with_no_language_label() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "here's the change:\n```diff\n-old line\n+new line\n```".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

    assert!(!out.contains("diff"), "a diff fence must not label itself \"diff\": {out:?}");
    assert!(out.contains("old line") && out.contains("new line"), "the diff content itself must still be shown: {out:?}");
    for corner in ['┌', '└', '┐', '┘', '│'] {
        assert!(!out.contains(corner), "nothing inside a frame is stroked any more — found {corner:?} in: {out:?}");
    }

    // `CONTENT_INDENT`, not column 0 — the turn's label column sits ahead
    // of the field. With no border there is no inner edge to step past:
    // the fill starts in that very cell.
    let removed_row = find_row(&buffer, "old line");
    let added_row = find_row(&buffer, "new line");
    let field = CONTENT_INDENT as u16;
    assert_eq!(buffer[(field, removed_row)].bg, DARK.del_row, "a removed line should carry the removed-line fill from the field's first cell");
    assert_eq!(buffer[(field, added_row)].bg, DARK.add_row, "an added line should carry the added-line fill from the field's first cell");
}

/// A diff fence at the very start of an assistant message (no leading
/// prose) must still render its full field, indented under the `harness`
/// label column the same as any other content.
#[test]
fn a_diff_fence_as_the_very_first_thing_in_a_message_still_renders_its_field() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "```diff\n-old line\n```".into() });
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let removed_row = find_row(&buffer, "old line");
    assert_eq!(buffer[(CONTENT_INDENT as u16, removed_row)].bg, DARK.del_row, "the diff row's fill must reach the field's first cell even with no leading prose ahead of it");
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

/// The empty state — the design system's `14d`. Replaces the removed
/// mascot-art tests (the hero has no art; see `intro_content`) and the
/// tagline/version/commit block Turn 14 took off this screen.
#[test]
fn the_empty_state_shows_the_wordmark_and_the_three_facts_of_this_directory() {
    let mut app = app();
    app.current_provider = Some("anthropic".into());
    let out = rendered(&mut app, 110, 40);

    assert!(out.contains("  M J O L N I R  "), "the mark identifies a frame with no transcript to identify it: {out:?}");
    assert!(out.contains("anthropic"), "the provider row names the catalogue row this session runs on: {out:?}");
    assert!(out.contains("claude-sonnet-5"), "beside the model it answers with: {out:?}");
    assert!(out.contains("read:deny") && out.contains("shell:deny") && out.contains("edit:deny"), "and what this directory permits");
    assert!(out.contains("Ask for a change, or / for commands."), "the one line saying what to do next: {out:?}");
    assert_eq!(intro_content(&app, ctx(80)).len(), super::transcript::INTRO_ROWS, "intro_content must stay in sync with INTRO_ROWS");
}

/// Turn 14 took the build's version and commit off this screen. The version
/// is still on the top bar; the commit is not on the resting screen at all.
#[test]
fn the_empty_state_carries_no_build_identity_and_no_tagline() {
    let mut app = app();
    app.status.commit = "feedface".into();
    let out = rendered(&mut app, 110, 40);
    assert!(!out.contains("feedface"), "the commit is off the resting screen: {out:?}");
    assert!(!out.contains("every strike is yours to call."), "the tagline went with the fact block: {out:?}");
}

/// A hand-written endpoint has no catalogue row, which is a real
/// configuration and not an error — the model still names itself, without a
/// dangling separator where the provider would have been.
#[test]
fn the_provider_row_falls_back_to_the_model_alone_for_an_unnamed_endpoint() {
    let mut app = app();
    assert_eq!(app.current_provider, None);
    let out = rendered(&mut app, 110, 40);
    assert!(out.contains("claude-sonnet-5"), "{out:?}");
    assert!(!out.contains(" · claude-sonnet-5"), "no separator with nothing on its left: {out:?}");
}

/// The empty state sits against the composer, where the first turn will
/// appear — not centred. A centred hero made the screen jump on the first
/// message and drift upward as the terminal grew.
#[test]
fn the_empty_state_is_anchored_to_the_bottom_of_the_log() {
    let mut app = app();
    let (width, height) = (110u16, 40u16);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let last = (0..height).filter(|y| (0..width).any(|x| buffer[(x, *y)].bg == DARK.ground)).max().expect("a log row");
    let prose = find_row(&buffer, "Ask for a change");
    assert_eq!(last - prose, 1, "one blank row between the prose and the composer band, as `14d` has it");
}

/// Regression guard for the reported defect: the top bar printed
/// `env!("CARGO_PKG_VERSION")` directly, and the workspace manifest was
/// never bumped at release time, so every build claimed to be v0.1.0 no
/// matter which release it was. It now renders whatever `StatusInfo`
/// carries, which `App::new` fills from `version::VERSION` — so this
/// asserts the wiring, and `version.rs`'s own tests assert that constant
/// tracks the manifest.
///
/// This covered the welcome banner's commit too, until Turn 14 took the
/// commit off that screen (see
/// `the_empty_state_carries_no_build_identity_and_no_tagline`). Nothing was
/// weakened by that: the defect this guards was the *version*, and the
/// version is still asserted here.
#[test]
fn the_top_bar_reports_the_running_builds_version() {
    let mut app = app();
    app.status.version = "9.9.9".into();
    let out = rendered(&mut app, 110, 40);
    assert!(out.contains("v9.9.9"), "the top bar must show the running build's version, not a hardcoded one: {out:?}");
}

/// The top bar's two groups are measured together, so they can never touch
/// and neither is ever clipped without saying so.
///
/// They used to be two independent half-width rects that could not see each
/// other, and each filled to its own boundary. Below ~56 columns the working
/// directory ran straight into the model name with no gap at all
/// (`~/Projects/mjolnir-harnesclaude-sonnet-5`), and above that both were
/// cut at the seam with nothing marking it — a bar reading
/// `~/Projects/mjolnir-harnes` and `v0.1.`, neither of which is true. A
/// clipped path still reads as a path and a clipped version still reads as a
/// version, which is what made it worth fixing rather than tolerating.
#[test]
fn the_top_bar_groups_never_collide_and_never_clip_silently() {
    for width in [36u16, 44, 52, 56, 60, 68, 76, 80, 84, 110, 120, 160] {
        let mut app = app();
        app.status.version = "9.9.9".into();
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..width).map(|x| buffer[(x, 1)].symbol().to_string()).collect();

        assert!(row.starts_with("   mjolnir"), "the brand always renders, on the margin: {width} -> {row:?}");

        // A version is shown whole or not at all — never a prefix of one.
        if let Some(at) = row.find('v') {
            assert!(row[at..].starts_with("v9.9.9"), "a partial version would be read as a real one: {width} -> {row:?}");
        }

        // Where both groups are present they are parted by at least
        // `--group-gap`. The model name is the right group's first word.
        if let Some(at) = row.find("claude-sonnet-5") {
            let left_end = row[..at].trim_end().chars().count();
            let gap = row[..at].chars().count() - left_end;
            assert!(gap >= GROUP_GAP, "only {gap} cells part the two groups at {width}: {row:?}");
        }

        // Nothing runs into the right margin, and a shortened path says so.
        assert!(row.chars().count() <= width as usize, "{width} -> {row:?}");
        let cwd = app.status.cwd.clone().unwrap_or_default();
        if !cwd.is_empty() && !row.contains(&cwd) {
            assert!(row.contains('…'), "a shortened path must carry the elision glyph: {width} -> {row:?}");
        }
    }
}

/// The status line has the identity bar's shape and had the same defect one
/// row down. It reserved the `^c to exit` hint's exact width, so the hint
/// itself never clipped — but the activity group still filled to the seam,
/// and at 52 columns the row read `0 messages^c to exit`, one fact running
/// straight into the next.
///
/// Found by screenshotting the *top* bar at the widths its own collision
/// lives at, which put this row in the same frame.
///
/// Unlike the top bar's version, the activity group is elided rather than
/// dropped: a shortened `tools: rea…` is still true, and saying what is
/// happening right now is this row's whole job.
#[test]
fn the_status_line_keeps_a_gap_before_its_key_hint() {
    for width in [36u16, 44, 52, 60, 80, 120] {
        let mut app = app();
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let y = find_row(&buffer, "^c to");
        let row: String = (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect();

        let at = row.find("^c to").expect("the hint");
        assert!(row[at..].starts_with("^c to exit"), "the hint is never clipped: {width} -> {row:?}");
        let left_end = row[..at].trim_end().chars().count();
        let gap = row[..at].chars().count() - left_end;
        assert!(gap >= GROUP_GAP, "only {gap} cells before the hint at {width}: {row:?}");
        assert!(row.chars().count() <= width as usize, "{width} -> {row:?}");
        // The activity group leads the row and is never dropped whole — the
        // one thing this line exists to say.
        assert!(row.trim_start().starts_with("idle"), "{width} -> {row:?}");
    }
}

#[test]
fn a_fresh_session_shows_the_empty_state_before_any_log_entries() {
    let mut app = app();
    assert!(app.log.is_empty());
    let out = rendered(&mut app, 110, 40);
    assert!(out.contains("Ask for a change, or / for commands."));
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
    let bottom = height - 1 - (1 + chrome::Composer::new("", width).height() + 1 + 1 + 1) - 1;
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
fn an_underscore_inside_a_word_is_a_literal_underscore() {
    // `ANTHROPIC_API_KEY` used to render as `ANTHROPICAPIKEY` — italic `API`,
    // both underscores eaten — because `_` opened emphasis anywhere. CommonMark
    // and GFM both forbid intraword `_` for exactly this reason, and a harness
    // whose transcript is full of snake_case identifiers, env-var names and
    // paths cannot silently delete characters out of them.
    let spans = parse_inline("export ANTHROPIC_API_KEY first", Style::default().fg(DARK.body), ctx(80));
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "export ANTHROPIC_API_KEY first");
    assert!(spans.iter().all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)));
}

#[test]
fn an_underscore_delimited_word_is_still_italic() {
    // The fix narrows `_`; it does not remove it. A delimiter with a word
    // boundary on the outside is the spelling CommonMark keeps.
    let spans = parse_inline("that is _neat_ stuff", Style::default().fg(DARK.body), ctx(80));
    let italic = spans.iter().find(|s| s.content.as_ref() == "neat").expect("italic span present");
    assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
    assert!(spans.iter().all(|s| !s.content.contains('_')), "literal underscores must not reach the screen");
}

#[test]
fn intraword_asterisks_still_emphasise() {
    // `*` and `_` differ deliberately: CommonMark allows intraword `*`, and
    // narrowing both would be a bigger change than the defect asked for.
    let spans = parse_inline("un*frigging*believable", Style::default().fg(DARK.body), ctx(80));
    let italic = spans.iter().find(|s| s.content.as_ref() == "frigging").expect("italic span present");
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

/// The flat text of one built row — what the terminal would show on it.
fn line_text(line: &ratatui::text::Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Where `needle` starts, counted in *cells* rather than bytes — a drawn
/// table's `│` and `─` are three bytes each, so `str::find`'s byte offset is
/// not the column anything appears in.
fn cell_pos(haystack: &str, needle: &str) -> Option<usize> {
    let at = haystack.find(needle)?;
    Some(haystack[..at].chars().count())
}

#[test]
fn a_markdown_table_lays_every_column_on_one_edge() {
    let table = "| name | status |\n| --- | --- |\n| read | allow |\n| shell | deny |\n";
    let text: Vec<String> = render_prose(table, ctx(80)).iter().map(line_text).collect();

    assert_eq!(text.len(), 6, "top rule, header, header rule, two rows, bottom rule: {text:?}");
    // `shell` is the widest cell in column one (5 cells), which with its two
    // pad cells and the two rules left of it puts column two's content on
    // cell 10 in every row of the block.
    for (row, cell) in [(1, "status"), (3, "allow"), (4, "deny")] {
        assert_eq!(cell_pos(&text[row], cell), Some(10), "column two must start on one edge in every row: {text:?}");
    }
    assert!(text.iter().all(|l| !l.contains('|')), "the source's ASCII pipes must not reach the screen: {text:?}");
}

/// A table is the design system's one stroked component — see ADR 0002. The
/// rest of Turn 13's rule still holds (a turn break, a markdown `---` and
/// every band boundary are bands), so this pins the exception's *shape*: a
/// closed box, every row the same width, and the interior rules on one
/// column all the way down.
#[test]
fn a_table_is_a_closed_drawn_grid() {
    let text: Vec<String> = render_prose("| a | bb |\n| --- | --- |\n| 1 | 2 |\n", ctx(40)).iter().map(line_text).collect();

    assert_eq!(text.len(), 5, "top rule, header, header rule, one row, bottom rule: {text:?}");
    for (row, (open, close)) in [(0, ('┌', '┐')), (2, ('├', '┤')), (4, ('└', '┘'))] {
        assert!(text[row].starts_with(open) && text[row].ends_with(close), "rule row {row} must be corner to corner: {text:?}");
        assert!(text[row][open.len_utf8()..].trim_end_matches(close).chars().all(|c| c == '─' || c == '┬' || c == '┼' || c == '┴'));
    }
    for row in [1, 3] {
        assert!(text[row].starts_with('│') && text[row].ends_with('│'), "a content row is closed on both sides: {text:?}");
    }

    let widths: Vec<usize> = text.iter().map(|l| l.chars().count()).collect();
    assert!(widths.iter().all(|w| *w == widths[0]), "every row of the box is the same width, or it does not close: {widths:?}");
    // The interior boundary sits on one column in all five rows — the
    // junction glyph changes, the column does not.
    // Skipping the row's own opening glyph, which is at column 0 in every
    // row and would otherwise be the match.
    let interior: Vec<Option<usize>> = text.iter().map(|l| l.chars().skip(1).position(|c| matches!(c, '┬' | '┼' | '┴' | '│')).map(|p| p + 1)).collect();
    assert!(interior.iter().all(|p| *p == interior[0]), "the interior rule must hold one column down the whole table: {text:?}");
}

/// A table arrives one line at a time and the transcript re-renders on every
/// delta, so every prefix of one has to render without panicking — and the
/// prefixes are the awkward states: a header with no delimiter yet, a
/// half-typed delimiter, a row cut mid-cell. It becomes a table only when the
/// delimiter row completes, and stays prose until then.
#[test]
fn a_table_renders_at_every_prefix_as_it_streams() {
    let full = "| tool | scope |\n| --- | ---: |\n| read | project |\n| shell | global |\n";
    let mut committed_at = None;

    for end in 1..=full.len() {
        if !full.is_char_boundary(end) {
            continue;
        }
        let text: Vec<String> = render_prose(&full[..end], ctx(60)).iter().map(line_text).collect();
        let drawn = text.iter().any(|l| l.starts_with('┌'));
        if drawn && committed_at.is_none() {
            committed_at = Some(end);
        }
        for line in &text {
            let width: usize = line.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
            assert!(width <= 60, "a partial table must still fit its column: {:?}", &full[..end]);
        }
    }

    // `| tool | scope |\n| --- | -` is the first prefix whose delimiter row
    // parses for both columns — one cell earlier there is only `| --- | `,
    // which has no dashes in its second cell and is therefore still prose.
    let at = committed_at.expect("the table must commit once its delimiter row is complete");
    assert_eq!(&full[..at], "| tool | scope |\n| --- | -", "a table commits on the delimiter row, not on the header: {:?}", &full[..at]);
}

/// A column bottoms out at one cell, so `n` columns need `3n + 1` cells.
/// Below that the box cannot close, and the deliberate choice (see
/// `render_table`'s doc) is to clip with a visible `…` rather than either
/// draw a `┐` where the table does not end or silently drop columns.
#[test]
fn a_table_with_more_columns_than_cells_clips_rather_than_lying() {
    let head = (0..14).map(|i| format!("| c{i} ")).collect::<String>() + "|";
    let delim = (0..14).map(|_| "| --- ").collect::<String>() + "|";
    let row = (0..14).map(|i| format!("| v{i} ")).collect::<String>() + "|";
    let text: Vec<String> = render_prose(&format!("{head}\n{delim}\n{row}\n"), ctx(20)).iter().map(line_text).collect();

    for line in &text {
        let width: usize = line.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
        assert_eq!(width, 20, "a clipped row still fills its column exactly, never overhangs it: {text:?}");
    }
    assert!(text[0].starts_with('┌'), "the table still opens: {text:?}");
    assert!(text.iter().any(|l| l.contains('…')), "the clip must be visible, not silent: {text:?}");
    assert!(!text[0].ends_with('┐'), "and it must not draw an edge where the table does not end: {text:?}");
}

/// The rules are `quiet` — the tier below `dim` — so the grid carries the
/// structure without competing with the cells for the reader's eye.
#[test]
fn a_tables_rules_are_quieter_than_its_content() {
    let lines = render_prose("| a | bb |\n| --- | --- |\n| 1 | 2 |\n", ctx(40));
    assert_eq!(lines[0].spans[0].style.fg, Some(DARK.quiet), "a rule row is drawn in `quiet`");
    let body = &lines[3];
    assert_eq!(body.spans[0].style.fg, Some(DARK.quiet), "a content row's own `│` is a rule too");
    let cell = body.spans.iter().find(|s| s.content.as_ref() == "1").expect("the cell's text is present");
    assert_eq!(cell.style.fg, Some(DARK.body), "the cell itself stays body-toned");
}

/// A header row alone is not a table — which is also what makes this safe
/// mid-stream, since a table arrives one line at a time and renders on
/// every delta.
#[test]
fn a_pipe_in_prose_is_not_a_table_without_a_delimiter_row() {
    let text: Vec<String> = render_prose("read a | b as either\nand carry on\n", ctx(80)).iter().map(line_text).collect();
    assert_eq!(text.len(), 2, "two prose lines, unchanged: {text:?}");
    assert!(text[0].contains('|'), "a stray pipe in prose stays literal text: {text:?}");
}

/// `Transcript`'s invariant — a built row is a screen row, and nothing
/// downstream wraps — applies to a table too, and a table is the one block
/// whose natural width has no relation to the column it lands in.
#[test]
fn a_table_wider_than_its_column_is_elided_rather_than_overhanging() {
    let wide = format!("| {} | {} |\n| --- | --- |\n| {} | b |\n", "x".repeat(60), "y".repeat(60), "z".repeat(60));
    for line in render_prose(&wide, ctx(40)) {
        let width: usize = line.spans.iter().map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref())).sum();
        assert!(width <= 40, "every table row must fit the column it is drawn in, got {width}: {:?}", line_text(&line));
    }
}

#[test]
fn a_right_aligned_column_shares_its_right_edge() {
    let text: Vec<String> = render_prose("| item | count |\n| --- | ---: |\n| a | 7 |\n", ctx(80)).iter().map(line_text).collect();
    let header_end = cell_pos(&text[1], "count").expect("the header cell is present") + "count".len();
    let cell_end = cell_pos(&text[3], "7").expect("the body cell is present") + 1;
    assert_eq!(cell_end, header_end, "a `---:` column's cells are flush to the column's right edge, not its left: {text:?}");
}

#[test]
fn a_table_in_an_assistant_reply_renders_as_a_grid_not_pipes() {
    let mut app = app();
    app.log.push(LogEntry::AssistantText { text: "| tool | access |\n| --- | --- |\n| read | allow |\n".into() });
    let out = rendered(&mut app, 100, 20);
    assert!(!out.contains('|'), "the source's ASCII pipes must not reach the screen: {out:?}");
    assert!(out.contains('┌') && out.contains('┼') && out.contains('┘'), "the grid must actually be drawn: {out:?}");
    assert!(out.contains("tool") && out.contains("allow"), "the table's own content must survive: {out:?}");
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

// ── The model picker's panel ─────────────────────────────────────────────

/// An `App` with a catalogue, as mjolnir-cli's bootstrap hands one in.
fn app_with_catalogue() -> App {
    app().with_catalogue(crate::first_run::sample_providers(), Some("bravo".into()))
}

fn picking(app: &mut App) {
    app.input = "/model".into();
    app.handle_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
}

/// The panel takes the composer's band exactly as a pending decision does —
/// there is nothing to type into while a list is waiting on an answer.
#[test]
fn the_picker_replaces_the_composer_and_lists_every_provider() {
    let mut app = app_with_catalogue();
    picking(&mut app);
    let out = rendered(&mut app, 120, 36);
    for provider in crate::first_run::sample_providers() {
        assert!(out.contains(&provider.id), "the whole catalogue is on offer: {} missing", provider.id);
    }
    assert!(!out.contains("Ask anything"), "the composer is not drawn while the picker is open: {out:?}");
}

/// The row the session is running on says so, so it stays findable once the
/// cursor has moved off it.
#[test]
fn the_running_provider_is_marked_current_on_its_row() {
    let mut app = app_with_catalogue();
    picking(&mut app);
    let out = rendered(&mut app, 120, 36);
    assert!(out.contains("· current"), "the row in use is named as such: {out:?}");
}

/// Taking a provider narrows the same control to that provider's models,
/// and the panel's badge names whose catalogue is on screen.
#[test]
fn taking_a_provider_shows_its_models_and_names_it() {
    let mut app = app_with_catalogue();
    picking(&mut app);
    app.handle_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
    let out = rendered(&mut app, 120, 36);
    assert!(out.contains("bravo-large"), "the chosen provider's models: {out:?}");
    assert!(out.contains("bravo-small"), "{out:?}");
    assert!(out.contains("bravo"), "the badge names the catalogue being shown: {out:?}");
}

/// The transcript recedes behind the picker for the same reason it recedes
/// behind a permission panel: the panel is the one live surface.
#[test]
fn the_transcript_fades_behind_the_picker() {
    let mut app = app_with_catalogue();
    app.log.push(LogEntry::AssistantText { text: "a line of history".into() });
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let before = terminal.backend().buffer().clone();
    let history = find_row(&before, "a line of history");
    let lit = before[(CONTENT_INDENT as u16, history)].fg;

    picking(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let after = terminal.backend().buffer().clone();
    assert_ne!(after[(CONTENT_INDENT as u16, history)].fg, lit, "the history behind the panel must recede");
}

#[test]
#[ignore = "visual aid; run with --ignored to eyeball the picker"]
fn dump_picker() {
    let mut app = app_with_catalogue();
    app.log.push(LogEntry::AssistantText { text: "a line of history".into() });
    picking(&mut app);
    for stage in 0..2 {
        if stage == 1 {
            app.handle_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
        }
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        println!("\n=== stage {stage} ===");
        for y in 0..24 {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            println!("{y:2}|{row}|");
        }
    }
}
