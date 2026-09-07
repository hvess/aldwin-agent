//! The frame's fixed furniture: the identity bar at the top, the live
//! activity strip, and the composer. Everything here writes to a `Frame`
//! directly rather than returning rows — none of it participates in the
//! log's scroll or the panel's row budget.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::grid::{elide, Ctx, CONTENT_INDENT, GROUP_GAP, MARGIN_X};
use crate::app::{cursor_line_col, App, RunningTool};
use crate::palette::Palette;
use crate::log::LogEntry;

/// The harness name, and nothing else — "the name is the brand, and a pip
/// there indicated nothing". Written once because two screens draw this bar:
/// the session's and first run's.
pub(super) const BRAND: &str = "mjolnir";

/// Cells between the brand and whatever follows it in the identity bar, so
/// that what follows lands on the body column. Derived from `BRAND`'s own
/// width rather than stated as a number — cell 13 is fixed by the grid, and
/// a second statement of it could only ever drift.
pub(super) fn brand_pad() -> usize {
    CONTENT_INDENT.saturating_sub(MARGIN_X).saturating_sub(BRAND.width())
}

/// `--spinner-frames` from `tokens/motion.css` — a quarter-block cycling at
/// roughly 100ms per frame. `App::tick` advances this every 120ms
/// (`run.rs`) — close enough to the token's ~100ms that a redraw-driven
/// tick (not a dedicated timer) reads as continuous motion.
const SPINNER_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// `▶` plus two spaces — the reference's own composer row is
/// `<span>▶</span><span>  </span>`, putting the draft's first character in
/// cell 6 (the grid's 3-cell `MARGIN_X`, the glyph, then the two). Every
/// cursor placement on line 0 has to add this back, since neither
/// `cursor_line_col` nor ratatui's `Wrap` has any notion of the prefix.
const PROMPT_PREFIX_LEN: u16 = 3;

/// Rows the composer itself needs: just the draft's own, since the blank
/// rows above and below it belong to the bottom bar
/// (`BottomBar.jsx`'s blank/composer/blank/status/blank), not to the
/// composer's own padding.
pub(super) fn input_height(input: &str) -> u16 {
    input.matches('\n').count() as u16 + 1
}

/// Persistent 3-row identity bar — `TopBar.jsx`'s "Session" section. Left:
/// the harness name alone, primary text, no glyph — per the design
/// system's own revision log ("the top bar carries no accent mark: the name
/// is the brand, and a pip there indicated nothing"), then the working
/// directory. Right: the model name and the running build version — the
/// closest real facts Mjolnir has to the reference's `model · gauge · cost`
/// group; a context-window gauge and a per-session cost aren't tracked
/// anywhere in `StatusInfo`, so neither is fabricated here.
pub(super) fn draw_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    // No rule under the bar. It is "its own ground, a step off the
    // transcript" — the whole 3-row band is `bar`, and the step down to
    // the transcript's `ground` beneath it is the boundary. Content still
    // sits on row 1, centred in the band.
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar)), area);
    let content_row = Rect { y: area.y + 1, height: 1, ..area };
    let width = area.width as usize;
    let on_bar = |fg| Style::default().fg(fg).bg(pal.bar);

    // Laid out by `identity_bar_row`, which first run's bar shares — see it
    // for why the two groups are measured together and what each gives up
    // when they do not both fit.
    //
    // Three tokens on the right, not one: the reference's right group is
    // `quiet` for the model, `dim` for the `·` separators, and `text` for
    // the last fact in the group — rendering the whole group in a single
    // `label` flattened a deliberate three-step hierarchy into one tone. One
    // space each side of the `·`: these are facts *within* one group, and
    // `--group-gap` is what parts this group from the identity one.
    let model = app.status.model_name.clone();
    let version = format!("v{}", app.status.version);
    let cwd = app.status.cwd.clone().unwrap_or_default();

    let row = identity_bar_row(
        width,
        &cwd,
        pal.quiet,
        vec![
            vec![
                Span::styled(model.clone(), on_bar(pal.quiet)),
                Span::styled(" · ", on_bar(pal.dim)),
                Span::styled(version, on_bar(pal.text)),
            ],
            vec![Span::styled(model, on_bar(pal.quiet))],
            Vec::new(),
        ],
        pal,
    );
    frame.render_widget(Paragraph::new(row).style(Style::default().bg(pal.bar)), content_row);
}

