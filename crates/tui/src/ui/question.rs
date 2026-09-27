//! The design's `QuestionPanel` and `CommandRow`, both over one
//! [`crate::list::List`], on `--panel` with the current row on `--field`.
//! The question panel replaces the field; the command list sits above it.

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

/// Frame E's `QuestionPanel` insets: the question and its detail are padded
/// `PANEL_PAD` inside the panel (`padding:0 3ch`), each option row inset
/// `OPTION_INSET` (`margin:0 1ch`).
///
/// Literals, not `tokens.rs`: `tokens/layout.css` has no token for them
/// (baseline `question-panel-insets-are-untokenised`). Move them to tokens
/// once the design names them.
pub(super) const PANEL_PAD: usize = 3;
pub(super) const OPTION_INSET: usize = 1;

/// Cells the question and its detail wrap to.
fn text_width(width: u16) -> usize {
    (width as usize)
        .saturating_sub(2 * (MARGIN_X + PANEL_PAD))
        .max(1)
}

/// Rows the panel takes at `width`. `options` is `None` while the question
/// is answered in words: the panel then shows the question alone.
///
/// Must match the rows `draw_panel` pushes.
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

/// `› 1  Yes, limit them by address`, with `detail` (a provider's purpose, a
/// session's date) right-flush where the panel's text column ends.
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

/// One row of `row`'s surface. Callers must elide content to fit: only the
/// first wrapped row is kept.
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

/// Rows the command panel takes; must match `draw_commands`.
pub(super) fn commands_rows(menu: &CommandMenu) -> u16 {
    menu.list.rows.len() as u16 + 2
}

/// A command row, inset like an option: the name without its `/` in
/// `--command-col` (the `typed` prefix in `label`), then the purpose, with an
/// empty `--mark-col` held at the end as in the frame.
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
