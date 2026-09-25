//! The question panel and the command list — the design's `QuestionPanel`
//! and `CommandRow`, both over the one [`crate::list::List`].
//!
//! A question: a blank row, the question in weight 600, one line of why in
//! `label2`, a blank row, the numbered options, a blank row — all on
//! `--panel`, in place of the field and inset by the margin as the field
//! is. The current option sits on `--field` with the accent `›` in the mark
//! column and a `label2` number; the other numbers are `label3`.
//!
//! The commands: the same panel, directly above the field — a blank row,
//! the matching commands inset as the options are, a blank row. Each name
//! fills the `--command-col` column with no slash, what is typed of it in
//! `label` and the rest in `label2`, then its purpose. The current row sits
//! on `--field` with the `›` and its purpose in `label`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use aldwin_core::Question;

use super::grid::{column, elide, justified, Ctx, COMMAND_COL, MARGIN_X, MARK_COL, NUMBER_COL};
use super::row::Row;
use super::wrap::wrap_line;
use crate::app::CommandMenu;
use crate::list::List;
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
    (width as usize)
        .saturating_sub(2 * (MARGIN_X + PANEL_PAD))
        .max(1)
}

/// Rows the panel takes at `width`: a blank row, the question and its
/// detail wrapped, a blank row — and with `options`, each option and a
/// blank row after them. Without, it is the question alone, as it stays
/// above the field while it is answered in words.
pub(super) fn panel_rows(question: &Question, options: Option<&List>, width: u16) -> u16 {
    let text_width = text_width(width);
    let heading = wrap_line(Line::from(question.question.clone()), text_width).len();
    let detail = if question.detail.is_empty() {
        0
    } else {
        wrap_line(Line::from(question.detail.clone()), text_width).len()
    };
    let options = options.map_or(0, |list| list.rows.len() + 1);
    (1 + heading + detail + 1 + options) as u16
}

pub(super) fn draw_panel(
    frame: &mut Frame,
    area: Rect,
    question: &Question,
    options: Option<&List>,
    pal: &Palette,
) {
    let inner = Rect {
        x: area.x + MARGIN_X as u16,
        width: area.width.saturating_sub(2 * MARGIN_X as u16),
        ..area
    };
    let on_panel = Style::default().bg(pal.panel);
    frame.render_widget(Block::new().style(Style::default().bg(pal.win)), area);
    frame.render_widget(Block::new().style(on_panel), inner);
    let ctx = Ctx::new(pal, inner.width);
    let text_width = text_width(area.width);

    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    let heading = Line::from(Span::styled(
        question.question.clone(),
        Style::default()
            .fg(pal.label)
            .bg(pal.panel)
            .add_modifier(Modifier::BOLD),
    ));
    lines.extend(
        wrap_line(heading, text_width)
            .into_iter()
            .map(|l| padded(l, pal.panel)),
    );
    if !question.detail.is_empty() {
        let detail = Line::from(Span::styled(
            question.detail.clone(),
            Style::default().fg(pal.label2).bg(pal.panel),
        ));
        lines.extend(
            wrap_line(detail, text_width)
                .into_iter()
                .map(|l| padded(l, pal.panel)),
        );
    }
    lines.push(Line::default());
    if let Some(list) = options {
        for (i, row) in list.rows.iter().enumerate() {
            lines.push(option_row(
                i,
                &row.label,
                &row.detail,
                i == list.selected,
                ctx,
                pal.panel,
            ));
        }
        lines.push(Line::default());
    }
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), inner);
}

