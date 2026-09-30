//! The design's `PlanStep` marks, and frame P's plan card: the plan docked
//! above the field on `--tint` while the turn has something staged.

use std::iter::once;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use aldwin_core::{PlanStep, StepState};

use super::grid::{column, elide, justified, truncate_spans, Ctx, MARGIN_X, MARK_COL};
use super::markdown::parse_inline;
use super::question::OPTION_INSET;
use crate::palette::Palette;
use crate::review::StagedFile;

/// Frame P's title row, flush right.
const DRAFT: &str = "Draft, nothing saved";

/// A step's glyph and its colour, the same in the conversation and the card.
pub(super) fn mark(state: StepState, pal: &Palette) -> (&'static str, Color) {
    match state {
        StepState::Done => ("✓", pal.accent),
        StepState::Running => ("●", pal.amber),
        StepState::Pending => ("○", pal.label3),
    }
}

/// The note under a running step; a finished or waiting one has none drawn.
fn shown_note(step: &PlanStep) -> Option<&str> {
    (step.state == StepState::Running)
        .then_some(step.note.as_deref())
        .flatten()
}

/// Rows the card takes; must match the rows `draw_card` pushes.
pub(super) fn card_rows(steps: &[PlanStep]) -> u16 {
    let notes = steps.iter().filter_map(shown_note).count();
    (4 + steps.len() + notes) as u16
}

/// Frame P: a blank row, the title with `Draft, nothing saved`, a blank row,
/// each step with its file and staged counts, and a blank row, on `--tint`
/// `MARGIN_X` in from each side.
pub(super) fn draw_card(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    steps: &[PlanStep],
    staged: &[StagedFile],
    pal: &Palette,
) {
    let inner = Rect {
        x: area.x + MARGIN_X as u16,
        width: area.width.saturating_sub(2 * MARGIN_X as u16),
        ..area
    };
    let on_tint = Style::default().bg(pal.tint);
    frame.render_widget(Block::new().style(Style::default().bg(pal.win)), area);
    frame.render_widget(Block::new().style(on_tint), inner);
    // Rows are inset `OPTION_INSET` (`padding:0 1ch`) and hold an empty
    // mark column at their right end, as the frame's do.
    let width = (inner.width as usize).saturating_sub(2 * OPTION_INSET + MARK_COL);
    let ctx = Ctx::new(pal, width as u16);

    let status = Span::styled(DRAFT, Style::default().fg(pal.label2));
    let title = elide(title, width.saturating_sub(MARK_COL + DRAFT.width() + 1));
    let mut lines = vec![
        Line::default(),
        inset(justified(
            vec![
                Span::raw(" ".repeat(MARK_COL)),
                Span::styled(title, Style::default().fg(pal.label)),
            ],
            vec![status],
            width,
        )),
        Line::default(),
    ];
    lines.extend(steps.iter().flat_map(|step| {
        let note = shown_note(step).map(|note| {
            let mut spans = vec![Span::raw(" ".repeat(MARK_COL))];
            spans.extend(parse_inline(note, Style::default().fg(pal.label2), ctx));
            inset(Line::from(truncate_spans(spans, width)))
        });
        once(inset(step_row(step, staged, width, pal))).chain(note)
    }));
    lines.push(Line::default());
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_tint), inner);
}

fn inset(line: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(OPTION_INSET))];
    spans.extend(line.spans);
    Line::from(spans)
}

/// `● Turn away requests over the limit      router.rs  +6 −1`. Only the
/// running step is in `label`; the rest step back to `label3` (frame P).
fn step_row(step: &PlanStep, staged: &[StagedFile], width: usize, pal: &Palette) -> Line<'static> {
    let (glyph, glyph_fg) = mark(step.state, pal);
    let (text_fg, file_fg) = match step.state {
        StepState::Running => (pal.label, pal.label2),
        StepState::Done | StepState::Pending => (pal.label3, pal.label3),
    };
    let mut right = Vec::new();
    if let Some(file) = &step.file {
        right.push(Span::styled(file.clone(), Style::default().fg(file_fg)));
        // A name two staged files answer to gets no counts rather than
        // the wrong file's.
        let mut named = staged.iter().filter(|s| s.is(file));
        if let (Some(counts), None) = (named.next(), named.next()) {
            right.extend(counts_spans(counts, pal));
        }
    }
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let text = elide(&step.text, width.saturating_sub(MARK_COL + right_w + 1));
    justified(
        vec![
            Span::styled(column(glyph, MARK_COL), Style::default().fg(glyph_fg)),
            Span::styled(text, Style::default().fg(text_fg)),
        ],
        right,
        width,
    )
}

/// `  +6 −1`; a zero count is left out, so a new file reads `+48`.
fn counts_spans(counts: &StagedFile, pal: &Palette) -> Vec<Span<'static>> {
    [(counts.added, '+', pal.add), (counts.removed, '−', pal.del)]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .enumerate()
        .flat_map(|(i, (n, sign, fg))| {
            let gap = if i == 0 { "  " } else { " " };
            [
                Span::raw(gap),
                Span::styled(format!("{sign}{n}"), Style::default().fg(fg)),
            ]
        })
        .collect()
}
