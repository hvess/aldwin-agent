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

/// Every cell the frame paints must carry palette colours, not the
/// terminal's own defaults — the invariant behind `palette.rs` existing at
/// all, asserted here rather than left to a reader spotting a bare
/// `Span::raw` in review.
///
/// `Color::Reset` means "whatever this terminal paints by default", so a
/// cell carrying one is outside the design system: it renders near-black on
/// a light-profile terminal and near-white on a dark one, and no palette
/// change can move it. Two separate rules, because the two channels fail
/// differently:
///
/// * **Background** — never `Reset` anywhere, blank cells included. A
///   `Reset` background is a hole in the opaque canvas `ui::draw` paints
///   first, showing the developer's terminal through the frame.
/// * **Foreground** — never `Reset` on a cell that actually carries a
///   glyph. Whitespace is exempt: `grid`/`row` build margins and gutters
///   from bare `Span::raw`, which paints no ink, so constraining those
///   would forbid an idiom that is genuinely colourless.
///
/// Found one real violation when written: the `, ` between tool names in
/// `chrome::draw_status_line` was a bare `Span::raw`, the single cell in
/// the whole 315k-cell corpus painting a visible glyph in the terminal's
/// foreground rather than a token.
#[test]
fn every_painted_cell_uses_a_palette_colour_never_the_terminals_own() {
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let where_ = |x: u16, y: u16| format!("{theme:?} {scene_name} {width}x{height} at ({x},{y})");
                for y in 0..height {
                    for x in 0..width {
                        let cell = &buffer[(x, y)];
                        assert_ne!(
                            cell.bg,
                            ratatui::style::Color::Reset,
                            "{}: background is Color::Reset — the terminal's own background shows through the frame here",
                            where_(x, y)
                        );
                        if cell.symbol().trim().is_empty() {
                            continue;
                        }
                        assert_ne!(
                            cell.fg,
                            ratatui::style::Color::Reset,
                            "{}: glyph {:?} is painted in Color::Reset — the terminal's own foreground, not a palette token \
                             (a bare `Span::raw`/`Style::default()` carrying visible text)",
                            where_(x, y),
                            cell.symbol()
                        );
                    }
                }
            }
        }
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

// ── Grid conformance ────────────────────────────────────────────────────
//
// The snapshot above proves a render is *unchanged*. These prove it is
// *correct* — that every scene sits on the design system's grid rather than
// merely on the grid it sat on yesterday. Without them a wrong column is
// preserved as faithfully as a right one, which is how the label column
// stayed at 12 cells for as long as it did.
//
// Measured off the rendered buffer, never off the constants, per this
// project's "render it before trusting your reading of it" rule: reading
// `MARGIN_X` back out of the code and asserting it equals itself proves
// nothing about what a developer sees.

/// The design system's horizontal landmarks, in cells
/// (`tokens/cells.css`). Restated here on purpose: a test that imported
/// them from the code under test could only ever agree with it.
const MARGIN: usize = 3;
const LABEL_COL: usize = 8;
const LABEL_GUTTER: usize = 2;
const BODY_COL: usize = MARGIN + LABEL_COL + LABEL_GUTTER; // cell 13

/// Cell index of the first non-blank glyph on a row, or `None` if blank.
///
/// Counts *cells*, not bytes — `▌` is three bytes, and measuring columns
/// with `str::find` reports everything past a mark two cells right of where
/// it is.
fn first_glyph(buffer: &Buffer, y: u16) -> Option<usize> {
    (0..buffer.area.width).find(|x| buffer[(*x, y)].symbol().trim() != " " && !buffer[(*x, y)].symbol().trim().is_empty()).map(|x| x as usize)
}

fn last_glyph(buffer: &Buffer, y: u16) -> Option<usize> {
    (0..buffer.area.width).rev().find(|x| !buffer[(*x, y)].symbol().trim().is_empty()).map(|x| x as usize)
}

/// Every scene, both themes, at the design's own 120×36 frame.
fn every_scene(mut f: impl FnMut(&str, Theme, &Buffer)) {
    for theme in [Theme::Dark, Theme::Light] {
        for name in SCENES {
            let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
            scene(name, &mut app);
            let buffer = render(&mut app, 120, 36);
            f(name, theme, &buffer);
        }
    }
}

