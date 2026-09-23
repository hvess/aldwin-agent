//! The launch card: the brand mark beside four facts, on every launch and
//! after `/clear`. `LaunchCard.jsx`: `padding: 0 var(--body-x)`, the mark,
//! a `--body-x` gap, then `Aldwin  1.0` and the `Project`/`Branch`/`Model`
//! rows with their labels in a `--fact-col` field. Two blank rows above it.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::grid::{column, BODY_X, FACT_COL};
use crate::app::App;
use crate::tokens::{MARK_CELL, MARK_COLS, MARK_ROWS};

/// Rows the card's facts take: the name row and three facts.
const FACT_ROWS: usize = 4;

/// `Blank / Blank / LaunchCard` — the two rows above the card in the frame.
const ROWS_ABOVE: usize = 2;

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let lines = lines(app);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// The card's rows, the mark's six with the facts centred beside them.
pub(super) fn lines(app: &App) -> Vec<Line<'static>> {
    let pal = app.theme.palette();
    let mark = pal.mark();
    let facts = facts(app);
    // `align-items: center`: four fact rows against six mark rows leaves
    // one blank mark row above and below them.
    let offset = MARK_ROWS.saturating_sub(FACT_ROWS) / 2;

    let mut lines: Vec<Line<'static>> = (0..ROWS_ABOVE).map(|_| Line::default()).collect();
    for (r, row) in mark.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(MARK_COLS + 3);
        spans.push(Span::raw(" ".repeat(BODY_X)));
        for (top, bottom) in row {
            spans.push(Span::styled(MARK_CELL.to_string(), Style::default().fg(*top).bg(*bottom)));
        }
        if let Some(fact) = r.checked_sub(offset).and_then(|i| facts.get(i)) {
            spans.push(Span::raw(" ".repeat(BODY_X)));
            spans.extend(fact.iter().cloned());
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// `Aldwin  0.4.0`, then the three facts — labels in `label2` in the
/// `--fact-col` field, values in `label`.
fn facts(app: &App) -> Vec<Vec<Span<'static>>> {
    let pal = app.theme.palette();
    let label = |s: &str| Span::styled(column(s, FACT_COL), Style::default().fg(pal.label2));
    let value = |s: String| Span::styled(s, Style::default().fg(pal.label));
    let status = &app.status;
    let model = if status.model_name.is_empty() { "not set".to_string() } else { status.model_name.clone() };
    vec![
        vec![
            Span::styled("Aldwin", Style::default().fg(pal.label).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", status.version), Style::default().fg(pal.label2)),
        ],
        vec![label("Project"), value(status.project.clone())],
        vec![label("Branch"), value(status.branch.clone().unwrap_or_else(|| "none".into()))],
        vec![label("Model"), value(model)],
    ]
}
