use mjolnir_permissions::PromptPayload;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{cursor_line_col, App, DecisionOption, GrantSummary, PatternScope, PendingFront, PermState, RunningTool, StatusInfo};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};
use crate::palette::{self, Palette};

/// `--spinner-frames` from the Mjolnir Design System's `tokens/motion.css`
/// — a quarter-block cycling at roughly 100ms per frame, replacing the
/// earlier Braille-dot spinner that traced the (now-removed) hammer mark's
/// own glyph family. `App::tick` still advances this every 120ms
/// (`run.rs`) — close enough to the token's ~100ms that a redraw-driven
/// tick (not a dedicated timer) reads as continuous motion.
const SPINNER_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let pal = app.theme.palette();
    let area = frame.area();
    // Opaque canvas, drawn first and under everything else — without this,
    // every gap between panels (margins, the status-line row) renders as
    // the terminal's own background, which is exactly the "transparent app"
    // look the redesign is replacing. See `Palette::ground`'s doc comment
    // for the tier this belongs to.
    frame.render_widget(Block::default().style(Style::default().bg(pal.ground)), area);
    let input_height = input_area_height(&app.input);
    // Five bands: the body (conversation log), a 1-row blank spacer, a
    // 1-row status line (identity/activity — see `draw_status_line`), the
    // decision panel (Edit approval / permission prompt — see
    // `decision_panel_lines`, zero-height and invisible when nothing is
    // pending), then the input box. The old separate 1-row header and
    // 1-row footer are gone — per explicit developer feedback against a
    // real screenshot, a header full of permission chips at the top and a
    // footer full of half-dead keybinding hints at the bottom read as two
    // disconnected, mostly-noise bars; one status line, positioned right
    // above the input where the developer's eye already is while typing,
    // replaces both.
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
    //
    // `panel_lines` (and therefore `panel_height`) is computed once, up
    // front, the same discipline `log_inner`'s width/height follow below —
    // there must never be a second, independently-derived height for what
    // the panel actually renders, or the two can drift the way `log_inner`'s
    // own doc comment describes two real historical bugs happening.
    //
    // `panel_height` counts *wrapped* rows (`panel_row_count`, the same
    // `Paragraph::line_count` technique `log_row_count` already uses — see
    // its doc comment), not `panel_lines.len()` — a permission prompt's
    // title/keys can easily be wider than the frame (an arbitrarily long
    // shell command in `PromptPayload::Tool`'s `target`, say), and this
    // panel's own `Paragraph` below wraps rather than truncates, same as
    // the log panel's own render path.
    let panel_lines = decision_panel_lines(app, area.width, area.height);
    let panel_height = panel_row_count(&panel_lines, area.width) as u16;

    // Top bar reintroduced per the Mjolnir Design System's reference
    // screens — every one of the five (session/permission/review/commands/
    // first-run) opens with a persistent 3-row identity bar plus a 1-row
    // rule below it (`tokens/cells.css`'s `--bar-top-h`, `TopBar.jsx`'s
    // `borderBottom`). This is a structural addition, not a bare reskin —
    // the prior visual-redesign pass had folded identity into a single
    // status line right above the input; the source design puts identity
    // back at the top and leaves that line for live turn activity only
    // (see `draw_top_bar`/`draw_status_line`'s own doc comments for what
    // each now owns).
    // `BottomBar.jsx`, exactly as the reference lays it out: a `line` edge,
    // then five rows — blank, composer, blank, status, blank. The status
    // line sits *below* the composer, not above it; an earlier pass had the
    // two swapped (reported directly: "the status line is above the text
    // field input, but ... it is below in the designs").
    //
    // While a decision is pending the panel takes those rows instead —
    // "The panel takes the composer's rows as well as its own, because
    // input is disabled while a permission is pending: there is nothing to
    // type into, so the prompt row is not drawn at all." mjolnir already
    // blocks input then, so nothing is lost by not drawing it.
    let pending = panel_height > 0;
    let bottom_bar_height = if pending { panel_height } else { 1 + input_height + 1 + 1 + 1 };
    let [top_bar_area, top_rule_area, body_area, bottom_rule_area, bottom_bar_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(bottom_bar_height),
    ])
    .areas(area);
    let log_area = body_area;

    // Both bars are separated from the body by a real structural border in
    // `line` (`TopBar.jsx`'s `border-bottom` and `BottomBar.jsx`'s
    // `border-top`, both `1px solid var(--tui-line)`) — *not* the more
    // muted `rule`, which is only ever a freestanding separator *within*
    // content (a turn break, the rule above the options list).
    draw_top_bar(frame, top_bar_area, app);
    let edge = |fg: Color, bg: Color| {
        Paragraph::new(Line::from(Span::styled("─".repeat(area.width as usize), Style::default().fg(fg)))).block(Block::new().style(Style::default().bg(bg)))
    };
    frame.render_widget(edge(pal.line, pal.ground), top_rule_area);
    // While a decision is pending this row is the *panel's* own top border,
    // and the reference gives that one `modal_line`, not `line` — a heavier
    // edge for a surface that has taken the composer's place. Drawing it
    // here rather than inside `panel_band` is what keeps it to a single
    // rule: the bottom bar's edge and the panel's border are the same row,
    // not two stacked ones.
    let (edge_fg, edge_bg) = if pending { (pal.modal_line, pal.bar) } else { (pal.line, pal.ground) };
    frame.render_widget(edge(edge_fg, edge_bg), bottom_rule_area);

    // No drawn border and no title — the reference screenshot that prompted
    // this pass shows no box anywhere around the conversation, just filled
    // cards floating directly on the frame background (see
    // `palette::BG_BASE`'s doc comment). A right-aligned "live"/"scrolled"
    // title badge used to live here (modeled on posting's
    // `border-title-status`) but per explicit developer feedback it was
    // meaningless noise in the corner of the screen — removed outright, not
    // replaced, so `log_block` is now a plain background fill with nothing
    // reserving a title row.
    let log_block = Block::new().style(Style::default().bg(pal.ground));
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

    if pending {
        // The transcript recedes while a decision is open — the reference
        // puts the whole conversation column at `opacity:.35` in both of its
        // panel scenes, so the panel reads as the one live surface rather
        // than as another card competing with the history above it. Applied
        // as a post-pass over the already-drawn cells rather than by
        // threading a second faded palette through every `render_entry` arm:
        // the effect is uniform over the region by definition, so
        // compositing it once here can't drift from the panel's own colors
        // the way a parallel palette would.
        fade_area(frame, log_area, palette::PANEL_TRANSCRIPT_OPACITY);
        draw_decision_panel(frame, bottom_bar_area, panel_lines);
    } else {
        // blank / composer / blank / status / blank — `BottomBar.jsx`'s own
        // five rows, on its own raised ground.
        frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), bottom_bar_area);
        let [_pad_top, composer_area, _pad_mid, status_area, _pad_bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(input_height),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(bottom_bar_area);
        draw_input(frame, composer_area, app);
        draw_status_line(frame, status_area, app);
    }
}

/// Persistent 3-row identity bar — `TopBar.jsx`/`readme.md`'s "Session"
/// screen section. Left: the harness name alone, primary text, no glyph —
/// per the design system's own revision log ("the top bar carries no
/// accent mark: the name is the brand, and a pip there indicated
/// nothing"). Right: the model name and the running build version — the
/// closest real facts Mjolnir has to the reference's `model · gauge ·
/// cost` group; a context-window gauge and a per-session cost aren't
/// tracked anywhere in `StatusInfo`, so neither is fabricated here (see
/// this crate's `App::StatusInfo` — `model_name`/`turn`/`step`/
/// `running_tools`/permission state only). Facts inside a group sit on the
/// `·`-separated rhythm the revision log settled on, not the wider 6-cell
/// gap that only ever separates *unrelated* groups.
fn draw_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar)), area);
    let content_row = Rect { y: area.y + 1, height: 1, ..area };
    let [left_area, right_area] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(content_row);

    // "mjolnir      ~/src/gateway" — six cells part the name from the
    // directory, the gap the reference reserves for two *unrelated* groups
    // (facts within one group ride the tighter ` · ` rhythm instead — see
    // the right group below). The working directory is real, always-
    // available process state (`std::env::current_dir`), not fabricated;
    // a git branch/dirty marker would need a new capability (shelling out
    // to git at runtime) this pass doesn't add, so the identity group stops
    // at cwd rather than showing a branch this crate has no way to know.
    let mut left_spans = vec![Span::styled("mjolnir", Style::default().fg(pal.text))];
    if let Some(cwd) = current_dir_display() {
        left_spans.push(Span::raw("      "));
        left_spans.push(Span::styled(cwd, Style::default().fg(pal.quiet)));
    }
    let left = Paragraph::new(Line::from(left_spans)).block(Block::new().padding(Padding::left(3)));
    frame.render_widget(left, left_area);

    // Three tokens, not one: the reference's right group is `quiet` for the
    // model, `dim` for the `·` separators, and `text` for the last fact in
    // the group (`$0.42` there, the build version here) — rendering the
    // whole group in a single `label` flattened a deliberate three-step
    // hierarchy into one tone. One space each side of the `·`, not two:
    // these are facts *within* one group, and the wider six-cell gap is
    // reserved for parting groups from each other.
    let right = Paragraph::new(Line::from(vec![
        Span::styled(app.status.model_name.clone(), Style::default().fg(pal.quiet)),
        Span::styled(" · ", Style::default().fg(pal.dim)),
        Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(pal.text)),
    ]))
    .alignment(ratatui::layout::Alignment::Right)
    .block(Block::new().padding(Padding::right(3)));
    frame.render_widget(right, right_area);
}

/// The session's working directory, `~`-shortened like a shell prompt —
/// `TopBar.jsx`'s own left-group fact (`~/src/gateway`). `None` only if the
/// process's cwd genuinely can't be read — not worth a placeholder for a
/// case this rare.
fn current_dir_display() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Some(cwd.display().to_string());
    };
    if cwd == home {
        return Some("~".to_string());
    }
    match cwd.strip_prefix(&home) {
        Ok(rest) if !rest.as_os_str().is_empty() => Some(format!("~/{}", rest.display())),
        _ => Some(cwd.display().to_string()),
    }
}

/// Maximum rows the decision panel (see `decision_panel_lines`) is allowed
/// to claim, derived from the frame's total height rather than fixed —
/// reserves room for at least one row of the conversation log plus the
/// spacer/status-line/input bands below it, so an unusually large diff (a
/// new file, a big added block — see `clamp_panel`'s own doc comment on why
/// the existing context-collapsing alone doesn't bound this) can never push
/// the rest of the UI off-frame the way an unbounded `Constraint::Length`
/// could. Floors at 6 (enough for a short title/keys/padding-only panel)
/// even on a terminal too short to honor the reservation in full — a
/// degenerate case, not one worth failing gracefully out of.
fn panel_max_height(frame_height: u16) -> usize {
    // While a decision is pending the panel *is* the bottom bar — it takes
    // the composer's and status line's rows rather than stacking above them
    // (see `draw`), so those aren't reserved here any more.
    const RESERVED_FOR_REST_OF_UI: u16 = 3 /* top bar */ + 1 /* top bar rule */ + 1 /* one row of log */ + 1 /* bottom bar edge */;
    // `decision_panel_lines` adds `panel_band`'s 2-row chrome and the
    // footer's 3-row chrome (padding + rule + hint row) *outside* the
    // budget this bounds (see that function's own doc comment on why) —
    // reserved here too, so the combined total (band + clamped body +
    // footer) still fits the same overall budget, not just the clamped
    // body alone.
    const PANEL_CHROME: u16 = 1 /* panel_band */ + 3 /* footer padding + rule + hint */;
    (frame_height.saturating_sub(RESERVED_FOR_REST_OF_UI + PANEL_CHROME) as usize).max(6)
}

