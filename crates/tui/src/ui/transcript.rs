//! The conversation log: one `LogEntry` at a time turned into rows, the
//! welcome hero that stands in for an empty log, and the panel that scrolls
//! them.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Frame;

use super::diff;
use super::grid::{justified_line, with_label_column, Ctx};
use super::markdown::{self, Segment};
use super::row::Row;
use super::wrap::wrap_line;
use crate::app::{App, PermState, StatusInfo};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};

/// Builds every line the log panel's *inner* area can show, at `ctx.width`
/// × `height` (the panel's inner rect — see `super::draw` on why this must
/// be the inner, not outer, rect). Shared by [`draw_log`] (renders it) and
/// [`row_count`] (counts its wrapped rows for scroll math), so the two can
/// never disagree about what the log contains. An empty log shows the
/// welcome hero instead of any entries — the two are mutually exclusive, so
/// there's no "separate the banner from the first real entry" case.
fn build_lines(app: &App, ctx: Ctx, height: u16) -> Vec<Line<'static>> {
    if app.log.is_empty() {
        return hero_lines(&app.status, height, ctx);
    }
    let mut lines: Vec<Line> = Vec::new();
    for entry in app.log.iter() {
        // An entry can render to nothing at all (a routine `TurnEnded` is
        // folded into the status line's own activity indicator instead of
        // getting its own log row), so the blank separator is keyed on
        // whether anything has actually been pushed yet, not on the entry's
        // index — otherwise a silent entry would still claim a blank row.
        let rendered = render_entry(entry, ctx);
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
                lines.push(Line::from(Span::styled("─".repeat(ctx.width as usize), Style::default().fg(ctx.pal.rule))));
                lines.push(Line::default());
            } else {
                lines.push(Line::default());
            }
        }
        lines.extend(rendered);
    }
    // No spinner row is appended here — per explicit developer feedback, an
    // active turn used to get an animated "thinking…"/"working…" row both
    // here (trailing the log) *and* in the status line right above the
    // input, which read as a plain duplicate of the same information. The
    // status line is now the one place live turn activity shows.
    lines
}

/// Renders the log panel: `block` onto `outer`, its content onto `inner`
/// (already computed once by `super::draw`), plus a `Scrollbar` on the
/// inner-right edge when there's more content than the viewport can show
/// and the log isn't showing the (never-scrollable) hero.
pub(super) fn draw_log(frame: &mut Frame, outer: Rect, inner: Rect, block: Block<'static>, app: &App) {
    let ctx = Ctx::new(app.theme.palette(), inner.width);
    let lines = build_lines(app, ctx, inner.height);
    // `scroll.offset` is in *wrapped screen rows* (see [`row_count`]), so it
    // must go through `Paragraph::scroll`, which advances the same wrapping
    // line-composer `Paragraph::line_count` uses internally — not a
    // `.skip()` on `lines` beforehand, which would count in logical
    // (pre-wrap) rows instead and drift out of sync the moment anything
    // wraps.
    let offset = app.scroll.offset.min(u16::MAX as usize) as u16;
    let total = app.total_lines();

    frame.render_widget(block, outer);
    frame.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).scroll((offset, 0)), inner);

    if !app.log.is_empty() && total > inner.height as usize {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).style(Style::default().fg(ctx.pal.line));
        let mut state = ScrollbarState::new(total).position(offset as usize);
        // Renders into the block's own right-border column, inset by 1 row
        // top/bottom so it doesn't overwrite the panel's corners — the
        // standard ratatui pattern (see `Scrollbar`'s own doc example).
        frame.render_stateful_widget(scrollbar, outer.inner(Margin { vertical: 1, horizontal: 0 }), &mut state);
    }
}

/// The number of terminal rows the log panel's inner area needs to fully
/// render at `width` × `height` — what `ScrollState` actually compares
/// against viewport height, wrapping included. Delegates to ratatui's own
/// `Paragraph::line_count`, which runs the exact same word-wrapper
/// [`draw_log`]'s render path uses, rather than re-deriving wrap behaviour
/// by hand — see mjolnir-tui.md's 2026-08-29 scrolling-fix and
/// wrapped-row-scroll-math Progress notes for the two incidents this
/// discipline exists to prevent from recurring. `height` only matters for
/// the empty-log hero path (it vertically centres the hero); the non-empty
/// path's row count is width-only.
pub(crate) fn row_count(app: &App, width: u16, height: u16) -> usize {
    let lines = build_lines(app, Ctx::new(app.theme.palette(), width), height);
    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).line_count(width)
}

