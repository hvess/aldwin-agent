use mjolnir_permissions::PromptPayload;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{cursor_line_col, App, PermState, StatusInfo};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};
use crate::palette::{ACCENT, BG_BASE, BG_ELEMENT, BG_INPUT, BRIGHT, CODE_BG, CODE_FG, DIFF_ADD_BG, DIFF_ADD_FG, DIFF_DEL_BG, DIFF_DEL_FG, DIM, PANEL_BORDER, TOOL_PALETTE, USER_FG, WARNING_FG};

/// Braille-dot spinner frames — the same glyph family `MJOLNIR_ART` traces
/// the hammer in, so the "ascii trick" loading indicator reads as part of
/// the same visual language rather than a mismatched borrowed spinner.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // Opaque canvas, drawn first and under everything else — without this,
    // every gap between panels (margins, the status-line row) renders as
    // the terminal's own background, which is exactly the "transparent app"
    // look the redesign is replacing. See `palette::BG_BASE`'s doc comment
    // for the tier this belongs to.
    frame.render_widget(Block::default().style(Style::default().bg(BG_BASE)), area);
    let input_height = input_area_height(&app.input);
    // Four bands: the body (conversation log), a 1-row blank spacer, a
    // 1-row status line (identity/activity — see `draw_status_line`), then
    // the input box. The old separate 1-row header and 1-row footer are
    // gone — per explicit developer feedback against a real screenshot, a
    // header full of permission chips at the top and a footer full of
    // half-dead keybinding hints at the bottom read as two disconnected,
    // mostly-noise bars; one status line, positioned right above the input
    // where the developer's eye already is while typing, replaces both.
    //
    // The spacer sits *above* the status line, not below it — per a later
    // round of explicit developer feedback: the original placement (spacer
    // between the status line and the input box) left the log's last line
    // glued directly to the status line above it (no top padding) while
    // stacking two rows of breathing room below it (this spacer plus the
    // input box's own internal top `Padding` row — see `draw_input`),
    // which read as an oversized gap under the status line. Moving the one
    // spacer here gives the status line padding on both sides: top padding
    // from this row, bottom padding from the input box's own internal
    // padding, neither doubled up. The spacer row is otherwise blank (the
    // opaque `BG_BASE` canvas painted above already covers it).
    let [body_area, _spacer_area, status_area, input_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
    ])
    .areas(area);
    let log_area = body_area;

    // No drawn border and no title — the reference screenshot that prompted
    // this pass shows no box anywhere around the conversation, just filled
    // cards floating directly on the frame background (see
    // `palette::BG_BASE`'s doc comment). A right-aligned "live"/"scrolled"
    // title badge used to live here (modeled on posting's
    // `border-title-status`) but per explicit developer feedback it was
    // meaningless noise in the corner of the screen — removed outright, not
    // replaced, so `log_block` is now a plain background fill with nothing
    // reserving a title row.
    let log_block = Block::new().style(Style::default().bg(BG_BASE));
    // `Block::inner` is a pure function of the block's border/title config
    // and the outer rect — computed exactly once here, and this same `Rect`
    // is what both `App::render_width`/`render_height` (cached for scroll
    // math between draws) and `draw_log`'s own content pass use. There must
    // never be a second, independently-derived "inner width" anywhere else
    // in this call graph — see mjolnir-tui.md's scrolling-fix and
    // wrapped-row-scroll-math Progress notes for the two real bugs that
    // came from exactly this kind of divergence before.
    let log_inner = log_block.inner(log_area);
    app.render_width = log_inner.width;
    app.render_height = log_inner.height;
    app.scroll.set_viewport_height(log_inner.height as usize, app.total_lines());

    draw_log(frame, log_area, log_inner, log_block, app);
    draw_status_line(frame, status_area, app);
    draw_input(frame, input_area, app);
}

fn input_area_height(input: &str) -> u16 {
    // No border at all (see `draw_input`) — just top/bottom padding (1 row
    // each, `Padding::new(2, 1, 1, 1)`) around the content, so a single-line
    // draft sits centered in the box rather than glued to one edge of it.
    let lines = input.matches('\n').count() as u16 + 1;
    lines + 2
}

/// Builds every line the log panel's *inner* area can show, at `width` ×
/// `height` (the bordered panel's inner rect — see `draw`'s doc comment on
/// why this must be the inner, not outer, rect). Shared by `draw_log`
/// (renders it) and `log_row_count` (counts its wrapped rows for scroll
/// math), so the two can never disagree about what the log contains. An
/// empty log shows the welcome hero instead of any entries — the banner
/// used to render above the log on every draw regardless of content, which
/// is what made it eat real screen space mid-conversation; now the two are
/// mutually exclusive, so there's no longer a "separate the banner from the
/// first real entry" case to special-case either.
fn build_log_lines(app: &App, width: u16, height: u16) -> Vec<Line<'static>> {
    if app.log.is_empty() {
        return hero_lines(&app.status, height);
    }
    let mut lines: Vec<Line> = Vec::new();
    for entry in app.log.iter() {
        // An entry can render to nothing at all now (a routine `TurnEnded`
        // — see its arm in `render_entry` — is folded into the status
        // line's own activity indicator instead of getting its own log
        // row), so the blank separator is keyed on whether anything has
        // actually been pushed yet, not on the entry's index — otherwise a
        // silent entry would still claim a blank row for itself.
        let rendered = render_entry(entry, width);
        if rendered.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(rendered);
    }
    // No spinner row appended here any more — per explicit developer
    // feedback, an active turn used to get an animated "thinking…"/
    // "working…" row both here (trailing the log) *and* in `draw_status_line`
    // right above the input, which read as a plain duplicate of the same
    // information. The status line is now the one place live turn activity
    // shows; see its own doc comment.
    lines
}

/// Renders the log panel: `block` onto `outer`, its content onto `inner`
/// (already computed once by `draw` — see its doc comment), plus a
/// `Scrollbar` on the inner-right edge when there's more content than the
/// viewport can show and the log isn't showing the (never-scrollable) hero.
fn draw_log(frame: &mut Frame, outer: Rect, inner: Rect, block: Block<'static>, app: &App) {
    let lines = build_log_lines(app, inner.width, inner.height);
    // `scroll.offset` is in *wrapped screen rows* (see `log_row_count`), so
    // it must go through `Paragraph::scroll`, which advances the same
    // wrapping line-composer `Paragraph::line_count` uses internally —
    // not a `.skip()` on `lines` beforehand, which would count in logical
    // (pre-wrap) rows instead and drift out of sync the moment anything
    // wraps.
    let offset = app.scroll.offset.min(u16::MAX as usize) as u16;
    let total = app.total_lines();

    frame.render_widget(block, outer);
    let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).scroll((offset, 0));
    frame.render_widget(paragraph, inner);

    if !app.log.is_empty() && total > inner.height as usize {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).style(Style::default().fg(PANEL_BORDER));
        let mut state = ScrollbarState::new(total).position(offset as usize);
        // Renders into the block's own right-border column, inset by 1 row
        // top/bottom so it doesn't overwrite the panel's rounded corners —
        // the standard ratatui pattern (see `Scrollbar`'s own doc example).
        frame.render_stateful_widget(scrollbar, outer.inner(Margin { vertical: 1, horizontal: 0 }), &mut state);
    }
}