/// Wrapped-row count of `lines` at `width` — same `Paragraph::line_count`
/// technique as `log_row_count`, applied to the decision panel instead of
/// the conversation log, so the two can never disagree about how tall a
/// wrapping line actually renders.
fn panel_row_count(lines: &[Line<'static>], width: u16) -> usize {
    Paragraph::new(Text::from(lines.to_vec())).wrap(Wrap { trim: false }).line_count(width)
}

/// Composites every already-drawn cell in `area` toward its own background
/// at `alpha`, the way CSS `opacity` would — see
/// `palette::PANEL_TRANSCRIPT_OPACITY` for why the transcript needs it.
/// Each cell fades toward *its own* `bg`, not one shared ground, so a cell
/// sitting on a card or a diff band recedes against that surface rather than
/// against the frame behind it. Backgrounds themselves are left alone: they
/// are the surfaces being faded onto, and dissolving them too would erase
/// the card edges the fade is supposed to preserve.
fn fade_area(frame: &mut Frame, area: Rect, alpha: f32) {
    let buf = frame.buffer_mut();
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = palette::fade(cell.fg, cell.bg, alpha);
        }
    }
}

fn draw_decision_panel(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if area.height == 0 {
        return;
    }
    // `Wrap { trim: false }` — matches `panel_row_count`'s own wrap mode, so
    // `draw`'s precomputed `panel_height` and what actually renders here can
    // never desync (see its doc comment).
    frame.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }), area);
}

fn input_area_height(input: &str) -> u16 {
    // Just the draft's own rows — the blank rows above and below it are the
    // bottom bar's (`BottomBar.jsx`'s blank/composer/blank/status/blank),
    // not the composer's own padding, so they're laid out once in `draw`
    // rather than baked in here.
    input.matches('\n').count() as u16 + 1
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
    let pal = app.theme.palette();
    if app.log.is_empty() {
        return hero_lines(&app.status, height, pal);
    }
    let mut lines: Vec<Line> = Vec::new();
    for entry in app.log.iter() {
        // An entry can render to nothing at all now (a routine `TurnEnded`
        // — see its arm in `render_entry` — is folded into the status
        // line's own activity indicator instead of getting its own log
        // row), so the blank separator is keyed on whether anything has
        // actually been pushed yet, not on the entry's index — otherwise a
        // silent entry would still claim a blank row for itself.
        let rendered = render_entry(entry, width, pal);
        if rendered.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            // A fresh `UserMessage`/`AssistantText` starts a new
            // conversational turn and gets a real `rule` row between it and
            // whatever came before — `readme.md`: "turns are parted by a
            // flat rule one step more muted than the frame's borders," not
            // just blank space. Tool activity/retry/error/notice entries
            // continue the current turn rather than starting a new one, so
            // they only get the plain blank row a turn's own internal
            // groups get in the reference.
            if matches!(entry, LogEntry::UserMessage { .. } | LogEntry::AssistantText { .. }) {
                lines.push(Line::default());
                lines.push(Line::from(Span::styled("─".repeat(width as usize), Style::default().fg(pal.rule))));
                lines.push(Line::default());
            } else {
                lines.push(Line::default());
            }
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
    let pal = app.theme.palette();
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
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).style(Style::default().fg(pal.line));
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

/// The welcome hero shown in place of the conversation log while it's empty
/// — replaces the former hand-traced Braille hammer/FIGlet wordmark
/// mascot outright, per the Mjolnir Design System's own explicit rule:
/// "No logo. No mark was supplied and none was invented... every mark is a
/// Unicode box-drawing or block character," and its Assets section is
/// blunter still — "None. No images, no icons." A traced photo rendered as
/// Braille dots is exactly the kind of image-as-logo the source rules out;
/// this isn't a stylistic trim, it's bringing the one element that never
/// matched the system's own stated identity model into line with it. The
/// harness's identity now lives only in the top bar (`draw_top_bar`,
/// plain "mjolnir" text, no glyph — the design system's revision log:
/// "the top bar carries no accent mark: the name is the brand"); this hero
/// is just the tagline plus the same stat facts the old banner's info
/// column carried, laid out as `MetaRow`-style label/value pairs.
fn access_spans(label: &'static str, state: PermState, pal: &Palette) -> Vec<Span<'static>> {
    // No filled chip — the design system's own rule is that the accent is
    // "a mark or a line, never a filled field," and none of its own
    // components (`MetaRow`, `OptionRow`) use a background-filled badge for
    // a state word; add/del (green/red) plain text already reads as
    // allow/deny at a glance, the same "state at a glance" job the old chip
    // did.
    let (word, word_fg) = match state {
        PermState::Allowed => ("allow", pal.add),
        PermState::Denied => ("deny", pal.del),
    };
    vec![Span::styled(format!("{label}:"), Style::default().fg(pal.label)), Span::styled(format!("{word} "), Style::default().fg(word_fg))]
}

/// The welcome hero's content: a sentence of body prose, then `model` /
/// `version` / `commit` / `access` facts on the transcript's own 12-cell
/// label-column convention (`Turn.jsx`'s label gutter, echoed here since
/// this hero has no art to sit beside any more).
fn intro_content(status: &StatusInfo, pal: &Palette) -> Vec<Line<'static>> {
    let tagline_style = Style::default().fg(pal.body);
    let stat_label = Style::default().fg(pal.label);
    let stat_value = Style::default().fg(pal.value);
    const LEFT_MARGIN: &str = "   ";

    let mut content: Vec<Line<'static>> = Vec::with_capacity(8);
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("every strike is yours to call. nothing moves without you.", tagline_style)]));
    content.push(Line::default());
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("model    ", stat_label), Span::styled(status.model_name.clone(), stat_value)]));
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("version  ", stat_label), Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), stat_value)]));
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("commit   ", stat_label), Span::styled(env!("MJOLNIR_GIT_HASH"), stat_value)]));
    let mut access = vec![Span::raw(LEFT_MARGIN), Span::styled("access   ", stat_label)];
    access.extend(access_spans("read", status.read, pal));
    access.push(Span::raw("  "));
    access.extend(access_spans("shell", status.shell, pal));
    access.push(Span::raw("  "));
    access.extend(access_spans("edit", status.edit, pal));
    content.push(Line::from(access));
    content
}

/// Vertically centers `intro_content` within the log panel's inner
/// `height`. On a terminal short enough that the content doesn't fit,
/// `pad_top` saturates to 0 and the content simply starts at the top and
/// scrolls like any other tall log content would.
fn hero_lines(status: &StatusInfo, height: u16, pal: &Palette) -> Vec<Line<'static>> {
    let content = intro_content(status, pal);
    let pad_top = (height as usize).saturating_sub(content.len()) / 2;
    let mut lines = Vec::with_capacity(pad_top + content.len());
    lines.extend(std::iter::repeat_with(Line::default).take(pad_top));
    lines.extend(content);
    lines
}

/// The design system's grid, in cells (`tokens/cells.css`, confirmed
/// against every measurement in the reference frames themselves):
///
/// * `MARGIN_X` — `--margin-x: 27px` = 3 cells. *Every* content row in a
///   frame carries this left/right margin: transcript turns, the top bar,
///   the status line, the decision panel's own text rows, its footer. The
///   one deliberate exception is a selectable option row, which the
///   reference renders flush to the frame's left edge so its `▌` mark sits
///   in cell 0 (see `render_decision_options`).
/// * `LABEL_COL_WIDTH` — `--label-col: 108px` = 12 cells, the speaker /
///   meta-label column.
/// * `LABEL_GUTTER` — `--label-gutter: 18px` = 2 cells.
/// * `CONTENT_INDENT` — `--body-col: 153px` = 17 cells from the frame
///   edge, which is exactly `MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER`;
///   body text in a turn always starts here.
///
/// An earlier pass used 10/2 with no margin at all, so every transcript row
/// started 5 cells left of where the grid puts it — reported directly as
/// "the chat rows themselves appear misaligned and do not follow the
/// cell/grid system."
const MARGIN_X: usize = 3;
const LABEL_COL_WIDTH: usize = 12;
const LABEL_GUTTER: usize = 2;
const CONTENT_INDENT: usize = MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER;

/// Cells a turn's body column actually has to work with at a given frame
/// width: everything left of `CONTENT_INDENT` belongs to the margin and the
/// label column, and `MARGIN_X` more is held back on the right — the
/// reference's turn container is `padding: 0 27px`, a margin on *both*
/// sides, so prose wraps and filled blocks (code fences, diff boxes) end
/// one margin short of the frame's edge rather than running into it.
fn body_column_width(width: u16) -> usize {
    (width as usize).saturating_sub(CONTENT_INDENT).saturating_sub(MARGIN_X)
}

