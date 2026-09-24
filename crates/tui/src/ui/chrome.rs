//! The frame's fixed furniture: the field, the comment field, and the
//! footer with its context bar. Everything here writes to a `Frame`
//! directly rather than returning rows — none of it scrolls.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::grid::{truncate_spans, Ctx, GROUP_GAP, MARGIN_X, MARK_COL};
use super::question;
use aldwin_core::ReviewOutcome;

use crate::app::{App, Mode};
use crate::draft;
use crate::log::LogEntry;
use crate::palette::Palette;
use crate::tokens::{gauge_filled, GAUGE_SEGMENTS};

/// The most rows the field may take, however long the draft is. Past this
/// the draft scrolls inside the band, keeping the caret in view.
pub(super) const COMPOSER_MAX_ROWS: u16 = 10;

/// The cell the drawn caret takes, held back from the draft's column so a
/// row filled to its last character still has somewhere to put it.
const CARET_LEN: u16 = 1;

/// The draft, wrapped to the field's column — measured **once** per frame
/// and used for everything downstream of that measurement.
pub(super) struct Composer {
    layout: draft::Layout,
    width:  u16,
}

impl Composer {
    /// `frame_width` less the field's two margins, the mark column, the
    /// caret's cell, and `reserve` more on the right for an action.
    pub(super) fn new(input: &str, frame_width: u16, reserve: u16) -> Self {
        let width = frame_width
            .saturating_sub(MARGIN_X as u16 * 2)
            .saturating_sub(MARK_COL as u16)
            .saturating_sub(CARET_LEN)
            .saturating_sub(reserve)
            .max(1);
        Self { layout: draft::Layout::new(input, width as usize), width }
    }

    pub(super) fn height(&self) -> u16 {
        self.layout.row_count().clamp(1, COMPOSER_MAX_ROWS as usize) as u16
    }
}

/// What the bottom band holds, and the rows it needs — decided once so the
/// layout and the draw cannot disagree.
pub(super) enum Bottom {
    /// blank / field / blank / footer / blank
    Field(Composer),
    /// blank / question / detail / blank / options / blank, then blank /
    /// footer / blank on the window ground.
    Question { rows: u16 },
    /// blank / rows / blank / field / blank / footer / blank
    Commands { rows: u16, composer: Composer },
}

impl Bottom {
    pub(super) fn measure(app: &App, width: u16) -> Self {
        match &app.mode {
            Mode::Question(asking) => Bottom::Question { rows: question::panel_rows(asking, width) },
            Mode::Commands(menu) => Bottom::Commands { rows: menu.list.rows.len() as u16, composer: Composer::new(&app.input, width, 0) },
            Mode::Conversation | Mode::Review(_) => Bottom::Field(Composer::new(&app.input, width, 0)),
        }
    }

    pub(super) fn height(&self) -> u16 {
        match self {
            Bottom::Field(c) => c.height() + 4,
            Bottom::Question { rows } => rows + 3,
            Bottom::Commands { rows, composer } => rows + composer.height() + 5,
        }
    }

    pub(super) fn draw(self, frame: &mut Frame, area: Rect, app: &mut App) {
        match self {
            Bottom::Field(composer) => {
                let [_, field, _, footer, _] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(composer.height()),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                draw_field(frame, field, app, &composer, None);
                draw_footer(frame, footer, app);
            }
            Bottom::Question { rows } => {
                let [panel, _, footer, _] =
                    Layout::vertical([Constraint::Length(rows), Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)]).areas(area);
                if let Mode::Question(asking) = &app.mode {
                    question::draw_panel(frame, panel, asking, app.theme.palette());
                }
                draw_footer(frame, footer, app);
            }
            Bottom::Commands { rows, composer } => {
                let [_, list, _, field, _, footer, _] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(rows),
                    Constraint::Length(1),
                    Constraint::Length(composer.height()),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                if let Mode::Commands(menu) = &app.mode {
                    question::draw_commands(frame, list, menu, app.theme.palette());
                }
                draw_field(frame, field, app, &composer, None);
                draw_footer(frame, footer, app);
            }
        }
    }
}

