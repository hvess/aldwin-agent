use mjolnir_permissions::PromptPayload;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{cursor_line_col, App, PermState};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const BRIGHT: Color = Color::White;
// A dedicated LightGreen was tried first for user/assistant separation
// (see the git history) but read as too loud against real terminal color
// schemes, per explicit developer feedback — swapped for a muted gray text
// color plus a subtle background tint, which separates user input from
// both assistant text (BRIGHT, no bg) and dim metadata without fighting
// the terminal's own palette. Fixed RGB rather than a named ANSI color so
// the "subtle" tint doesn't get reinterpreted by whatever the terminal
// theme maps that ANSI slot to.
const USER_FG: Color = Color::Rgb(190, 190, 195);
const USER_BG: Color = Color::Rgb(40, 40, 46);

/// Inline `` `code` `` in assistant prose used `Modifier::REVERSED` (fg/bg
/// swapped) to stand out, which reads as a jarring bright-white block
/// against most terminal themes — per explicit developer feedback, swapped
/// for a plain distinguishing color, same fixed-RGB-not-named-ANSI
/// reasoning as `USER_FG`/`USER_BG` above. A one-off, scoped exception to
/// mjolnir-tui.md's "modifiers only, never a new color" rule for inline
/// markdown — that rule was written when the alternative on the table was
/// REVERSED, not a plain fg color; superseded by this developer's later,
/// more specific ask.
const CODE_FG: Color = Color::Rgb(224, 175, 104);

/// Approval-card diff coloring: a full-width background tint (same
/// technique as `USER_BG`) behind added/removed lines so a diff reads at a
/// glance instead of every line rendering in the same plain `BRIGHT`, which
/// is what the card did before — per explicit developer feedback that
/// diffs "are not very clear."
const DIFF_ADD_BG: Color = Color::Rgb(28, 46, 30);
const DIFF_ADD_FG: Color = Color::Rgb(150, 210, 160);
const DIFF_DEL_BG: Color = Color::Rgb(48, 28, 28);
const DIFF_DEL_FG: Color = Color::Rgb(220, 150, 150);

/// Braille-dot spinner frames — the same glyph family `MJOLNIR_ART` traces
/// the hammer in, so the "ascii trick" loading indicator reads as part of
/// the same visual language rather than a mismatched borrowed spinner.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// `tick` is `App::tick` — a free-running frame counter, not wall-clock
/// time, so this stays deterministic and testable without a real clock.
fn spinner_line(tick: u64, label: &str) -> Line<'static> {
    let frame = SPINNER_FRAMES[tick as usize % SPINNER_FRAMES.len()];
    Line::from(Span::styled(format!("{frame} {label}…"), Style::default().fg(DIM)))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let input_height = input_area_height(&app.input);
    // A blank row between the log and the status bar — without it the last
    // rendered line of LLM output butts directly against the status line,
    // per explicit developer feedback that the two need breathing room.
    let [log_area, _spacer, status_area, input_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
    ])
    .areas(area);

    // Scroll math needs the real render width before `total_lines` (called
    // by `set_viewport_height` below) can count wrapped rows correctly —
    // see `App::render_width`'s doc comment.
    app.render_width = log_area.width;
    let log_inner_height = log_area.height as usize;
    app.scroll.set_viewport_height(log_inner_height, app.total_lines());

    draw_log(frame, log_area, app);
    draw_status(frame, status_area, app);
    draw_input(frame, input_area, app);
}

fn input_area_height(input: &str) -> u16 {
    // +2 for the border; at least 3 total so a single-line draft still gets
    // a visible box, matching "multi-line textarea" without it collapsing
    // to a single row when empty.
    let lines = input.matches('\n').count() as u16 + 1;
    (lines + 2).max(3)
}

/// Builds every line the log area can show — banner, entries, separators,
/// the transient "thinking…" indicator — at `width`. Shared by `draw_log`
/// (renders it) and `log_row_count` (counts its wrapped rows for scroll
/// math), so the two can never disagree about what the log contains.
fn build_log_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = intro_lines(&app.status.model_name, width);
    // Separates the banner from the first real entry, same as the
    // inter-entry separator below — skipped when the log is still empty so
    // a fresh session doesn't end in a trailing blank line.
    if !app.log.is_empty() {
        lines.push(Line::default());
    }
    for (i, entry) in app.log.iter().enumerate() {
        // Blank line between entries — not just at the user/assistant
        // boundary, since every entry kind benefits from more breathing
        // room, per explicit feedback that the log felt visually cramped.
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(render_entry(entry, width));
    }
    // An animated spinner rather than static text — per explicit developer
    // feedback that waiting for the next turn gave no loading/progress
    // feedback at all. `thinking` (an extended-thinking block) takes
    // priority over the broader `turn_active` (true for the whole turn,
    // including the stretch between tool calls and before the first token
    // streams back, which `thinking` alone doesn't cover) since only one of
    // the two labels is shown at a time.
    if app.thinking {
        lines.push(spinner_line(app.tick, "thinking"));
    } else if app.turn_active {
        lines.push(spinner_line(app.tick, "working"));
    }
    lines
}

fn draw_log(frame: &mut Frame, area: Rect, app: &App) {
    let lines = build_log_lines(app, area.width);
    // `scroll.offset` is in *wrapped screen rows* (see `log_row_count`), so
    // it must go through `Paragraph::scroll`, which advances the same
    // wrapping line-composer `Paragraph::line_count` uses internally —
    // not a `.skip()` on `lines` beforehand, which would count in logical
    // (pre-wrap) rows instead and drift out of sync the moment anything
    // wraps.
    let offset = app.scroll.offset.min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).scroll((offset, 0));
    frame.render_widget(paragraph, area);
}

/// The number of terminal rows the log area needs to fully render at
/// `width` — what `ScrollState` (see `scroll.rs`) actually compares against
/// viewport height, wrapping included. Delegates to ratatui's own
/// `Paragraph::line_count`, which runs the exact same word-wrapper
/// `draw_log`'s render path uses, rather than re-deriving wrap behaviour by
/// hand — the previous approach (`log::line_count`, counting logical source
/// lines) silently under- or over-counted the moment any single line — a
/// long tool-result summary, a long retry message, a long assistant line —
/// was wide enough to wrap, corrupting `ScrollState`'s offset math and
/// clipping content at the bottom of the log area. See mjolnir-tui.md's
/// 2026-08-29 scrolling-fix Progress note for the incident this replaces.
pub(crate) fn log_row_count(app: &App, width: u16) -> usize {
    let lines = build_log_lines(app, width);
    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).line_count(width)
}

