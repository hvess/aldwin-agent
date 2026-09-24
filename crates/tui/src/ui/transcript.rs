//! The conversation log: one `LogEntry` at a time turned into rows, and the
//! cache that keeps them.

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

/// One entry's rows, at `ctx.width` (the body's width). Every line this
/// returns is **already one screen row**: no caller wraps afterwards, so
/// `lines.len()` *is* the row count. See [`Transcript`] for why.
///
/// `first` says this is the first entry in the log that rendered anything,
/// which decides whether the block opens with a blank row.
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

/// The transcript's screen rows, kept **one log entry at a time**.
///
/// **A row here is a row on screen.** Nothing downstream wraps, so
/// [`Transcript::len`] is exactly the number of terminal rows the
/// conversation occupies and `ScrollState::offset` indexes straight into it.
///
/// **A rebuild costs one entry, not the conversation.** A streaming reply
/// appends to the *last* log entry once per token; caching the whole flat
/// row list re-rendered everything per token — measured at 58% of a core at
/// four turns. Each entry keeps its own rows beside a copy of the entry they
/// were built from, and [`Transcript::sync`] re-renders only the entries
/// whose value changed, compared with `==`.
#[derive(Default)]
pub(crate) struct Transcript {
    width: u16,
    theme: Option<Theme>,
    blocks: Vec<CachedBlock>,
    /// `starts[i]` is the screen row `blocks[i]` begins on; one longer than
    /// `blocks`, so the last element is the total row count.
    starts: Vec<usize>,
    /// How many blocks the last [`Transcript::sync`] actually rebuilt — the
    /// incremental guarantee, countable.
    rebuilt: usize,
}

struct CachedBlock {
    entry: LogEntry,
    first: bool,
    rows: Vec<Line<'static>>,
}

impl Transcript {
    /// Brings the cache up to date with `app` at `width`, re-rendering only
    /// what changed.
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

    /// Total screen rows — what `ScrollState` measures its offset against.
    pub(crate) fn len(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    /// The rows to draw for a viewport of `height` rows starting at
    /// `offset`, or fewer at the end.
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

/// Renders the log band: the rows `super::draw` already sliced out of the
/// [`Transcript`], from the top. No scrollbar — the design lists none, and
/// the live end of the conversation is what is on screen unless you moved.
pub(super) fn draw_log(frame: &mut Frame, area: Rect, visible: Vec<Line<'static>>) {
    frame.render_widget(Paragraph::new(Text::from(visible)), area);
}

fn render_entry(entry: &LogEntry, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    match entry {
        // `UserEcho`: the request on `--tint`, `margin: 0 3ch`, a `›` in the
        // mark column in `label3` and the words in `label2` — it is what
        // you said, not what is happening.
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
        // `Prose`: plain text at `--body-x` in `label`; markdown's fences
        // and tables render, everything else is a sentence.
        LogEntry::AssistantText { text } => at_body(render_assistant_text(text, ctx.body())),
        // `Disclosure`: the summary with `⌄` open or `›` closed, both in
        // `label2`; open, the `DetailRow`s — verb in a `--detail-col` field,
        // target, fact flush right. A detail row is `padding: 0 5ch` like
        // prose, so its fact ends where prose does.
        LogEntry::Work { items, open } => {
            let width = ctx.body().width as usize;
            let summary = summarise_work(items);
            let glyph = if *open { "⌄" } else { "›" };
            let head = vec![
                Span::styled(summary, Style::default().fg(pal.label2)),
                Span::styled(format!("  {glyph}"), Style::default().fg(pal.label2)),
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
                            Style::default().fg(pal.label),
                        ),
                    ];
                    let right = vec![Span::styled(fact, Style::default().fg(pal.label2))];
                    lines.push(justified(left, right, width));
                }
            }
            at_body(lines)
        }
        // `PlanStep`: the glyph in the mark column at the margin, the
        // outcome at `--body-x`. `✓` accent over `label2`; `●` amber over
        // `label`; `○` `label3` over `label2` (frame B).
        LogEntry::Plan { steps } => steps
            .iter()
            .map(|step| {
                let (glyph, glyph_fg, text_fg) = match step.state {
                    StepState::Done => ("✓", pal.accent, pal.label2),
                    StepState::Running => ("●", pal.amber, pal.label),
                    StepState::Pending => ("○", pal.label3, pal.label2),
                };
                // A plan step is `padding: 0 3ch` (frame B), not prose.
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
        // A settled question, as one `label2` line: the question, then
        // ` · ` and what was answered. Unanswered, the panel is showing it.
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
        // What a review left behind. `✓ Saved 3 files · 1 comment
        // resolved`: the `✓` in accent — it is yours — the count in `label`,
        // the rest in `label2`.
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
        LogEntry::Notice { message } => at_body(wrap_line(
            Line::from(Span::styled(
                message.clone(),
                Style::default().fg(pal.label2),
            )),
            ctx.body().width as usize,
        )),
        // A failure is a sentence in `label`; its detail one disclosure
        // below in `label2` when open (ADR 0009 §5). No glyph, no hue.
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
                let glyph = if *open { "⌄" } else { "›" };
                if let Some(first) = lines.first_mut() {
                    first.spans.push(Span::styled(
                        format!("  {glyph}"),
                        Style::default().fg(pal.label2),
                    ));
                }
                if *open {
                    for l in detail.lines().take(40) {
                        lines.extend(wrap_line(
                            Line::from(Span::styled(
                                l.to_string(),
                                Style::default().fg(pal.label2),
                            )),
                            width,
                        ));
                    }
                }
            }
            at_body(lines)
        }
        // The blank row between turns: `block_rows` already puts one before
        // every entry, so a break is a second one.
        LogEntry::TurnBreak => vec![Line::default()],
    }
}

/// The agent's prose: markdown, at the body column. Fences render on
/// `--tint` in `label2` under a `label3` caption; tables draw (ADR 0002).
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

/// A fenced code block: a caption row — the language in `label3`, the line
/// count flush right — then the code on `--tint` in `label2`, one row a
/// line, elided rather than wrapped so a line stays a line.
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
                        Style::default().fg(pal.label2).bg(pal.tint),
                    )],
                    ctx,
                )
                .remove(0),
        );
    }
    lines
}