/// The field's right-hand action: `Approve  ⌃↩`, `Send 1 Comment  ⌃↩`.
/// Grey until it can run.
pub(super) struct Action {
    pub label: String,
    pub key:   &'static str,
    pub ready: bool,
}

impl Action {
    pub fn width(&self) -> u16 {
        // label, two spaces, key, and the 1-cell pad the frame keeps.
        (self.label.width() + 2 + self.key.width() + 1) as u16
    }
}

/// The field: `margin: 0 3ch`, on `--field`, the accent `›` in the mark
/// column, then the draft, with an optional action flush right. An empty
/// field is the `›` and the caret and nothing else — no placeholder in any
/// state. In the commands mode the draft is the `/` and the filter.
pub(super) fn draw_field(frame: &mut Frame, area: Rect, app: &mut App, composer: &Composer, action: Option<Action>) {
    let pal = app.theme.palette();
    let field = Style::default().bg(pal.field);
    let inner = Rect { x: area.x + MARGIN_X as u16, width: area.width.saturating_sub(MARGIN_X as u16 * 2), ..area };
    frame.render_widget(Block::new().style(field), inner);

    let prompt = |glyph: &str| Span::styled(format!("{glyph:<width$}", width = MARK_COL), Style::default().fg(pal.accent).bg(pal.field));

    // The commands mode: `/` in the mark column like the `›` it replaces,
    // whatever was typed after it on the body column, the caret after that.
    if let Mode::Commands(menu) = &app.mode {
        let line = Line::from(vec![
            prompt("/"),
            Span::styled(menu.filter.clone(), Style::default().fg(pal.label).bg(pal.field)),
            caret(pal, app.tick),
        ]);
        frame.render_widget(Paragraph::new(line).style(field), inner);
        return;
    }

    let action_spans = action.as_ref().map(|a| {
        let fg = if a.ready { pal.accent } else { pal.label3 };
        vec![Span::styled(format!("{}  ", a.label), Style::default().fg(fg).bg(pal.field)), Span::styled(a.key, Style::default().fg(fg).bg(pal.field)), Span::styled(" ", field)]
    });
    let action_width = action.as_ref().map_or(0, Action::width);

    if app.input.is_empty() {
        app.composer_top = 0;
        let mut spans = vec![prompt("›"), caret(pal, app.tick)];
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        if let Some(action) = action_spans {
            let gap = (inner.width as usize).saturating_sub(used).saturating_sub(action_width as usize);
            spans.push(Span::styled(" ".repeat(gap), field));
            spans.extend(action);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).style(field), inner);
        return;
    }

    let layout = &composer.layout;
    app.composer_width = composer.width;
    let (cursor_row, cursor_col) = layout.position(app.cursor);
    let height = inner.height.max(1) as usize;
    let last_top = layout.row_count().saturating_sub(height);
    let mut top = app.composer_top.min(last_top);
    top = top.min(cursor_row);
    top = top.max((cursor_row + 1).saturating_sub(height));
    app.composer_top = top;

    let ctx = Ctx::new(pal, inner.width);
    let mut lines: Vec<Line<'static>> = (top..(top + height).min(layout.row_count()))
        .map(|i| {
            let text = layout.row_text(i);
            let mut line = if i == cursor_row { caret_row(&text, cursor_col, ctx, app.tick) } else { Line::from(Span::styled(text, Style::default().fg(pal.label).bg(pal.field))) };
            line.spans.insert(0, if i == 0 { prompt("›") } else { Span::styled(" ".repeat(MARK_COL), field) });
            line
        })
        .collect();

    // The action, or `4 lines`, flush right on the first row — whole or
    // not at all.
    if let Some(first) = lines.first_mut() {
        let used: usize = first.spans.iter().map(|s| s.content.width()).sum();
        let room = (inner.width as usize).saturating_sub(used);
        if let Some(action) = action_spans {
            if room >= GROUP_GAP + action_width as usize {
                first.spans.push(Span::styled(" ".repeat(room - action_width as usize), field));
                first.spans.extend(action);
            }
        } else if layout.row_count() > 1 {
            let count = format!("{} lines ", layout.row_count());
            if room >= GROUP_GAP + count.width() {
                first.spans.push(Span::styled(" ".repeat(room - count.width()), field));
                first.spans.push(Span::styled(count, Style::default().fg(pal.label2).bg(pal.field)));
            }
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)).style(field), inner);
}

