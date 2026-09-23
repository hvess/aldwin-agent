//! The question panel and the command list — the design's `QuestionPanel`
//! and `CommandRow`, both over the one [`crate::list::List`].
//!
//! A question: a blank row, the question in weight 600, one line of why in
//! `label2`, a blank row, the numbered options, a blank row — all on
//! `--panel`, in place of the field. The current option sits on `--field`
//! with the accent `›` in the mark column; the numbers are `label3`.
//!
//! The commands: rows on the window ground above the field, `/name` in the
//! `--command-col` field (the slash in accent), the purpose in `label2`;
//! the current row on `--field` with the `›`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::grid::{column, elide, Ctx, BODY_X, COMMAND_COL, MARGIN_X, MARK_COL, NUMBER_COL};
use super::wrap::wrap_line;
use crate::app::{Asking, CommandMenu};
use crate::palette::Palette;

/// Rows the panel takes at `width`: the fixed five plus the options, with
/// the question and its detail wrapped.
pub(super) fn panel_rows(asking: &Asking, width: u16) -> u16 {
    let text_width = (width as usize).saturating_sub(BODY_X + MARGIN_X).max(1);
    let question = wrap_line(Line::from(asking.question.question.clone()), text_width).len();
    let detail = if asking.question.detail.is_empty() { 0 } else { wrap_line(Line::from(asking.question.detail.clone()), text_width).len() };
    (1 + question + detail + 1 + asking.list.rows.len() + 1) as u16
}

pub(super) fn draw_panel(frame: &mut Frame, area: Rect, asking: &Asking, pal: &Palette) {
    let on_panel = Style::default().bg(pal.panel);
    frame.render_widget(Block::new().style(on_panel), area);
    let ctx = Ctx::new(pal, area.width);
    let text_width = (area.width as usize).saturating_sub(BODY_X + MARGIN_X).max(1);

    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    let question = Line::from(Span::styled(asking.question.question.clone(), Style::default().fg(pal.label).bg(pal.panel).add_modifier(Modifier::BOLD)));
    lines.extend(wrap_line(question, text_width).into_iter().map(|l| at_body(l, pal.panel)));
    if !asking.question.detail.is_empty() {
        let detail = Line::from(Span::styled(asking.question.detail.clone(), Style::default().fg(pal.label2).bg(pal.panel)));
        lines.extend(wrap_line(detail, text_width).into_iter().map(|l| at_body(l, pal.panel)));
    }
    lines.push(Line::default());
    for (i, row) in asking.list.rows.iter().enumerate() {
        lines.push(option_row(i, &row.label, &row.detail, i == asking.list.selected, ctx, pal.panel));
    }
    lines.push(Line::default());
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), area);
}

fn at_body(mut line: Line<'static>, bg: ratatui::style::Color) -> Line<'static> {
    line.spans.insert(0, Span::styled(" ".repeat(BODY_X), Style::default().bg(bg)));
    line
}

/// `› 1  Yes, limit them by address` — the option row. The current one is
/// on `--field` with the accent `›`; every other has an empty mark column.
/// A row with a detail (a provider's purpose, a session's date) carries it
/// after the label in `label2`.
fn option_row(i: usize, label: &str, detail: &str, current: bool, ctx: Ctx, ground: ratatui::style::Color) -> Line<'static> {
    let pal = ctx.pal;
    let bg = if current { pal.field } else { ground };
    let text_fg = if current { pal.label } else { pal.label2 };
    let mut spans = vec![Span::styled(" ".repeat(MARGIN_X), Style::default().bg(bg))];
    if current {
        spans.push(Span::styled(format!("{:<width$}", "›", width = MARK_COL), Style::default().fg(pal.accent).bg(bg)));
    } else {
        spans.push(Span::styled(" ".repeat(MARK_COL), Style::default().bg(bg)));
    }
    spans.push(Span::styled(column(&(i + 1).to_string(), NUMBER_COL), Style::default().fg(pal.label3).bg(bg)));
    let used = MARGIN_X + MARK_COL + NUMBER_COL;
    let room = (ctx.width as usize).saturating_sub(used).saturating_sub(MARGIN_X);
    let label_text = elide(label, room);
    spans.push(Span::styled(label_text.clone(), Style::default().fg(text_fg).bg(bg)));
    if !detail.is_empty() {
        let left = room.saturating_sub(label_text.width() + 2);
        if left > 3 {
            spans.push(Span::styled(format!("  {}", elide(detail, left)), Style::default().fg(pal.label2).bg(bg)));
        }
    }
    let width: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::styled(" ".repeat((ctx.width as usize).saturating_sub(width)), Style::default().bg(bg)));
    Line::from(spans)
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
/// `label2`. The current row on `--field`.
fn command_row(name: &str, purpose: &str, current: bool, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let bg = if current { pal.field } else { pal.win };
    let mut spans = vec![Span::styled(" ".repeat(MARGIN_X), Style::default().bg(bg))];
    if current {
        spans.push(Span::styled(format!("{:<width$}", "›", width = MARK_COL), Style::default().fg(pal.accent).bg(bg)));
    } else {
        spans.push(Span::styled(" ".repeat(MARK_COL), Style::default().bg(bg)));
    }
    let bare = name.strip_prefix('/').unwrap_or(name);
    spans.push(Span::styled("/", Style::default().fg(pal.accent).bg(bg)));
    spans.push(Span::styled(column(bare, COMMAND_COL.saturating_sub(1)), Style::default().fg(pal.label).bg(bg)));
    let used = MARGIN_X + MARK_COL + COMMAND_COL;
    let room = (ctx.width as usize).saturating_sub(used).saturating_sub(MARGIN_X);
    spans.push(Span::styled(elide(purpose, room), Style::default().fg(pal.label2).bg(bg)));
    let width: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::styled(" ".repeat((ctx.width as usize).saturating_sub(width)), Style::default().bg(bg)));
    Line::from(spans)
}