/// The identity bar's one content row, composed whole.
///
/// Both bars that draw it — the session's and first run's — go through here,
/// because their doc comments already promised the two "must not disagree"
/// and they had drifted: each was laying its own groups out by hand.
///
/// The two groups are measured **together**. They used to be two independent
/// half-width rects that could not see each other, so each filled to its own
/// boundary: below ~56 cells the working directory ran straight into the
/// model name with no gap at all, and above that both were cut at the seam
/// with nothing marking it — a bar reading `~/Projects/mjolnir-harnes` and
/// `v0.1.`, neither of which is a true statement. Composing one line is what
/// makes `--group-gap` guaranteed rather than incidental.
///
/// `right` is the right-hand group in descending order of preference. The
/// first candidate that still leaves the working directory a readable
/// stretch wins; if none does, the group is dropped. The asymmetry is
/// deliberate: a right-hand **fact goes whole or not at all**, because a
/// clipped `v0.1.` is not a version and would be read as one, while the cwd
/// is **elided**, because `…` states plainly that a path was shortened. Pass
/// an empty `Vec` as the last candidate to allow dropping the group.
pub(super) fn identity_bar_row(
    width: usize,
    cwd: &str,
    cwd_fg: Color,
    right: Vec<Vec<Span<'static>>>,
    pal: &Palette,
) -> Line<'static> {
    let on_bar = |fg| Style::default().fg(fg).bg(pal.bar);
    let field = Style::default().bg(pal.bar);

    // What the identity group must keep before the right-hand one starts
    // giving anything up. An elided path shorter than this says nothing
    // useful — `~/P…` is not a location.
    const MIN_CWD: usize = 12;
    let wanted = cwd.width().min(MIN_CWD);
    // Cells the cwd would have left, if the right group took `right_w`.
    let room_for = |right_w: usize| width.saturating_sub(CONTENT_INDENT + GROUP_GAP + MARGIN_X + right_w);
    let group_width = |g: &Vec<Span<'static>>| -> usize { g.iter().map(|s| s.content.width()).sum() };

    let chosen = right
        .into_iter()
        .find(|candidate| room_for(group_width(candidate)) >= wanted)
        .unwrap_or_default();
    let right_w = group_width(&chosen);

    // "mjolnir   ~/src/gateway" — the directory starts on the body column,
    // cell 13, like every other left-hand word in the system. That is a pad
    // derived from the brand's own width, not a gap; see `grid::GROUP_GAP`.
    let cwd = elide(cwd, room_for(right_w));
    let mut spans = vec![Span::styled(" ".repeat(MARGIN_X), field), Span::styled(BRAND, on_bar(pal.text))];
    if !cwd.is_empty() {
        spans.push(Span::styled(" ".repeat(brand_pad()), field));
        spans.push(Span::styled(cwd.clone(), on_bar(cwd_fg)));
    }

    let used = MARGIN_X + BRAND.width() + if cwd.is_empty() { 0 } else { brand_pad() + cwd.width() };
    let gap = width.saturating_sub(used).saturating_sub(right_w).saturating_sub(MARGIN_X);
    spans.push(Span::styled(" ".repeat(gap), field));
    spans.extend(chosen);
    spans.push(Span::styled(" ".repeat(MARGIN_X), field));
    Line::from(spans)
}

/// A `RunningTool`'s display name — `name` comes from a `ToolUseRequested`
/// looked up by `call_id` and falls back to an empty string if that lookup
/// ever misses; falling back to the `call_id` itself here (rather than
/// showing nothing) is what the status line's trailing tools list already
/// did, shared so [`activity_label`]'s leading word can't drift from it.
fn running_tool_name(tool: &RunningTool) -> &str {
    if tool.name.is_empty() {
        &tool.call_id
    } else {
        &tool.name
    }
}

/// Present-progressive fragment for the status line's leading activity word
/// — a separate small table from the permission prompt's own humanization,
/// since the two need different grammar ("The agent wants to read a file"
/// vs "reading a file…") for what's otherwise the same handful of tool
/// kinds; not worth a shared abstraction for three arms each.
fn tool_gerund(kind: &str) -> String {
    match kind {
        "read" => "reading a file".into(),
        "shell" => "running a shell command".into(),
        "explain" => "inspecting code".into(),
        other => format!("using {other}"),
    }
}

/// What to say next to the spinner while `app.turn_active` and not
/// `app.thinking` (thinking has its own, more specific text) — per direct
/// developer feedback that a bare "working…" for the entire stretch of a
/// turn gave no sense of what was actually happening. Built entirely from
/// state `App` already tracks: a running tool's own name, the count when
/// more than one tool is running at once (parallel dispatch), or, with no
/// tool in flight, whether assistant text is already streaming for this
/// step versus still waiting on the step's first token or tool call.
fn activity_label(app: &App) -> String {
    match app.status.running_tools.as_slice() {
        [] => {
            if matches!(app.log.last(), Some(LogEntry::AssistantText { .. })) {
                "responding…".into()
            } else {
                "working…".into()
            }
        }
        [one] => format!("{}…", tool_gerund(running_tool_name(one))),
        many => format!("running {} tools…", many.len()),
    }
}