/// The caret: a `label` block, blinking on the tick (`--caret-period`
/// 1.05s, stepped — ~9 ticks on, 9 off at 120ms).
fn caret(pal: &Palette, tick: u64) -> Span<'static> {
    let on = (tick / 9).is_multiple_of(2);
    let bg = if on { pal.label } else { pal.field };
    Span::styled(" ", Style::default().bg(bg))
}

/// One draft row with the caret drawn into it at display column `col`.
fn caret_row(text: &str, col: usize, ctx: Ctx, tick: u64) -> Line<'static> {
    let pal = ctx.pal;
    let style = Style::default().fg(pal.label).bg(pal.field);
    let mut out: Vec<Span<'static>> = Vec::new();
    let (mut before, mut after) = (String::new(), String::new());
    let mut under = None;
    let mut at = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(1);
        if at < col {
            before.push(c);
        } else if under.is_none() {
            under = Some(c);
        } else {
            after.push(c);
        }
        at += w;
    }
    if !before.is_empty() {
        out.push(Span::styled(before, style));
    }
    match under {
        // Mid-text the caret takes the cell of the character it sits on.
        Some(c) => {
            let on = (tick / 9).is_multiple_of(2);
            let s = if on { Style::default().fg(pal.field).bg(pal.label) } else { style };
            out.push(Span::styled(c.to_string(), s));
            if let Some(pad) = c.width().map(|w| w.saturating_sub(1)).filter(|&p| p > 0) {
                out.push(Span::styled(" ".repeat(pad), style));
            }
        }
        None => out.push(caret(pal, tick)),
    }
    if !after.is_empty() {
        out.push(Span::styled(after, style));
    }
    Line::from(out)
}

/// A footer key: glyph, two spaces, verb.
pub(super) struct KeyHint {
    pub glyph: &'static str,
    pub verb:  &'static str,
}

impl KeyHint {
    const fn new(glyph: &'static str, verb: &'static str) -> Self {
        Self { glyph, verb }
    }
}

/// The footer's leading status: a word, or `● Working…` with the running
/// amber dot in the mark column.
enum Status {
    Ready,
    Working,
    Waiting,
    /// The review: no status word; the row opens with the keys.
    None,
}

/// What the footer says now — only the keys that work in this state.
struct Footer {
    status: Status,
    /// The keys of the moment, after the status word.
    keys:   Vec<KeyHint>,
    /// `/  Commands`: not a key of the moment but the way to everything
    /// else, and frame A sets it apart — right-flush, one group gap before
    /// the context bar.
    aside:  Option<KeyHint>,
}

impl Footer {
    fn new(status: Status, keys: Vec<KeyHint>) -> Self {
        Self { status, keys, aside: None }
    }
}