/// The welcome banner shown above the conversation log on every draw: the
/// "MJOLNIR" wordmark, then a Mjolnir (Thor's hammer) mark beside the
/// tagline/version line — a mascot pivot away from mjolnir.md's
/// originally-decided little owl, per explicit developer direction toward
/// something more "aggressive/directive" (see mjolnir.md's Mascot section
/// for the superseded rationale, and this function's own history for the
/// several prior hammer designs: two procedural crosshatch-weave textures,
/// then this literal photo trace rendered as Braille dots). `MJOLNIR_ART`
/// is a literal trace of a real reference photo (thresholded, trimmed,
/// resized preserving aspect, Gaussian-blurred before thresholding so fine
/// knotwork linework survives as continuous strokes instead of fragmenting
/// into speckle, read back one source pixel per Braille dot — 2×4 real
/// sub-character dots per cell) rather than a hand-drawn or
/// procedurally-generated shape — per explicit developer feedback that
/// procedural attempts "weren't a true representation" of the reference
/// images supplied. This is that original 21-row trace, restored after a
/// detour: a later "fix trace quality, shrink it" pass replaced it with a
/// smaller, differently-traced 16-row mark to fit beside the info block,
/// and a further pass mirror-symmetrized *that* mark's head — but the
/// smaller trace's head read as flat noise next to its own eye/loop, and
/// no amount of further hand-tuning (restoring its pre-symmetry-fix
/// linework, then a from-scratch symmetric double-loop) matched the
/// detail of this original — per explicit developer feedback across that
/// whole detour. Wider (35 vs. 27 chars) and taller (21 vs. 16 rows) than
/// the mark it replaces; the info column beside it, the vertical-centering
/// math, and `log::INTRO_LINE_COUNT` all derive from `MJOLNIR_ART.len()`/
/// `MJOLNIR_ART_WIDTH` rather than hardcoding the row count, so they
/// scale with it automatically. `WORDMARK_ART` is FIGlet's "ANSI Shadow" font — found
/// after several earlier wordmark attempts (hand-drawn angular block
/// letters; FIGlet's "Colossal"; FIGlet's "Whimsy", found by grepping the
/// ~370-font xero/figlet-fonts collection for a fragment the developer had
/// pasted; two further reference pastes that turned out not to be
/// standard FIGlet fonts at all, most likely output from a gradient-shaded
/// text-art generator rather than a monospace font file) — until the
/// developer pasted a code snippet naming a `LOGO_ART` constant in this
/// exact font rendering a different two-word product name, asking for the
/// same treatment on "MJOLNIR"; the font itself (already fetched earlier
/// in the session while chasing a different lead) needed no rediscovery,
/// just re-rendering. See this crate's git history for the generating
/// scripts; neither is kept in the repo since they're one-time art
/// pipelines, not runtime code. Always exactly `log::INTRO_LINE_COUNT`
/// lines — that constant is a plain `usize` (not derived from this
/// function) so `ui.rs`'s own tests can compute banner-relative row
/// offsets without duplicating this shape (see `INTRO_LINE_COUNT`'s doc
/// comment); keep the two in sync by hand if either array or the border
/// changes shape. Styled uniformly ACCENT+BOLD — a
/// traced silhouette has no shading gradient to speak of, so per-glyph
/// styling would be pointless; ACCENT is still the one deliberate
/// expansion of accent beyond "card border and focused input only" (see
/// the Palette Progress note in mjolnir-tui.md).
const WORDMARK_ART: [&str; 6] = [
    "███╗   ███╗     ██╗ ██████╗ ██╗     ███╗   ██╗██╗██████╗ ",
    "████╗ ████║     ██║██╔═══██╗██║     ████╗  ██║██║██╔══██╗",
    "██╔████╔██║     ██║██║   ██║██║     ██╔██╗ ██║██║██████╔╝",
    "██║╚██╔╝██║██   ██║██║   ██║██║     ██║╚██╗██║██║██╔══██╗",
    "██║ ╚═╝ ██║╚█████╔╝╚██████╔╝███████╗██║ ╚████║██║██║  ██║",
    "╚═╝     ╚═╝ ╚════╝  ╚═════╝ ╚══════╝╚═╝  ╚═══╝╚═╝╚═╝  ╚═╝",
];

const MJOLNIR_ART: [&str; 21] = [
    "⠀⠀⠀⠀⠀⠀⠀⠀⢠⣤⠶⠶⠒⣛⣛⡛⠛⣛⠛⢛⣛⣓⠲⠶⢶⣤⡀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⢀⡏⡜⢠⣾⠟⠛⠛⠿⣿⣿⣿⠟⠛⠛⠻⣦⡀⢇⢻⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⢸⡇⡇⣾⡃⢰⢋⠙⢦⣈⠟⢁⡴⠋⢙⡆⢸⡇⢸⢸⡆⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠸⣇⡇⠘⣧⣈⠛⣁⡾⠋⣠⠻⣧⡈⠋⣁⡾⠃⢸⣸⠁⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠙⠳⢦⡀⠙⠛⠋⣠⣾⠿⣦⣈⠙⠛⠋⢁⡴⠟⠉⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢳⡀⠀⣀⠙⢿⣶⡿⠋⡀⠀⢠⡟⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⡇⠀⣿⣷⡴⠋⢠⣾⣿⠀⢸⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⡇⢀⣿⡏⠠⣷⠄⢹⣿⠀⢸⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⡇⢸⣿⣿⠞⢁⠴⣿⣿⡇⢸⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣼⠀⣈⣿⡁⠰⣿⠆⢸⣿⣀⠘⣇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢠⡏⢠⡏⢠⡿⠊⣁⠰⢿⡄⢹⡀⢻⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⡞⠀⠸⣧⣄⣤⣾⣿⣷⣤⣠⣾⠇⠈⢷⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⣠⢤⣤⣤⣤⣤⣤⠴⠾⠤⠶⠟⠛⠛⠛⢉⣉⣉⠙⠛⠛⠻⠶⠼⠷⠤⠤⢤⣤⡤⠤⠤⣄⠀",
    "⠀⡇⢠⣤⣤⣤⡤⠤⠀⠶⣶⣶⣶⡾⠟⠛⠉⠛⠻⢿⣿⣷⣶⠖⢀⣠⣤⣤⡤⣤⣤⣤⡄⢸⡆",
    "⢸⡇⢸⣿⣿⡿⠀⣴⡶⣦⠈⣿⠏⣠⡶⠛⠛⠛⢶⡄⠹⣿⡏⢠⣿⣿⡿⠿⠧⠼⣿⣿⡇⠸⡇",
    "⢸⠁⣼⣿⣿⣿⣄⣉⣀⡾⠀⡏⠀⣿⠁⣼⣿⣧⣨⡿⠀⣿⣇⠘⣟⣁⣤⣶⠶⢦⣤⡈⠻⠀⣷",
    "⣿⠀⠉⣠⣤⣤⣈⣉⣉⣤⣾⣷⠀⢿⣄⠙⠻⠿⠟⢁⣼⣿⠟⢦⣈⣉⣉⣁⣴⣿⣿⣿⣆⠀⣿",
    "⠙⢦⣤⡤⢤⣤⣈⣉⠙⠛⠿⣿⣷⣄⠙⠻⠶⣶⣾⠿⠛⢁⣴⣿⠿⠟⠛⢉⣉⣠⣤⣤⣤⡤⠟",
    "⠀⠀⠀⠀⠀⠀⠀⠉⠙⠓⠦⣄⣉⠙⠻⣶⣦⣤⣤⣴⠾⠛⢉⣠⠴⠖⠛⠉⠁⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠛⠦⣄⠙⠻⠋⣠⡴⠚⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠻⠶⠞⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
];

