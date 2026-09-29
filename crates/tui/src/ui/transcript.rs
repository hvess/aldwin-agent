//! The conversation log: each `LogEntry` rendered to screen rows, and the
//! per-entry cache that keeps them.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::grid::{
    at_body, column, elide, justified, marked, Ctx, BODY_X, DETAIL_COL, MARGIN_X, MARK_COL,
};
use super::markdown::{self, Segment};
use super::row::Row;
use super::wrap::wrap_line;
use aldwin_core::{ReviewOutcome, StepState};

use crate::app::App;
use crate::draft::expand_tabs;
use crate::log::{plural, summarise_work, LogEntry};
use crate::palette::Theme;

/// One entry's rows at `ctx.width`; each line must be exactly one screen
/// row (see [`Transcript`]). `first` marks the first entry that rendered
/// anything, which gets no leading blank row.
fn block_rows(entry: &LogEntry, first: bool, ctx: Ctx) -> Vec<Line<'static>> {
    let rendered = render_entry(entry, ctx);
    if rendered.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    // "Vertical spacing is blank rows, never padding": one between groups.
    if !first {
        lines.push(Line::default());
    }
    lines.extend(rendered);
    lines
}

/// The transcript's screen rows, cached per log entry.
///
/// A row here is a row on screen: nothing downstream wraps, so
/// [`Transcript::len`] is the conversation's height and
/// `ScrollState::offset` indexes straight into it.
///
/// Per-entry, not one flat list: a streaming reply changes the last entry
/// per token, and re-rendering everything cost 58% of a core at four turns.
/// [`Transcript::sync`] re-renders only entries that differ by `==` from
/// their cached copy.
#[derive(Debug, Default)]
pub(crate) struct Transcript {
    width: u16,
    theme: Option<Theme>,
    blocks: Vec<CachedBlock>,
    /// `starts[i]` is the screen row `blocks[i]` begins on; one longer than
    /// `blocks`, so the last element is the total row count.
    starts: Vec<usize>,
    /// Blocks the last [`Transcript::sync`] rebuilt; tests assert on it.
    rebuilt: usize,
}

#[derive(Debug)]
struct CachedBlock {
    entry: LogEntry,
    first: bool,
    rows: Vec<Line<'static>>,
}

impl Transcript {
    /// Brings the cache up to date with `app` at `width`. A width or theme
    /// change drops the whole cache.
    pub(crate) fn sync(&mut self, app: &App, width: u16) {
        let theme = app.theme;
        if self.width != width || self.theme != Some(theme) {
            self.blocks.clear();
            self.width = width;
            self.theme = Some(theme);
        }
        let ctx = Ctx::new(theme.palette(), width);
        self.rebuilt = 0;
        self.blocks.truncate(app.log.len());

        let mut first = true;
        for (i, entry) in app.log.iter().enumerate() {
            let hit =
                matches!(self.blocks.get(i), Some(b) if b.first == first && b.entry == *entry);
            if !hit {
                self.rebuilt += 1;
                let block = CachedBlock {
                    entry: entry.clone(),
                    first,
                    rows: block_rows(entry, first, ctx),
                };
                match self.blocks.get_mut(i) {
                    Some(slot) => *slot = block,
                    None => self.blocks.push(block),
                }
            }
            if !self.blocks[i].rows.is_empty() {
                first = false;
            }
        }

        self.starts.clear();
        self.starts.reserve(self.blocks.len() + 1);
        let mut acc = 0;
        self.starts.push(acc);
        for block in &self.blocks {
            acc += block.rows.len();
            self.starts.push(acc);
        }
    }

    #[cfg(test)]
    pub(crate) fn rebuilt(&self) -> usize {
        self.rebuilt
    }

    /// Total screen rows; `ScrollState` measures its offset against it.
    pub(crate) fn len(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    /// Up to `height` rows starting at row `offset`.
    pub(crate) fn viewport(&self, offset: usize, height: usize) -> Vec<Line<'static>> {
        let mut i = self
            .starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1)
            .min(self.blocks.len());
        let mut row = offset.saturating_sub(self.starts.get(i).copied().unwrap_or(0));
        let mut out = Vec::with_capacity(height.min(self.len().saturating_sub(offset)));
        while out.len() < height && i < self.blocks.len() {
            let rows = &self.blocks[i].rows;
            while row < rows.len() && out.len() < height {
                out.push(rows[row].clone());
                row += 1;
            }
            i += 1;
            row = 0;
        }
        out
    }
}