/// The number of terminal rows the log panel's inner area needs to fully
/// render at `width` × `height` — what `ScrollState` (see `scroll.rs`)
/// actually compares against viewport height, wrapping included. Delegates
/// to ratatui's own `Paragraph::line_count`, which runs the exact same
/// word-wrapper `draw_log`'s render path uses, rather than re-deriving wrap
/// behaviour by hand — see mjolnir-tui.md's 2026-08-29 scrolling-fix and
/// wrapped-row-scroll-math Progress notes for the two incidents this
/// discipline exists to prevent from recurring. `height` only matters for
/// the empty-log hero path (`hero_lines` uses it to vertically center); the
/// non-empty path's row count is width-only, same as before.
pub(crate) fn log_row_count(app: &App, width: u16, height: u16) -> usize {
    let lines = build_log_lines(app, width, height);
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

/// Downscaled 2026-08-31 to 13×21 (from the 21×35 trace above/still in git
/// history) per explicit developer feedback that the full-size mark read
/// as "quite large" in the banner. Not a fresh trace or a hand edit: a
/// script decoded every Braille cell of the original back into its 2×4 dot
/// bitmap (84×70 dots), box-filtered that bitmap down by a uniform 0.6 in
/// both dimensions (so the mark stays *proportionate* — same aspect ratio,
/// not squashed on one axis), thresholded each output dot at ≥30% coverage,
/// and re-encoded the result into Braille cells — same technique the
/// original trace used going the other direction, just resampling
/// pixel data instead of hand-placing it. The script isn't kept in the
/// repo, same reasoning as the original trace/wordmark pipelines noted
/// above.
const MJOLNIR_ART: [&str; 13] = [
    "⠀⠀⠀⠀⢀⣶⢛⣯⣿⣯⣿⣽⣿⣽⡛⣦⡀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⢸⣿⣿⢱⡒⣭⡟⣡⢒⡎⣷⣿⡇⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠘⢿⣘⠶⠵⣫⣾⡻⠮⠾⣃⡿⠃⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠘⡆⢠⡹⡿⢏⡄⢰⠃⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⡇⣸⡟⣧⢻⣇⢸⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⢀⡇⢿⠟⣵⢻⡿⢸⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⣼⢹⠫⠗⣋⠾⠝⡟⣇⠀⠀⠀⠀⠀⠀",
    "⢀⣀⣀⣀⣀⣴⣣⡼⠷⠿⣿⡿⠾⢧⣼⣆⣀⣀⣀⣀⡀",
    "⣸⢰⣶⡶⢒⣐⢶⡶⢛⣯⣝⡻⣿⡶⢢⣴⣶⢶⣶⡆⣷",
    "⡟⣼⣿⣧⣛⡹⢸⢳⡟⣶⣦⣿⢸⣇⢿⣫⣭⢭⣍⠳⢹",
    "⢧⣀⣒⡒⠶⢶⣿⣏⠳⣭⣛⣵⡿⣫⣶⣶⠶⢟⣛⣓⣸",
    "⠀⠉⠈⠉⠉⠓⠮⣭⡛⢶⣭⡵⢞⣫⠵⠚⠋⠉⠉⠉⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠉⠳⣬⠞⠋⠀⠀⠀⠀⠀⠀⠀⠀",
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
const MJOLNIR_ART_WIDTH: usize = 21;

/// Reuses the diff-tint colors (`DIFF_ADD_FG`/`DIFF_DEL_FG`) rather than
/// inventing new ones, since green-means-allowed/red-means-denied is the
/// same "state at a glance" job those already do for added/removed diff
/// lines. Rendered as a small padded chip (colored background, not just
/// colored text) — per the posting-inspired UX pass: a categorical state
/// word reads faster as a filled badge than as plain colored text sitting
/// on the panel background, the same reasoning behind posting's
/// `border-title-status`/method-color chips. Used only by the welcome hero
/// (`intro_content`) as of the 2026-08-31 status-line correction — the
/// permission summary was dropped from the always-visible status line
/// per explicit developer request; this stays the one place the current
/// directory's read/shell/edit grants are surfaced on screen.
fn access_spans(label: &'static str, state: PermState) -> Vec<Span<'static>> {
    let (word, fg, bg) = match state {
        PermState::Allowed => ("allow", BRIGHT, DIFF_ADD_BG),
        PermState::Denied => ("deny", BRIGHT, DIFF_DEL_BG),
    };
    // Right-padded only (no space before the word) so the flattened text
    // stays exactly `"{label}:{word} "` — preserves the `"read:deny"`-style
    // substring several tests and the hero/header both already key on —
    // while still giving the word itself a colored chip background.
    vec![Span::styled(format!("{label}:"), Style::default().fg(DIM)), Span::styled(format!("{word} "), Style::default().fg(fg).bg(bg))]
}

/// The welcome hero's content — Mjolnir hammer art beside the wordmark/
/// tagline/stats column — unbordered. `hero_lines` (called only when the
/// log is empty; see `build_log_lines`) centers this vertically within the
/// log panel's own inner height and lets that panel's ratatui-drawn rounded
/// border frame it. This used to end with a hand-drawn `┌─┐`/`└─┘` border of
/// its own (`bordered()`, since removed) when the hero sat directly on the
/// terminal background with no panel of its own — wrapping it in a second
/// border now that it's nested inside the log panel's border just double-
/// boxed the same content (tried during the redesign, discarded after
/// screenshotting both).
fn intro_content(status: &StatusInfo) -> Vec<Line<'static>> {
    debug_assert!(
        MJOLNIR_ART.iter().all(|row| row.chars().count() == MJOLNIR_ART_WIDTH),
        "MJOLNIR_ART rows must stay fixed-width or the info column drifts off-alignment — see every_mjolnir_art_row_is_exactly_mjolnir_art_width_chars"
    );
    let wordmark_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let tagline_style = Style::default().fg(BRIGHT).add_modifier(Modifier::ITALIC);
    let stat_label = Style::default().fg(DIM);
    let stat_value = Style::default().fg(BRIGHT);

    // Beside the art, not above or below it — per the standing developer
    // rule (art left-aligned, text alongside it on the right). The
    // wordmark block sits at the top of this column, tagline/stats below
    // it, the whole column vertically centered against the art's height.
    // Stats render as separate labeled lines (model/version/commit) —
    // per explicit developer request, not packed onto one line. `access`
    // is the fourth stat line, added 2026-08-31 per explicit developer
    // request that the banner surface the current permission model (what's
    // allowed/denied in this directory) rather than making the developer
    // discover it only by triggering a prompt — the same merged
    // session/project/global view (against an empty target, so it reads
    // as "the broadest grant currently in force") already computed for the
    // status bar's own read/shell/edit indicator (`App::refresh_permissions`
    // / `perm_state`), just surfaced a second time where it's visible before
    // the first turn even starts.
    let mut info: Vec<Vec<Span<'static>>> = WORDMARK_ART.iter().map(|row| vec![Span::styled(*row, wordmark_style)]).collect();
    info.push(vec![]);
    info.push(vec![Span::styled("every strike is yours to call. nothing moves without you.", tagline_style)]);
    info.push(vec![]);
    info.push(vec![Span::styled("model    ", stat_label), Span::styled(status.model_name.clone(), stat_value)]);
    info.push(vec![Span::styled("version  ", stat_label), Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), stat_value)]);
    info.push(vec![Span::styled("commit   ", stat_label), Span::styled(env!("MJOLNIR_GIT_HASH"), stat_value)]);
    let mut access = vec![Span::styled("access   ", stat_label)];
    access.extend(access_spans("read", status.read));
    access.push(Span::raw("  "));
    access.extend(access_spans("shell", status.shell));
    access.push(Span::raw("  "));
    access.extend(access_spans("edit", status.edit));
    info.push(access);
    let info_offset = (MJOLNIR_ART.len().saturating_sub(info.len())) / 2;

    // A small fixed left margin (matching the log panel's own left border +
    // a little breathing room) rather than centering — per the standing
    // developer rule that the banner should read left-to-right (art, then
    // wordmark/info beside it), not sit centered in the middle of a wide
    // terminal. A blank line above and below the art gives it vertical
    // breathing room too, per the earlier explicit "padding all the way
    // around" request — still honored, just no longer via a hand-drawn
    // border's own margin math.
    const LEFT_MARGIN: &str = "   ";
    let mut content: Vec<Line<'static>> = Vec::with_capacity(MJOLNIR_ART.len() + 2);
    content.push(Line::default());
    content.extend(MJOLNIR_ART.iter().enumerate().map(|(i, art_row)| {
        let art_style = Style::default().fg(mjolnir_row_color(i, MJOLNIR_ART.len())).add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::raw(LEFT_MARGIN), Span::styled(*art_row, art_style)];
        if let Some(row_i) = i.checked_sub(info_offset) {
            if let Some(line_spans) = info.get(row_i) {
                spans.push(Span::raw("   "));
                spans.extend(line_spans.iter().cloned());
            }
        }
        Line::from(spans)
    }));
    content.push(Line::default());
    content
}