/// Per-row color for `MJOLNIR_ART`: bright electric cyan-white at the top
/// fading to a deep blue at the base, evoking current arcing down through
/// the hammer — per explicit developer request for something "fancier"
/// than a flat single color on the mascot art specifically. Interpolates
/// linearly in RGB space; not a general loosening of the one-accent-color
/// rule elsewhere (see the Palette Progress note in mjolnir-tui.md), just
/// a further scoped expansion of it for this one mark, same as ACCENT
/// itself already was.
fn mjolnir_row_color(row: usize, total: usize) -> Color {
    let t = row as f32 / (total.saturating_sub(1)).max(1) as f32;
    let lerp = |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * t).round() as u8 };
    Color::Rgb(lerp(210, 40), lerp(255, 90), lerp(255, 210))
}

/// Every `MJOLNIR_ART` row is exactly this many chars (not trimmed of
/// trailing blank Braille cells), so the info column in `intro_lines`
/// starts at the same screen column on every row regardless of how much
/// art content that particular row has. `WORDMARK_ART` doesn't need this
/// — nothing sits beside it — so its rows aren't held to a matching
/// invariant.
const MJOLNIR_ART_WIDTH: usize = 35;

fn intro_lines(model_name: &str, width: u16) -> Vec<Line<'static>> {
    debug_assert!(
        MJOLNIR_ART.iter().all(|row| row.chars().count() == MJOLNIR_ART_WIDTH),
        "MJOLNIR_ART rows must stay fixed-width or the info column drifts off-alignment — see every_mjolnir_art_row_is_exactly_mjolnir_art_width_chars"
    );
    let frame = Style::default().fg(ACCENT);
    let wordmark_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let tagline_style = Style::default().fg(BRIGHT).add_modifier(Modifier::ITALIC);
    let stat_label = Style::default().fg(DIM);
    let stat_value = Style::default().fg(BRIGHT);

    // Beside the art, not above or below it — per the standing developer
    // rule (art left-aligned, text alongside it on the right). The
    // wordmark block sits at the top of this column, tagline/stats below
    // it, the whole column vertically centered against the art's height.
    // Stats render as separate labeled lines (model/version/commit) —
    // per explicit developer request, not packed onto one line.
    let mut info: Vec<Vec<Span<'static>>> = WORDMARK_ART.iter().map(|row| vec![Span::styled(*row, wordmark_style)]).collect();
    info.push(vec![]);
    info.push(vec![Span::styled("every strike is yours to call. nothing moves without you.", tagline_style)]);
    info.push(vec![]);
    info.push(vec![Span::styled("model    ", stat_label), Span::styled(model_name.to_string(), stat_value)]);
    info.push(vec![Span::styled("version  ", stat_label), Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), stat_value)]);
    info.push(vec![Span::styled("commit   ", stat_label), Span::styled(env!("MJOLNIR_GIT_HASH"), stat_value)]);
    let info_offset = (MJOLNIR_ART.len().saturating_sub(info.len())) / 2;

    // A blank line above and below the art gives it breathing room inside
    // the border, on top of the existing left margin and the border's own
    // fill-to-width on the right — per explicit developer request for
    // padding "all the way around" the hammer, not just on one side.
    let mut content: Vec<Line<'static>> = Vec::with_capacity(MJOLNIR_ART.len() + 2);
    content.push(Line::default());
    content.extend(MJOLNIR_ART.iter().enumerate().map(|(i, art_row)| {
        let art_style = Style::default().fg(mjolnir_row_color(i, MJOLNIR_ART.len())).add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::styled(*art_row, art_style)];
        if let Some(row_i) = i.checked_sub(info_offset) {
            if let Some(line_spans) = info.get(row_i) {
                spans.push(Span::raw("   "));
                spans.extend(line_spans.iter().cloned());
            }
        }
        Line::from(spans)
    }));
    content.push(Line::default());
    bordered(width, content, frame)
}

/// Wraps `content` in a border that spans the full render width (`width`,
/// the log area's actual `Rect::width` — art alone can't know this, so it's
/// threaded in from `draw_log` at render time), left-aligning each line
/// with a small fixed margin rather than centering — per explicit
/// developer direction that the banner should read left-to-right (art,
/// then wordmark/info beside it), not sit centered in the middle of a wide
/// terminal. `content_width` sums `Span::content` char counts, which only
/// holds up for single-width glyphs — true of every char used here
/// (box-drawing and Braille dot patterns are Unicode East Asian Width
/// "Narrow"/"Neutral") but would need adjustment for wide (CJK/emoji) text.
fn bordered(width: u16, content: Vec<Line<'static>>, border_style: Style) -> Vec<Line<'static>> {
    const LEFT_MARGIN: usize = 3;
    let inner_width = (width as usize).saturating_sub(2);
    let mut out = Vec::with_capacity(content.len() + 2);
    out.push(Line::from(Span::styled(format!("┌{}┐", "─".repeat(inner_width)), border_style)));
    for line in content {
        let content_width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        let avail = inner_width.saturating_sub(LEFT_MARGIN + 1); // trailing space before the right border
        let right_pad = avail.saturating_sub(content_width);
        let mut spans = vec![Span::styled(format!("│{}", " ".repeat(LEFT_MARGIN)), border_style)];
        spans.extend(line.spans);
        spans.push(Span::styled(format!("{} │", " ".repeat(right_pad)), border_style));
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(format!("└{}┘", "─".repeat(inner_width)), border_style)));
    out
}

fn render_entry(entry: &LogEntry, width: u16) -> Vec<Line<'static>> {
    match entry {
        // Palette per mjolnir-tui.md: bright = assistant, muted gray +
        // subtle background = user — these must not share a style, or the
        // two speakers become indistinguishable in the log. A slash command
        // is user input that never reaches the model (see mjolnir-cli's
        // interceptor) — dim marks it as directed at the harness itself,
        // not conversation, the same way tool metadata and notices are dim
        // (and it skips the background tint, since it isn't a chat message).
        LogEntry::UserMessage { text } => {
            let style = if is_command(text) { Style::default().fg(DIM) } else { Style::default().fg(USER_FG).bg(USER_BG) };
            // Padded to the full render width so the background tint reads
            // as a chat bubble even for a short message, not just a tinted
            // "> " prefix — per explicit developer feedback. Only exact for
            // a source line that fits on one screen row: a line long enough
            // to wrap under Paragraph's own Wrap{trim:false} gets this
            // padding appended past the wrap point, not per wrapped row.
            // Padding is sized in display columns (`UnicodeWidthStr::width`),
            // not `chars().count()` — a chat message can contain CJK/emoji
            // double-width glyphs, and undercounting those overshoots the
            // real render width, pushing the "single-row bubble" onto an
            // extra wrapped row (see mjolnir-tui.md's wide-char Progress
            // note; `bordered`'s own char-count shortcut is fine since it
            // only ever renders the narrow-glyph banner art).
            text.lines()
                .map(|l| {
                    let content = format!("> {l}");
                    let pad = (width as usize).saturating_sub(content.width());
                    Line::from(Span::styled(format!("{content}{}", " ".repeat(pad)), style))
                })
                .collect()
        }
        LogEntry::AssistantText { text } => render_assistant_text(text),
        LogEntry::ToolActivity { calls, .. } => calls
            .iter()
            .map(|c| {
                let label = if c.name.is_empty() { c.call_id.clone() } else { format!("{} ({})", c.name, c.call_id) };
                let text = match &c.status {
                    ToolActivityStatus::Running => format!("  [running] {label}"),
                    ToolActivityStatus::Completed { is_error, summary } => {
                        let tag = if *is_error { "error" } else { "done" };
                        format!("  [{tag}] {label}: {summary}")
                    }
                };
                Line::from(Span::styled(text, Style::default().fg(DIM)))
            })
            .collect(),
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            vec![Line::from(Span::styled(
                format!("  [retry {}] {} {status}: {}", info.attempt, info.provider, info.message),
                Style::default().fg(DIM),
            ))]
        }
        LogEntry::ApprovalCard { diff, resolution, .. } => render_approval_card(diff, *resolution, width),
        LogEntry::PermissionPrompt { payload, resolution, .. } => render_prompt_card(payload, resolution.as_deref()),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            // "— turn ended —" read as flat/mechanical for the ordinary
            // case — per explicit developer feedback — so it's replaced
            // with "answered", which names what actually happened instead
            // of describing internal turn-lifecycle plumbing. Cancelled/
            // error keep their own distinct wording since those already
            // read as intended and name a different outcome.
            let text = match reason {
                TurnEndReasonKind::EndTurn => "— answered —".to_string(),
                TurnEndReasonKind::Cancelled => "— turn cancelled —".to_string(),
                TurnEndReasonKind::Error(message) => format!("— turn ended in error: {message} —"),
            };
            vec![Line::from(Span::styled(text, Style::default().fg(DIM)))]
        }
        LogEntry::Error { message } => vec![Line::from(Span::styled(format!("error: {message}"), Style::default().fg(Color::Red)))],
        LogEntry::Notice { message } => vec![Line::from(Span::styled(format!("— {message} —"), Style::default().fg(DIM)))],
    }
}

