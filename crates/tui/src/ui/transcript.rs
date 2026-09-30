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
use super::plan;
use super::row::Row;
use super::wrap::wrap_line;
use aldwin_core::{ReviewOutcome, StepState};

use crate::app::App;
use crate::draft::expand_tabs;
use crate::log::{plural, summarise_work, Act, LogEntry, Took, WorkItem};
use crate::palette::Theme;

/// An entry's rendered rows, each exactly one screen row (see
/// [`Transcript`]), after a blank row unless `first`: the first entry that
/// rendered anything gets none, and an entry that renders nothing gets none.
fn spaced(rendered: Vec<Line<'static>>, first: bool) -> Vec<Line<'static>> {
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

/// An entry whose text streams in: a fixed head, then a body rendered from
/// the text. The body up to a boundary renders the same however the text
/// goes on, so a re-render keeps those rows and renders only the rest.
struct Streaming<'a> {
    text: &'a str,
    head: Vec<Line<'static>>,
    body: fn(&str, Ctx) -> Vec<Line<'static>>,
    /// The last offset in the text where its body can be split.
    boundary: fn(&str) -> usize,
}

/// The rows of a streaming text that no later delta can change: `bytes` of
/// its text rendered into the block's first `rows` rows.
#[derive(Debug, Clone, Copy)]
struct Settled {
    bytes: usize,
    rows: usize,
}

/// What one entry draws: finished rows, or a text still streaming in.
enum Rendered<'a> {
    Rows(Vec<Line<'static>>),
    Stream(Streaming<'a>),
}

/// Just past the last blank line outside a code fence: fences and tables
/// never span one, and every other line renders alone (`markdown`).
fn prose_boundary(text: &str) -> usize {
    let (mut at, mut boundary, mut fenced) = (0, 0, false);
    for line in text.split_inclusive('\n') {
        at += line.len();
        if !line.ends_with('\n') {
            break;
        }
        if fenced {
            fenced = !markdown::fence_closes(line);
        } else if markdown::fence_opens(line).is_some() {
            fenced = true;
        } else if line.trim().is_empty() {
            boundary = at;
        }
    }
    boundary
}

/// `stream`'s rows after `first`'s spacing, reusing the first `settled.rows`
/// of `kept` (rendered from the same first `settled.bytes`); and how much is
/// settled now.
fn stream_rows(
    stream: &Streaming,
    first: bool,
    ctx: Ctx,
    kept: Option<(Vec<Line<'static>>, Settled)>,
) -> (Vec<Line<'static>>, Option<Settled>) {
    // `spaced`'s rule, applied in place: prepending would move every
    // settled row on every frame.
    let spacing = usize::from(!first);
    let head = at_body(stream.head.clone());
    let (mut rows, from) = match kept {
        Some((mut rows, settled)) => {
            rows.truncate(settled.rows);
            rows.splice(spacing..spacing + head.len(), head);
            (rows, settled.bytes)
        }
        None => {
            let mut rows = vec![Line::default(); spacing];
            rows.extend(head);
            (rows, 0)
        }
    };
    let rest = &stream.text[from..];
    let split = from + (stream.boundary)(rest);
    rows.extend(at_body((stream.body)(&stream.text[from..split], ctx)));
    let settled = Settled {
        bytes: split,
        rows: rows.len(),
    };
    rows.extend(at_body((stream.body)(&stream.text[split..], ctx)));
    if rows.len() == spacing {
        return (Vec::new(), None);
    }
    (rows, Some(settled))
}

/// The transcript's screen rows, cached per log entry.
///
/// A row here is a row on screen: nothing downstream wraps, so
/// [`Transcript::len`] is the conversation's height and
/// `ScrollState::offset` indexes straight into it.
///
/// Per-entry, not one flat list: a streaming reply changes the last entry
/// per token, and re-rendering everything cost 58% of a core at four turns.
/// [`Transcript::sync`] looks only at entries from the first one the log
/// says changed, and re-renders those that differ by `==` from their cached
/// copy; a streaming text keeps its settled rows ([`Settled`]).
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
    /// For a streaming entry, the rows its next render keeps.
    settled: Option<Settled>,
}

impl CachedBlock {
    fn render(entry: &LogEntry, first: bool, ctx: Ctx, old: Option<&mut CachedBlock>) -> Self {
        let (rows, settled) = match render_entry(entry, ctx) {
            Rendered::Stream(stream) => {
                let kept = old.and_then(|old| {
                    let settled = old.settled?;
                    // Settled only while streaming, so this renders no body: an
                    // open `Work`'s head (its earlier acts) is all it builds.
                    // The head is spliced in place, so it must keep its height:
                    // a new thought in the same `Work` changes it.
                    let same_prefix = old.first == first
                        && std::mem::discriminant(&old.entry) == std::mem::discriminant(entry)
                        && matches!(render_entry(&old.entry, ctx), Rendered::Stream(was)
                            if was.head.len() == stream.head.len()
                            && stream.text.as_bytes().get(..settled.bytes)
                                == was.text.as_bytes().get(..settled.bytes));
                    same_prefix.then(|| (std::mem::take(&mut old.rows), settled))
                });
                stream_rows(&stream, first, ctx, kept)
            }
            Rendered::Rows(rows) => (spaced(rows, first), None),
        };
        CachedBlock {
            entry: entry.clone(),
            first,
            rows,
            settled,
        }
    }
}

impl Transcript {
    /// Brings the cache up to date with `app` at `width`, trusting every
    /// entry before `changed` (`Log::take_changed`) to be as cached. A width
    /// or theme change drops the whole cache.
    pub(crate) fn sync(&mut self, app: &App, width: u16, changed: usize) {
        let theme = app.theme;
        if self.width != width || self.theme != Some(theme) {
            self.blocks.clear();
            self.width = width;
            self.theme = Some(theme);
        }
        let ctx = Ctx::new(theme.palette(), width);
        self.rebuilt = 0;
        let queued = (!app.queued.is_empty()).then(|| LogEntry::Queued {
            messages: app.queued.clone(),
        });
        self.blocks
            .truncate(app.log.len() + usize::from(queued.is_some()));

        // The queued messages sit past the log's end, so they are always
        // looked at.
        let start = changed.min(app.log.len()).min(self.blocks.len());
        let mut first = match start.checked_sub(1).map(|i| &self.blocks[i]) {
            Some(before) => before.first && before.rows.is_empty(),
            None => true,
        };
        for (i, entry) in app.log.iter().chain(&queued).enumerate().skip(start) {
            let hit =
                matches!(self.blocks.get(i), Some(b) if b.first == first && b.entry == *entry);
            if !hit {
                self.rebuilt += 1;
                let block = CachedBlock::render(entry, first, ctx, self.blocks.get_mut(i));
                match self.blocks.get_mut(i) {
                    Some(slot) => *slot = block,
                    None => self.blocks.push(block),
                }
            }
            if !self.blocks[i].rows.is_empty() {
                first = false;
            }
        }

        self.starts.truncate(start + 1);
        let mut acc = self.starts.last().copied().unwrap_or(0);
        if self.starts.is_empty() {
            self.starts.push(acc);
        }
        for block in &self.blocks[start..] {
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

fn render_entry<'a>(entry: &'a LogEntry, ctx: Ctx) -> Rendered<'a> {
    let pal = ctx.pal;
    Rendered::Rows(match entry {
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
        LogEntry::AssistantText { text } => {
            return Rendered::Stream(Streaming {
                text,
                head: Vec::new(),
                body: |text, ctx| render_assistant_text(text, ctx.body()),
                boundary: prose_boundary,
            })
        }
        // `Disclosure` and its `DetailRow`s, on the prose column so a fact
        // ends where prose does; a thought's row is followed by its
        // reasoning, unabridged (ADR 0015, ADR 0018).
        LogEntry::Work { acts, open } => {
            let head = Line::from(vec![
                Span::styled(summarise_work(acts), Style::default().fg(pal.label2)),
                disclosure_glyph(*open, ctx),
            ]);
            if !*open {
                return Rendered::Rows(at_body(vec![head]));
            }
            let width = ctx.body().width as usize;
            let mut lines = vec![head];
            // A thought still streaming is the last act: its reasoning is
            // the stream, everything above it the fixed head.
            let (done, streaming) = match acts.split_last() {
                Some((Act::Thought { text, took }, done)) if *took == Took::Running => {
                    (done, Some(text))
                }
                _ => (&acts[..], None),
            };
            for act in done {
                match act {
                    Act::Call(item) => lines.push(call_row(item, width, ctx)),
                    Act::Thought { text, took } => {
                        lines.push(thought_row(*took, width, ctx));
                        lines.extend(disclosed(text.lines(), width, ctx));
                    }
                }
            }
            if let Some(text) = streaming {
                lines.push(thought_row(Took::Running, width, ctx));
                return Rendered::Stream(Streaming {
                    text,
                    head: lines,
                    body: |text, ctx| disclosed(text.lines(), ctx.body().width as usize, ctx),
                    boundary: |text| text.rfind('\n').map_or(0, |i| i + 1),
                });
            }
            at_body(lines)
        }
        // Drawn by the card above the field instead (frame P).
        LogEntry::Plan { docked: true, .. } => Vec::new(),
        // `PlanStep`, frame B.
        LogEntry::Plan { steps, .. } => steps
            .iter()
            .map(|step| {
                let (glyph, glyph_fg) = plan::mark(step.state, pal);
                let text_fg = match step.state {
                    StepState::Running => pal.label,
                    StepState::Done | StepState::Pending => pal.label2,
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
        // A second blank row: `spaced` already adds one before it.
        LogEntry::TurnBreak => vec![Line::default()],
        // Frame K: the echo's band, all `label3`, `○` and `Queued` on its
        // first row only.
        LogEntry::Queued { messages } => {
            let row = Row::band(pal.tint, pal.win);
            let style = Style::default().fg(pal.label3).bg(pal.tint);
            messages
                .iter()
                .flat_map(|m| m.lines())
                .enumerate()
                .flat_map(|(i, l)| {
                    let text = Span::styled(l.to_string(), style);
                    if i == 0 {
                        let glyph = Span::styled(format!("{:<MARK_COL$}", "○"), style);
                        let tag = Span::styled("Queued", style);
                        row.build_tagged(glyph, text, tag, ctx)
                    } else {
                        let indent = Span::styled(" ".repeat(MARK_COL), style);
                        row.build_indented(vec![indent, text], MARK_COL, ctx)
                    }
                })
                .collect()
        }
    })
}

/// The glyph after a disclosure's summary, in its `label2`: `›` closed, `⌄`
/// open.
fn disclosure_glyph(open: bool, ctx: Ctx) -> Span<'static> {
    let glyph = if open { "⌄" } else { "›" };
    Span::styled(format!("  {glyph}"), Style::default().fg(ctx.pal.label2))
}

/// A call's `DetailRow`: verb, target, and its fact right-flush.
fn call_row(item: &WorkItem, width: usize, ctx: Ctx) -> Line<'static> {
    let fact = item.fact.clone().unwrap_or_else(|| "…".into());
    let left = vec![
        Span::styled(
            column(item.verb.word(), DETAIL_COL),
            Style::default().fg(ctx.pal.label2),
        ),
        Span::styled(
            elide(
                &item.target,
                width.saturating_sub(DETAIL_COL + fact.width() + 2),
            ),
            Style::default().fg(ctx.pal.code),
        ),
    ];
    let right = vec![Span::styled(fact, Style::default().fg(ctx.pal.label2))];
    justified(left, right, width)
}

/// A thought's `DetailRow`: `Thought` and its time right-flush (`…` while
/// it runs), the reasoning below it.
fn thought_row(took: Took, width: usize, ctx: Ctx) -> Line<'static> {
    let time = match took {
        Took::Running => "…".into(),
        took => took.time().unwrap_or_default(),
    };
    let style = Style::default().fg(ctx.pal.label2);
    justified(
        vec![Span::styled(column("Thought", DETAIL_COL), style)],
        vec![Span::styled(time, style)],
        width,
    )
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