fn footer_state(app: &App) -> Footer {
    let details = || if app.details_open { KeyHint::new("Space", "Hide Details") } else { KeyHint::new("Space", "Show Details") };
    match &app.mode {
        Mode::Question(_) => Footer::new(Status::Waiting, vec![KeyHint::new("↑↓", "Choose"), KeyHint::new("↩", "Select")]),
        Mode::Commands(_) => Footer::new(Status::Ready, vec![KeyHint::new("↩", "Run"), KeyHint::new("⎋", "Close")]),
        // Shift and Tab are words, as Space is: the glyph table has no mark
        // for either, and "if it is not in the table, do not draw one."
        Mode::Review(r) if r.keys_shown => {
            let mut keys = vec![KeyHint::new("↑↓", "Scroll"), KeyHint::new("Shift ↑↓", "Select")];
            if r.file().has_folds() {
                keys.push(KeyHint::new("Space", "Show All Lines"));
            }
            keys.extend([
                KeyHint::new("↩", "Comment"),
                KeyHint::new("⌃↩", "Approve"),
                KeyHint::new("Tab", "Next file"),
                KeyHint::new("⎋", "Discard"),
            ]);
            Footer::new(Status::None, keys)
        }
        Mode::Review(_) => Footer::new(Status::None, vec![KeyHint::new("?", "Keys")]),
        Mode::Conversation if app.turn_active || app.awaiting_turn => {
            let mut keys = vec![KeyHint::new("⎋", "Stop")];
            if app.input.is_empty() && has_details(app) {
                keys.push(details());
            }
            Footer::new(Status::Working, keys)
        }
        Mode::Conversation if app.answering.is_some() => Footer::new(Status::Waiting, vec![KeyHint::new("↩", "Send")]),
        Mode::Conversation if !app.input.is_empty() => Footer::new(Status::Ready, vec![KeyHint::new("↩", "Send")]),
        // Frame J: after a turn that saved, the footer is the context bar
        // alone — `↺  Undo` is not offered (baseline `frame-j-offers-undo`)
        // and nothing takes its place.
        Mode::Conversation if just_saved(app) => Footer::new(Status::None, Vec::new()),
        Mode::Conversation => {
            let keys = if has_details(app) { vec![details()] } else { Vec::new() };
            Footer { status: Status::Ready, keys, aside: Some(KeyHint::new("/", "Commands")) }
        }
    }
}

/// The last turn ended in an approve: it holds a `Saved` review row.
fn just_saved(app: &App) -> bool {
    app.this_turn().iter().any(|e| matches!(e, LogEntry::Review { outcome: ReviewOutcome::Saved { .. } }))
}

fn has_details(app: &App) -> bool {
    app.this_turn().iter().any(|e| matches!(e, LogEntry::Work { .. } | LogEntry::Failure { detail: Some(_), .. }))
}

/// The footer: `padding: 0 3ch`, in `label2`. The status in the mark column
/// and after it, the key groups `--group-gap` apart, and the context bar
/// flush right.
pub(super) fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    let Footer { status, keys, aside } = footer_state(app);
    let dim = Style::default().fg(pal.label2);
    // Every footer glyph is `label2`, as every footer in frames A–J draws
    // it: the accent is for the action at the field's right edge, which is
    // the one that is ready, not for a key the footer merely names.
    let group = |key: KeyHint| vec![Span::styled(key.glyph, dim), Span::styled(format!("  {}", key.verb), dim)];

    let mut groups: Vec<Vec<Span<'static>>> = Vec::new();
    match status {
        Status::Ready => groups.push(vec![Span::raw(" ".repeat(MARK_COL)), Span::styled("Ready", dim)]),
        Status::Waiting => groups.push(vec![Span::raw(" ".repeat(MARK_COL)), Span::styled("Waiting for you", dim)]),
        Status::Working => {
            // The dot blinks with the tick, as the caret does — the one
            // thing the design animates besides it.
            groups.push(vec![Span::styled(format!("{:<width$}", "●", width = MARK_COL), Style::default().fg(pal.amber)), Span::styled("Working…", dim)])
        }
        Status::None => groups.push(vec![Span::raw(" ".repeat(MARK_COL))]),
    }
    groups.extend(keys.into_iter().map(group));

    let mut left: Vec<Span<'static>> = Vec::new();
    for (i, group) in groups.into_iter().enumerate() {
        if i > 0 {
            left.push(Span::raw(" ".repeat(GROUP_GAP)));
        }
        left.extend(group);
    }
    // The first group is the status, whose leading spaces are the mark
    // column; a `Status::None` footer's first key sits right after them.
    if let (Some(first), Some(second)) = (left.first().cloned(), left.get(1).cloned()) {
        if first.content.trim().is_empty() && second.content.trim().is_empty() {
            left.remove(1);
        }
    }

    // The context bar is never cut, and the status and the keys of the
    // moment come before the aside: when the row cannot hold all three, the
    // aside goes first — the way to the commands is `/` whether it is
    // named or not.
    let width = area.width as usize;
    let bar = context_bar(app.status.context_percent(), pal);
    let span_w = |spans: &[Span]| spans.iter().map(|s| s.content.width()).sum::<usize>();
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(aside) = aside {
        let aside = group(aside);
        let fits = span_w(&left) + GROUP_GAP + span_w(&aside) + GROUP_GAP + span_w(&bar) <= width.saturating_sub(MARGIN_X * 2);
        if fits {
            right.extend(aside);
            right.push(Span::raw(" ".repeat(GROUP_GAP)));
        }
    }
    right.extend(bar);
    let right_w = span_w(&right);
    let budget = width.saturating_sub(MARGIN_X * 2).saturating_sub(GROUP_GAP).saturating_sub(right_w);
    let left = truncate_spans(left, budget);
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let gap = width.saturating_sub(MARGIN_X * 2).saturating_sub(used).saturating_sub(right_w);

    let mut line = vec![Span::raw(" ".repeat(MARGIN_X))];
    line.extend(left);
    line.push(Span::raw(" ".repeat(gap)));
    line.extend(right);
    frame.render_widget(Paragraph::new(Line::from(line)), area);
}