/// One piece of assistant text — either prose or a fenced code block.
enum Segment {
    Prose(String),
    Code { lang: String, body: String },
}

/// Splits on ` ``` ` fences (optionally followed by a language tag on the
/// opening fence). An unterminated fence — the closing ` ``` ` hasn't
/// streamed in yet — still renders as code up to the end of the buffer
/// rather than falling back to prose, since re-rendering happens on every
/// delta and the fence will close on a later redraw.
fn split_code_fences(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut prose = String::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        match line.trim_start().strip_prefix("```") {
            Some(lang) => {
                if !prose.is_empty() {
                    segments.push(Segment::Prose(std::mem::take(&mut prose)));
                }
                let mut body = String::new();
                for line in lines.by_ref() {
                    if line.trim() == "```" {
                        break;
                    }
                    body.push_str(line);
                    body.push('\n');
                }
                segments.push(Segment::Code { lang: lang.trim().to_string(), body });
            }
            None => {
                prose.push_str(line);
                prose.push('\n');
            }
        }
    }
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
    segments
}

fn render_assistant_text(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in split_code_fences(text) {
        match segment {
            Segment::Prose(s) => {
                lines.extend(s.lines().map(render_markdown_line));
            }
            Segment::Code { lang, body } => {
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.push(Line::from(Span::styled(format!("┌─ {label}"), Style::default().fg(DIM))));
                for code_line in highlight::highlight_lines(&lang, &body) {
                    let mut spans = vec![Span::styled("│ ", Style::default().fg(DIM))];
                    spans.extend(code_line);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(Span::styled("└─", Style::default().fg(DIM))));
            }
        }
    }
    // A single marker on the very first rendered line — prose or a code
    // fence's header, whichever comes first — scans as "here's where the
    // assistant's turn starts" without repeating on every line.
    if let Some(first) = lines.first_mut() {
        first.spans.insert(0, Span::styled("● ", Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)));
    }
    lines
}

/// Renders one prose line (never a fenced-code line — those are already
/// pulled out by `split_code_fences`) of LLM-authored markdown. Hand-rolled
/// rather than pulling in a CommonMark crate — a real block-level parser
/// normalizes blank lines and reflows paragraphs, which would fight the
/// line-for-line streaming render this does on every delta. Per-line
/// block-prefix detection (heading, list, blockquote, rule) plus a
/// recursive-descent inline pass covers what LLMs actually emit. Styling is
/// modifiers only
/// (bold/italic/underline/reversed/crossed-out) — mjolnir-tui.md reserves
/// the one accent color for the approval card and focused input.
fn render_markdown_line(line: &str) -> Line<'static> {
    let base = Style::default().fg(BRIGHT);
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];

    if is_hr(trimmed_start) {
        return Line::from(Span::styled("─".repeat(20), Style::default().fg(DIM)));
    }
    if let Some((level, rest)) = parse_heading(trimmed_start) {
        let style = if level <= 2 { base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { base.add_modifier(Modifier::BOLD) };
        return Line::from(parse_inline(rest, style));
    }
    if let Some(rest) = trimmed_start.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let mut spans = vec![Span::styled(format!("{indent}▎ "), Style::default().fg(DIM))];
        spans.extend(parse_inline(rest, base.add_modifier(Modifier::ITALIC)));
        return Line::from(spans);
    }
    if let Some(rest) = parse_bullet(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}• "), base)];
        spans.extend(parse_inline(rest, base));
        return Line::from(spans);
    }
    if let Some((marker, rest)) = parse_ordered(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}{marker} "), base)];
        spans.extend(parse_inline(rest, base));
        return Line::from(spans);
    }
    Line::from(parse_inline(line, base))
}

/// Recursive-descent inline pass: `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, `~~strike~~`, `[text](url)`. Delimiters nest via recursion
/// (e.g. `**bold *and italic***`) rather than a flat token stream, which
/// keeps this a single small function instead of a tokenizer + AST.
fn parse_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    fn flush(buf: &mut String, style: Style, spans: &mut Vec<Span<'static>>) {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    }

    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(stripped[..end].to_string(), Style::default().fg(CODE_FG)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::BOLD)));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("~~") {
            if let Some(end) = stripped.find("~~") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::CROSSED_OUT)));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if rest.starts_with('*') || rest.starts_with('_') {
            let delim = &rest[..1];
            let stripped = &rest[1..];
            if let Some(end) = stripped.find(delim) {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::ITALIC)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((label, url, remainder)) = parse_link(rest) {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(label.to_string(), base.add_modifier(Modifier::UNDERLINED)));
                if !url.is_empty() && url != label {
                    spans.push(Span::styled(format!(" ({url})"), Style::default().fg(DIM)));
                }
                rest = remainder;
                continue;
            }
        }

        let ch_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        buf.push_str(&rest[..ch_len]);
        rest = &rest[ch_len..];
    }
    flush(&mut buf, base, &mut spans);
    spans
}

fn parse_link(text: &str) -> Option<(&str, &str, &str)> {
    let after_bracket = &text[1..];
    let close = after_bracket.find(']')?;
    let label = &after_bracket[..close];
    let after_label = &after_bracket[close + 1..];
    let after_paren = after_label.strip_prefix('(')?;
    let close_paren = after_paren.find(')')?;
    let url = &after_paren[..close_paren];
    let remainder = &after_paren[close_paren + 1..];
    Some((label, url, remainder))
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(|rest| (hashes as u8, rest))
}