/// Vertically centers `intro_content` within the log panel's inner
/// `height`; `height` is real render-time information, so this can only
/// happen at draw time via `build_log_lines`, same as `MJOLNIR_ART`'s width
/// used to be threaded through `bordered()`'s `width` parameter before the
/// hero's own border was removed. On a terminal short enough that the
/// content doesn't fit, `pad_top` saturates to 0 and the content simply
/// starts at the top and scrolls like any other tall log content would.
fn hero_lines(status: &StatusInfo, height: u16) -> Vec<Line<'static>> {
    let content = intro_content(status);
    let pad_top = (height as usize).saturating_sub(content.len()) / 2;
    let mut lines = Vec::with_capacity(pad_top + content.len());
    lines.extend(std::iter::repeat_with(Line::default).take(pad_top));
    lines.extend(content);
    lines
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
            // Slash-command lines skip the fill treatment entirely — dim,
            // unfilled text — since they aren't a chat message (see
            // `is_command`'s own doc comment: directed at the harness, never
            // the model).
            if is_command(text) {
                return text.lines().map(|l| Line::from(Span::styled(format!("> {l}"), Style::default().fg(DIM)))).collect();
            }
            // Flat filled "bubble", no left accent bar — per explicit
            // developer feedback that the bar (mirroring OpenCode's own
            // `border={["left"]}` treatment) was unwanted borrowed
            // decoration; the background tint alone, padded to the full
            // render width via `card_line` (which also gives every row its
            // `BOX_PAD_H` left/right inset), already reads as a chat bubble
            // even for a short message, not just a tinted prefix. A blank
            // `BG_ELEMENT`-filled row above and below the text (see
            // `card_padding_line`) gives the bubble the same top/bottom
            // padding its own left/right inset already has — per explicit
            // developer feedback that a chat message needs "padding on the
            // top, the right, the left, and the bottom," equally on every
            // side. Padding is sized in display columns
            // (`UnicodeWidthStr::width`, inside `card_line`/`filled_line`),
            // not `chars().count()` — a chat message can contain CJK/emoji
            // double-width glyphs, and undercounting those overshoots the
            // real render width, pushing the "single-row bubble" onto an
            // extra wrapped row (see mjolnir-tui.md's wide-char Progress
            // note).
            let style = Style::default().fg(USER_FG).bg(BG_ELEMENT);
            let mut lines = vec![card_padding_line(BG_ELEMENT, width)];
            lines.extend(text.lines().map(|l| card_line(l, style, width)));
            lines.push(card_padding_line(BG_ELEMENT, width));
            lines
        }
        LogEntry::AssistantText { text } => render_assistant_text(text, width),
        // A leading glyph per status — running/done/error — instead of a
        // bracketed text tag, so a scan of the log reads statuses at a
        // glance the same way the diff/access indicators already do
        // elsewhere. Done/error reuse the diff-tint colors (green/red) —
        // the same "state at a glance" job those already do — rather than
        // introducing new ones.
        LogEntry::ToolActivity { calls, .. } => calls
            .iter()
            .map(|c| {
                let label = if c.name.is_empty() { c.call_id.clone() } else { format!("{} ({})", c.name, c.call_id) };
                let (glyph, color, text) = match &c.status {
                    ToolActivityStatus::Running => ("▸", DIM, label),
                    ToolActivityStatus::Completed { is_error: false, summary } => ("✓", DIFF_ADD_FG, format!("{label}: {summary}")),
                    ToolActivityStatus::Completed { is_error: true, summary } => ("✗", DIFF_DEL_FG, format!("{label}: {summary}")),
                };
                Line::from(Span::styled(format!("  {glyph} {text}"), Style::default().fg(color)))
            })
            .collect(),
        // Amber — the one new color the redesign adds (`WARNING_FG`) — so a
        // retry reads as worth noticing, not just more dim tool metadata.
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            vec![Line::from(Span::styled(
                format!("  ⟳ [retry {}] {} {status}: {}", info.attempt, info.provider, info.message),
                Style::default().fg(WARNING_FG),
            ))]
        }
        LogEntry::ApprovalCard { diff, resolution, .. } => render_approval_card(diff, *resolution, width),
        LogEntry::PermissionPrompt { payload, resolution, .. } => render_prompt_card(payload, resolution.as_deref(), width),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            // The ordinary case renders nothing at all — per explicit
            // developer feedback that a "— answered —" row was redundant
            // the moment the status line started showing live
            // idle/thinking/working activity (`draw_status_line`); saying
            // the turn ended is no longer new information by the time this
            // entry appears. Cancelled/error keep their own inline text
            // since neither outcome is otherwise visible anywhere once the
            // turn ends.
            match reason {
                TurnEndReasonKind::EndTurn => vec![],
                TurnEndReasonKind::Cancelled => vec![Line::from(Span::styled("— turn cancelled —", Style::default().fg(DIM)))],
                TurnEndReasonKind::Error(message) => {
                    vec![Line::from(Span::styled(format!("— turn ended in error: {message} —"), Style::default().fg(DIM)))]
                }
            }
        }
        // `DIFF_DEL_FG` rather than a bare `Color::Red` — cohesion with the
        // rest of the error/removed/deny semantic group instead of a color
        // that belongs to no other role in the palette.
        LogEntry::Error { message } => vec![Line::from(Span::styled(format!("✗ error: {message}"), Style::default().fg(DIFF_DEL_FG)))],
        LogEntry::Notice { message } => vec![Line::from(Span::styled(format!("ℹ {message}"), Style::default().fg(DIM)))],
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

fn render_assistant_text(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in split_code_fences(text) {
        match segment {
            // Left-inset by `BOX_PAD_H` (`indent_prose_line`) — no fill of
            // its own (assistant prose deliberately stays unfilled; see
            // mjolnir-tui.md's Palette section), but per explicit developer
            // feedback every chat component should carry the same amount of
            // padding, so plain prose still starts at the same column a
            // filled chat bubble's own text does, via `filled_line`'s
            // identical `BOX_PAD_H` inset.
            Segment::Prose(s) => {
                // Pre-wrapped here (rather than left to the log paragraph's
                // own `Wrap`) and indented per resulting row — see
                // `wrap_prose_line`'s doc comment for why: `Wrap` has no
                // concept of this line's left padding, so a wrapped
                // continuation row it produced came out flush against the
                // panel edge instead of under the inset every other row gets.
                let inner_width = (width as usize).saturating_sub(BOX_PAD_H);
                lines.extend(
                    s.lines()
                        .flat_map(|l| wrap_prose_line(render_markdown_line(l), inner_width))
                        .map(indent_prose_line),
                );
            }
            // A fenced ```diff block gets the same full-width red/green
            // per-line treatment (now with a line-number gutter — see
            // `number_diff_lines`) as the Edit approval card
            // (`render_diff_line`/`parse_diff_body`) instead of the generic
            // code-block box below — per explicit developer feedback that a
            // proposed diff inside assistant prose showing a box labeled
            // "diff" around plain unhighlighted +/- text was the wrong
            // treatment: the card mechanism already exists precisely for
            // "show a diff," so this reuses it rather than inventing a
            // second diff presentation.
            Segment::Code { lang, body } if lang.eq_ignore_ascii_case("diff") => {
                let (_, diff_body) = parse_diff_body(&body);
                lines.extend(number_diff_lines(diff_body).iter().map(|line| render_diff_line(line, width)));
            }
            // A real filled code-block box — dark `CODE_BG`, a language
            // label instead of the fence's own literal ` ``` ` markers, no
            // hand-drawn `╭─`/`│ `/`╰─` ASCII border — per explicit
            // developer feedback that the border read as "ugly ASCII art"
            // and a code block should look like "a real code block in a
            // document," the same "colored box, not a hand-drawn frame"
            // treatment the diff/approval cards already got. `filled_line`
            // gives every row (label included) the box's own left/right
            // padding and a `card_padding_line` spacer under the label and
            // at the bottom gives it top/bottom padding too, same as every
            // other filled box in the log.
            Segment::Code { lang, body } => {
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.push(card_line(&label, Style::default().fg(DIM).bg(CODE_BG), width));
                lines.push(card_padding_line(CODE_BG, width));
                for code_line in highlight::highlight_lines(&lang, &body) {
                    let spans: Vec<Span<'static>> = code_line.into_iter().map(|s| Span::styled(s.content, s.style.bg(CODE_BG))).collect();
                    lines.push(filled_line(spans, CODE_BG, width));
                }
                lines.push(card_padding_line(CODE_BG, width));
            }
        }
    }
    // No extra leading/trailing blank rows of its own any more — per
    // explicit developer feedback that assistant messages read with
    // noticeably more top/bottom padding than the user's own input. The
    // reason: `build_log_lines` already inserts one blank separator row
    // between every pair of rendered entries. A filled bubble (user
    // messages, code blocks, diff/approval cards) pads with its own
    // `card_padding_line`, which is visually distinct from that blank
    // separator (colored fill vs. plain gap), so the two don't read as
    // doubled. Assistant prose has no fill to pad with, so it used to add
    // its own *blank* row on top of the separator's blank row — two
    // indistinguishable blank rows stacking into a gap twice the size of
    // every other component's. Leaving padding to the separator alone
    // matches assistant messages to the same single-row gap everything
    // else gets.
    lines
}