/// Renders rows already sliced from the [`Transcript`], from the top. No
/// scrollbar: the design has none.
pub(super) fn draw_log(frame: &mut Frame, area: Rect, visible: Vec<Line<'static>>) {
    frame.render_widget(Paragraph::new(Text::from(visible)), area);
}

fn render_entry(entry: &LogEntry, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    match entry {
        // `UserEcho`: `label3` and `label2`, not blue: it is past input, not
        // the current prompt.
        LogEntry::UserMessage { text } => {
            let row = Row::band(pal.tint, pal.win);
            let mut lines = Vec::new();
            for (i, l) in text.lines().enumerate() {
                let glyph = if i == 0 { "›" } else { "" };
                let spans = vec![
                    Span::styled(
                        format!("{glyph:<width$}", width = MARK_COL),
                        Style::default().fg(pal.label3).bg(pal.tint),
                    ),
                    Span::styled(l.to_string(), Style::default().fg(pal.label2).bg(pal.tint)),
                ];
                lines.extend(row.build_indented(spans, MARK_COL, ctx));
            }
            if lines.is_empty() {
                lines.push(row.blank(ctx));
            }
            lines
        }
        LogEntry::AssistantText { text } => at_body(render_assistant_text(text, ctx.body())),
        // A `Disclosure` whose detail is the reasoning, unabridged (ADR 0015).
        LogEntry::Thinking { text, took, open } => {
            let mut lines = vec![Line::from(vec![
                Span::styled(took.summary(), Style::default().fg(pal.label2)),
                disclosure_glyph(*open, ctx),
            ])];
            if *open {
                lines.extend(disclosed(text.lines(), ctx.body().width as usize, ctx));
            }
            at_body(lines)
        }
        // `Disclosure` and its `DetailRow`s, on the prose column so a fact
        // ends where prose does.
        LogEntry::Work { items, open } => {
            let width = ctx.body().width as usize;
            let summary = summarise_work(items);
            let head = vec![
                Span::styled(summary, Style::default().fg(pal.label2)),
                disclosure_glyph(*open, ctx),
            ];
            let mut lines = vec![Line::from(head)];
            if *open {
                for item in items {
                    let fact = item.fact.clone().unwrap_or_else(|| "…".into());
                    let left = vec![
                        Span::styled(
                            column(item.verb.word(), DETAIL_COL),
                            Style::default().fg(pal.label2),
                        ),
                        Span::styled(
                            elide(
                                &item.target,
                                width.saturating_sub(DETAIL_COL + fact.width() + 2),
                            ),
                            Style::default().fg(pal.code),
                        ),
                    ];
                    let right = vec![Span::styled(fact, Style::default().fg(pal.label2))];
                    lines.push(justified(left, right, width));
                }
            }
            at_body(lines)
        }
        // `PlanStep`, frame B.
        LogEntry::Plan { steps } => steps
            .iter()
            .map(|step| {
                let (glyph, glyph_fg, text_fg) = match step.state {
                    StepState::Done => ("✓", pal.accent, pal.label2),
                    StepState::Running => ("●", pal.amber, pal.label),
                    StepState::Pending => ("○", pal.label3, pal.label2),
                };
                // Frame B's `padding: 0 3ch`: the right edge is `MARGIN_X`,
                // not prose's `BODY_X`.
                let text = elide(
                    &step.text,
                    (ctx.width as usize).saturating_sub(BODY_X + MARGIN_X),
                );
                marked(
                    Span::styled(glyph, Style::default().fg(glyph_fg)),
                    vec![Span::styled(text, Style::default().fg(text_fg))],
                )
            })
            .collect(),
        // An unanswered question is drawn by the question panel instead.
        LogEntry::Question {
            question,
            answer: Some(answer),
        } => {
            let line = Line::from(vec![
                Span::styled(question.clone(), Style::default().fg(pal.label2)),
                Span::styled(" · ", Style::default().fg(pal.label3)),
                Span::styled(answer.clone(), Style::default().fg(pal.label)),
            ]);
            at_body(wrap_line(line, ctx.body().width as usize))
        }
        LogEntry::Question { answer: None, .. } => Vec::new(),
        // The `✓` is accent: the save was the developer's.
        LogEntry::Review { outcome } => match outcome {
            ReviewOutcome::Saved {
                files,
                comments_resolved,
            } => {
                let mut content = vec![Span::styled(
                    format!("Saved {}", plural(files.len(), "file")),
                    Style::default().fg(pal.label),
                )];
                if *comments_resolved > 0 {
                    content.push(Span::styled(
                        format!(" · {} resolved", plural(*comments_resolved, "comment")),
                        Style::default().fg(pal.label2),
                    ));
                }
                vec![marked(
                    Span::styled("✓", Style::default().fg(pal.accent)),
                    content,
                )]
            }
            ReviewOutcome::Discarded { files } => {
                let text = format!("Nothing saved; {} discarded.", plural(files.len(), "file"));
                at_body(vec![Line::from(Span::styled(
                    text,
                    Style::default().fg(pal.label2),
                ))])
            }
            ReviewOutcome::Commented { comments } => {
                let text = format!("Sent {}.", plural(*comments, "comment"));
                at_body(vec![Line::from(Span::styled(
                    text,
                    Style::default().fg(pal.label2),
                ))])
            }
        },
        LogEntry::Notice { message } | LogEntry::Stopping { message } => at_body(wrap_line(
            Line::from(Span::styled(
                message.clone(),
                Style::default().fg(pal.label2),
            )),
            ctx.body().width as usize,
        )),
        // ADR 0009 §5: a failure is a sentence in `label`; no glyph, no hue.
        LogEntry::Failure {
            message,
            detail,
            open,
        } => {
            let width = ctx.body().width as usize;
            let mut lines = wrap_line(
                Line::from(Span::styled(
                    message.clone(),
                    Style::default().fg(pal.label),
                )),
                width,
            );
            if let Some(detail) = detail {
                if let Some(first) = lines.first_mut() {
                    first.spans.push(disclosure_glyph(*open, ctx));
                }
                if *open {
                    lines.extend(disclosed(detail.lines().take(40), width, ctx));
                }
            }
            at_body(lines)
        }
        // A second blank row: `block_rows` already adds one before it.
        LogEntry::TurnBreak => vec![Line::default()],
    }
}