/// Lays `lines` out under `Turn.jsx`'s label column: `label` (if any) sits
/// on the first row only, left-padded to `CONTENT_INDENT`; every other row
/// — the first row too, when `label` is `None` — gets a blank
/// `CONTENT_INDENT` prefix instead, so a tool-activity/retry/error/notice
/// entry (which continues the previous turn rather than starting a new one;
/// see `render_entry`) lines its content up under whichever speaker's turn
/// it belongs to without repeating that speaker's name.
fn with_label_column(lines: Vec<Line<'static>>, label: Option<(&str, Color)>) -> Vec<Line<'static>> {
    let blank = " ".repeat(CONTENT_INDENT);
    let margin = " ".repeat(MARGIN_X);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let mut spans = Vec::with_capacity(line.spans.len() + 2);
            match (i, label) {
                // The label starts at the 3-cell margin — cell 3, not cell
                // 0 — and the body column still lands on cell 17 regardless
                // of how long the label itself is.
                (0, Some((text, color))) => {
                    let pad = (LABEL_COL_WIDTH + LABEL_GUTTER).saturating_sub(text.width());
                    spans.push(Span::raw(margin.clone()));
                    spans.push(Span::styled(text.to_string(), Style::default().fg(color)));
                    spans.push(Span::raw(" ".repeat(pad)));
                }
                _ => spans.push(Span::raw(blank.clone())),
            }
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Right-flushes `right` against `left` within `width` columns — `ToolLine.jsx`'s
/// own shape (glyph/name/target on the left, a result summary flush to the
/// right edge). Falls back to a single-space gap rather than clipping when
/// the two sides don't leave room to space apart properly.
fn justified_line(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let gap = width.saturating_sub(left_w).saturating_sub(right_w).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}

fn render_entry(entry: &LogEntry, width: u16, pal: &Palette) -> Vec<Line<'static>> {
    match entry {
        // `Turn.jsx`: the `you` label in `speaker-you` (accent-toned), the
        // `harness` label in `speaker-agent` (neutral) — content in `text`
        // (primary) for a `you` turn, `body` for the agent's (see
        // `render_assistant_text`). No filled background any more: the
        // design system's own components never fill a chat message's
        // background — flat colored text on the panel ground is the whole
        // treatment; a slash command (directed at the harness, not the
        // model — see `is_command`'s doc comment) skips the speaker label
        // too, since it isn't conversational content.
        LogEntry::UserMessage { text } => {
            if is_command(text) {
                return text.lines().map(|l| Line::from(Span::styled(format!("> {l}"), Style::default().fg(pal.dim)))).collect();
            }
            let inner_width = body_column_width(width);
            let style = Style::default().fg(pal.text);
            let content: Vec<Line<'static>> =
                text.lines().flat_map(|l| wrap_prose_line(Line::from(Span::styled(l.to_string(), style)), inner_width)).collect();
            with_label_column(content, Some(("you", pal.speaker_you)))
        }
        LogEntry::AssistantText { text } => with_label_column(render_assistant_text(text, width, pal), Some(("harness", pal.speaker_agent))),
        // `ToolLine.jsx`: a status glyph, the tool name, a right-flush
        // result summary. mjolnir's `ToolActivityEntry` carries no separate
        // target path distinct from the tool's own name (unlike the
        // reference's `read src/gateway/mod.rs`), so the call id stands in
        // for it, parenthesized, the same information this crate showed
        // before this pass. The design system's glyph table has no distinct
        // "failed" mark (`readme.md`'s Iconography table: only `●` done /
        // `◐` running / `○` pending / `✔` accepted — "if a mark is needed
        // and it is not in that table, do not draw one") — an error keeps
        // the `●` done glyph but in `del` (red) instead of `add`, the same
        // "colour carries the meaning" rule the rest of this system leans
        // on throughout. No label of its own — a tool-activity group
        // continues whichever turn's content column it renders under
        // (`build_log_lines` never puts one between a turn and its own
        // tool calls).
        LogEntry::ToolActivity { calls, .. } => {
            let inner_width = body_column_width(width);
            let content: Vec<Line<'static>> = calls
                .iter()
                .map(|c| {
                    let target = if c.name.is_empty() { c.call_id.clone() } else { format!("{} ({})", c.name, c.call_id) };
                    // A *running* call's name is `accent_text` in the
                    // reference ("`◐  bash  cargo test…`" — the live row is
                    // the one the eye should land on), a finished one's is
                    // ordinary `body`.
                    let (glyph, text_color, summary) = match &c.status {
                        ToolActivityStatus::Running => (Span::styled("◐ ", Style::default().fg(pal.glyph_running)), pal.accent_text, None),
                        ToolActivityStatus::Completed { is_error: false, summary } => (Span::styled("● ", Style::default().fg(pal.glyph_done)), pal.body, Some(summary.clone())),
                        ToolActivityStatus::Completed { is_error: true, summary } => (Span::styled("● ", Style::default().fg(pal.del)), pal.body, Some(summary.clone())),
                    };
                    let left = vec![glyph, Span::styled(target, Style::default().fg(text_color))];
                    let right = match summary {
                        Some(s) => vec![Span::styled(s, Style::default().fg(pal.dim))],
                        None => vec![],
                    };
                    justified_line(left, right, inner_width)
                })
                .collect();
            with_label_column(content, None)
        }
        // No glyph and no dedicated "warning" color — the design system has
        // neither (its palette is ground/bar/text/body/code/context/value/
        // label/dim/quiet/mark/band/accent-text/speaker/gauge/glyph/hunk/
        // modal/diff, nothing named for a transient retry). `label` keeps it
        // a quiet, informational fact rather than inventing a color outside
        // that fixed vocabulary.
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            let line = Line::from(vec![
                Span::styled("retry ", Style::default().fg(pal.label).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · attempt {} · {status}: {}", info.provider, info.attempt, info.message), Style::default().fg(pal.dim)),
            ]);
            with_label_column(vec![line], None)
        }
        // While pending (`resolution: None`), this renders nothing at all
        // here — the decision panel (`decision_panel_lines`, a fixed
        // full-width band above the input, driven by `App::
        // pending_approvals`/`pending_prompts`) is the only place an
        // unresolved request is interactive, per the design system's own
        // "Permission prompt" screen. Once resolved, it still renders here
        // as before — the log remains the permanent record. Not laid out
        // under the label column — a decision card is its own full-width
        // panel-styled block, not conversational turn content.
        LogEntry::ApprovalCard { diff, resolution: Some(approved), .. } => render_approval_card(diff, Some(*approved), Vec::new(), pal, width),
        LogEntry::ApprovalCard { resolution: None, .. } => Vec::new(),
        LogEntry::PermissionPrompt { payload, resolution: Some(r), .. } => render_prompt_card(payload, Some(r.as_str()), Vec::new(), pal, width),
        LogEntry::PermissionPrompt { resolution: None, .. } => Vec::new(),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            let line = match reason {
                TurnEndReasonKind::EndTurn => return vec![],
                TurnEndReasonKind::Cancelled => Line::from(Span::styled("— turn cancelled —", Style::default().fg(pal.dim))),
                TurnEndReasonKind::Error(message) => Line::from(Span::styled(format!("— turn ended in error: {message} —"), Style::default().fg(pal.dim))),
            };
            with_label_column(vec![line], None)
        }
        LogEntry::Error { message } => with_label_column(vec![Line::from(vec![
            Span::styled("error: ", Style::default().fg(pal.del).add_modifier(Modifier::BOLD)),
            Span::styled(message.clone(), Style::default().fg(pal.del)),
        ])], None),
        LogEntry::Notice { message } => with_label_column(vec![Line::from(vec![
            Span::styled("notice: ", Style::default().fg(pal.quiet)),
            Span::styled(message.clone(), Style::default().fg(pal.dim)),
        ])], None),
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

/// Builds the `harness` turn's content — never indented itself; the caller
/// (`render_entry`) lays the whole result out under the label column via
/// `with_label_column`, so every box built here (diff, code block) sizes
/// itself against `width - CONTENT_INDENT`, not the full panel width, or it
/// would overflow past the right edge once that column is added back.
fn render_assistant_text(text: &str, width: u16, pal: &Palette) -> Vec<Line<'static>> {
    let inner_width = body_column_width(width) as u16;
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in split_code_fences(text) {
        match segment {
            // No fill of its own — `Prose.jsx` is plain colored text on the
            // panel ground, no background.
            Segment::Prose(s) => {
                lines.extend(s.lines().flat_map(|l| wrap_prose_line(render_markdown_line(l, pal), inner_width as usize)));
            }
            // A fenced ```diff block gets the same full-width red/green
            // per-line treatment (with a line-number gutter — see
            // `number_diff_lines`) as the Edit approval card
            // (`render_diff_line`/`parse_diff_body`) instead of the generic
            // code-block box below — the card mechanism already exists
            // precisely for "show a diff" (`InlineDiff.jsx`'s own job), so
            // this reuses it rather than inventing a second diff
            // presentation.
            Segment::Code { lang, body } if lang.eq_ignore_ascii_case("diff") => {
                let (_, diff_body) = parse_diff_body(&body);
                lines.extend(boxed_diff_lines(&number_diff_lines(diff_body), Inset::FLUSH, pal, inner_width));
            }
            // A real filled code-block box, with a dim language label
            // instead of the fence's own literal ` ``` ` markers, on
            // `diff_box` — the design system's one nested-quote surface,
            // already carrying the inline diff for the same reason (a
            // quoted block inside prose), and re-tinted light by its own
            // `.tui-light` scope. This used to be a fixed-dark constant
            // because `highlight::highlight_lines` was pinned to syntect's
            // `base16-ocean.dark` in both app themes, which would have been
            // illegible on a light field; it now picks the matching half of
            // the `base16-ocean` pair from the app theme, so the surface is
            // free to follow the palette like every other one.
            // `filled_line` gives
            // every row (label included) the box's own left/right padding
            // and a `card_padding_line` spacer under the label and at the
            // bottom gives it top/bottom padding too, same as every other
            // filled box in the log.
            Segment::Code { lang, body } => {
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.extend(card_line(&label, Style::default().fg(pal.label).bg(pal.diff_box), pal, inner_width));
                lines.push(card_padding_line(pal.diff_box, pal, inner_width));
                for code_line in highlight::highlight_lines(&lang, &body, pal.theme) {
                    let spans: Vec<Span<'static>> = code_line.into_iter().map(|s| Span::styled(s.content, s.style.bg(pal.diff_box))).collect();
                    lines.extend(filled_line(spans, pal.diff_box, inner_width));
                }
                lines.push(card_padding_line(pal.diff_box, pal, inner_width));
            }
        }
    }
    lines
}

/// Word-wraps one logical prose `Line` to `max_width` display columns,
/// breaking only at whitespace and preserving each span's style across a
/// break, into however many `Line`s it takes.
///
/// This exists instead of leaning on the log paragraph's own
/// `Wrap { trim: false }` (`draw_log`) because `Wrap` has no concept of the
/// label-column inset `with_label_column` applies afterward: it treats one
/// logical `Line`'s spans as a single continuous run of styled graphemes,
/// so a wrapped continuation row it produced would come out flush against
/// the panel edge instead of under the rest of the turn's content (reported
/// as: "the first line of text is correctly in line, but when the text
/// wraps onto a second line, it doesn't respect the padding"). Doing the
/// wrap here means every row this returns is already ≤ `max_width` columns
/// before `with_label_column` insets it, so `Wrap` never has to touch it —
/// the wrapping happens once, not twice.
///
/// Doesn't hang-indent list/blockquote markers under wrapped continuation
/// text (a wrapped `• ` bullet's second row starts at the same column every
/// other prose row does, not under the first row's text) — only the flat
/// inset every prose row gets from `with_label_column` regardless of what
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
fn render_markdown_line(line: &str, pal: &Palette) -> Line<'static> {
    let base = Style::default().fg(pal.body);
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];

    if is_hr(trimmed_start) {
        return Line::from(Span::styled("─".repeat(20), Style::default().fg(pal.dim)));
    }
    if let Some((level, rest)) = parse_heading(trimmed_start) {
        let style = if level <= 2 { base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { base.add_modifier(Modifier::BOLD) };
        return Line::from(parse_inline(rest, style, pal));
    }
    if let Some(rest) = trimmed_start.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let mut spans = vec![Span::styled(format!("{indent}▎ "), Style::default().fg(pal.dim))];
        spans.extend(parse_inline(rest, base.add_modifier(Modifier::ITALIC), pal));
        return Line::from(spans);
    }
    if let Some(rest) = parse_bullet(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}• "), base)];
        spans.extend(parse_inline(rest, base, pal));
        return Line::from(spans);
    }
    if let Some((marker, rest)) = parse_ordered(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}{marker} "), base)];
        spans.extend(parse_inline(rest, base, pal));
        return Line::from(spans);
    }
    Line::from(parse_inline(line, base, pal))
}