/// Word-wraps one logical prose `Line` to `max_width` display columns,
/// breaking only at whitespace and preserving each span's style across a
/// break, into however many `Line`s it takes.
///
/// This exists instead of leaning on the log paragraph's own
/// `Wrap { trim: false }` (`draw_log`) because `Wrap` has no concept of a
/// per-row left inset: it treats one logical `Line`'s spans as a single
/// continuous run of styled graphemes and only ever emits `indent_prose_line`'s
/// inserted padding span wherever it happens to land in the first wrapped
/// row — a real paragraph longer than one screen row came out with its
/// first row correctly inset and every wrapped continuation row flush
/// against the log panel's left edge (reported as: "the first line of text
/// is correctly in line, but when the text wraps onto a second line, it
/// doesn't respect the padding"). Doing the wrap here means every row this
/// returns is already ≤ `max_width` columns before the caller insets it, so
/// `Wrap` never has to touch it — the wrapping happens once, not twice.
///
/// Doesn't hang-indent list/blockquote markers under wrapped continuation
/// text (a wrapped `• ` bullet's second row starts at the same column every
/// other prose row does, not under the first row's text) — only the flat
/// inset every prose row gets from `indent_prose_line` regardless of what
/// produced it.
fn wrap_prose_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    #[derive(Clone)]
    struct Grapheme {
        text:     String,
        style:    Style,
        width:    usize,
        is_space: bool,
    }

    if max_width == 0 {
        return vec![line];
    }

    let graphemes: Vec<Grapheme> = line
        .spans
        .into_iter()
        .flat_map(|span| {
            let style = span.style;
            span.content
                .chars()
                .map(move |ch| Grapheme { text: ch.to_string(), style, width: ch.width().unwrap_or(0), is_space: ch.is_whitespace() })
                .collect::<Vec<_>>()
        })
        .collect();
    if graphemes.is_empty() {
        return vec![Line::default()];
    }

    let mut rows: Vec<Vec<Grapheme>> = vec![Vec::new()];
    let mut row_width = 0usize;
    let mut i = 0;

    // The line's own genuine leading whitespace (if any) is kept as literal
    // content on the first row, same as before this function existed — only
    // whitespace a wrap decision below introduces at a row break gets
    // dropped, so a rare hand-indented prose line doesn't lose that
    // indentation just because it happens to be short enough to fit on one
    // row anyway.
    if graphemes[0].is_space {
        let end = graphemes.iter().position(|g| !g.is_space).unwrap_or(graphemes.len());
        row_width = graphemes[..end].iter().map(|g| g.width).sum();
        rows[0].extend_from_slice(&graphemes[..end]);
        i = end;
    }

    // Greedy fill: walk whitespace/non-whitespace runs in order, breaking
    // before whichever run would overflow the current row. A run of
    // whitespace is only ever kept mid-row (never used to open one), so a
    // wrapped row never starts with the space that caused the break — same
    // "don't start a wrapped line with the space that broke it" convention
    // ratatui's own word-wrapper follows.
    while i < graphemes.len() {
        let is_space = graphemes[i].is_space;
        let start = i;
        while i < graphemes.len() && graphemes[i].is_space == is_space {
            i += 1;
        }
        let run = &graphemes[start..i];
        let run_width: usize = run.iter().map(|g| g.width).sum();

        if is_space {
            if !rows.last().unwrap().is_empty() {
                if row_width + run_width > max_width {
                    rows.push(Vec::new());
                    row_width = 0;
                } else {
                    row_width += run_width;
                    rows.last_mut().unwrap().extend_from_slice(run);
                }
            }
            continue;
        }

        if row_width > 0 && row_width + run_width > max_width {
            rows.push(Vec::new());
            row_width = 0;
        }
        if run_width > max_width {
            // A single word wider than the whole row (e.g. a long URL):
            // hard-break it grapheme by grapheme rather than overflowing.
            for g in run {
                if row_width > 0 && row_width + g.width > max_width {
                    rows.push(Vec::new());
                    row_width = 0;
                }
                row_width += g.width;
                rows.last_mut().unwrap().push(g.clone());
            }
        } else {
            row_width += run_width;
            rows.last_mut().unwrap().extend_from_slice(run);
        }
    }

    rows.into_iter()
        .map(|row| {
            // A wrap decision can leave trailing whitespace dangling at a
            // row's end (the space that caused the break, kept out of the
            // *next* row but already appended to this one before the
            // overflow check above); trim it so it doesn't count toward
            // width for anyone measuring this row later.
            let end = row.iter().rposition(|g| !g.is_space).map(|i| i + 1).unwrap_or(0);
            let mut spans: Vec<Span<'static>> = Vec::new();
            for g in &row[..end] {
                match spans.last_mut() {
                    Some(Span { content, style }) if *style == g.style => {
                        let mut merged = content.to_string();
                        merged.push_str(&g.text);
                        *content = merged.into();
                    }
                    _ => spans.push(Span::styled(g.text.clone(), g.style)),
                }
            }
            Line::from(spans)
        })
        .collect()
}

/// Left-insets an unfilled line by `BOX_PAD_H` columns — a plain raw space,
/// not a styled span, since there's no background to carry; see
/// `render_assistant_text`'s `Prose` arm for why this exists.
fn indent_prose_line(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw(" ".repeat(BOX_PAD_H)));
    line
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

/// Horizontal inset applied inside every filled box in the log — chat
/// bubbles, code blocks, and diff/approval-card rows — so text doesn't sit
/// flush against the box's own left/right edge. Per explicit developer
/// feedback that chat messages need "padding on the top, the right, the
/// left, and the bottom," and that every chat component should carry the
/// same amount of it: one shared constant, applied by the one shared
/// primitive below (`filled_line`), keeps every box's padding identical by
/// construction instead of separately hand-tuned per call site.
const BOX_PAD_H: usize = 1;