/// Live activity strip, one row, directly below the composer —
/// `StatusLine.jsx`'s own job, distinct from [`draw_top_bar`]'s static
/// identity row. Shows what's actually happening with the model: live
/// activity (thinking/working/idle, with a spinner), turn/step, any tools
/// in flight by name (in `value`, uniformly — a tool's *state* carries
/// colour here, not its identity), a running message count, and a
/// right-aligned Ctrl+C hint.
///
/// Permission state (read/shell/edit) is deliberately absent — it belongs
/// to the once-per-session welcome hero and to an actual permission prompt
/// when one fires, not to a line that repaints every frame.
pub(super) fn draw_status_line(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    let s = &app.status;
    // Raised ground — `StatusLine.jsx` sits inside `BottomBar.jsx`'s own
    // `bar-bottom` field, not the plain frame `ground` the log panel uses.
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), area);
    // Activity leads the row — per explicit developer request that "what's
    // the LLM doing right now" is the single most useful thing this line
    // can say, so it shouldn't be buried after the model name/turn counter.
    let spinner = SPINNER_FRAMES[app.tick as usize % SPINNER_FRAMES.len()];
    let running = |text: String| {
        vec![Span::styled(format!("{spinner} "), Style::default().fg(pal.glyph_running)), Span::styled(text, Style::default().fg(pal.label))]
    };
    let mut spans = if app.thinking {
        running("thinking…  ".into())
    // `awaiting_turn` counts as active here as well as in the hint below —
    // between submitting and `TurnStarted` landing the harness is waiting
    // on the provider, and reporting that stretch as "idle" is exactly the
    // no-progress-feedback complaint `activity_label` exists to answer.
    } else if app.turn_active || app.awaiting_turn {
        running(format!("{}  ", activity_label(app)))
    } else {
        vec![Span::styled("idle  ", Style::default().fg(pal.label))]
    };

    let turn_step = match (s.turn, s.step) {
        (Some(t), Some(st)) => format!("T{t} S{st}"),
        (Some(t), None) => format!("T{t}"),
        _ => "-".to_string(),
    };
    spans.push(Span::styled(format!("{}  {turn_step}  ", s.model_name), Style::default().fg(pal.label)));

    if !s.running_tools.is_empty() {
        spans.push(Span::styled("tools: ", Style::default().fg(pal.label)));
        for (i, tool) in s.running_tools.iter().enumerate() {
            if i > 0 {
                // `label`, not a bare `Span::raw` — the separator carries a
                // visible glyph, and `Style::default()` is the *terminal's*
                // default foreground, not a token: on a light-profile
                // terminal it renders near-black against this bar and on a
                // dark one near-white, the one cell in the whole frame not
                // under the palette's control. Same rule
                // `highlight_command_tokens` states for composer words; the
                // whitespace `Span::raw`s elsewhere are exempt only because
                // they paint no glyph.
                spans.push(Span::styled(", ", Style::default().fg(pal.label)));
            }
            spans.push(Span::styled(running_tool_name(tool).to_string(), Style::default().fg(pal.value)));
        }
        spans.push(Span::raw("  "));
    }

    let messages = app.log.len();
    spans.push(Span::styled(format!("{messages} message{}", if messages == 1 { "" } else { "s" }), Style::default().fg(pal.label)));

    // Right-aligned key hint — `StatusLine.jsx`'s own `right` prop (`esc to
    // stop`), adapted to Mjolnir's real binding: Ctrl+C, not Esc, is what
    // cancels a turn or exits an idle session. Must read the same "is
    // anything running" state `App::cancel_or_quit` acts on, or the hint
    // promises one thing and the key does the other.
    let hint = if app.turn_active || app.awaiting_turn { "^c to cancel" } else { "^c to exit" };
    let [left_area, right_area] = Layout::horizontal([Constraint::Min(1), Constraint::Length(hint.width() as u16 + MARGIN_X as u16)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)).block(Block::new().padding(Padding::left(MARGIN_X as u16))), left_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(pal.dim)))).alignment(Alignment::Right).block(Block::new().padding(Padding::right(MARGIN_X as u16))),
        right_area,
    );
}

/// Every word `cli::slash::intercept` actually dispatches on, `/`-prefixed
/// here to match whole-word input tokens directly. Duplicated because tui
/// can't depend on cli; purely a hint for [`highlight_command_tokens`], so
/// keep in sync by hand if slash.rs's arms change — `/theme` was added
/// 2026-09-02 alongside that command, caught only by remembering this
/// comment's own instruction, not by a compiler or test forcing the two
/// files to agree.
const KNOWN_COMMAND_WORDS: [&str; 5] = ["/help", "/clear", "/exit", "/reload-config", "/theme"];