/// Recursive-descent inline pass: `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, `~~strike~~`, `[text](url)`. Delimiters nest via recursion
/// (e.g. `**bold *and italic***`) rather than a flat token stream, which
/// keeps this a single small function instead of a tokenizer + AST.
fn parse_inline(text: &str, base: Style, pal: &Palette) -> Vec<Span<'static>> {
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
                spans.push(Span::styled(stripped[..end].to_string(), Style::default().fg(pal.code)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::BOLD), pal));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("~~") {
            if let Some(end) = stripped.find("~~") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::CROSSED_OUT), pal));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if rest.starts_with('*') || rest.starts_with('_') {
            let delim = &rest[..1];
            let stripped = &rest[1..];
            if let Some(end) = stripped.find(delim) {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::ITALIC), pal));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((label, url, remainder)) = parse_link(rest) {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(label.to_string(), base.add_modifier(Modifier::UNDERLINED)));
                if !url.is_empty() && url != label {
                    spans.push(Span::styled(format!(" ({url})"), Style::default().fg(pal.dim)));
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
/// `match rest` arms in `slash.rs`, `/`-prefixed here to match whole-word
/// input tokens directly) — duplicated for the same reason as `is_command`
/// above: tui can't depend on cli. Purely a hint for
/// `highlight_command_tokens` below; keep in sync by hand if slash.rs's
/// arms change — `/theme` added 2026-09-02 alongside that command, caught
/// only by remembering this comment's own instruction, not by a compiler
/// or test forcing the two files to agree (see
/// `command_word_is_dimmed_live_even_mid_message`'s sibling test for
/// `/theme` specifically, added the same day as a direct guard against
/// this exact drift happening silently next time).
const KNOWN_COMMAND_WORDS: [&str; 5] = ["/help", "/clear", "/exit", "/reload-config", "/theme"];

/// Dims every word in `line` that exactly matches a known command, no
/// matter where it falls — per explicit developer direction, this is a
/// cosmetic hint only and deliberately does *not* mirror `is_command`'s
/// "whole message must start with `/`" rule: a real slash command only
/// fires when it's the very first thing in the message (`is_command`,
/// enforced for real in `cli::slash::intercept`), so `/exit` typed
/// mid-sentence never actually gets intercepted — it's still worth
/// flagging live so the developer notices they typed a recognized command
/// word, wherever it landed.
fn highlight_command_tokens(line: &str, pal: &Palette) -> Line<'static> {
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
        // An ordinary word must carry an explicit `BRIGHT` fg, not bare
        // `Style::default()` (terminal-default foreground) — this is the one
        // place in the whole log/input rendering path that left a
        // content-bearing span without one (every other span in this file
        // sets an explicit palette color; see `palette::BG_BASE`'s doc
        // comment on why the app paints its own opaque background
        // everywhere). `draw_input`'s block always fills `BG_INPUT`, a fixed
        // dark navy, regardless of the developer's own terminal theme — on a
        // light-mode terminal profile, "terminal-default foreground" is
        // typically dark (meant to sit on a light background), so an
        // ordinary typed word rendered dark-on-our-own-dark-navy, unreadable
        // while typing. Reported directly: "text is dark on light mode and
        // it clashes with the dark background."
        let style = if KNOWN_COMMAND_WORDS.contains(&word) { Style::default().fg(pal.dim) } else { Style::default().fg(pal.text) };
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

/// Horizontal inset for a filled panel/card content row — the grid's own
/// `MARGIN_X` (3 cells, `--margin-x: 27px`), which is what every
/// `padding: 0 27px` row in the reference frames resolves to. Was 1 cell
/// before the grid audit, which left every decision-panel row two cells
/// left of where the design puts it.
const BOX_PAD_H: usize = MARGIN_X;

/// The shared padding primitive every filled box in the log builds its rows
/// from: `BOX_PAD_H` columns of `bg`, then `spans`, then `bg`-filled columns
/// out to `width` — giving left+right padding and a full-width fill in one
/// step. `spans` must already carry whatever `bg` they should show against
/// (this only pads around them, it doesn't recolor them), so a caller
/// mixing a semantic tint (e.g. `DIFF_ADD_BG`) into an otherwise-`bg`
/// row still reads correctly.
///
/// Returns more than one `Line` when `spans` is too wide for `width` —
/// word-wrapped via `wrap_prose_line` (the same primitive assistant prose
/// already used), with the `BOX_PAD_H` inset and `bg` fill applied to *every*
/// resulting row here, not just assumed to happen once downstream. Before
/// this, a card/diff/code-block row wider than its panel relied on the
/// caller's own `Paragraph::wrap` (`draw_log`/`draw_decision_panel`) to
/// split it — but `Wrap` treats one `Line`'s spans as a single flat run of
/// graphemes with no idea this function had already inset/filled it, so it
/// only ever produced the inset/fill on whichever row the wrap decision
/// happened to land the start of the content on (normally the first),
/// leaving every wrapped continuation row flush against the edge with no
/// background at all. Doing the wrap here, before any padding is added,
/// means every row this function returns is already ≤ `width` and already
/// fully padded/filled on its own — `Wrap` downstream never has to split
/// anything this function produces, the same discipline `wrap_prose_line`
/// itself already established for prose.
fn filled_line(spans: Vec<Span<'static>>, bg: Color, width: u16) -> Vec<Line<'static>> {
    // `padding: 0 27px` in the reference is a margin on *both* sides, so
    // content wraps at `width - 2 * BOX_PAD_H` even though the filled row
    // itself still runs the full width (it's the card's own surface).
    let avail = (width as usize).saturating_sub(2 * BOX_PAD_H);
    wrap_prose_line(Line::from(spans), avail)
        .into_iter()
        .map(|row| {
            let content_width: usize = row.spans.iter().map(|s| s.content.width()).sum();
            let pad = (width as usize).saturating_sub(BOX_PAD_H).saturating_sub(content_width);
            let mut out = Vec::with_capacity(row.spans.len() + 2);
            out.push(Span::styled(" ".repeat(BOX_PAD_H), Style::default().bg(bg)));
            out.extend(row.spans);
            out.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
            Line::from(out)
        })
        .collect()
}

/// `filled_line` without the `MARGIN_X` inset — content starts in cell 0.
/// Only for a selectable option row (`render_decision_options`), the one
/// row type the reference deliberately runs flush to the frame's own left
/// edge so its `▌` selection mark lands in cell 0.
fn flush_line(spans: Vec<Span<'static>>, bg: Color, width: u16) -> Vec<Line<'static>> {
    wrap_prose_line(Line::from(spans), width as usize)
        .into_iter()
        .map(|row| {
            let content_width: usize = row.spans.iter().map(|s| s.content.width()).sum();
            let pad = (width as usize).saturating_sub(content_width);
            let mut out = row.spans;
            out.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
            Line::from(out)
        })
        .collect()
}

/// A real drawn one-cell border (`InlineDiff.jsx`: `border: 1px solid
/// var(--tui-line)`) around a quoted diff, matching the design system's
/// square-cornered, one-cell-thick box convention exactly — not a flat
/// rule standing in for it. `diff_box_border(true, ...)` is the top edge
/// (`┌─…─┐`), `diff_box_border(false, ...)` the bottom (`└─…─┘`); the
/// vertical `│` sides come from `boxed_line` on every row in between.
/// How far a bordered box is held off the edge of the surface it sits on,
/// and what that held-off strip paints. The reference nests a diff box two
/// different ways: inside the permission card it rides the card's own
/// `padding: 0 27px` margin (`Inset::card`), while inside a turn's body
/// column it sits flush against the column's left edge with no second
/// margin of its own (`Inset::FLUSH`) — the body column's `CONTENT_INDENT`
/// is already the only offset it needs.
#[derive(Clone, Copy)]
struct Inset {
    cells: usize,
    surround: Color,
}

impl Inset {
    /// Flush against whatever column already positions the box — no margin,
    /// so `surround` is never painted and its value doesn't matter.
    const FLUSH: Inset = Inset { cells: 0, surround: Color::Reset };

    /// The permission card's own `MARGIN_X` margin, painted in the card's
    /// `bar` surface so the strip reads as card, not as diff.
    fn card(pal: &Palette) -> Inset {
        Inset { cells: MARGIN_X, surround: pal.bar }
    }
}

fn diff_box_border(top: bool, pal: &Palette, bg: Color, inset: Inset, width: u16) -> Line<'static> {
    let (left, right) = if top { ('┌', '┐') } else { ('└', '┘') };
    let inner = (width as usize).saturating_sub(2 * inset.cells).saturating_sub(2);
    let margin = || Span::styled(" ".repeat(inset.cells), Style::default().bg(inset.surround));
    Line::from(vec![
        margin(),
        Span::styled(format!("{left}{}{right}", "─".repeat(inner)), Style::default().fg(pal.line).bg(bg)),
        margin(),
    ])
}

/// Like `filled_line`, but the left/right edge columns are a drawn `│`
/// border (styled `line`, on `bg`) instead of blank fill — the vertical
/// sides of `diff_box_border`'s box. `avail` reserves exactly one column on
/// each side for the border itself, so wrapped content can never encroach
/// on it.
fn boxed_line(spans: Vec<Span<'static>>, bg: Color, inset: Inset, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    let avail = (width as usize).saturating_sub(2 * inset.cells).saturating_sub(2);
    wrap_prose_line(Line::from(spans), avail)
        .into_iter()
        .map(|row| {
            let content_width: usize = row.spans.iter().map(|s| s.content.width()).sum();
            let pad = avail.saturating_sub(content_width);
            let mut out = Vec::with_capacity(row.spans.len() + 4);
            out.push(Span::styled(" ".repeat(inset.cells), Style::default().bg(inset.surround)));
            out.push(Span::styled("│", Style::default().fg(pal.line).bg(bg)));
            out.extend(row.spans);
            out.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
            out.push(Span::styled("│", Style::default().fg(pal.line).bg(bg)));
            out.push(Span::styled(" ".repeat(inset.cells), Style::default().bg(inset.surround)));
            Line::from(out)
        })
        .collect()
}

/// One or more lines of a filled "card": `content`, styled per
/// `content_style` and padded (via `filled_line`) to the full render width
/// so the fill reads as one continuous card rather than per-line background
/// patches — more than one row when `content` is wider than `width` (see
/// `filled_line`'s doc comment). Used to carry a left accent bar glyph
/// (mirroring OpenCode's own `border={["left"]}` input) — dropped per
/// explicit developer feedback that it read as stray decoration borrowed
/// from OpenCode rather than something Mjolnir's own cards needed; the flat
/// full-width fill alone already reads as "this is a card." `content_style`
/// carries whatever bg the caller wants (the neutral `BG_ELEMENT` card fill,
/// or a semantic tint like `DIFF_ADD_BG` that should win over it) — this
/// helper doesn't pick one, it just reads it back out to pad with the
/// matching color.
fn card_line(content: &str, content_style: Style, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    let bg = content_style.bg.unwrap_or(pal.ground);
    filled_line(vec![Span::styled(content.to_string(), content_style)], bg, width)
}

/// A full-width flat rule inside a card/panel — the design system's own
/// revision log settled every freestanding rule as flat and single-color,
/// not a fading gradient (see `Palette::rule`'s doc comment) — used above
/// the decision panel's options list (`FadingRule`'s job in `readme.md`'s
/// Permission screen) and, with `accent_fg: true`, as the panel's own
/// one-cell top edge (`--tui-modal-line`).
fn card_rule(fg: Color, bg: Color, width: u16) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width as usize), Style::default().fg(fg).bg(bg)))
}