/// The shared padding primitive every filled box in the log builds its rows
/// from: `BOX_PAD_H` columns of `bg`, then `spans`, then `bg`-filled columns
/// out to `width` — giving left+right padding and a full-width fill in one
/// step. `spans` must already carry whatever `bg` they should show against
/// (this only pads around them, it doesn't recolor them), so a caller
/// mixing a semantic tint (e.g. `DIFF_ADD_BG`) into an otherwise-`bg`
/// row still reads correctly.
fn filled_line(mut spans: Vec<Span<'static>>, bg: Color, width: u16) -> Line<'static> {
    let content_width: usize = spans.iter().map(|s| s.content.width()).sum();
    let pad = (width as usize).saturating_sub(BOX_PAD_H).saturating_sub(content_width);
    let mut out = Vec::with_capacity(spans.len() + 2);
    out.push(Span::styled(" ".repeat(BOX_PAD_H), Style::default().bg(bg)));
    out.append(&mut spans);
    out.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    Line::from(out)
}

/// One line of a filled "card": `content`, styled per `content_style` and
/// padded (via `filled_line`) to the full render width so the fill reads as
/// one continuous card rather than per-line background patches. Used to
/// carry a left accent bar glyph (mirroring OpenCode's own
/// `border={["left"]}` input) — dropped per explicit developer feedback
/// that it read as stray decoration borrowed from OpenCode rather than
/// something Mjolnir's own cards needed; the flat full-width fill alone
/// already reads as "this is a card." `content_style` carries whatever bg
/// the caller wants (the neutral `BG_ELEMENT` card fill, or a semantic tint
/// like `DIFF_ADD_BG` that should win over it) — this helper doesn't pick
/// one, it just reads it back out to pad with the matching color.
fn card_line(content: &str, content_style: Style, width: u16) -> Line<'static> {
    let bg = content_style.bg.unwrap_or(BG_BASE);
    filled_line(vec![Span::styled(content.to_string(), content_style)], bg, width)
}

/// The Edit approval card: filled title and keys (same shape as
/// `render_card`) around a diff-aware body — added/removed lines get a
/// full-width background tint (see `DIFF_ADD_BG`/`DIFF_DEL_BG`), and
/// unchanged context beyond `DIFF_CONTEXT_RADIUS` lines from the nearest
/// change collapses to a single "N unchanged lines" marker — per explicit
/// developer feedback that the card previously rendered every diff line in
/// the same plain style, which made it hard to tell what actually changed
/// at a glance.
fn render_approval_card(diff: &str, resolution: Option<bool>, width: u16) -> Vec<Line<'static>> {
    let (path, body) = parse_diff_body(diff);
    let body = number_diff_lines(body);
    // A blank filled row top and bottom (see `card_padding_line`'s doc
    // comment) — plain terminal text sat flush against the card's edges,
    // which read as cramped next to the reference's generous interior
    // padding.
    let mut lines = vec![card_padding_line(BG_ELEMENT, width), card_line("Approve this edit?", Style::default().fg(ACCENT).bg(BG_ELEMENT).add_modifier(Modifier::BOLD), width)];
    if let Some(path) = path {
        lines.push(card_line(&path, Style::default().fg(DIM).bg(BG_ELEMENT), width));
    }

    let n = body.len();
    let mut keep = vec![false; n];
    for (i, line) in body.iter().enumerate() {
        if !matches!(line.kind, DiffLineKind::Context) {
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
            let line = &body[i];
            lines.push(render_diff_line(line, width));
            i += 1;
        } else {
            let elided_start = i;
            while i < n && !keep[i] {
                i += 1;
            }
            let count = i - elided_start;
            lines.push(card_line(
                &format!("⋯ {count} unchanged line{} ⋯", if count == 1 { "" } else { "s" }),
                Style::default().fg(DIM).bg(BG_ELEMENT),
                width,
            ));
        }
    }

    match resolution {
        Some(approved) => lines.push(card_line(
            &format!("resolved: {}", if approved { "approved" } else { "denied" }),
            Style::default().fg(ACCENT).bg(BG_ELEMENT),
            width,
        )),
        None => lines.push(card_line(approval_key_hint(), Style::default().fg(ACCENT).bg(BG_ELEMENT), width)),
    }
    lines.push(card_padding_line(BG_ELEMENT, width));
    lines
}

/// A blank, filled row — same fill mechanism as `card_line`, just with
/// empty content — used as a leading/trailing spacer inside a card so its
/// content doesn't sit flush against the card's own top/bottom edge.
fn card_padding_line(bg: Color, width: u16) -> Line<'static> {
    card_line("", Style::default().bg(bg), width)
}

/// The approval card's own key labels — also shown in the footer key-hint
/// bar (`draw_footer`) while the card is pending, via this exact function,
/// so the two can never drift apart (guarded by
/// `footer_and_approval_card_show_identical_key_labels`).
fn approval_key_hint() -> &'static str {
    "[y] approve   [n] deny   [Ctrl+C] deny"
}

/// Appends a "+N more pending" suffix to a card's own key hint when its
/// queue (`App::pending_approvals`/`pending_prompts`) holds more than the
/// one currently interactive entry — see `draw_status_line`. Kept separate
/// from `approval_key_hint`/`prompt_key_hint` themselves rather than
/// threading a count through them: those two are also what each card's own
/// key row in the log renders with (per `draw_status_line`'s doc comment,
/// "the exact same functions"), and a per-card queue-depth suffix would be
/// wrong there — a card only ever represents itself, not how many other
/// cards are waiting behind it.
fn queue_hint(hint: String, queue_len: usize) -> String {
    match queue_len.saturating_sub(1) {
        0 => hint,
        n => format!("{hint}   (+{n} more pending)"),
    }
}

/// One diff body line plus the line number(s) it carries in each side of the
/// change — see `number_diff_lines`.
struct DiffLine {
    kind: DiffLineKind,
    text: String,
    old_no: Option<usize>,
    new_no: Option<usize>,
}

/// Assigns old-file/new-file line numbers to a parsed diff body — per
/// explicit developer feedback that diffs rendered with no line numbers at
/// all. `mjolnir_tools::diff::unified` emits no `@@ -a,b +c,d @@` hunk
/// header (see its own doc comment: it diffs a single already-replaced
/// hunk, not a whole file), so there's no absolute file offset to anchor
/// on — these are relative to the start of the shown diff, numbered from 1
/// on each side, the same convention a hunk header's own numbers use
/// relative to itself. Context lines advance both counters (they exist on
/// both sides); removed lines only the old counter; added lines only the
/// new one — mirroring the two-column gutter GitHub/most diff UIs show.
fn number_diff_lines(body: Vec<(DiffLineKind, String)>) -> Vec<DiffLine> {
    let mut old_no = 1usize;
    let mut new_no = 1usize;
    body.into_iter()
        .map(|(kind, text)| {
            let (o, n) = match kind {
                DiffLineKind::Context => (Some(old_no), Some(new_no)),
                DiffLineKind::Removed => (Some(old_no), None),
                DiffLineKind::Added => (None, Some(new_no)),
            };
            if o.is_some() {
                old_no += 1;
            }
            if n.is_some() {
                new_no += 1;
            }
            DiffLine { kind, text, old_no: o, new_no: n }
        })
        .collect()
}

/// Right-aligned `old │ new` line-number gutter, blank on whichever side a
/// line doesn't exist on (an added line has no old-file number, a removed
/// line has no new-file number) — same shape as `card_line`'s own filled
/// rows, just multi-span so the gutter can carry its own dim color
/// independent of the marker/text's semantic fg.
fn diff_gutter(old_no: Option<usize>, new_no: Option<usize>, bg: Color) -> Span<'static> {
    let o = old_no.map(|n| n.to_string()).unwrap_or_default();
    let n = new_no.map(|n| n.to_string()).unwrap_or_default();
    Span::styled(format!("{o:>4} {n:>4} │ "), Style::default().fg(DIM).bg(bg))
}

/// Renders one kept diff line via `filled_line`, prefixed with its
/// old/new line-number gutter (see `diff_gutter`). Added/removed lines get
/// their semantic `DIFF_ADD_BG`/`DIFF_DEL_BG` tint (which wins over the
/// card's own neutral fill) so a change reads as a colored row at a glance,
/// not just a leading +/- character; context lines get the plain
/// `BG_ELEMENT` card fill, same as every other card line, since only the
/// changed lines' brighter tint should compete for attention.
fn render_diff_line(line: &DiffLine, width: u16) -> Line<'static> {
    let (marker, fg, bg) = match line.kind {
        DiffLineKind::Added => ("+", DIFF_ADD_FG, DIFF_ADD_BG),
        DiffLineKind::Removed => ("-", DIFF_DEL_FG, DIFF_DEL_BG),
        DiffLineKind::Context => (" ", BRIGHT, BG_ELEMENT),
    };
    let spans = vec![diff_gutter(line.old_no, line.new_no, bg), Span::styled(format!("{marker}{}", line.text), Style::default().fg(fg).bg(bg))];
    filled_line(spans, bg, width)
}