fn render_entry(entry: &LogEntry, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    match entry {
        // `Turn.jsx`: the `you` label in `speaker-you` (accent-toned), the
        // `harness` label in `speaker-agent` (neutral) — content in `text`
        // (primary) for a `you` turn, `body` for the agent's. No filled
        // background: the design system's own components never fill a chat
        // message's background — flat coloured text on the panel ground is
        // the whole treatment. A slash command (directed at the harness,
        // not the model) skips the speaker label too, since it isn't
        // conversational content.
        LogEntry::UserMessage { text } => {
            if is_command(text) {
                return text.lines().map(|l| Line::from(Span::styled(format!("> {l}"), Style::default().fg(pal.dim)))).collect();
            }
            let style = Style::default().fg(pal.text);
            let content: Vec<Line<'static>> =
                text.lines().flat_map(|l| wrap_line(Line::from(Span::styled(l.to_string(), style)), ctx.body().width as usize)).collect();
            with_label_column(content, Some(("you", pal.speaker_you)))
        }
        LogEntry::AssistantText { text } => with_label_column(render_assistant_text(text, ctx), Some(("harness", pal.speaker_agent))),
        // `ToolLine.jsx`: a status glyph, the tool name, a right-flush
        // result summary. `ToolActivityEntry` carries no separate target
        // path distinct from the tool's own name (unlike the reference's
        // `read src/gateway/mod.rs`), so the call id stands in for it,
        // parenthesized. The design system's glyph table has no distinct
        // "failed" mark (`readme.md`'s Iconography table: only `●` done /
        // `◐` running / `○` pending / `✔` accepted — "if a mark is needed
        // and it is not in that table, do not draw one") — an error keeps
        // the `●` done glyph but in `del` (red) instead of `add`, the same
        // "colour carries the meaning" rule the rest of this system leans
        // on. No label of its own — a tool-activity group continues
        // whichever turn's content column it renders under.
        LogEntry::ToolActivity { calls, .. } => {
            let inner_width = ctx.body().width as usize;
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
        // No glyph and no dedicated "warning" colour — the design system
        // has neither, and its palette has nothing named for a transient
        // retry. `label` keeps it a quiet, informational fact rather than
        // inventing a colour outside that fixed vocabulary.
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            let line = Line::from(vec![
                Span::styled("retry ", Style::default().fg(pal.label).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · attempt {} · {status}: {}", info.provider, info.attempt, info.message), Style::default().fg(pal.dim)),
            ]);
            with_label_column(vec![line], None)
        }
        // While pending (`resolution: None`), a decision renders nothing at
        // all here — the decision panel (`super::decision`, a fixed
        // full-width band above the input) is the only place an unresolved
        // request is interactive, per the design system's own "Permission
        // prompt" screen. Once resolved it still renders here — the log
        // remains the permanent record. Not laid out under the label
        // column: a decision card is its own full-width panel-styled block,
        // not conversational turn content.
        LogEntry::ApprovalCard { diff, resolution: Some(approved), .. } => super::decision::approval_card(diff, Some(*approved), Vec::new(), ctx),
        LogEntry::PermissionPrompt { payload, resolution: Some(r), .. } => super::decision::prompt_card(payload, Some(r.as_str()), Vec::new(), ctx),
        LogEntry::ApprovalCard { resolution: None, .. } | LogEntry::PermissionPrompt { resolution: None, .. } => Vec::new(),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            let line = match reason {
                TurnEndReasonKind::EndTurn => return vec![],
                TurnEndReasonKind::Cancelled => Line::from(Span::styled("— turn cancelled —", Style::default().fg(pal.dim))),
                TurnEndReasonKind::Error(message) => Line::from(Span::styled(format!("— turn ended in error: {message} —"), Style::default().fg(pal.dim))),
            };
            with_label_column(vec![line], None)
        }
        LogEntry::Error { message } => with_label_column(
            vec![Line::from(vec![
                Span::styled("error: ", Style::default().fg(pal.del).add_modifier(Modifier::BOLD)),
                Span::styled(message.clone(), Style::default().fg(pal.del)),
            ])],
            None,
        ),
        LogEntry::Notice { message } => with_label_column(
            vec![Line::from(vec![Span::styled("notice: ", Style::default().fg(pal.quiet)), Span::styled(message.clone(), Style::default().fg(pal.dim))])],
            None,
        ),
    }
}