fn parse_bullet(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    line[marker.len_utf8()..].strip_prefix(' ')
}

fn parse_ordered(line: &str) -> Option<(String, &str)> {
    let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
    if digits_end == 0 {
        return None;
    }
    let (digits, rest) = line.split_at(digits_end);
    let sep = rest.chars().next()?;
    if sep != '.' && sep != ')' {
        return None;
    }
    let rest = rest[sep.len_utf8()..].strip_prefix(' ')?;
    Some((format!("{digits}{sep}"), rest))
}

/// A line of 3+ `-`, `*`, or `_` (ignoring interior spaces, so `- - -`
/// counts) and nothing else — CommonMark's thematic break.
fn is_hr(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3 && (stripped.chars().all(|c| c == '-') || stripped.chars().all(|c| c == '*') || stripped.chars().all(|c| c == '_'))
}

/// Mirrors mjolnir-cli's own `/`-prefix check (`text.trim_start().strip_prefix('/')`
/// in `slash.rs`) — this crate can't depend on that one to reuse it
/// directly (cli depends on tui, not the other way around), so the rule is
/// duplicated; keep the two in sync if it ever changes.
fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

/// Every word `cli::slash::intercept` actually dispatches on (see its
/// `match rest.trim()` arms in `slash.rs`, `/`-prefixed here to match
/// whole-word input tokens directly) — duplicated for the same reason as
/// `is_command` above: tui can't depend on cli. Purely a hint for
/// `highlight_command_tokens` below; keep in sync by hand if slash.rs's
/// arms change.
const KNOWN_COMMAND_WORDS: [&str; 4] = ["/help", "/clear", "/exit", "/reload-config"];

/// Dims every word in `line` that exactly matches a known command, no
/// matter where it falls — per explicit developer direction, this is a
/// cosmetic hint only and deliberately does *not* mirror `is_command`'s
/// "whole message must start with `/`" rule: a real slash command only
/// fires when it's the very first thing in the message (`is_command`,
/// enforced for real in `cli::slash::intercept`), so `/exit` typed
/// mid-sentence never actually gets intercepted — it's still worth
/// flagging live so the developer notices they typed a recognized command
/// word, wherever it landed.
fn highlight_command_tokens(line: &str) -> Line<'static> {
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
        let style = if KNOWN_COMMAND_WORDS.contains(&word) { Style::default().fg(DIM) } else { Style::default() };
        spans.push(Span::styled(word.to_string(), style));
        rest = tail;
    }
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    Line::from(spans)
}

/// One line of a unified diff (`mjolnir_tools::diff::unified`'s output),
/// tagged by its leading marker (` `/`+`/`-`). The `--- path`/`+++ path`
/// header pair is pulled out separately by `parse_diff_body` since it's
/// shown once as a label, not per line.
enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// How many lines of unmodified context to keep immediately before/after a
/// change — per explicit developer feedback that in an approval card,
/// unchanged lines are only relevant this close to what actually changed; a
/// longer run of context collapses to a single elision marker instead of
/// listing every line, and pure context isn't colored at all (see
/// `render_diff_line`) — only the changed lines are, so they're the only
/// thing competing for attention.
const DIFF_CONTEXT_RADIUS: usize = 2;

/// Splits a unified diff into its path (from the `--- path` header line;
/// `+++ path` names the same path, so it's dropped) and its body lines,
/// each tagged with the kind its leading marker encodes.
fn parse_diff_body(diff: &str) -> (Option<String>, Vec<(DiffLineKind, String)>) {
    let mut path = None;
    let mut body = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            path.get_or_insert_with(|| rest.to_string());
        } else if line.starts_with("+++ ") {
            // Same path as the "---" line — nothing new to show.
        } else if let Some(rest) = line.strip_prefix('+') {
            body.push((DiffLineKind::Added, rest.to_string()));
        } else if let Some(rest) = line.strip_prefix('-') {
            body.push((DiffLineKind::Removed, rest.to_string()));
        } else {
            body.push((DiffLineKind::Context, line.strip_prefix(' ').unwrap_or(line).to_string()));
        }
    }
    (path, body)
}

/// The Edit approval card: bordered title/keys (same shape as
/// `render_card`) around a diff-aware body — added/removed lines get a
/// full-width background tint (see `DIFF_ADD_BG`/`DIFF_DEL_BG`), and
/// unchanged context beyond `DIFF_CONTEXT_RADIUS` lines from the nearest
/// change collapses to a single "N unchanged lines" marker — per explicit
/// developer feedback that the card previously rendered every diff line in
/// the same plain style, which made it hard to tell what actually changed
/// at a glance.
fn render_approval_card(diff: &str, resolution: Option<bool>, width: u16) -> Vec<Line<'static>> {
    let (path, body) = parse_diff_body(diff);
    let mut lines = vec![Line::from(Span::styled("┌─ Approve this edit?", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))];
    if let Some(path) = path {
        lines.push(Line::from(Span::styled(format!("│ {path}"), Style::default().fg(DIM))));
    }

    let n = body.len();
    let mut keep = vec![false; n];
    for (i, (kind, _)) in body.iter().enumerate() {
        if !matches!(kind, DiffLineKind::Context) {
            let start = i.saturating_sub(DIFF_CONTEXT_RADIUS);
            let end = (i + DIFF_CONTEXT_RADIUS).min(n.saturating_sub(1));
            for k in &mut keep[start..=end] {
                *k = true;
            }
        }
    }

    let mut i = 0;
    while i < n {
        if keep[i] {
            let (kind, text) = &body[i];
            lines.push(render_diff_line(kind, text, width));
            i += 1;
        } else {
            let elided_start = i;
            while i < n && !keep[i] {
                i += 1;
            }
            let count = i - elided_start;
            lines.push(Line::from(Span::styled(
                format!("│ ⋯ {count} unchanged line{} ⋯", if count == 1 { "" } else { "s" }),
                Style::default().fg(DIM),
            )));
        }
    }

    match resolution {
        Some(approved) => lines.push(Line::from(Span::styled(
            format!("└─ resolved: {}", if approved { "approved" } else { "denied" }),
            Style::default().fg(ACCENT),
        ))),
        None => lines.push(Line::from(Span::styled("└─ [y] approve   [n] deny   [Ctrl+C] deny", Style::default().fg(ACCENT)))),
    }
    lines
}

/// Renders one kept diff line. Added/removed lines get a full-width
/// background tint — same "pad to render width" technique `LogEntry::
/// UserMessage` uses for its chat-bubble background — so a change reads as
/// a colored row at a glance, not just a leading +/- character in an
/// otherwise uniformly-styled card; context lines stay plain (no
/// background at all), since only the changed lines should compete for
/// attention.
fn render_diff_line(kind: &DiffLineKind, text: &str, width: u16) -> Line<'static> {
    let (marker, style) = match kind {
        DiffLineKind::Added => ("+", Style::default().fg(DIFF_ADD_FG).bg(DIFF_ADD_BG)),
        DiffLineKind::Removed => ("-", Style::default().fg(DIFF_DEL_FG).bg(DIFF_DEL_BG)),
        DiffLineKind::Context => (" ", Style::default().fg(BRIGHT)),
    };
    let content = format!("│{marker}{text}");
    if matches!(kind, DiffLineKind::Context) {
        return Line::from(Span::styled(content, style));
    }
    // Display-column width, not char count — see the matching note on
    // `LogEntry::UserMessage`'s padding above; diff bodies can equally
    // contain double-width glyphs.
    let pad = (width as usize).saturating_sub(content.width());
    Line::from(Span::styled(format!("{content}{}", " ".repeat(pad)), style))
}

