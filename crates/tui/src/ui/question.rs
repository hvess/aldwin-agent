//! The question panel and the command list — the design's `QuestionPanel`
//! and `CommandRow`, both over the one [`crate::list::List`].
//!
//! A question: a blank row, the question in weight 600, one line of why in
//! `label2`, a blank row, the numbered options, a blank row — all on
//! `--panel`, in place of the field and inset by the margin as the field
//! is. The current option sits on `--field` with the accent `›` in the mark
//! column and a `label2` number; the other numbers are `label3`.
//!
//! The commands: rows on the window ground above the field, `/name` in the
//! `--command-col` field (the slash in accent), the purpose in `label2`;
//! the current row on `--tint` with the `›`, between the margins.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::grid::{column, elide, justified, Ctx, COMMAND_COL, MARGIN_X, MARK_COL, NUMBER_COL};
use super::row::Row;
use super::wrap::wrap_line;
use crate::app::{Asking, CommandMenu};
use crate::palette::Palette;

/// Frame E's `QuestionPanel`: the panel is a band inset by `MARGIN_X`
/// (`margin:0 3ch`), its question and why padded `PANEL_PAD` further in
/// (`padding:0 3ch`, so they land on cell 6), and each option row inset
/// `OPTION_INSET` inside the panel (`margin:0 1ch`) — its mark on cell 4,
/// its number on cell 6, its answer on cell 9.
///
/// Written here rather than generated: `tokens/layout.css` has no token for
/// either — frame E states them as bare `ch` values. Baseline
/// `question-panel-insets-are-untokenised` records the gap; when the design
/// names them, these become `tokens.rs` constants like every other measure.
pub(super) const PANEL_PAD: usize = 3;
pub(super) const OPTION_INSET: usize = 1;

/// Cells the question and its why wrap to: the window less the panel's two
/// margins and its two paddings.
fn text_width(width: u16) -> usize {
    (width as usize).saturating_sub(2 * (MARGIN_X + PANEL_PAD)).max(1)
}

/// Rows the panel takes at `width`: the fixed five plus the options, with
/// the question and its detail wrapped.
pub(super) fn panel_rows(asking: &Asking, width: u16) -> u16 {
    let text_width = text_width(width);
    let question = wrap_line(Line::from(asking.question.question.clone()), text_width).len();
    let detail = if asking.question.detail.is_empty() { 0 } else { wrap_line(Line::from(asking.question.detail.clone()), text_width).len() };
    (1 + question + detail + 1 + asking.list.rows.len() + 1) as u16
}

pub(super) fn draw_panel(frame: &mut Frame, area: Rect, asking: &Asking, pal: &Palette) {
    let inner = Rect { x: area.x + MARGIN_X as u16, width: area.width.saturating_sub(2 * MARGIN_X as u16), ..area };
    let on_panel = Style::default().bg(pal.panel);
    frame.render_widget(Block::new().style(Style::default().bg(pal.win)), area);
    frame.render_widget(Block::new().style(on_panel), inner);
    let ctx = Ctx::new(pal, inner.width);
    let text_width = text_width(area.width);

    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    let question = Line::from(Span::styled(asking.question.question.clone(), Style::default().fg(pal.label).bg(pal.panel).add_modifier(Modifier::BOLD)));
    lines.extend(wrap_line(question, text_width).into_iter().map(|l| padded(l, pal.panel)));
    if !asking.question.detail.is_empty() {
        let detail = Line::from(Span::styled(asking.question.detail.clone(), Style::default().fg(pal.label2).bg(pal.panel)));
        lines.extend(wrap_line(detail, text_width).into_iter().map(|l| padded(l, pal.panel)));
    }
    lines.push(Line::default());
    for (i, row) in asking.list.rows.iter().enumerate() {
        lines.push(option_row(i, &row.label, &row.detail, i == asking.list.selected, ctx, pal.panel));
    }
    lines.push(Line::default());
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), inner);
}

fn padded(mut line: Line<'static>, bg: ratatui::style::Color) -> Line<'static> {
    line.spans.insert(0, Span::styled(" ".repeat(PANEL_PAD), Style::default().bg(bg)));
    line
}

/// `› 1  Yes, limit them by address` — the option row, built across the
/// panel's width (`ctx.width`) and inset `OPTION_INSET` inside it. The
/// current one is on `--field` with the accent `›` and a `label2` number;
/// every other has an empty mark column, a `label3` number and `label2`
/// text. A row with a detail (a provider's purpose, a session's date)
/// carries it as a fact: `label2`, right-flush where the panel's text
/// column ends.
fn option_row(i: usize, label: &str, detail: &str, current: bool, ctx: Ctx, ground: ratatui::style::Color) -> Line<'static> {
    let pal = ctx.pal;
    let bg = if current { pal.field } else { ground };
    let text_fg = if current { pal.label } else { pal.label2 };
    let number_fg = if current { pal.label2 } else { pal.label3 };
    let row = Row::band(bg, ground).inset(OPTION_INSET);
    // The fact ends where the question's text does: the panel pads
    // `PANEL_PAD` and this row is already `OPTION_INSET` in.
    let width = (ctx.width as usize).saturating_sub(2 * OPTION_INSET + (PANEL_PAD - OPTION_INSET));
    let mark = if current { Span::styled(column("›", MARK_COL), Style::default().fg(pal.accent)) } else { Span::raw(" ".repeat(MARK_COL)) };
    let label = elide(label, width.saturating_sub(MARK_COL + NUMBER_COL));
    let room = width.saturating_sub(MARK_COL + NUMBER_COL + label.width() + 2);
    let left = vec![mark, Span::styled(column(&(i + 1).to_string(), NUMBER_COL), Style::default().fg(number_fg)), Span::styled(label, Style::default().fg(text_fg))];
    let fact = if detail.is_empty() || room <= 3 { Vec::new() } else { vec![Span::styled(detail.to_string(), Style::default().fg(pal.label2))] };
    one_row(row, justified(left, fact, width).spans, ctx)
}

/// One row of `row`'s surface. Every caller elides its content to fit, so
/// it never wraps; the blank fallback is for an empty build only.
fn one_row(row: Row, spans: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
    row.build(spans, ctx).into_iter().next().unwrap_or_else(|| row.blank(ctx))
}

pub(super) fn draw_commands(frame: &mut Frame, area: Rect, menu: &CommandMenu, pal: &Palette) {
    let ctx = Ctx::new(pal, area.width);
    let lines: Vec<Line<'static>> = menu
        .list
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| command_row(&row.label, &row.detail, i == menu.list.selected, ctx))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// `› /changes     Everything changed since you started` — the slash in
/// accent, the name in `label`, the purpose at `--command-col` in
/// `label2`. The current row on `--tint`, frame F's "idle selection":
/// the focus stays in the `/` field below, which is what holds `--field`.
fn command_row(name: &str, purpose: &str, current: bool, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let row = Row::band(if current { pal.tint } else { pal.win }, pal.win);
    let mark = if current { Span::styled(column("›", MARK_COL), Style::default().fg(pal.accent)) } else { Span::raw(" ".repeat(MARK_COL)) };
    let bare = name.strip_prefix('/').unwrap_or(name);
    let room = (ctx.width as usize).saturating_sub(2 * MARGIN_X + MARK_COL + COMMAND_COL);
    let spans = vec![
        mark,
        Span::styled("/", Style::default().fg(pal.accent)),
        Span::styled(column(bare, COMMAND_COL.saturating_sub(1)), Style::default().fg(pal.label)),
        Span::styled(elide(purpose, room), Style::default().fg(pal.label2)),
    ];
    one_row(row, spans, ctx)
}