/// A single-row footer inside a card/panel: `left` spans flush to the
/// content's own left inset, `right` text flush to the right inset — the
/// same left/right split `KeyHints.jsx`/`Modal.jsx`'s footer row uses
/// (key hints on the left, a where-state-lives fact on the right). Assumes
/// `left` plus `right` fit on one row (true for every real call site: a
/// handful of short key hints, and a config path) rather than routing
/// through `filled_line`'s wrap machinery for what's always short, fixed
/// chrome text.
fn card_footer_line(left: Vec<Span<'static>>, right: &str, bg: Color, pal: &Palette, width: u16) -> Line<'static> {
    let left_width: usize = left.iter().map(|s| s.content.width()).sum();
    let avail = (width as usize).saturating_sub(BOX_PAD_H * 2);
    // On a terminal too narrow for both halves the right-hand token is
    // dropped outright, not wrapped: this line is laid out by hand rather
    // than by `Paragraph`'s wrapper, so an overlong row would spill onto a
    // row *outside* the panel — the frame's own ground showing through
    // under a fragment of provenance text. The keys on the left are what a
    // developer actually needs to answer the prompt; the note on the right
    // is the half that can go.
    // `<`, not `<=`: at least one cell of gap has to survive between the two
    // halves, or they'd read as one run-on string.
    let right = if left_width + right.width() < avail { right } else { "" };
    let right_width = right.width();
    let gap = avail.saturating_sub(left_width.min(avail)).saturating_sub(right_width);
    let mut spans = vec![Span::styled(" ".repeat(BOX_PAD_H), Style::default().bg(bg))];
    spans.extend(left);
    spans.push(Span::styled(" ".repeat(gap.max(1)), Style::default().bg(bg)));
    if !right.is_empty() {
        spans.push(Span::styled(right.to_string(), Style::default().fg(pal.dim).bg(bg)));
    }
    spans.push(Span::styled(" ".repeat(BOX_PAD_H), Style::default().bg(bg)));
    Line::from(spans)
}

/// The decision panel's title band — `Modal.jsx`'s title row: a field of
/// `band` (accent-900) carrying the plain-lowercase kind (`permission`) in
/// `accent_text`, no glyph (per the design system's revision log: a `▌`
/// pip "indicated nothing" here), and the payload's own kind right-aligned
/// in `gauge_fill` (accent-600) — the same role `Modal`'s `badge` prop
/// plays for `bash` in the reference. Prepended above whatever
/// `render_approval_card`/`render_prompt_card` returns, and — together with
/// `card_rule`'s accent top edge — kept outside `clamp_panel`'s budget
/// entirely (see `decision_panel_lines`), so it can never be the thing that
/// gets truncated away.
fn panel_band(title: &str, badge: &str, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    // No rule of its own: the panel's single `border-top` is drawn by
    // `draw`, on the row the bottom bar's own edge would otherwise occupy —
    // see there for why the two can't both draw one.
    vec![
        card_footer_line(vec![Span::styled(title.to_string(), Style::default().fg(pal.accent_text))], badge, pal.band, pal, width),
    ]
}

/// "↑↓ to move   1-N to pick   ⏎ to confirm" — `KeyHints.jsx`'s key-colored/
/// verb-muted pair convention, groups apart the same way `BarGroup`'s 6-cell
/// gap separates unrelated facts. Replaces the per-row shortcut column the
/// panel used to need (`render_decision_options` now numbers every option
/// instead), per the design system's own revision log on the permission
/// screen: "the keys that were on the rows moved into the footer."
fn decision_footer_hint(option_count: usize, pal: &Palette) -> Vec<Span<'static>> {
    let key = Style::default().fg(pal.mark);
    let verb = Style::default().fg(pal.quiet);
    vec![
        Span::styled("↑↓", key),
        Span::styled(" to move      ", verb),
        Span::styled(format!("1-{option_count}"), key),
        Span::styled(" to pick      ", verb),
        Span::styled("⏎", key),
        Span::styled(" to confirm", verb),
    ]
}

/// The Edit approval card: filled title and options list (same shape as
/// `render_card`) around a diff-aware body — added/removed lines get a
/// full-width background tint (see `DIFF_ADD_BG`/`DIFF_DEL_BG`), and
/// unchanged context beyond `DIFF_CONTEXT_RADIUS` lines from the nearest
/// change collapses to a single "N unchanged lines" marker — per explicit
/// developer feedback that the card previously rendered every diff line in
/// the same plain style, which made it hard to tell what actually changed
/// at a glance. Shared by two very different call sites: the decision panel
/// (`decision_panel_lines`, `resolution: None`, `pending_tail` carries the
/// live numbered options list — see `render_decision_options` — plus any
/// queue-count note and the card's own closing padding, built and owned
/// entirely by the caller) and a resolved entry's permanent record inline in
/// the log (`render_entry`, `resolution: Some(_)`, `pending_tail` unused
/// since the "resolved: …" line takes its place instead).
fn render_approval_card(diff: &str, resolution: Option<bool>, pending_tail: Vec<Line<'static>>, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    let (path, body) = parse_diff_body(diff);
    let body = number_diff_lines(body);
    // A blank filled row top and bottom (see `card_padding_line`'s doc
    // comment) — plain terminal text sat flush against the card's edges,
    // which read as cramped next to the reference's generous interior
    // padding. `bar` — the panel/card surface, per the design system's own
    // "ground the chrome-bar colour" spec for the permission panel.
    // Plain sentence, `body` color — not bold, not accent — matching the
    // reference's own permission-body sentence ("The agent wants to run a
    // shell command."): the title *band* above this (see `panel_band`)
    // already carries the accent weight this row doesn't need to repeat.
    // "The agent," not "Claude" — `readme.md`'s Content Fundamentals: third
    // person for the model when the harness is speaking about it.
    let mut lines = vec![card_padding_line(pal.bar, pal, width)];
    lines.extend(card_line("The agent wants to edit this file.", Style::default().fg(pal.body).bg(pal.bar), pal, width));
    if let Some(path) = path {
        lines.extend(card_line(&path, Style::default().fg(pal.label).bg(pal.bar), pal, width));
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

    // The diff quote is a real bordered box (`InlineDiff.jsx`: `border: 1px
    // solid var(--tui-line)`) on `diff_box` — one step darker/lighter than
    // the card's `bar` field, so it reads as "a quoted block inside this
    // card," the same nesting `CommandBlock.jsx` (a `ground`-colored field
    // inside the `bar`-colored permission panel) uses for a different
    // payload kind.
    lines.push(card_padding_line(pal.bar, pal, width));
    lines.push(diff_box_border(true, pal, pal.diff_box, Inset::card(pal), width));
    let mut i = 0;
    while i < n {
        if keep[i] {
            let line = &body[i];
            lines.extend(render_diff_line(line, Inset::card(pal), pal, width));
            i += 1;
        } else {
            let elided_start = i;
            while i < n && !keep[i] {
                i += 1;
            }
            let count = i - elided_start;
            let spans = vec![Span::styled(format!("⋯ {count} unchanged line{} ⋯", if count == 1 { "" } else { "s" }), Style::default().fg(pal.dim))];
            lines.extend(boxed_line(spans, pal.diff_box, Inset::card(pal), pal, width));
        }
    }
    lines.push(diff_box_border(false, pal, pal.diff_box, Inset::card(pal), width));

    match resolution {
        Some(approved) => {
            let (word, fg) = if approved { ("approved", pal.add) } else { ("denied", pal.del) };
            lines.push(card_padding_line(pal.bar, pal, width));
            lines.extend(card_line(&format!("resolved: {word}"), Style::default().fg(fg).bg(pal.bar), pal, width));
            lines.push(card_padding_line(pal.bar, pal, width));
        }
        None => lines.extend(pending_tail),
    }
    lines
}

/// A blank, filled row — same fill mechanism as `card_line`, just with
/// empty content — used as a leading/trailing spacer inside a card so its
/// content doesn't sit flush against the card's own top/bottom edge. Always
/// exactly one row (empty content never wraps), so this stays single-`Line`
/// for its many `.push` call sites rather than propagating `card_line`'s
/// `Vec` return all the way through every padding site too.
fn card_padding_line(bg: Color, pal: &Palette, width: u16) -> Line<'static> {
    card_line("", Style::default().bg(bg), pal, width).into_iter().next().expect("card_line(\"\", ..) never wraps empty content, so it always returns exactly one row")
}

/// Renders the decision panel's numbered, keyboard-navigable list of
/// choices — one row per `DecisionOption` — matching `OptionRow.jsx`'s own
/// selection convention: the accent `▌` mark plus the `band` field
/// together (never the mark alone), the number in `accent_text` on the
/// selected row and `label` otherwise, per the design system's revision
/// log on the permission screen ("options are numbered 1–4... the number is
/// accent-300 on the selected row and neutral-600 on the rest") — no
/// trailing per-row *shortcut* column any more; that moved into the panel's
/// own footer (`decision_footer_hint`).
///
/// Each row does carry a second column now: the option's `detail`, dim, in
/// a column aligned across the whole list — what choosing this option
/// concretely does ("saved to .mjolnir/permissions.yaml"), per the
/// developer feedback recorded on `DecisionOption::detail` itself. The
/// column is dropped wholesale (never per-row, which would leave the list
/// visibly ragged) on a frame too narrow to seat it without wrapping every
/// row: the labels alone still resolve the list, and the panel's body above
/// already states the rule in full.
fn render_decision_options(options: &[DecisionOption], selected: usize, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    // Cells 0-5 are the mark and number columns (see the span layout
    // below); `DETAIL_GAP` parts the label column from the detail one, and
    // `MARGIN_X` keeps the longest detail off the frame's right edge.
    const DETAIL_GAP: usize = 3;
    let label_width = options.iter().map(|o| o.label.width()).max().unwrap_or(0);
    let detail_width = options.iter().map(|o| o.detail.width()).max().unwrap_or(0);
    let show_details = detail_width > 0 && 6 + label_width + DETAIL_GAP + detail_width + MARGIN_X <= width as usize;
    options
        .iter()
        .enumerate()
        .flat_map(|(i, opt)| {
            let is_selected = i == selected;
            let bg = if is_selected { pal.band } else { pal.bar };
            let mark_fg = if is_selected { pal.mark } else { pal.mark_idle };
            let number_fg = if is_selected { pal.accent_text } else { pal.label };
            let label_fg = if is_selected { pal.text } else { pal.body };
            // Flush to the frame's left edge — the one row type in the
            // system that skips `MARGIN_X` ("Four option rows, flush to the
            // frame's left edge like the command rows in 5c"). The exact
            // cell positions the reference lays out: `▌` in cell 0, two
            // spaces, the number in cell 3, two more spaces, then the label
            // starting in cell 6 ("the number is a direct-pick accelerator,
            // one cell after the mark and two cells before the label"). No
            // period after the number.
            let mut spans = vec![
                Span::styled("▌  ", Style::default().fg(mark_fg).bg(bg)),
                Span::styled(format!("{}  ", i + 1), Style::default().fg(number_fg).bg(bg)),
                Span::styled(opt.label.clone(), Style::default().fg(label_fg).bg(bg)),
            ];
            if show_details {
                let pad = label_width - opt.label.width() + DETAIL_GAP;
                spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
                spans.push(Span::styled(opt.detail.clone(), Style::default().fg(pal.dim).bg(bg)));
            }
            flush_line(spans, bg, width)
        })
        .collect()
}