/// Dims every word in `line` that exactly matches a known command, no
/// matter where it falls — per explicit developer direction, this is a
/// cosmetic hint only and deliberately does *not* mirror
/// `transcript::is_command`'s "whole message must start with `/`" rule: a
/// real slash command only fires when it's the very first thing in the
/// message, so `/exit` typed mid-sentence never actually gets intercepted —
/// it's still worth flagging live so the developer notices they typed a
/// recognized command word, wherever it landed.
///
/// An ordinary word must carry an explicit `text` fg rather than a bare
/// `Style::default()` (terminal-default foreground): the composer fills its
/// own background regardless of the developer's terminal theme, and on a
/// light-mode profile "terminal-default foreground" is typically dark,
/// which rendered a typed word dark-on-dark. Reported directly: "text is
/// dark on light mode and it clashes with the dark background."
pub(super) fn highlight_command_tokens(line: &str, ctx: Ctx) -> Line<'static> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let ws_len: usize = rest.chars().take_while(|c| c.is_whitespace()).map(|c| c.len_utf8()).sum();
        if ws_len > 0 {
            let (ws, tail) = rest.split_at(ws_len);
            spans.push(Span::raw(ws.to_string()));
            rest = tail;
            continue;
        }
        let word_len: usize = rest.chars().take_while(|c| !c.is_whitespace()).map(|c| c.len_utf8()).sum();
        let (word, tail) = rest.split_at(word_len);
        let fg = if KNOWN_COMMAND_WORDS.contains(&word) { ctx.pal.dim } else { ctx.pal.text };
        spans.push(Span::styled(word.to_string(), Style::default().fg(fg)));
        rest = tail;
    }
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    Line::from(spans)
}

/// The composer: no drawn border at all — per explicit developer feedback
/// against a real screenshot that the left accent bar it used to carry read
/// as stray decoration, not something the input needed. `bar_bottom` is its
/// field; `Composer.jsx` has no surface of its own beyond `BottomBar.jsx`'s
/// raised ground, and the prompt `▶` and caret carry the accent instead.
pub(super) fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    let ctx = Ctx::new(pal, area.width);
    // `MARGIN_X` horizontally (the grid's `padding: 0 27px`), no vertical
    // padding — the blank rows around the composer belong to the bottom bar
    // itself, not to this widget.
    let block = Block::new().style(Style::default().bg(pal.bar_bottom)).padding(Padding::horizontal(MARGIN_X as u16));
    let inner = block.inner(area);
    let blocked = !app.pending_approvals.is_empty() || !app.pending_prompts.is_empty();
    let prompt = || Span::styled("▶  ", Style::default().fg(pal.mark));

    // Dim placeholder text when the draft is empty — an empty filled box
    // gave no hint at all that this was where a message goes. While
    // blocked, the placeholder says so instead of inviting a keystroke it
    // would silently drop.
    if app.input.is_empty() {
        let text = if blocked { "waiting on your decision above…" } else { "Ask Mjolnir anything" };
        let placeholder = Line::from(vec![prompt(), Span::styled(text, Style::default().fg(pal.dim))]);
        frame.render_widget(Paragraph::new(placeholder).block(block), area);
        if !blocked {
            frame.set_cursor_position((inner.x + PROMPT_PREFIX_LEN, inner.y));
        }
        return;
    }

    // Live counterpart to the dim styling of an already-submitted slash
    // command in the log — without this, a command only reads as "directed
    // at the harness, not the model" after Enter, not while it's being
    // typed. The prompt glyph marks the first line only (a multi-line draft
    // is Mjolnir's own extension beyond the reference's single-line
    // composer).
    let lines: Vec<Line> = app
        .input
        .split('\n')
        .enumerate()
        .map(|(i, l)| {
            let mut line = highlight_command_tokens(l, ctx);
            if i == 0 {
                line.spans.insert(0, prompt());
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block).wrap(Wrap { trim: false }), area);

    // No visible cursor at all was a standing complaint. Ratatui doesn't
    // draw one; `set_cursor_position` asks the real terminal cursor to sit
    // there instead. Skipped while a decision is pending (input is blocked
    // then, and the placeholder says so). `cursor_line_col` counts by
    // source line, not wrapped screen row, so a single logical line long
    // enough to wrap past the box's width would place the cursor past the
    // visible text — clamped to `inner`'s last column/row below so it never
    // lands outside the box, rather than fixing the underlying wrap
    // mismatch.
    if !blocked {
        let (line, col) = cursor_line_col(&app.input, app.cursor);
        let prefix = if line == 0 { PROMPT_PREFIX_LEN } else { 0 };
        let x = (inner.x + prefix + col as u16).min(inner.x + inner.width.saturating_sub(1));
        let y = (inner.y + line as u16).min(inner.y + inner.height.saturating_sub(1));
        frame.set_cursor_position((x, y));
    }
}