fn render_prompt_card(payload: &PromptPayload, resolution: Option<&str>) -> Vec<Line<'static>> {
    let (title, keys) = match payload {
        PromptPayload::Tool { kind, target } => {
            (format!("Allow {kind}: {target}?"), "[o]nce [s]ession [p]roject [a]lways   Shift = deny at the same tier   Ctrl+C = deny once".to_string())
        }
        PromptPayload::ContextFile { path } => (format!("Inject context file {}?", path.display()), "[s]ession [p]roject [n]o   Ctrl+C = no".to_string()),
        PromptPayload::Edit { kind } => (format!("Edit approval for {kind}"), String::new()),
    };
    render_card(&title, "", &keys, resolution.map(str::to_string))
}

fn render_card(title: &str, body: &str, keys: &str, resolution: Option<String>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(format!("┌─ {title}"), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))];
    for l in body.lines() {
        lines.push(Line::from(Span::styled(format!("│ {l}"), Style::default().fg(BRIGHT))));
    }
    match resolution {
        Some(r) => lines.push(Line::from(Span::styled(format!("└─ resolved: {r}"), Style::default().fg(ACCENT)))),
        None => lines.push(Line::from(Span::styled(format!("└─ {keys}"), Style::default().fg(ACCENT)))),
    }
    lines
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let s = &app.status;
    let turn_step = match (s.turn, s.step) {
        (Some(t), Some(st)) => format!("T{t} S{st}"),
        (Some(t), None) => format!("T{t}"),
        _ => "-".to_string(),
    };
    let perm = |label: &str, state: PermState| format!("{label}:{}", if state == PermState::Allowed { "allow" } else { "deny" });
    let tools = if s.running_tools.is_empty() { String::new() } else { format!(" | tools: {}", s.running_tools.join(" ")) };

    let text = format!(
        "{}  {turn_step}  {} {} {}{tools}  |  Ctrl+C: cancel/quit",
        s.model_name,
        perm("read", s.read),
        perm("shell", s.shell),
        perm("edit", s.edit),
    );
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, Style::default().fg(DIM)))), area);
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).border_style(if app.pending_approval.is_some() || app.pending_prompt.is_some() {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(ACCENT)
    });
    // Live counterpart to `is_command`'s dim styling of an already-submitted
    // slash command in the log (see `render_entry`) — without this, a
    // command only reads as "directed at the harness, not the model" after
    // Enter, not while the developer is still typing it. Unlike
    // `is_command`, this checks every word on every line — see
    // `highlight_command_tokens`'s doc comment for why a mid-message
    // `/exit` still gets flagged even though it would never actually be
    // intercepted as a command.
    let lines: Vec<Line> = app.input.split('\n').map(highlight_command_tokens).collect();
    let paragraph = Paragraph::new(Text::from(lines)).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);

    // No visible cursor at all was a standing complaint — per explicit
    // developer feedback that it was hard to tell where the cursor sat in
    // the input box. Ratatui doesn't draw one on its own; `set_cursor_position`
    // asks the real terminal cursor to sit there instead. Skipped while a
    // card is pending (input is blocked then, and the border already dims
    // to say so — see the `border_style` above). `cursor_line_col` counts
    // by source line, not wrapped screen row (see its doc comment), so a
    // single logical line long enough to wrap past the box's width places
    // the terminal cursor past the visible text — clamped to the inner
    // area's last column/row below so it never lands outside the box
    // rather than fixing the underlying wrap mismatch.
    if app.pending_approval.is_none() && app.pending_prompt.is_none() {
        let (line, col) = cursor_line_col(&app.input, app.cursor);
        let inner_right = area.x + area.width.saturating_sub(2);
        let inner_bottom = area.y + area.height.saturating_sub(2);
        let x = (area.x + 1 + col as u16).min(inner_right);
        let y = (area.y + 1 + line as u16).min(inner_bottom);
        frame.set_cursor_position((x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use mjolnir_config::Config;
    use mjolnir_permissions::Engine;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        App::new("claude-sonnet-5".into(), Arc::new(Engine::new(config)))
    }

    fn rendered(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("")
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

        // Log area gets roughly height-2 rows (status bar + input box eat
        // the rest) — nowhere near the ~59 rows ten 5-line entries plus
        // nine separators need.
        let out = rendered(&mut app, 100, 12);

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

    #[test]
    fn status_bar_shows_model_name_and_permission_summary() {
        let mut app = app();
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("claude-sonnet-5"));
        assert!(out.contains("read:deny"));
        assert!(out.contains("shell:deny"));
        assert!(out.contains("edit:deny"));
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
        app.thinking = true;
        assert!(rendered(&mut app, 100, 20).contains("thinking…"));
        app.thinking = false;
        assert!(!rendered(&mut app, 100, 20).contains("thinking…"));
    }

    #[test]
    fn working_spinner_shows_during_an_active_turn_with_no_thinking_block() {
        let mut app = app();
        app.turn_active = true;
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("working…"), "an active turn with no other feedback should still show loading progress: {out:?}");

        app.turn_active = false;
        assert!(!rendered(&mut app, 100, 20).contains("working…"), "no active turn means no spinner");
    }

    #[test]
    fn thinking_takes_priority_over_the_working_spinner() {
        let mut app = app();
        app.turn_active = true;
        app.thinking = true;
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("thinking…"));
        assert!(!out.contains("working…"), "only one spinner label should show at a time");
    }

    #[test]
    fn the_spinner_animates_across_ticks() {
        assert_ne!(spinner_line(0, "working").spans[0].content, spinner_line(1, "working").spans[0].content, "advancing the tick should change the spinner glyph");
    }

    #[test]
    fn approval_card_shows_labeled_keys_and_the_diff() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("approve"));
        assert!(out.contains("deny"));
        assert!(out.contains("old"));
        assert!(out.contains("new"));
    }

    #[test]
    fn approval_card_colors_added_and_removed_lines_distinctly() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let find_row = |needle: &str| -> u16 {
            for y in 0..buffer.area.height {
                let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
                if row.contains(needle) {
                    return y;
                }
            }
            panic!("row containing {needle:?} not found");
        };
        let removed_row = find_row("old");
        let added_row = find_row("new");
        assert_eq!(buffer[(0, removed_row)].bg, DIFF_DEL_BG, "a removed line should carry the removed-line background across the row");
        assert_eq!(buffer[(0, added_row)].bg, DIFF_ADD_BG, "an added line should carry the added-line background across the row");
        assert_ne!(buffer[(0, removed_row)].bg, buffer[(0, added_row)].bg, "added and removed lines must be visually distinct");
    }

    #[test]
    fn approval_card_collapses_unchanged_context_beyond_the_radius() {
        let diff = "--- f.rs\n+++ f.rs\n far\n context\n a\n b\n-old\n+new\n c\n d\n near\n";
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: diff.into(), resolution: None });
        let out = rendered(&mut app, 100, 30);
        assert!(out.contains("unchanged line"), "a long run of unmodified context should collapse to an elision marker: {out:?}");
        assert!(!out.contains("far"), "context far from any change should be elided");
        assert!(out.contains("old") && out.contains("new"), "the change itself must still be shown");
        assert!(out.contains("a") && out.contains("b"), "the 2 lines of context immediately before a change must be kept");
        assert!(out.contains("c") && out.contains("d"), "the 2 lines of context immediately after a change must be kept");
    }

    #[test]
    fn user_and_assistant_messages_are_visually_distinct() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.log.push(LogEntry::AssistantText { text: "hi".into() });

        // Tall enough that the banner plus both entries fit without
        // triggering auto-follow scroll — otherwise the offset below
        // (which assumes the viewport shows everything from row 0) would
        // be reading the wrong rows entirely.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // The welcome banner (log::INTRO_LINE_COUNT rows) plus its own
        // separator come first, then the same "row 1 is the blank
        // separator between entries, row 2 is the second entry" shape as
        // before, just offset past the banner.
        let base = intro_offset();
        let user_cell = &buffer[(2, base)]; // "> hi"
        let assistant_cell = &buffer[(2, base + 2)]; // "● hi"
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

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let base = intro_offset();
        let plain_cell = &buffer[(2, base)]; // "> hi"
        let command_cell = &buffer[(2, base + 2)]; // "> /exit" — the next row is the blank separator line
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
        let line = highlight_command_tokens("/clear now");
        let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
        assert_eq!(styled, vec![("/clear", Some(DIM)), (" ", None), ("now", None)]);
    }

    /// The bug report this responds to: dimming only checked the input's
    /// very first character, so a recognized command word typed anywhere
    /// past position 0 never got flagged even though it's the same word.
    #[test]
    fn highlight_command_tokens_dims_a_command_word_mid_message() {
        let line = highlight_command_tokens("please run /exit for me");
        let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
        assert_eq!(
            styled,
            vec![("please", None), (" ", None), ("run", None), (" ", None), ("/exit", Some(DIM)), (" ", None), ("for", None), (" ", None), ("me", None)]
        );
    }

    #[test]
    fn highlight_command_tokens_leaves_plain_text_unstyled() {
        let line = highlight_command_tokens("hello world");
        assert!(line.spans.iter().all(|s| s.style.fg.is_none()));
    }

    #[test]
    fn highlight_command_tokens_requires_an_exact_word_match() {
        // "/exiting" isn't the recognized "/exit" word, and "cleared" isn't
        // "/clear" — a substring match would false-positive on either.
        let line = highlight_command_tokens("/exiting cleared");
        assert!(line.spans.iter().all(|s| s.style.fg.is_none()));
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

        // Single-line draft -> 3-row input box (see `input_area_height`) at
        // the very bottom of a 20-row frame; content sits on the middle row
        // (y=18), one cell in from the left border (x=1).
        let slash_cell = &buffer[(1, 18)]; // '/'
        let arg_cell = &buffer[(8, 18)]; // 'n' of "now"
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

        let leading_cell = &buffer[(1, 18)]; // 'h' of "hi"
        let slash_cell = &buffer[(4, 18)]; // '/' of "/exit"
        let trailing_cell = &buffer[(10, 18)]; // 't' of "there"
        assert_eq!(leading_cell.symbol(), "h");
        assert_eq!(slash_cell.symbol(), "/");
        assert_eq!(trailing_cell.symbol(), "t");
        assert_ne!((leading_cell.fg, leading_cell.modifier), (slash_cell.fg, slash_cell.modifier), "a mid-message command word must still be dimmed");
        assert_ne!((trailing_cell.fg, trailing_cell.modifier), (slash_cell.fg, slash_cell.modifier), "text after a mid-message command word must not also be dimmed");
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
        // Input box is the last Length(3) row of the layout: border at
        // height-3, content row at height-2.
        assert_eq!(pos.y, 20 - 2, "cursor should sit on the input box's one content row");
        assert_eq!(pos.x, 1 + 2, "cursor should sit right after \"hi\" (1 for the left border, 2 for the two typed chars)");
    }

    #[test]
    fn the_terminal_cursor_is_hidden_while_an_approval_card_is_pending() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "diff".into(), resolution: None });
        app.pending_approval = Some(crate::app::PendingApproval { call_id: "c1".into() });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert!(!terminal.backend().cursor_visible(), "input is blocked while a card is pending — no cursor should show");
    }

    /// Rows the welcome banner always occupies before the first real log
    /// entry: `log::INTRO_LINE_COUNT` art/text rows plus the one separator
    /// `draw_log` inserts between the banner and the log (present here
    /// since every caller pushes at least one entry before measuring).
    fn intro_offset() -> u16 {
        (crate::log::INTRO_LINE_COUNT + 1) as u16
    }

    /// The row-index assumptions the two style-comparison tests above make
    /// (the row right after the banner is blank, the second entry lands two
    /// rows after that) only hold because `draw_log` inserts exactly one
    /// blank line between entries — pin that down directly so a change to
    /// the spacing logic fails loudly here instead of silently making those
    /// tests compare the wrong cells.
    #[test]
    fn a_blank_line_separates_consecutive_log_entries() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "first".into() });
        app.log.push(LogEntry::UserMessage { text: "second".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = intro_offset() + 1;
        let row_text: String = (0..110).map(|x| buffer[(x, row)].symbol().to_string()).collect();
        assert_eq!(row_text.trim(), "", "the row after the first entry must be the blank separator between the two entries");
    }

    #[test]
    fn assistant_text_gets_a_marker_that_user_text_does_not() {
        let mut assistant_app = app();
        assistant_app.log.push(LogEntry::AssistantText { text: "hi".into() });
        assert!(rendered(&mut assistant_app, 100, 20).contains('●'), "assistant text should start with a marker");

        let mut user_app = app();
        user_app.log.push(LogEntry::UserMessage { text: "hi".into() });
        assert!(!rendered(&mut user_app, 100, 20).contains('●'), "user text should not get the assistant marker");
    }

    #[test]
    fn fenced_code_block_is_stripped_of_its_fences_and_syntax_highlighted() {
        let mut app = app();
        app.log.push(LogEntry::AssistantText { text: "here:\n```rust\nfn main() {}\n```\ndone".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

        assert!(!out.contains("```"), "the literal fence markers must not reach the screen");
        assert!(out.contains("rust"), "the language tag should appear in the block's header");
        assert!(out.contains("fn main"), "the code itself must still be shown");

        // At least two distinct foreground colors within the code line —
        // proof it went through the highlighter, not just plain dim text.
        // No blank-line separator here beyond the banner's own: draw_log
        // only inserts one between entries, and this is all one
        // AssistantText entry. Restricted to the line's own width so
        // unstyled padding cells past the printed text can't manufacture a
        // spurious second color.
        let code_row = intro_offset() + 2; // "● here:" / "┌─ rust" / "│ fn main() {}"
        let colors: std::collections::HashSet<Color> = (0..20).map(|x| buffer[(x, code_row)].fg).collect();
        assert!(colors.len() > 1, "expected the highlighted code line to use more than one color, got {colors:?}");
    }

    #[test]
    fn every_mjolnir_art_row_is_exactly_mjolnir_art_width_chars() {
        for (i, row) in MJOLNIR_ART.iter().enumerate() {
            assert_eq!(row.chars().count(), MJOLNIR_ART_WIDTH, "row {i} isn't fixed-width — the info column beside the art would drift off-alignment");
        }
    }

    #[test]
    fn mjolnir_row_color_sweeps_from_light_at_the_top_to_dark_at_the_base() {
        let top = mjolnir_row_color(0, MJOLNIR_ART.len());
        let bottom = mjolnir_row_color(MJOLNIR_ART.len() - 1, MJOLNIR_ART.len());
        assert_ne!(top, bottom, "the hammer should read as a gradient, not a flat single color");
        let Color::Rgb(tr, tg, tb) = top else { panic!("expected an Rgb color") };
        let Color::Rgb(br, bg, bb) = bottom else { panic!("expected an Rgb color") };
        let brightness = |r: u8, g: u8, b: u8| r as u32 + g as u32 + b as u32;
        assert!(brightness(tr, tg, tb) > brightness(br, bg, bb), "the top of the hammer should be brighter than the base");
    }

    #[test]
    fn the_wordmark_and_tagline_render_beside_the_hammer_art() {
        let mut app = app();
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // The right-hand column (WORDMARK_ART, then blank/tagline/blank/
        // model/version/commit) is vertically centered against
        // MJOLNIR_ART's 16 rows: offset = (16 - (6 wordmark + 6 info)) / 2
        // = 2. Screen row 0 is the top border, row 1 is the padding blank
        // line above the art, so the wordmark's first row is 2 + 2 = 4.
        let info_len = WORDMARK_ART.len() + 6;
        let offset = (MJOLNIR_ART.len() - info_len) / 2;
        let wordmark_row = (2 + offset) as u16;
        let wordmark_row_text: String = (0..110).map(|x| buffer[(x, wordmark_row)].symbol().to_string()).collect();
        assert!(wordmark_row_text.contains(WORDMARK_ART[0].trim()), "expected the wordmark's first row beside the hammer art, got: {wordmark_row_text:?}");
        assert!(wordmark_row_text.contains('⣿') || wordmark_row_text.contains('⠀'), "that same row should still carry hammer art content to its left, not just the wordmark alone");

        // Tagline is right after the wordmark block plus one blank line.
        let tagline_row = (2 + offset + WORDMARK_ART.len() + 1) as u16;
        let tagline_row_text: String = (0..110).map(|x| buffer[(x, tagline_row)].symbol().to_string()).collect();
        assert!(tagline_row_text.contains("every strike is yours to call."), "expected the tagline beside the hammer art, got: {tagline_row_text:?}");
    }

    #[test]
    fn intro_banner_shows_the_active_model_and_is_exactly_intro_line_count_rows() {
        assert_eq!(intro_lines("claude-sonnet-5", 80).len(), crate::log::INTRO_LINE_COUNT, "ui::intro_lines must stay in sync with log::INTRO_LINE_COUNT");
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view — see the sizing comment on
        // user_and_assistant_messages_are_visually_distinct.
        let out = rendered(&mut app(), 110, 40);
        assert!(out.contains("claude-sonnet-5"), "the active model should appear in the welcome banner");
        assert!(out.contains(WORDMARK_ART[3].trim()), "the wordmark should appear in the welcome banner");
        assert!(out.contains(env!("MJOLNIR_GIT_HASH")), "the build's git commit should appear in the welcome banner, distinct from the static crate version");
        assert!(out.contains(MJOLNIR_ART[0]), "the traced Mjolnir art should appear in the welcome banner");
    }

    #[test]
    fn a_fresh_session_shows_the_banner_before_any_log_entries() {
        let mut app = app();
        assert!(app.log.is_empty());
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing the wordmark (near the top) out of view.
        let out = rendered(&mut app, 110, 40);
        assert!(out.contains(WORDMARK_ART[3].trim()));
    }

    #[test]
    fn plain_user_messages_get_a_muted_background_but_slash_commands_do_not() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.log.push(LogEntry::UserMessage { text: "/exit".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let base = intro_offset();
        let plain_cell = &buffer[(2, base)]; // "> hi"
        let command_cell = &buffer[(2, base + 2)]; // "> /exit"
        assert_eq!(plain_cell.bg, USER_BG, "a plain user message should carry the subtle background tint");
        assert_ne!(command_cell.bg, USER_BG, "a slash command must not carry the chat-message background tint");
    }

    #[test]
    fn a_short_user_message_gets_the_background_tint_all_the_way_to_the_right_edge() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = intro_offset();
        let far_right_cell = &buffer[(99, row)]; // well past "> hi"
        assert_eq!(far_right_cell.bg, USER_BG, "the background tint should fill the full row width, not just trail the text");
    }

    #[test]
    fn the_welcome_banner_is_framed_by_a_border_spanning_the_full_render_width() {
        let mut app = app();
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view — see the sizing comment on
        // user_and_assistant_messages_are_visually_distinct.
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        assert_eq!(buffer[(0, 0)].symbol(), "┌", "top-left corner of the banner's border");
        assert_eq!(buffer[(109, 0)].symbol(), "┐", "top-right corner should reach the full render width");
        let bottom = crate::log::INTRO_LINE_COUNT as u16 - 1;
        assert_eq!(buffer[(0, bottom)].symbol(), "└", "bottom-left corner of the banner's border");
        assert_eq!(buffer[(109, bottom)].symbol(), "┘", "bottom-right corner should reach the full render width");
    }

    #[test]
    fn bold_markdown_strips_asterisks_and_sets_the_bold_modifier() {
        let spans = parse_inline("say **hello** now", Style::default().fg(BRIGHT));
        let bold = spans.iter().find(|s| s.content.as_ref() == "hello").expect("bold span present");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        assert!(spans.iter().all(|s| !s.content.contains('*')), "literal asterisks must not reach the screen");
    }

    #[test]
    fn italic_markdown_sets_the_italic_modifier() {
        let spans = parse_inline("that is *neat* stuff", Style::default().fg(BRIGHT));
        let italic = spans.iter().find(|s| s.content.as_ref() == "neat").expect("italic span present");
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code_strips_backticks_and_uses_a_distinct_color() {
        let spans = parse_inline("run `cargo test` first", Style::default().fg(BRIGHT));
        let code = spans.iter().find(|s| s.content.as_ref() == "cargo test").expect("code span present");
        assert_eq!(code.style.fg, Some(CODE_FG), "inline code should read as a distinct color, not a reversed-video block");
        assert!(!code.style.add_modifier.contains(Modifier::REVERSED), "inline code must not use reversed video");
        assert!(spans.iter().all(|s| !s.content.contains('`')), "literal backticks must not reach the screen");
    }

    #[test]
    fn a_heading_line_drops_the_hashes_and_renders_bold() {
        let line = render_markdown_line("## Section Title");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Section Title");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_bullet_line_replaces_the_dash_with_a_bullet_marker() {
        let line = render_markdown_line("- first item");
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
}