/// The glyph after a disclosure's summary, in its `label2`: `›` closed, `⌄`
/// open.
fn disclosure_glyph(open: bool, ctx: Ctx) -> Span<'static> {
    let glyph = if open { "⌄" } else { "›" };
    Span::styled(format!("  {glyph}"), Style::default().fg(ctx.pal.label2))
}

/// A disclosure's text in `label2`, each line wrapped to `width`.
fn disclosed<'a>(
    lines: impl Iterator<Item = &'a str>,
    width: usize,
    ctx: Ctx,
) -> Vec<Line<'static>> {
    lines
        .flat_map(|l| {
            wrap_line(
                Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(ctx.pal.label2),
                )),
                width,
            )
        })
        .collect()
}

/// The agent's markdown prose, at the body column's width.
fn render_assistant_text(text: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in markdown::split_code_fences(text) {
        match segment {
            Segment::Prose(s) => lines.extend(markdown::render_prose(&s, ctx)),
            Segment::Code { lang, body: code } => lines.extend(code_block(&lang, &code, ctx)),
        }
    }
    lines
}

/// A fenced code block: a caption row, then one row per code line in
/// `--code` on `tint`, elided rather than wrapped so a line stays a line.
fn code_block(lang: &str, code: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let rows: Vec<&str> = code.lines().collect();
    let caption = if lang.is_empty() { "code" } else { lang };
    let count = plural(rows.len(), "line");
    let mut lines = vec![justified(
        vec![Span::styled(
            caption.to_string(),
            Style::default().fg(pal.label3),
        )],
        vec![Span::styled(count, Style::default().fg(pal.label3))],
        ctx.width as usize,
    )];
    let field = Row::field(pal.tint).pad(1);
    for row in rows {
        let text = elide(&expand_tabs(row), (ctx.width as usize).saturating_sub(2));
        lines.push(
            field
                .build(
                    vec![Span::styled(
                        text,
                        Style::default().fg(pal.code).bg(pal.tint),
                    )],
                    ctx,
                )
                .remove(0),
        );
    }
    lines
}