fn padded(mut line: Line<'static>, bg: Color) -> Line<'static> {
    line.spans.insert(
        0,
        Span::styled(" ".repeat(PANEL_PAD), Style::default().bg(bg)),
    );
    line
}

/// `› 1  Yes, limit them by address` — the option row, built across the
/// panel's width (`ctx.width`) and inset `OPTION_INSET` inside it. The
/// current one is on `--field` with the accent `›` and a `label2` number;
/// every other has an empty mark column, a `label3` number and `label2`
/// text. A row with a detail (a provider's purpose, a session's date)
/// carries it as a fact: `label2`, right-flush where the panel's text
/// column ends.
fn option_row(
    i: usize,
    label: &str,
    detail: &str,
    current: bool,
    ctx: Ctx,
    ground: Color,
) -> Line<'static> {
    let pal = ctx.pal;
    let bg = if current { pal.field } else { ground };
    let text_fg = if current { pal.label } else { pal.label2 };
    let number_fg = if current { pal.label2 } else { pal.label3 };
    let row = Row::band(bg, ground).inset(OPTION_INSET);
    // The fact ends where the question's text does: the panel pads
    // `PANEL_PAD` and this row is already `OPTION_INSET` in.
    let width = (ctx.width as usize).saturating_sub(2 * OPTION_INSET + (PANEL_PAD - OPTION_INSET));
    let mark = if current {
        Span::styled(column("›", MARK_COL), Style::default().fg(pal.accent))
    } else {
        Span::raw(" ".repeat(MARK_COL))
    };
    let label = elide(label, width.saturating_sub(MARK_COL + NUMBER_COL));
    let room = width.saturating_sub(MARK_COL + NUMBER_COL + label.width() + 2);
    let left = vec![
        mark,
        Span::styled(
            column(&(i + 1).to_string(), NUMBER_COL),
            Style::default().fg(number_fg),
        ),
        Span::styled(label, Style::default().fg(text_fg)),
    ];
    let fact = if detail.is_empty() || room <= 3 {
        Vec::new()
    } else {
        vec![Span::styled(
            detail.to_string(),
            Style::default().fg(pal.label2),
        )]
    };
    one_row(row, justified(left, fact, width).spans, ctx)
}

/// One row of `row`'s surface. Every caller elides its content to fit, so
/// it never wraps; the blank fallback is for an empty build only.
fn one_row(row: Row, spans: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
    row.build(spans, ctx)
        .into_iter()
        .next()
        .unwrap_or_else(|| row.blank(ctx))
}

pub(super) fn draw_commands(frame: &mut Frame, area: Rect, menu: &CommandMenu, pal: &Palette) {
    let inner = Rect {
        x: area.x + MARGIN_X as u16,
        width: area.width.saturating_sub(2 * MARGIN_X as u16),
        ..area
    };
    let on_panel = Style::default().bg(pal.panel);
    frame.render_widget(Block::new().style(Style::default().bg(pal.win)), area);
    frame.render_widget(Block::new().style(on_panel), inner);
    let ctx = Ctx::new(pal, inner.width);
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    lines.extend(menu.list.rows.iter().enumerate().map(|(i, row)| {
        command_row(
            &row.label,
            &row.detail,
            &menu.filter,
            i == menu.list.selected,
            ctx,
        )
    }));
    lines.push(Line::default());
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), inner);
}

/// Rows the command panel takes: a blank row, the rows, a blank row.
pub(super) fn commands_rows(menu: &CommandMenu) -> u16 {
    menu.list.rows.len() as u16 + 2
}

/// `› changes     Everything changed since you started` — built across the
/// panel's width and inset `OPTION_INSET` inside it, like an option. The
/// name fills `--command-col`, `typed` of it in `label` and the rest in
/// `label2`; the purpose follows, `label` on the current row and `label2`
/// on the others, with the frame's empty `--mark-col` held at the end.
fn command_row(name: &str, purpose: &str, typed: &str, current: bool, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let bg = if current { pal.field } else { pal.panel };
    let row = Row::band(bg, pal.panel).inset(OPTION_INSET);
    let mark = if current {
        Span::styled(column("›", MARK_COL), Style::default().fg(pal.accent))
    } else {
        Span::raw(" ".repeat(MARK_COL))
    };
    let bare = name.strip_prefix('/').unwrap_or(name);
    let rest = bare.strip_prefix(typed).unwrap_or(bare);
    let typed = &bare[..bare.len() - rest.len()];
    let room = (ctx.width as usize).saturating_sub(2 * OPTION_INSET + 2 * MARK_COL + COMMAND_COL);
    let spans = vec![
        mark,
        Span::styled(typed.to_string(), Style::default().fg(pal.label)),
        Span::styled(
            column(rest, COMMAND_COL.saturating_sub(typed.width())),
            Style::default().fg(pal.label2),
        ),
        Span::styled(
            elide(purpose, room),
            Style::default().fg(if current { pal.label } else { pal.label2 }),
        ),
    ];
    one_row(row, spans, ctx)
}