/// `Context ━━━━━━━━━━ 41%` — ten segments, the filled run ramping to
/// full accent at its leading edge, the rest on `--track`. With nothing
/// measured yet the bar is drawn empty at `0%`, as the launch frame does.
pub(super) fn context_bar(percent: Option<u8>, pal: &Palette) -> Vec<Span<'static>> {
    let pct = percent.unwrap_or(0);
    let filled = gauge_filled(pct);
    let colours = pal.gauge(filled);
    let mut spans = vec![Span::styled("Context ", Style::default().fg(pal.label2))];
    for colour in colours.iter().take(GAUGE_SEGMENTS) {
        spans.push(Span::styled("━", Style::default().fg(*colour)));
    }
    spans.push(Span::styled(format!(" {pct}%"), Style::default().fg(pal.label2)));
    spans
}

/// The comment field, two rows in place of the field while a selection is
/// being commented on: the selection label on `--select`, the draft on
/// `--field`, both with the accent `▎` edge.
pub(super) fn draw_comment_field(frame: &mut Frame, area: Rect, app: &App, lines: &str, location: &str, draft: &str, cursor: usize) {
    let pal = app.theme.palette();
    let inner = Rect { x: area.x + MARGIN_X as u16, width: area.width.saturating_sub(MARGIN_X as u16 * 2), ..area };
    let [label_row, draft_row] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    let on_select = Style::default().bg(pal.select);
    let edge = |bg: Color| Span::styled("▎", Style::default().fg(pal.accent).bg(bg));
    let left = vec![
        edge(pal.select),
        Span::styled("Commenting on ", Style::default().fg(pal.label).bg(pal.select)),
        Span::styled(lines.to_string(), Style::default().fg(pal.label).bg(pal.select).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {location}"), Style::default().fg(pal.label2).bg(pal.select)),
    ];
    let right = vec![Span::styled("esc ", Style::default().fg(pal.label2).bg(pal.select))];
    frame.render_widget(Block::new().style(on_select), label_row);
    frame.render_widget(Paragraph::new(justify(left, right, inner.width as usize, pal.select)).style(on_select), label_row);

    let on_field = Style::default().bg(pal.field);
    frame.render_widget(Block::new().style(on_field), draft_row);
    let ctx = Ctx::new(pal, inner.width);
    let mut row = caret_row(draft, cursor, ctx, app.tick);
    row.spans.insert(0, edge(pal.field));
    let used: usize = row.spans.iter().map(|s| s.content.width()).sum();
    let gap = (inner.width as usize).saturating_sub(used).saturating_sub(2);
    row.spans.push(Span::styled(" ".repeat(gap), on_field));
    row.spans.push(Span::styled("↩ ", Style::default().fg(pal.accent).bg(pal.field)));
    frame.render_widget(Paragraph::new(row).style(on_field), draft_row);
}

fn justify(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize, bg: Color) -> Line<'static> {
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let left = if left_w + right_w + 1 > width { truncate_spans(left, width.saturating_sub(right_w + 1)) } else { left };
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let mut spans = left;
    spans.push(Span::styled(" ".repeat(width.saturating_sub(left_w).saturating_sub(right_w)), Style::default().bg(bg)));
    spans.extend(right);
    Line::from(spans)
}