fn render_prompt_card(payload: &PromptPayload, resolution: Option<&str>, width: u16) -> Vec<Line<'static>> {
    let title = match payload {
        PromptPayload::Tool { kind, target } => format!("Allow {kind}: {target}?"),
        PromptPayload::ContextFile { path } => format!("Inject context file {}?", path.display()),
        PromptPayload::Edit { kind } => format!("Edit approval for {kind}"),
    };
    render_card(&title, "", &prompt_key_hint(payload), resolution.map(str::to_string), width)
}

/// The permission prompt's own key labels, by payload shape — also shown in
/// the footer key-hint bar (`draw_footer`) while the prompt is pending, via
/// this exact function, so the two can never drift apart (same guard as
/// `approval_key_hint`).
fn prompt_key_hint(payload: &PromptPayload) -> String {
    match payload {
        PromptPayload::Tool { .. } => "[o]nce [s]ession [p]roject [a]lways   Shift = deny at the same tier   Ctrl+C = deny once".to_string(),
        PromptPayload::ContextFile { .. } => "[s]ession [p]roject [n]o   Ctrl+C = no".to_string(),
        PromptPayload::Edit { .. } => String::new(),
    }
}

fn render_card(title: &str, body: &str, keys: &str, resolution: Option<String>, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![card_padding_line(BG_ELEMENT, width), card_line(title, Style::default().fg(ACCENT).bg(BG_ELEMENT).add_modifier(Modifier::BOLD), width)];
    for l in body.lines() {
        lines.push(card_line(l, Style::default().fg(BRIGHT).bg(BG_ELEMENT), width));
    }
    match resolution {
        Some(r) => lines.push(card_line(&format!("resolved: {r}"), Style::default().fg(ACCENT).bg(BG_ELEMENT), width)),
        None => lines.push(card_line(keys, Style::default().fg(ACCENT).bg(BG_ELEMENT), width)),
    }
    lines.push(card_padding_line(BG_ELEMENT, width));
    lines
}

/// Deterministic per-tool-name color from `TOOL_PALETTE` — the same tool
/// name always lands on the same color (a stable hash, not an assignment
/// order that could shift between draws or sessions), so a scan of the
/// status line's tool list distinguishes categories by color the same way
/// posting's per-HTTP-method colors do, without needing to track a
/// name-to-color table anywhere in `App`.
fn tool_color(name: &str) -> Color {
    let hash = name.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    TOOL_PALETTE[hash as usize % TOOL_PALETTE.len()]
}

/// Persistent identity/activity strip, one row, positioned directly above
/// the input box rather than at the top of the frame — replaces the old
/// separate 1-row header (model/turn/permission chips), 1-row footer
/// (keybinding legend), and the sidebar panel entirely, per explicit
/// developer feedback against a real screenshot: three separate ambient-state
/// surfaces (top bar, bottom bar, right column) read as noisy and too close
/// to OpenCode's own layout rather than something distinctly Mjolnir's.
/// While a card or prompt is pending, still shows that card's own keys (via
/// the exact same functions the card itself renders with, so the two can't
/// drift apart) since those are the only keys that do anything while input
/// is blocked. Otherwise shows what's actually happening with the model —
/// turn/step, live activity (thinking/working/idle, with the same spinner
/// the log uses), any tools currently in flight (colored per name via
/// `tool_color`, the same job the removed sidebar did), and a running
/// message count. Permission state (read/shell/edit) is deliberately
/// absent here — per explicit developer request, that belongs to the
/// once-per-session welcome hero (`intro_content`) and an actual
/// permission prompt when one fires, not a line that repaints every frame.
fn draw_status_line(frame: &mut Frame, area: Rect, app: &App) {
    // Both queues (see `App::pending_approvals`/`pending_prompts`) can hold
    // more than one entry when the model dispatched several approval- or
    // prompt-gated calls in one step — only the front is ever interactive,
    // so its hint is what's shown, with a "+N more pending" suffix instead
    // of silently leaving the developer to discover the next one only after
    // resolving this one.
    if let Some(prompt) = app.pending_prompts.front() {
        let hint = queue_hint(prompt_key_hint(&prompt.payload), app.pending_prompts.len());
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(DIM)))), area);
        return;
    }
    if !app.pending_approvals.is_empty() {
        let hint = queue_hint(approval_key_hint().to_string(), app.pending_approvals.len());
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(DIM)))), area);
        return;
    }

    let s = &app.status;
    // Activity leads the row — per explicit developer request that "what's
    // the LLM doing right now" is the single most useful thing this line
    // can say, so it shouldn't be buried after the model name/turn counter.
    let spinner = SPINNER_FRAMES[app.tick as usize % SPINNER_FRAMES.len()];
    let mut spans = if app.thinking {
        vec![Span::styled(format!("{spinner} thinking…  "), Style::default().fg(ACCENT))]
    } else if app.turn_active {
        vec![Span::styled(format!("{spinner} working…  "), Style::default().fg(ACCENT))]
    } else {
        vec![Span::styled("idle  ", Style::default().fg(DIM))]
    };

    let turn_step = match (s.turn, s.step) {
        (Some(t), Some(st)) => format!("T{t} S{st}"),
        (Some(t), None) => format!("T{t}"),
        _ => "-".to_string(),
    };
    spans.push(Span::styled(format!("{}  {turn_step}  ", s.model_name), Style::default().fg(DIM)));

    if !s.running_tools.is_empty() {
        spans.push(Span::styled("tools: ", Style::default().fg(DIM)));
        for (i, tool) in s.running_tools.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(", "));
            }
            let name = if tool.name.is_empty() { tool.call_id.as_str() } else { tool.name.as_str() };
            spans.push(Span::styled(name.to_string(), Style::default().fg(tool_color(name))));
        }
        spans.push(Span::raw("  "));
    }

    let messages = app.log.len();
    spans.push(Span::styled(format!("{messages} message{}", if messages == 1 { "" } else { "s" }), Style::default().fg(DIM)));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Solid filled "card" with no drawn border at all — per explicit developer