/// The decision panel's statement of what a *saved* answer would write —
/// one line naming the literal `kind:pattern` rule (`App::decision_grant`),
/// plus a second line naming the other scope Tab would switch to when the
/// target has one.
///
/// Both exist because of the same developer feedback: "permissions are not
/// clear, are we approving the tool? are we approving the directory? what
/// are we concretely doing." The tier labels below can't answer that on
/// their own — they stay identical whichever pattern is selected (see
/// `App::decision_options`) — and the line they replace only rendered for
/// path-like targets, so a `shell` prompt said nothing at all about whether
/// "allow" meant this command or the shell tool. Naming the rule verbatim
/// answers it in the same vocabulary the developer will later read back out
/// of `permissions.yaml`.
fn grant_lines(grant: &GrantSummary) -> Vec<String> {
    let mut lines = vec![format!("saving an answer adds the rule  {}", elide(&grant.rule, GRANT_RULE_MAX))];
    if let Some(alternate) = &grant.alternate {
        let alternate = elide(alternate, GRANT_RULE_MAX);
        lines.push(match grant.scope {
            PatternScope::Exact => format!("Tab  widen it to this whole directory  {alternate}"),
            PatternScope::Directory => format!("Tab  narrow it back to this one file  {alternate}"),
        });
    }
    lines
}

/// How much of a grant rule `grant_lines` spells out before eliding. A rule
/// is `kind:pattern` over an arbitrary tool target, and an arbitrarily long
/// one is ordinary input (a long shell command); the target is already shown
/// in full in the card body directly above this line, so wrapping a second
/// copy of it across four rows would spend the panel's row budget (see
/// `clamp_panel`, which sacrifices body rows to keep the options list
/// whole) repeating what the developer just read. The head is what carries
/// this line's meaning: which kind, and that the pattern is the literal
/// target rather than a wildcard.
const GRANT_RULE_MAX: usize = 56;

/// Truncates to `max` characters with a trailing `…` — the design system's
/// own elision glyph, already used by `clamp_panel`'s hidden-rows marker.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
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
fn diff_gutter(old_no: Option<usize>, new_no: Option<usize>, bg: Color, pal: &Palette) -> Span<'static> {
    let o = old_no.map(|n| n.to_string()).unwrap_or_default();
    let n = new_no.map(|n| n.to_string()).unwrap_or_default();
    Span::styled(format!("{o:>4} {n:>4} │ "), Style::default().fg(pal.dim).bg(bg))
}

/// Renders one kept diff line via `filled_line`, prefixed with its
/// old/new line-number gutter (see `diff_gutter`). Added/removed lines get
/// their semantic `add_bg`/`del_bg` tint (which wins over `diff_box`, the
/// surface the quoted diff sits on) so a change reads as a colored row at a
/// glance, not just a leading +/- character — `InlineDiff.jsx`'s own row
/// treatment; context lines get the plain `diff_box` fill and `context`
/// text color, since only the changed lines' brighter tint should compete
/// for attention.
fn render_diff_line(line: &DiffLine, inset: Inset, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    // Sign and code text are two different tokens in the source
    // (`--tui-add`/`--tui-del` for the `+`/`-` sign, `--tui-add-code`/
    // `--tui-del-code` for the code text itself) — kept as separate spans
    // rather than one combined color so both read exactly as `InlineDiff.jsx`
    // does.
    let (marker, sign_fg, code_fg, bg) = match line.kind {
        DiffLineKind::Added => ("+ ", pal.add, pal.add_code, pal.add_bg),
        DiffLineKind::Removed => ("- ", pal.del, pal.del_code, pal.del_bg),
        DiffLineKind::Context => ("  ", pal.diff_box, pal.context, pal.diff_box),
    };
    let spans = vec![
        diff_gutter(line.old_no, line.new_no, bg, pal),
        Span::styled(marker.to_string(), Style::default().fg(sign_fg).bg(bg)),
        Span::styled(line.text.clone(), Style::default().fg(code_fg).bg(bg)),
    ];
    boxed_line(spans, bg, inset, pal, width)
}

/// A quoted diff as `InlineDiff.jsx`'s own real bordered box — top edge,
/// one `boxed_line` row per kept diff line, bottom edge. `width` is the
/// box's own outer width (border columns included).
fn boxed_diff_lines(lines: &[DiffLine], inset: Inset, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    let mut out = vec![diff_box_border(true, pal, pal.diff_box, inset, width)];
    out.extend(lines.iter().flat_map(|line| render_diff_line(line, inset, pal, width)));
    out.push(diff_box_border(false, pal, pal.diff_box, inset, width));
    out
}

/// Shared by the decision panel (`resolution: None`, `pending_tail` is the
/// live numbered options list — see `render_approval_card`'s own doc
/// comment on the same param) and a resolved prompt's permanent record in
/// the log (`render_entry`, `resolution: Some(_)`, `pending_tail` unused).
/// `CommandBlock.jsx`: a `ground`-colored field (distinct from the card's
/// own `bar` surface, so it reads as an inset quoted block, the same
/// nesting `InlineDiff`'s `diff_box` uses for a different payload kind)
/// with the command prefixed by an accent `$`.
fn command_block_lines(command: &str, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![card_padding_line(pal.ground, pal, width)];
    let spans = vec![Span::styled("$ ", Style::default().fg(pal.speaker_you)), Span::styled(command.to_string(), Style::default().fg(pal.text))];
    lines.extend(filled_line(spans, pal.ground, width));
    lines.push(card_padding_line(pal.ground, pal, width));
    lines
}

fn render_prompt_card(payload: &PromptPayload, resolution: Option<&str>, pending_tail: Vec<Line<'static>>, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    // Plain sentence, `body` — matches the reference's own permission-body
    // styling ("The agent wants to run a shell command."); the title band
    // above this (`panel_band`) already carries the accent weight.
    let mut lines = vec![card_padding_line(pal.bar, pal, width)];
    lines.extend(card_line(&humanize_prompt(payload), Style::default().fg(pal.body).bg(pal.bar), pal, width));
    // A shell command gets `CommandBlock.jsx`'s own treatment — other kinds
    // (a file path, a context-file load) show their exact target as plain
    // label text instead (`readme.md`'s "Targets are exact" rule); a `$`
    // prompt only means something for an actual shell command.
    match payload {
        PromptPayload::Tool { kind, target, .. } if kind == "shell" => {
            lines.push(card_padding_line(pal.bar, pal, width));
            lines.extend(command_block_lines(target, pal, width));
        }
        _ => lines.extend(card_line(&raw_prompt_call(payload), Style::default().fg(pal.label).bg(pal.bar), pal, width)),
    }
    match resolution {
        Some(r) => {
            lines.extend(card_line(&format!("resolved: {r}"), Style::default().fg(pal.accent_text).bg(pal.bar), pal, width));
            lines.push(card_padding_line(pal.bar, pal, width));
        }
        None => lines.extend(pending_tail),
    }
    lines
}

/// A plain-English sentence naming what's actually being asked — the title
/// row a developer reads first to decide. Per explicit developer feedback
/// that the old title ("Allow shell: cargo test --release?") *was* the raw
/// tool call, with nothing telling a developer what that call actually
/// does at a glance; `raw_prompt_call` below still renders the literal call
/// underneath, dim, for whoever wants to verify the mechanism.
fn humanize_prompt(payload: &PromptPayload) -> String {
    match payload {
        PromptPayload::Tool { kind, .. } => humanize_tool_kind(kind),
        PromptPayload::ContextFile { path } => format!("The agent wants to load {} as context", path.display()),
        // Never actually reaches this card in production — `App::
        // decision_options`' Edit arm returns no options, since Edit uses
        // the separate ToolApprovalRequested/ApprovalCard path instead
        // (mjolnir-permissions.md's Edit Exception). Kept for a complete,
        // non-panicking match, not a live UI path.
        PromptPayload::Edit { .. } => "The agent wants to edit a file".into(),
    }
}

fn humanize_tool_kind(kind: &str) -> String {
    match kind {
        "read" => "The agent wants to read a file".into(),
        "shell" => "The agent wants to run a shell command".into(),
        "explain" => "The agent wants to inspect code".into(),
        other => format!("The agent wants to use \"{other}\""),
    }
}

/// A `RunningTool`'s display name — `App::apply_event`'s doc comment on
/// `pending_tool_names` notes `name` comes from a `ToolUseRequested` looked
/// up by `call_id` and falls back to an empty string
/// (`unwrap_or_default()`) if that lookup ever misses; falling back to the
/// `call_id` itself here (rather than showing nothing) is what the status
/// line's trailing tools list already did — shared so `activity_label`'s
/// leading word can't drift from it and show a blank/awkward name in a case
/// the list already handles.
fn running_tool_name(tool: &RunningTool) -> &str {
    if tool.name.is_empty() {
        &tool.call_id
    } else {
        &tool.name
    }
}

/// Present-progressive fragment for the status line's leading activity word
/// — a separate small table from `humanize_tool_kind` above rather than a
/// shared one, since the two need different grammar ("The agent wants to
/// read a file" vs "reading a file…") for what's otherwise the same handful
/// of tool kinds; not worth a shared abstraction for three arms each.
fn tool_gerund(kind: &str) -> String {
    match kind {
        "read" => "reading a file".into(),
        "shell" => "running a shell command".into(),
        "explain" => "inspecting code".into(),
        other => format!("using {other}"),
    }
}