/// Builds the `harness` turn's content — never indented itself; the caller
/// lays the whole result out under the label column via
/// `with_label_column`, so every box built here (diff, code block) sizes
/// itself against the body column, not the full panel width, or it would
/// overflow past the right edge once that column is added back.
fn render_assistant_text(text: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let body = ctx.body();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in markdown::split_code_fences(text) {
        match segment {
            // No fill of its own — `Prose.jsx` is plain coloured text on
            // the panel ground, no background.
            Segment::Prose(s) => lines.extend(s.lines().flat_map(|l| wrap_line(markdown::render_line(l, body), body.width as usize))),
            // A fenced ```diff block gets the same full-width red/green
            // per-line treatment (line-number gutter included) as the Edit
            // approval card, instead of the generic code-block box below —
            // the diff renderer already exists precisely for "show a diff"
            // (`InlineDiff.jsx`'s own job), so this reuses it rather than
            // inventing a second diff presentation. No row budget: the log
            // scrolls, so nothing here has to fit a fixed band.
            Segment::Code { lang, body: diff_text } if lang.eq_ignore_ascii_case("diff") => {
                let (_, parsed) = diff::parse_body(&diff_text);
                lines.extend(diff::boxed(&diff::number_lines(parsed), diff::Budget::default(), Row::boxed(pal.diff_box), body));
            }
            // A real filled code-block box, with a dim language label
            // instead of the fence's own literal ` ``` ` markers, on
            // `diff_box` — the design system's one nested-quote surface,
            // already carrying the inline diff for the same reason (a
            // quoted block inside prose). `highlight_lines` picks the
            // matching half of the `base16-ocean` pair from the app theme,
            // so the surface follows the palette like every other one.
            Segment::Code { lang, body: code } => {
                let row = Row::card(pal.diff_box);
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.extend(row.text(&label, pal.label, body));
                lines.push(row.blank(body));
                for code_line in highlight::highlight_lines(&lang, &code, pal.theme) {
                    let spans: Vec<Span<'static>> = code_line.into_iter().map(|s| Span::styled(s.content, s.style.bg(pal.diff_box))).collect();
                    lines.extend(row.build(spans, body));
                }
                lines.push(row.blank(body));
            }
        }
    }
    lines
}

/// Mirrors mjolnir-cli's own `/`-prefix check (`text.trim_start().strip_prefix('/')`
/// in `slash.rs`) — this crate can't depend on that one to reuse it
/// directly (cli depends on tui, not the other way around), so the rule is
/// duplicated; keep the two in sync if it ever changes.
fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

/// The welcome hero's content: a sentence of body prose, then `model` /
/// `version` / `commit` / `access` facts on the transcript's own 12-cell
/// label-column convention (`Turn.jsx`'s label gutter, echoed here since
/// this hero has no art to sit beside).
///
/// It replaces a former hand-traced Braille hammer/FIGlet wordmark outright,
/// per the design system's own explicit rule: "No logo. No mark was supplied
/// and none was invented... every mark is a Unicode box-drawing or block
/// character," and its Assets section is blunter still — "None. No images,
/// no icons." The harness's identity now lives only in the top bar (plain
/// "mjolnir" text, no glyph).
pub(super) fn intro_content(status: &StatusInfo, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let stat_label = Style::default().fg(pal.label);
    let stat_value = Style::default().fg(pal.value);
    const LEFT_MARGIN: &str = "   ";

    let mut content: Vec<Line<'static>> = Vec::with_capacity(8);
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("every strike is yours to call. nothing moves without you.", Style::default().fg(pal.body))]));
    content.push(Line::default());
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("model    ", stat_label), Span::styled(status.model_name.clone(), stat_value)]));
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("version  ", stat_label), Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), stat_value)]));
    content.push(Line::from(vec![Span::raw(LEFT_MARGIN), Span::styled("commit   ", stat_label), Span::styled(env!("MJOLNIR_GIT_HASH"), stat_value)]));
    let mut access = vec![Span::raw(LEFT_MARGIN), Span::styled("access   ", stat_label)];
    for (i, (label, state)) in [("read", status.read), ("shell", status.shell), ("edit", status.edit)].into_iter().enumerate() {
        if i > 0 {
            access.push(Span::raw("  "));
        }
        access.extend(access_spans(label, state, ctx));
    }
    content.push(Line::from(access));
    content
}

/// One `label: state` pair in the hero's access row. No filled chip — the
/// design system's own rule is that the accent is "a mark or a line, never
/// a filled field," and none of its components use a background-filled
/// badge for a state word; add/del (green/red) plain text already reads as
/// allow/deny at a glance.
fn access_spans(label: &str, state: PermState, ctx: Ctx) -> Vec<Span<'static>> {
    let (word, word_fg) = match state {
        PermState::Allowed => ("allow", ctx.pal.add),
        PermState::Denied => ("deny", ctx.pal.del),
    };
    vec![Span::styled(format!("{label}:"), Style::default().fg(ctx.pal.label)), Span::styled(format!("{word} "), Style::default().fg(word_fg))]
}

/// Vertically centres [`intro_content`] within the log panel's inner
/// `height`. On a terminal short enough that the content doesn't fit,
/// `pad_top` saturates to 0 and the content simply starts at the top and
/// scrolls like any other tall log content would.
fn hero_lines(status: &StatusInfo, height: u16, ctx: Ctx) -> Vec<Line<'static>> {
    let content = intro_content(status, ctx);
    let pad_top = (height as usize).saturating_sub(content.len()) / 2;
    let mut lines = Vec::with_capacity(pad_top + content.len());
    lines.extend(std::iter::repeat_with(Line::default).take(pad_top));
    lines.extend(content);
    lines
}