/// feedback against a real screenshot that the left accent bar this used to
/// carry (mirroring OpenCode's own `border={["left"]}` input) read as
/// stray decoration, not something the input needed: the `BG_INPUT` fill
/// plus its own padding already reads as "this is a text box" on its own,
/// same as the log panel and every card lost their drawn borders for. Top
/// and bottom padding are equal (`Padding::new(2, 1, 1, 1)`) so a
/// single-line draft sits vertically centered in the box rather than
/// pinned to one edge of it — the asymmetric top-only padding this
/// replaces was what made a short draft look like it had collapsed to the
/// bottom of the box.
fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    // `BG_INPUT`, not `BG_ELEMENT` — sampled as the lightest of the four
    // background tiers (see its doc comment), one step past the fill
    // message/card content uses, since the input is the one surface that's
    // always active/focused rather than passive content.
    let block = Block::new().style(Style::default().bg(BG_INPUT)).padding(Padding::new(2, 1, 1, 1));
    let inner = block.inner(area);
    // Dim placeholder text when the draft is empty — an empty filled box
    // gave no hint at all that this was where a message goes, versus every
    // other panel now carrying a title/label of its own. While blocked, the
    // placeholder says so instead of inviting a keystroke it would silently
    // drop.
    if app.input.is_empty() {
        let blocked = !app.pending_approvals.is_empty() || !app.pending_prompts.is_empty();
        let text = if blocked { "waiting on your decision above…" } else { "Ask Mjolnir anything" };
        let placeholder = Line::from(Span::styled(text, Style::default().fg(DIM)));
        frame.render_widget(Paragraph::new(placeholder).block(block), area);
        if !blocked {
            frame.set_cursor_position((inner.x, inner.y));
        }
        return;
    }
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
    // card is pending (input is blocked then, and the placeholder text says
    // so instead — see above). `cursor_line_col` counts by
    // source line, not wrapped screen row (see its doc comment), so a single
    // logical line long enough to wrap past the box's width places the
    // terminal cursor past the visible text — clamped to `inner`'s last
    // column/row below so it never lands outside the box rather than fixing
    // the underlying wrap mismatch.
    if app.pending_approvals.is_empty() && app.pending_prompts.is_empty() {
        let (line, col) = cursor_line_col(&app.input, app.cursor);
        let inner_right = inner.x + inner.width.saturating_sub(1);
        let inner_bottom = inner.y + inner.height.saturating_sub(1);
        let x = (inner.x + col as u16).min(inner_right);
        let y = (inner.y + line as u16).min(inner_bottom);
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

    /// Screen row the first real log entry starts on once the log is
    /// non-empty: row 0, the very top of the frame — no header, no border,
    /// and (as of the "live"/"scrolled" badge's removal) no reserved title
    /// row either. Fixed, unlike the old always-on-banner layout — the
    /// welcome hero and real log entries are mutually exclusive now (see
    /// `build_log_lines`), so there's no banner/separator height to add.
    fn content_base() -> u16 {
        0
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
    /// belongs "right above the input field," not in a separate panel.
    #[test]
    fn status_line_shows_activity_running_tools_and_message_count() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.turn_active = true;
        app.status.running_tools = vec![crate::app::RunningTool { call_id: "c1".into(), name: "shell".into() }];
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("working"), "an active turn should show in the status line: {out:?}");
        assert!(out.contains("shell"), "an in-flight tool's name should show in the status line: {out:?}");
        assert!(out.contains("1 message"), "the status line should show a running message count: {out:?}");
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
        // Anchored on "claude-sonnet-5", not "working" — an active turn also
        // shows its own "working" spinner at the bottom of the conversation
        // log itself (see `build_log_lines`), a different row than the
        // status line; the model name is unique to the status line.
        let row = find_row(&buffer, "claude-sonnet-5");
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
    fn approval_card_shows_labeled_keys_and_the_diff() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("approve"));
        assert!(out.contains("deny"));
        assert!(out.contains("old"));
        assert!(out.contains("new"));
    }

    /// Guards the `approval_key_hint`/`prompt_key_hint` extraction: the
    /// footer (`draw_footer`) and the inline card (`render_approval_card`/
    /// `render_prompt_card`) call the exact same functions for their key
    /// labels, so they can never silently drift apart the way two
    /// hand-duplicated strings could.
    #[test]
    fn footer_and_approval_card_show_identical_key_labels() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into() });
        let out = rendered(&mut app, 100, 20);
        let hint = approval_key_hint();
        // The hint text appears twice: once in the card itself, once in the footer.
        assert_eq!(out.matches(hint).count(), 2, "expected the approval card and the footer to show the exact same key labels, got: {out:?}");
    }

    #[test]
    fn footer_mirrors_a_pending_permission_prompts_keys() {
        let mut app = app();
        let payload = PromptPayload::Tool { kind: "shell".into(), target: "git status".into() };
        app.log.push(LogEntry::PermissionPrompt { call_id: "c1".into(), payload: payload.clone(), resolution: None });
        app.pending_prompts.push_back(crate::app::PendingPrompt { call_id: "c1".into(), payload: payload.clone() });
        let out = rendered(&mut app, 100, 20);
        let hint = prompt_key_hint(&payload);
        assert_eq!(out.matches(hint.as_str()).count(), 2, "expected the prompt card and the footer to show the exact same key labels, got: {out:?}");
    }

    /// A single pending approval must not claim there's more behind it — a
    /// bare "+0 more pending" or similar would be worse than no count at
    /// all. Companion to `two_pending_approvals_are_queued_not_overwritten_
    /// and_resolve_in_order` in `app.rs` (which covers that a second request
    /// actually queues); this covers the queue depth becoming visible to the
    /// developer once it does.
    #[test]
    fn footer_shows_no_queue_count_for_a_single_pending_approval() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into() });
        let out = rendered(&mut app, 100, 20);
        assert!(!out.contains("more pending"), "one pending approval must not claim there's another queued: {out:?}");
    }

    /// A second queued approval — the actual scenario the queueing fix
    /// covers — must surface as a visible count in the footer, not just be
    /// silently resolvable one at a time with no warning that another card
    /// is about to demand input right after this one.
    #[test]
    fn footer_shows_a_count_of_additional_pending_approvals() {
        let mut app = app();
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into() });
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c2".into() });
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c3".into() });
        let out = rendered(&mut app, 100, 20);
        assert!(out.contains("+2 more pending"), "expected the footer to show 2 more queued beyond the front card, got: {out:?}");
    }

    #[test]
    fn approval_card_colors_added_and_removed_lines_distinctly() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let removed_row = find_row(&buffer, "old");
        let added_row = find_row(&buffer, "new");
        // Column 3 lands inside "-old"/"+new" itself (no left accent bar or
        // panel border ahead of it any more — the card's fill starts at
        // column 0), so any column here works; picked to also land on real
        // text rather than the row's trailing padding.
        assert_eq!(buffer[(3, removed_row)].bg, DIFF_DEL_BG, "a removed line should carry the removed-line background across the row");
        assert_eq!(buffer[(3, added_row)].bg, DIFF_ADD_BG, "an added line should carry the added-line background across the row");
        assert_ne!(buffer[(3, removed_row)].bg, buffer[(3, added_row)].bg, "added and removed lines must be visually distinct");
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
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: diff.into(), resolution: None });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row_text = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol().to_string()).collect() };

        let context_row = row_text(find_row(&buffer, "one"));
        let removed_row = row_text(find_row(&buffer, "old"));
        let added_row = row_text(find_row(&buffer, "new"));
        assert!(context_row.contains("1    1 │  one"), "a context line should show the same line number on both sides: {context_row:?}");
        assert!(removed_row.contains("2      │ -old"), "a removed line should show only its old-file line number: {removed_row:?}");
        assert!(added_row.contains("2 │ +new"), "an added line should show only its new-file line number: {added_row:?}");
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
        let user_cell = &buffer[(2, user_row)];
        let assistant_cell = &buffer[(2, assistant_row)];
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

        // Single-line draft -> 3-row input card (see `input_area_height`:
        // 1 padding-top row + 1 content row + 1 padding-bottom row, no
        // border at all) at the very bottom of a 20-row frame; content sits
        // on the middle row (y=18), 2 cells in from the card's left edge
        // (the card's own left padding — see `draw_input`; there's no
        // border to add to it any more).
        let slash_cell = &buffer[(2, 18)]; // '/'
        let arg_cell = &buffer[(9, 18)]; // 'n' of "now"
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

        let leading_cell = &buffer[(2, 18)]; // 'h' of "hi"
        let slash_cell = &buffer[(5, 18)]; // '/' of "/exit"
        let trailing_cell = &buffer[(11, 18)]; // 't' of "there"
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
        // Input card is the last Length(3) row of the layout (see
        // `input_area_height`): padding-top row at height-3, content row at
        // height-2, padding-bottom row at height-1.
        assert_eq!(pos.y, 20 - 2, "cursor should sit on the input box's one content row");
        assert_eq!(pos.x, 2 + 2, "cursor should sit right after \"hi\" (2 for the card's own left padding — no border any more, 2 for the two typed chars)");
    }

    #[test]
    fn the_terminal_cursor_is_hidden_while_an_approval_card_is_pending() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "diff".into(), resolution: None });
        app.pending_approvals.push_back(crate::app::PendingApproval { call_id: "c1".into() });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert!(!terminal.backend().cursor_visible(), "input is blocked while a card is pending — no cursor should show");
    }

    /// Each entry now carries its own padding (a chat bubble's top/bottom
    /// blank fill rows — see `render_entry`'s `UserMessage` arm — or
    /// `render_assistant_text`'s own leading/trailing blank rows), so a
    /// hand-derived "the row right after entry one is the separator" offset
    /// no longer holds the way it used to; this checks the same underlying
    /// property (there's a genuinely blank, unfilled row between the two
    /// entries' own bubbles, not just their own padding) via `find_row`
    /// instead.
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
        let unfilled_row_between = (first_row + 1..second_row).any(|y| buffer[(1, y)].bg != BG_ELEMENT);
        assert!(unfilled_row_between, "there must be a genuinely blank row between the two entries' own bubble fills");
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

        let label_row = find_row(&buffer, "rust");
        assert_eq!(buffer[(0, label_row)].bg, CODE_BG, "the language label row should carry the code block's own dark background");

        // At least two distinct foreground colors within the code line —
        // proof it went through the highlighter, not just plain dim text.
        // Restricted to a narrow column range so unstyled padding cells
        // past the printed text can't manufacture a spurious second color.
        let code_row = find_row(&buffer, "fn main");
        assert_eq!(buffer[(0, code_row)].bg, CODE_BG, "the code line should carry the code block's own dark background, like a real code block in a document");
        let colors: std::collections::HashSet<Color> = (0..20).map(|x| buffer[(x, code_row)].fg).collect();
        assert!(colors.len() > 1, "expected the highlighted code line to use more than one color, got {colors:?}");
    }

    /// Regression test for explicit developer feedback: a ```diff fence
    /// used to get the same generic hand-drawn `╭─ diff`/`╰─` box as any
    /// other language — the ask was to drop that box and the "diff" label
    /// entirely and show a full-width red/green background per line
    /// instead, reusing the same mechanism the Edit approval card already
    /// uses for exactly this.
    #[test]
    fn a_diff_fenced_code_block_renders_full_width_colored_rows_with_no_box_or_label() {
        let mut app = app();
        app.log.push(LogEntry::AssistantText { text: "here's the change:\n```diff\n-old line\n+new line\n```".into() });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

        assert!(!out.contains('╭') && !out.contains('╰'), "a diff fence must not draw the generic code-block box: {out:?}");
        assert!(!out.contains("diff"), "a diff fence must not label itself \"diff\": {out:?}");
        assert!(out.contains("old line") && out.contains("new line"), "the diff content itself must still be shown: {out:?}");

        let removed_row = find_row(&buffer, "old line");
        let added_row = find_row(&buffer, "new line");
        assert_eq!(buffer[(0, removed_row)].bg, DIFF_DEL_BG, "a removed line should carry a full-width red background starting at column 0");
        assert_eq!(buffer[(0, added_row)].bg, DIFF_ADD_BG, "an added line should carry a full-width green background starting at column 0");
    }

    /// Regression test for explicit developer feedback: a diff fence at the
    /// very start of an assistant message (no leading prose) must not lose
    /// its full-width fill to the assistant-speaker marker (`● `) punching
    /// an unstyled gap at column 0 — see `render_assistant_text`'s
    /// `starts_with_diff` guard.
    #[test]
    fn a_diff_fence_as_the_very_first_thing_in_a_message_still_fills_column_zero() {
        let mut app = app();
        app.log.push(LogEntry::AssistantText { text: "```diff\n-old line\n```".into() });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let removed_row = find_row(&buffer, "old line");
        assert_eq!(buffer[(0, removed_row)].bg, DIFF_DEL_BG, "the diff row's full-width fill must reach column 0 even with no leading prose to carry the assistant marker instead");
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

        // find_row rather than hand-derived coordinates — the hero is now
        // vertically centered within the log panel's inner height (a
        // render-time value), so its exact screen row isn't worth
        // recomputing by hand here; what matters is the relative shape.
        let wordmark_row = find_row(&buffer, WORDMARK_ART[0].trim());
        let wordmark_row_text: String = (0..110).map(|x| buffer[(x, wordmark_row)].symbol().to_string()).collect();
        assert!(wordmark_row_text.contains('⣿') || wordmark_row_text.contains('⠀'), "the wordmark's row should still carry hammer art content to its left, not just the wordmark alone");

        let tagline_row = find_row(&buffer, "every strike is yours to call.");
        assert!(tagline_row > wordmark_row, "the tagline should render below the wordmark's first row");
    }

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
        assert_eq!(intro_content(&status).len(), crate::log::INTRO_LINE_COUNT, "ui::intro_content must stay in sync with log::INTRO_LINE_COUNT");
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view — see the sizing comment on
        // user_and_assistant_messages_are_visually_distinct.
        let out = rendered(&mut app(), 110, 40);
        assert!(out.contains("claude-sonnet-5"), "the active model should appear in the welcome banner");
        assert!(out.contains(WORDMARK_ART[3].trim()), "the wordmark should appear in the welcome banner");
        assert!(out.contains(env!("MJOLNIR_GIT_HASH")), "the build's git commit should appear in the welcome banner, distinct from the static crate version");
        assert!(out.contains(MJOLNIR_ART[0]), "the traced Mjolnir art should appear in the welcome banner");
        assert!(out.contains("read:deny") && out.contains("shell:deny") && out.contains("edit:deny"), "the banner should surface the current directory's permission model");
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

        // `find_row`, not a hand-derived offset — the plain message is now
        // a multi-row padded bubble (see `render_entry`'s `UserMessage`
        // arm), so "the command lands 2 rows after the plain message"
        // no longer holds.
        let plain_row = find_row(&buffer, "hi");
        let command_row = find_row(&buffer, "/exit");
        // Column 3 for the plain message: no accent bar or panel border
        // ahead of it any more (the fill starts at column 0), but "hi"'s
        // own padding fill is one uniformly-styled span covering the whole
        // row width, so any column here still reads the fill's background.
        // The slash command's unbarred "> {l}" shape (see `render_entry`)
        // is unchanged, so column 2 (its content) still applies there.
        let plain_cell = &buffer[(3, plain_row)]; // padding past "hi", same fill
        let command_cell = &buffer[(2, command_row)]; // "> /exit"
        assert_eq!(plain_cell.bg, BG_ELEMENT, "a plain user message should carry the subtle background tint");
        assert_ne!(command_cell.bg, BG_ELEMENT, "a slash command must not carry the chat-message background tint");
    }

    #[test]
    fn a_short_user_message_gets_the_background_tint_all_the_way_to_the_right_edge() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        let backend = TestBackend::new(110, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = content_base();
        let far_right_cell = &buffer[(99, row)]; // well past "> hi"
        assert_eq!(far_right_cell.bg, BG_ELEMENT, "the background tint should fill the full row width, not just trail the text");
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

        let top = 0; // the very top of the frame — no header above it any more
        let bottom = height - 1 - input_area_height("") - 1; // above the status line + input box
        for &(x, y) in &[(0, top), (width - 1, top), (0, bottom), (width - 1, bottom)] {
            let cell = &buffer[(x, y)];
            assert_ne!(cell.symbol(), "╭", "the log panel must not draw a border corner");
            assert_eq!(cell.bg, BG_BASE, "the log panel must still be opaque at its edges even without a drawn border");
        }
    }

    #[test]
    fn tool_color_is_stable_for_the_same_name_and_can_differ_for_different_names() {
        assert_eq!(tool_color("shell"), tool_color("shell"), "the same tool name must always get the same color");
        // Not a strict guarantee for every possible pair (a 6-color palette
        // can collide), but true for this project's actual builtin tool
        // names — a regression that flattened `tool_color` to a constant
        // would still be caught here.
        assert_ne!(tool_color("read"), tool_color("shell"));
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
            if let Some(inset) = row.iter().position(|&c| c == 'x') {
                insets.push(inset);
            }
        }
        assert!(insets.len() > 1, "expected the long line to wrap onto multiple rows, got insets {insets:?}");
        assert!(insets.iter().all(|&i| i == insets[0]), "every wrapped row must share the same left inset, got {insets:?}");
    }
}