/// What to say next to the spinner while `app.turn_active` and not
/// `app.thinking` (thinking has its own, more specific "thinking…" text) —
/// per direct developer feedback that a bare "working…" for the entire
/// stretch of a turn gave no sense of what was actually happening. Built
/// entirely from state `App` already tracks (no new event/data needed):
/// a running tool's own name (via `tool_gerund`, the same humanization the
/// permission prompt already applies to a tool kind), the count when more
/// than one tool is running at once (parallel dispatch — see
/// mjolnir-core's `dispatch_tools`), or, with no tool in flight, whether
/// assistant text is already streaming for this step (the log's tail entry
/// is a `LogEntry::AssistantText` for exactly that stretch) versus still
/// waiting on the first token or tool call of the step.
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

/// The literal `kind: target` the humanized sentence above is describing —
/// unchanged in substance from the old title text, just demoted to a dim
/// subtitle now that the title itself carries the explanation.
fn raw_prompt_call(payload: &PromptPayload) -> String {
    match payload {
        PromptPayload::Tool { kind, target, .. } => format!("{kind}: {target}"),
        PromptPayload::ContextFile { path } => format!("context_file: {}", path.display()),
        PromptPayload::Edit { kind } => format!("edit: {kind}"),
    }
}

/// Builds the fixed decision panel's content: whichever pending
/// approval/prompt is at the front of its queue, wrapped in `Modal.jsx`'s
/// own chrome — an accent-700 top rule and a title band (`panel_band`) —
/// then a footer (`decision_footer_hint`/the persisted-permissions path)
/// appended to the tail alongside the numbered options, matching the
/// design system's Permission screen structure exactly. Returns nothing at
/// all when both queues are empty (the panel band then collapses to zero
/// height — see `draw`). Checks `pending_approvals` before
/// `pending_prompts`, mirroring `App::handle_key`'s own priority (approvals
/// resolve first when both queues hold an entry) — what's shown here must
/// always be exactly what the next keypress actually resolves.
///
/// Returns the lines alongside how many trailing rows are the numbered
/// options list plus the footer (any queue-count note, the card's own
/// closing padding, the footer rule and hint row) — `clamp_panel` must
/// never truncate into that tail, since the options list and the footer
/// that explains how to use it are what a developer absolutely still needs
/// to see, however large the body above it gets.
/// `frame_height` sizes the `clamp_panel` budget (`panel_max_height`) for
/// whatever's actually rendered inside `render_approval_card`/`render_card`
/// — the panel's own chrome (`panel_band`'s top rule + title band, and the
/// footer's rule + hint row) is deliberately assembled *outside* that
/// budget, appended after clamping, so neither can ever be the thing a
/// large diff or a small terminal squeezes out: the footer explains how to
/// use the options list, so it needs the same "never truncated" guarantee
/// clamp_panel already gives the list itself, not just a best-effort
/// inclusion in the same protected-but-still-counted tail the options list
/// used to share it with.
fn decision_panel_lines(app: &App, width: u16, frame_height: u16) -> Vec<Line<'static>> {
    let pal = app.theme.palette();
    let options = app.decision_options();
    if options.is_empty() {
        return Vec::new();
    }
    // `App::pending_front` is the single source of truth for "approvals
    // before prompts" — consolidated here (a rust-skills audit flagged this
    // function, plus `App::decision_options`/`decline_outcome`, as each
    // independently re-deriving the same priority check) so it can never
    // drift from what `App::handle_key` actually resolves. The two arms
    // still each build their own "(+N more pending)" note, since that part
    // genuinely differs — it reads a different `VecDeque`'s length — but the
    // *which-queue-is-front* question is answered exactly once, here.
    // The flat `rule` above the options list — `readme.md`: freestanding
    // rules are "one step more muted than the structural borders they sit
    // beside." Shared by both arms below, since every payload kind's
    // options list gets the same rule ahead of it.
    let options_rule = || vec![card_padding_line(pal.bar, pal, width), card_rule(pal.rule, pal.bar, width), card_padding_line(pal.bar, pal, width)];
    let (body, badge, tail) = match app.pending_front() {
        PendingFront::Approval(pending) => {
            let mut tail = options_rule();
            tail.extend(render_decision_options(&options, app.decision_selected, pal, width));
            let queue_len = app.pending_approvals.len();
            if queue_len > 1 {
                tail.extend(card_line(&format!("(+{} more pending)", queue_len - 1), Style::default().fg(pal.dim).bg(pal.bar), pal, width));
            }
            (render_approval_card(&pending.diff, None, Vec::new(), pal, width), "edit".to_string(), tail)
        }
        PendingFront::Prompt(pending) => {
            // Present for every Tool prompt (its second line, the Tab
            // toggle, only when the target has an enclosing directory to
            // broaden to); absent for a ContextFile prompt, which persists a
            // path rather than a grant pattern and has no rule to state.
            let mut tail: Vec<Line<'static>> = match app.decision_grant() {
                // Its own padding row above: the rule restates the target
                // the card body just showed, so without a break the two sit
                // as adjacent near-identical rows ("read: ./x.rs" directly
                // over "…adds the rule  read:./x.rs") and read as a stutter
                // rather than as a statement about what happens next.
                Some(grant) => std::iter::once(card_padding_line(pal.bar, pal, width))
                    .chain(grant_lines(&grant).iter().flat_map(|line| card_line(line, Style::default().fg(pal.dim).bg(pal.bar), pal, width)))
                    .collect(),
                None => Vec::new(),
            };
            tail.extend(options_rule());
            tail.extend(render_decision_options(&options, app.decision_selected, pal, width));
            let queue_len = app.pending_prompts.len();
            if queue_len > 1 {
                tail.extend(card_line(&format!("(+{} more pending)", queue_len - 1), Style::default().fg(pal.dim).bg(pal.bar), pal, width));
            }
            let badge = match &pending.payload {
                PromptPayload::Tool { kind, .. } => kind.clone(),
                PromptPayload::ContextFile { .. } => "context".to_string(),
                PromptPayload::Edit { .. } => "edit".to_string(),
            };
            (render_prompt_card(&pending.payload, None, Vec::new(), pal, width), badge, tail)
        }
        PendingFront::None => return Vec::new(),
    };
    // `body` as returned above ends with an empty `pending_tail` (`None` ==
    // "nothing pending" was never true here, so this is really `Vec::new()`
    // standing in for "the caller appends the tail itself" — see
    // `render_approval_card`'s own `pending_tail` doc comment) — `tail` is
    // appended here instead, then clamped as one unit so `clamp_panel` still
    // protects the whole options list, not just whatever `render_approval_card`
    // happened to leave unclamped.
    let mut clampable = body;
    let tail_len = tail.len();
    clampable.extend(tail);
    let clamped = clamp_panel(clampable, panel_max_height(frame_height), tail_len, pal, width);

    let mut lines = panel_band("permission", &badge, pal, width);
    lines.extend(clamped);
    lines.push(card_padding_line(pal.bar, pal, width));
    // `line`, not the more muted `rule` — matches the reference's real
    // `border-top: 1px solid var(--tui-line)` on this footer row (the rule
    // above the *options* list, inside `render_decision_options`'
    // surrounding chrome, is the one place `rule` is actually correct).
    lines.push(card_rule(pal.line, pal.bar_bottom, width));
    // No right-hand provenance note. It used to read "saved to
    // .mjolnir/permissions.yaml" under every prompt, which was true of
    // exactly one of the tiers on offer — "allow once" and "allow for this
    // session" save nothing at all, and "always allow" writes to the global
    // file instead. Where each answer lands is now stated per option, on the
    // option's own row (`DecisionOption::detail`), which is the only place
    // it can be stated accurately.
    lines.push(card_footer_line(decision_footer_hint(options.len(), pal), "", pal.bar_bottom, pal, width));
    lines
}

/// Caps the decision panel to `max` rows so an unusually large diff can
/// never squeeze the rest of the UI off-frame (see `panel_max_height`) — the
/// existing `DIFF_CONTEXT_RADIUS` collapsing in `render_approval_card` only
/// elides unchanged *context* lines, so it does nothing for the common case
/// of one big added block (a new file, a large new function): every line of
/// that block is itself a change, so none of it collapses. Keeps the
/// leading rows (blank padding + title, and a path line if there is one) and
/// the caller-supplied `tail` rows (the numbered options list, any
/// queue-count note, and the closing padding — see `decision_panel_lines`)
/// intact — those are what a developer actually needs to make the call —
/// and collapses whatever body content doesn't fit between them into a
/// single marker line, the same "⋯ N more … ⋯" shape `render_approval_card`
/// already uses for elided context. `tail` (not a fixed constant) is what
/// makes this correct once the options list can be anywhere from 2 rows
/// (Approve/Deny) to 8 (a Tool prompt's four tiers × allow/deny) — a fixed
/// guess would either truncate real options away or protect rows that
/// aren't actually the list.
fn clamp_panel(lines: Vec<Line<'static>>, max: usize, tail: usize, pal: &Palette, width: u16) -> Vec<Line<'static>> {
    // `render_approval_card`/`render_card`'s own leading padding+title
    // rows — `decision_panel_lines` now applies `panel_band`'s chrome (the
    // accent top rule + the title band) and the footer (rule + hint row)
    // outside this function entirely, appended/prepended after clamping, so
    // they can never be at risk of truncation in the first place and don't
    // need to be counted here.
    const HEAD: usize = 2;
    if lines.len() <= max {
        return lines;
    }
    let mut lines = lines;
    let tail_lines = lines.split_off(lines.len().saturating_sub(tail));
    let head_lines: Vec<_> = lines.drain(..HEAD.min(lines.len())).collect();
    // `keep` (how many of the remaining body rows survive) is found by
    // shrinking until head + kept body + the marker actually fit in `max`,
    // re-measuring the marker itself on every attempt — regression fix:
    // this used to assume the marker was always exactly one row
    // (`max - head - tail - 1`), but `card_line` wraps it, just like any
    // other card row, once its text is wider than `width` (its own text is
    // ~70 columns, so this isn't a rare case — it wraps on any panel
    // narrower than that). The undercounted budget let the *tail* — the
    // options list, the one thing that must never be cut — get silently
    // pushed past the panel's real row budget and clipped off the bottom by
    // the outer layout, confirmed via a real render: an ordinary 8-option
    // Tool prompt on a modest terminal lost its last four options entirely,
    // with no on-screen indication anything was missing. `keep == 0` is the
    // floor — even then, no marker is added if there was nothing left to
    // hide (`hidden == 0`), fixing the companion bug where a degenerate
    // head+tail-only panel used to print a nonsensical "0 more lines not
    // shown" row it didn't need and couldn't afford.
    let mut keep = lines.len();
    loop {
        let hidden = lines.len() - keep;
        let marker: Vec<Line<'static>> = if hidden == 0 {
            Vec::new()
        } else {
            card_line(
                &format!("⋯ {hidden} more line{} not shown — deciding doesn't require scrolling them ⋯", if hidden == 1 { "" } else { "s" }),
                Style::default().fg(pal.dim).bg(pal.bar),
                pal,
                width,
            )
        };
        if keep == 0 || head_lines.len() + keep + marker.len() + tail_lines.len() <= max {
            let mut out = head_lines;
            out.extend(lines.into_iter().take(keep));
            out.extend(marker);
            out.extend(tail_lines);
            return out;
        }
        keep -= 1;
    }
}