/// Nothing is drawn inside the 3-cell left margin, and nothing inside the
/// 3-cell right margin.
///
/// The one deliberate exception is a selectable option row, which the
/// design runs flush to the frame's own left edge so its `▌` mark lands in
/// cell 0 ("Four option rows, flush to the frame's left edge"). That is the
/// *only* row type allowed to start before the margin, so the exception is
/// spelled as "cell 0 and the glyph is a mark" rather than as "anything
/// before cell 3".
#[test]
fn every_scene_respects_the_three_cell_margins() {
    every_scene(|name, theme, buffer| {
        for y in 0..buffer.area.height {
            let Some(first) = first_glyph(buffer, y) else { continue };
            let is_option_row = first == 0 && buffer[(0, y)].symbol() == "▌";
            assert!(
                first >= MARGIN || is_option_row,
                "{theme:?}/{name} row {y}: content starts in cell {first}, inside the 3-cell margin, and is not a flush option row"
            );
            let last = last_glyph(buffer, y).unwrap();
            let right_edge = buffer.area.width as usize - 1;
            assert!(
                last <= right_edge - MARGIN,
                "{theme:?}/{name} row {y}: content reaches cell {last}, inside the 3-cell right margin (frame ends at {right_edge})"
            );
        }
    });
}

/// A transcript turn puts its speaker on the margin and its content on the
/// body column. Both halves matter: the label proves the margin, and the
/// content proves the 8-cell label column plus its 2-cell gutter, which is
/// the measurement that was wrong for the longest.
#[test]
fn transcript_turns_use_the_label_column_and_the_body_column() {
    let mut seen = 0;
    every_scene(|name, theme, buffer| {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<Vec<_>>().join("");
            let cells: Vec<&str> = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect();
            for label in ["you", "harness"] {
                // The label must be at the margin, and must be the row's
                // first glyph — otherwise this is prose that merely
                // contains the word.
                if first_glyph(buffer, y) != Some(MARGIN) {
                    continue;
                }
                if !cells[MARGIN..MARGIN + label.len()].concat().eq(label) {
                    continue;
                }
                seen += 1;
                let after = cells[MARGIN + label.len()..BODY_COL].concat();
                assert!(after.trim().is_empty(), "{theme:?}/{name} row {y}: the label column must be padding after {label:?}, got {after:?}");
                let body_start = (BODY_COL..buffer.area.width as usize).find(|x| !cells[*x].trim().is_empty());
                assert_eq!(
                    body_start,
                    Some(BODY_COL),
                    "{theme:?}/{name} row {y}: a {label:?} turn's content must begin on the body column, cell {BODY_COL}: {row:?}"
                );
            }
        }
    });
    assert!(seen > 0, "the scenes must actually contain transcript turns, or this test proves nothing");
}

/// The chrome bands are whole rows of one tone, and the boundaries between
/// them are tonal rather than drawn — the Turn 13 rule, asserted over every
/// scene rather than the one screen it was first checked on.
#[test]
fn every_scene_parts_its_bands_by_tone_and_draws_no_rules() {
    every_scene(|name, theme, buffer| {
        // The top bar is three rows of one colour, and the row under it is
        // a different one.
        let bar = buffer[(0, 0)].bg;
        for y in 0..3u16 {
            assert_eq!(buffer[(0, y)].bg, bar, "{theme:?}/{name}: the top bar is 3 rows of one tone");
        }
        assert_ne!(buffer[(0, 3)].bg, bar, "{theme:?}/{name}: the band below the top bar must differ in tone — that step is the boundary");

        // Nothing anywhere is stroked, and no cell carries an underline
        // standing in for a border.
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                assert!(
                    !"─│┌┐└┘├┤┬┴┼╭╮╰╯━┃║╔╗╚╝▁▔".contains(cell.symbol()),
                    "{theme:?}/{name} at {x},{y}: {:?} is a box-drawing glyph — nothing inside a frame is stroked",
                    cell.symbol()
                );

            }

            // A border drawn as a cell attribute: a whole row of blank
            // cells carrying an underline, which is exactly the shape the
            // bars used before Turn 13. Asserted per row rather than per
            // cell, because an underlined *space inside a markdown heading*
            // is legitimate text styling, not a rule.
            let underlined_blanks = (0..buffer.area.width)
                .filter(|x| {
                    let cell = &buffer[(*x, y)];
                    cell.modifier.contains(ratatui::style::Modifier::UNDERLINED) && cell.symbol().trim().is_empty()
                })
                .count();
            assert!(
                underlined_blanks < buffer.area.width as usize / 2,
                "{theme:?}/{name} row {y}: {underlined_blanks} blank underlined cells — that is a border drawn as an attribute"
            );
        }
    });
}

/// Scrollbars are listed under "Deliberately absent" in the design system,
/// beside tabs, breadcrumbs and "any control that needs a mouse". One used
/// to render down the right edge whenever the transcript overflowed — the
/// single element in the frame that sat outside the right margin.
#[test]
fn no_scene_draws_a_scrollbar() {
    every_scene(|name, theme, buffer| {
        let right = buffer.area.width - 1;
        for y in 0..buffer.area.height {
            let symbol = buffer[(right, y)].symbol();
            assert!(
                symbol.trim().is_empty(),
                "{theme:?}/{name} row {y}: {symbol:?} in the frame's last column — a scrollbar track is deliberately absent"
            );
        }
    });
}