/// Live activity strip, one row, positioned directly above the decision
/// panel/input — `StatusLine.jsx`'s own job, distinct from `draw_top_bar`'s
/// static identity row (model/version) above the log. Always shows normal
/// turn/activity content, even while a decision is pending — the fixed
/// decision panel directly below it (`decision_panel_lines`) is the one
/// place pending keys show. Shows what's actually happening with the
/// model — turn/step, live activity (thinking/working/idle, with the same
/// spinner the log uses), any tools currently in flight by name (in
/// `value`, uniformly — the reference's `ToolLine` doesn't hash a color per
/// tool name the way this crate's earlier posting-inspired pass did; a
/// tool's *state* carries color here, not its identity), a running message
/// count, and a right-aligned Ctrl+C hint (`StatusLine.jsx`'s own `right`
/// prop, "esc to stop" in the reference — adapted to Mjolnir's real
/// binding). Permission state (read/shell/edit) is deliberately absent
/// here — it belongs to the once-per-session welcome hero (`intro_content`)
/// and an actual permission prompt when one fires, not a line that
/// repaints every frame.
fn draw_status_line(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    let s = &app.status;
    // Raised ground — `StatusLine.jsx` sits inside `BottomBar.jsx`'s own
    // `bar-bottom` field, not the plain frame `ground` the log panel uses.
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), area);
    // Activity leads the row — per explicit developer request that "what's
    // the LLM doing right now" is the single most useful thing this line
    // can say, so it shouldn't be buried after the model name/turn counter.
    // Glyph in `glyph_running` (`◐`, matching `StatusLine.jsx`'s
    // `state="working"` case), the words themselves in `label`.
    let spinner = SPINNER_FRAMES[app.tick as usize % SPINNER_FRAMES.len()];
    let mut spans = if app.thinking {
        vec![Span::styled(format!("{spinner} "), Style::default().fg(pal.glyph_running)), Span::styled("thinking…  ", Style::default().fg(pal.label))]
    // `awaiting_turn` counts as active here as well as in the hint below —
    // between submitting and `TurnStarted` landing the harness is waiting on
    // the provider, and reporting that stretch as "idle" is exactly the
    // no-progress-feedback complaint `activity_label` exists to answer.
    } else if app.turn_active || app.awaiting_turn {
        vec![Span::styled(format!("{spinner} "), Style::default().fg(pal.glyph_running)), Span::styled(format!("{}  ", activity_label(app)), Style::default().fg(pal.label))]
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
                spans.push(Span::raw(", "));
            }
            spans.push(Span::styled(running_tool_name(tool).to_string(), Style::default().fg(pal.value)));
        }
        spans.push(Span::raw("  "));
    }

    let messages = app.log.len();
    spans.push(Span::styled(format!("{messages} message{}", if messages == 1 { "" } else { "s" }), Style::default().fg(pal.label)));

    // Right-aligned key hint — `StatusLine.jsx`'s own `right` prop (`esc to
    // stop`), adapted to Mjolnir's real binding: Ctrl+C, not Esc, is what
    // actually cancels a turn or exits an idle session (`App::handle_key`).
    // Both halves sit inside the grid's own 3-cell margin, like every other
    // content row in a frame (`padding: 0 27px`).
    // Must read the same "is anything running" state `App::cancel_or_quit`
    // acts on, or the hint promises one thing and the key does the other.
    let hint = if app.turn_active || app.awaiting_turn { "^c to cancel" } else { "^c to exit" };
    let [left_area, right_area] = Layout::horizontal([Constraint::Min(1), Constraint::Length(hint.width() as u16 + MARGIN_X as u16)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)).block(Block::new().padding(Padding::left(MARGIN_X as u16))), left_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(pal.dim))))
            .alignment(ratatui::layout::Alignment::Right)
            .block(Block::new().padding(Padding::right(MARGIN_X as u16))),
        right_area,
    );
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
    let pal = app.theme.palette();
    // `bar_bottom` — `Composer.jsx` has no surface of its own beyond
    // `BottomBar.jsx`'s own raised ground; the composer prompt `▶` and
    // caret carry the accent, not a distinct input-only background tier.
    // `MARGIN_X` horizontally (the grid's `padding: 0 27px`), no vertical
    // padding — the blank rows around the composer belong to the bottom bar
    // itself (see `draw`), not to this widget.
    let block = Block::new().style(Style::default().bg(pal.bar_bottom)).padding(Padding::horizontal(MARGIN_X as u16));
    let inner = block.inner(area);
    // Dim placeholder text when the draft is empty — an empty filled box
    // gave no hint at all that this was where a message goes, versus every
    // other panel now carrying a title/label of its own. While blocked, the
    // placeholder says so instead of inviting a keystroke it would silently
    // drop.
    // `Composer.jsx`: accent `▶`, two spaces, the draft — only on the
    // textarea's first line (mjolnir's multi-line draft is its own
    // extension beyond the reference's single-line composer; continuation
    // rows aren't part of what the prompt glyph marks). `PROMPT_PREFIX_LEN`
    // is the fixed column offset every cursor placement below must add back
    // for line 0, since neither `cursor_line_col` nor ratatui's `Wrap` has
    // any notion of this prefix — see `moving_the_composer_cursor_accounts_
    // for_the_prompt_glyph_on_the_first_line` for the regression this
    // exists to prevent (the cursor landing 2 columns short of the real
    // caret position once the glyph pushed the actual text over).
    // `▶` plus two spaces — the reference's own composer row is
    // `<span>▶</span><span>  </span>`, putting the draft's first character
    // in cell 6 (the grid's 3-cell `MARGIN_X`, the glyph, then the two).
    const PROMPT_PREFIX_LEN: u16 = 3;
    if app.input.is_empty() {
        let blocked = !app.pending_approvals.is_empty() || !app.pending_prompts.is_empty();
        let text = if blocked { "waiting on your decision above…" } else { "Ask Mjolnir anything" };
        let placeholder = Line::from(vec![Span::styled("▶  ", Style::default().fg(pal.mark)), Span::styled(text, Style::default().fg(pal.dim))]);
        frame.render_widget(Paragraph::new(placeholder).block(block), area);
        if !blocked {
            frame.set_cursor_position((inner.x + PROMPT_PREFIX_LEN, inner.y));
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
    let lines: Vec<Line> = app
        .input
        .split('\n')
        .enumerate()
        .map(|(i, l)| {
            let mut line = highlight_command_tokens(l, pal);
            if i == 0 {
                line.spans.insert(0, Span::styled("▶  ", Style::default().fg(pal.mark)));
            }
            line
        })
        .collect();
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
        let prefix = if line == 0 { PROMPT_PREFIX_LEN } else { 0 };
        let inner_right = inner.x + inner.width.saturating_sub(1);
        let inner_bottom = inner.y + inner.height.saturating_sub(1);
        let x = (inner.x + prefix + col as u16).min(inner_right);
        let y = (inner.y + line as u16).min(inner_bottom);
        frame.set_cursor_position((x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;




    use crate::app::App;
    use crate::palette::DARK;
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
        assert_eq!(buffer[(BOX_PAD_H as u16, raw_row)].fg, DARK.label, "the raw call row must use the muted label color");
        assert_ne!(buffer[(BOX_PAD_H as u16, title_row)].fg, DARK.label, "the humanized title must not itself be the muted label color");
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
        assert!(out.contains("more line"), "a truncated panel should say how much was hidden: {out:?}");
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
        let line = highlight_command_tokens("/clear now", &DARK);
        let styled: Vec<(&str, Option<Color>)> = line.spans.iter().map(|s| (s.content.as_ref(), s.style.fg)).collect();
        assert_eq!(styled, vec![("/clear", Some(DARK.dim)), (" ", None), ("now", Some(DARK.text))]);
    }

    /// The bug report this responds to: dimming only checked the input's
    /// very first character, so a recognized command word typed anywhere
    /// past position 0 never got flagged even though it's the same word.
    #[test]
    fn highlight_command_tokens_dims_a_command_word_mid_message() {
        let line = highlight_command_tokens("please run /exit for me", &DARK);
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
        let line = highlight_command_tokens("hello world", &DARK);
        let fgs: Vec<Option<Color>> = line.spans.iter().map(|s| s.style.fg).collect();
        assert_eq!(fgs, vec![Some(DARK.text), None, Some(DARK.text)], "every word must set an explicit fg; only the whitespace between them may leave it unset");
    }

    #[test]
    fn highlight_command_tokens_requires_an_exact_word_match() {
        // "/exiting" isn't the recognized "/exit" word, and "cleared" isn't
        // "/clear" — a substring match would false-positive on either, i.e.
        // dim them like a real command word instead of leaving them DARK.text.
        let line = highlight_command_tokens("/exiting cleared", &DARK);
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
        let line = highlight_command_tokens("/theme light", &DARK);
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
        assert_eq!(intro_content(&status, &DARK).len(), crate::log::INTRO_LINE_COUNT, "ui::intro_content must stay in sync with log::INTRO_LINE_COUNT");
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
        let bottom = height - 1 - (1 + input_area_height("") + 1 + 1 + 1) - 1;
        for &(x, y) in &[(0, top), (width - 1, top), (0, bottom), (width - 1, bottom)] {
            let cell = &buffer[(x, y)];
            assert_ne!(cell.symbol(), "╭", "the log panel must not draw a border corner");
            assert_eq!(cell.bg, DARK.ground, "the log panel must still be opaque at its edges even without a drawn border");
        }
    }

    #[test]
    fn bold_markdown_strips_asterisks_and_sets_the_bold_modifier() {
        let spans = parse_inline("say **hello** now", Style::default().fg(DARK.body), &DARK);
        let bold = spans.iter().find(|s| s.content.as_ref() == "hello").expect("bold span present");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        assert!(spans.iter().all(|s| !s.content.contains('*')), "literal asterisks must not reach the screen");
    }

    #[test]
    fn italic_markdown_sets_the_italic_modifier() {
        let spans = parse_inline("that is *neat* stuff", Style::default().fg(DARK.body), &DARK);
        let italic = spans.iter().find(|s| s.content.as_ref() == "neat").expect("italic span present");
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code_strips_backticks_and_uses_a_distinct_color() {
        let spans = parse_inline("run `cargo test` first", Style::default().fg(DARK.body), &DARK);
        let code = spans.iter().find(|s| s.content.as_ref() == "cargo test").expect("code span present");
        assert_eq!(code.style.fg, Some(DARK.code), "inline code should read as a distinct color, not a reversed-video block");
        assert!(!code.style.add_modifier.contains(Modifier::REVERSED), "inline code must not use reversed video");
        assert!(spans.iter().all(|s| !s.content.contains('`')), "literal backticks must not reach the screen");
    }

    #[test]
    fn a_heading_line_drops_the_hashes_and_renders_bold() {
        let line = render_markdown_line("## Section Title", &DARK);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Section Title");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_bullet_line_replaces_the_dash_with_a_bullet_marker() {
        let line = render_markdown_line("- first item", &DARK);
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
}

