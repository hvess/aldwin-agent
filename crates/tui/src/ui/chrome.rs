//! The bottom band: the field, the comment field, and the footer with its
//! context bar. Draws to the `Frame` directly; none of it scrolls.

use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::grid::{truncate_spans, GROUP_GAP, MARGIN_X, MARK_COL};
use super::{question, working};
use aldwin_core::ReviewOutcome;

use crate::app::{App, Asker, Mode};
use crate::draft;
use crate::log::LogEntry;
use crate::motion::{ticks, Motion};
use crate::palette::Palette;
use crate::tokens::{gauge_filled, GAUGE_CELL, GAUGE_SEGMENTS};

/// The field's maximum height; a longer draft scrolls with the caret in view.
pub(super) const COMPOSER_MAX_ROWS: u16 = 10;

/// A cell reserved after the draft's column, so the caret has a place on a
/// full row.
const CARET_LEN: u16 = 1;

/// Ticks the caret shows, then hides: half of motion.css's
/// `--caret-period` (1.05s), whose keyframes are on for the first half, in
/// whole ticks (500ms).
/// Motion tokens are not generated into `tokens.rs`; keep in step by hand.
const CARET_TICKS: u64 = ticks(Duration::from_millis(525));

/// The draft wrapped to the field's column, measured once per frame.
pub(super) struct Composer {
    layout: draft::Layout,
    width: u16,
}

impl Composer {
    /// Wraps to `frame_width` less the margins, the mark column, the caret's
    /// cell, and `reserve` cells for an action.
    pub(super) fn new(input: &str, frame_width: u16, reserve: u16) -> Self {
        let width = frame_width
            .saturating_sub(MARGIN_X as u16 * 2)
            .saturating_sub(MARK_COL as u16)
            .saturating_sub(CARET_LEN)
            .saturating_sub(reserve)
            .max(1);
        Self {
            layout: draft::Layout::new(input, width as usize),
            width,
        }
    }

    pub(super) fn height(&self) -> u16 {
        self.layout.row_count().clamp(1, COMPOSER_MAX_ROWS as usize) as u16
    }
}

/// What the bottom band holds, decided once so `height` and `draw` agree.
/// Each variant's `height` must match the rows its `draw` lays out.
pub(super) enum Bottom {
    /// blank / field / blank / footer / blank.
    Field(Composer, Option<Action>),
    /// Question panel, then blank / footer / blank.
    Question { rows: u16 },
    /// blank / command panel / field / blank / footer / blank; the panel
    /// sits directly on the field (frame F).
    Commands { rows: u16, composer: Composer },
    /// The agent's question answered in words: the question panel, then
    /// blank / field / blank / footer / blank.
    Answering { rows: u16, composer: Composer },
}

impl Bottom {
    pub(super) fn measure(app: &App, width: u16) -> Self {
        let composer = Composer::new(app.draft.text(), width, 0);
        match (&app.mode, &app.answering) {
            (Mode::Question(asking), _) => Bottom::Question {
                rows: question::panel_rows(&asking.question, Some(&asking.list), width),
            },
            (Mode::Commands(menu), _) => Bottom::Commands {
                rows: question::commands_rows(menu),
                composer,
            },
            (_, Some(asking)) => Bottom::Answering {
                rows: question::panel_rows(&asking.question, None, width),
                composer,
            },
            _ => match send_action(app) {
                Some(action) => Bottom::Field(
                    Composer::new(app.draft.text(), width, action.width()),
                    Some(action),
                ),
                None => Bottom::Field(composer, None),
            },
        }
    }

    pub(super) fn height(&self) -> u16 {
        match self {
            Bottom::Field(c, _) => c.height() + 4,
            Bottom::Question { rows } => rows + 3,
            Bottom::Commands { rows, composer } => rows + composer.height() + 4,
            Bottom::Answering { rows, composer } => rows + composer.height() + 4,
        }
    }

    pub(super) fn draw(self, frame: &mut Frame, area: Rect, app: &mut App) {
        match self {
            Bottom::Field(composer, action) => {
                let [_, field, _, footer, _] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(composer.height()),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                draw_field(frame, field, app, &composer, action);
                draw_footer(frame, footer, app);
            }
            Bottom::Question { rows } => {
                let [panel, _, footer, _] = Layout::vertical([
                    Constraint::Length(rows),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                if let Mode::Question(asking) = &app.mode {
                    question::draw_panel(
                        frame,
                        panel,
                        &asking.question,
                        Some(&asking.list),
                        app.theme.palette(),
                    );
                }
                draw_footer(frame, footer, app);
            }
            Bottom::Answering { rows, composer } => {
                let [panel, _, field, _, footer, _] = Layout::vertical([
                    Constraint::Length(rows),
                    Constraint::Length(1),
                    Constraint::Length(composer.height()),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                if let Some(asking) = &app.answering {
                    question::draw_panel(frame, panel, &asking.question, None, app.theme.palette());
                }
                draw_field(frame, field, app, &composer, None);
                draw_footer(frame, footer, app);
            }
            Bottom::Commands { rows, composer } => {
                let [_, list, field, _, footer, _] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(rows),
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

/// The field's right-hand action, e.g. `Approve  ⌃↩`; accent only when
/// `ready`.
pub(super) struct Action {
    pub label: String,
    pub key: &'static str,
    pub ready: bool,
}

impl Action {
    pub fn width(&self) -> u16 {
        // Label, two spaces, key, and the frame's 1-cell pad.
        (self.label.width() + 2 + self.key.width() + 1) as u16
    }
}

/// The field: the accent `›`, the draft, and an optional action flush
/// right. No placeholder in any state. In commands mode the draft is `/`
/// and the filter, completed by the current command (frame F).
pub(super) fn draw_field(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    composer: &Composer,
    action: Option<Action>,
) {
    let pal = app.theme.palette();
    let field = Style::default().bg(pal.field);
    let inner = Rect {
        x: area.x + MARGIN_X as u16,
        width: area.width.saturating_sub(MARGIN_X as u16 * 2),
        ..area
    };
    frame.render_widget(Block::new().style(field), inner);

    let prompt = |glyph: &str| {
        Span::styled(
            format!("{glyph:<width$}", width = MARK_COL),
            Style::default().fg(pal.accent).bg(pal.field),
        )
    };

    if let Mode::Commands(menu) = &app.mode {
        let typed = format!("/{}", menu.filter);
        let typed_fg = if menu.spells_a_command() {
            pal.accent
        } else {
            pal.label
        };
        let caret_x = inner.x + (MARK_COL + typed.width()) as u16;
        let line = Line::from(vec![
            prompt("›"),
            Span::styled(typed, Style::default().fg(typed_fg).bg(pal.field)),
            Span::styled(
                menu.completion().to_string(),
                Style::default().fg(pal.label3).bg(pal.field),
            ),
        ]);
        frame.render_widget(Paragraph::new(line).style(field), inner);
        place_caret(frame, caret_x, inner.y, app.tick, app.motion);
        return;
    }

    let action_spans = action.as_ref().map(|a| {
        let fg = if a.ready { pal.accent } else { pal.label3 };
        vec![
            Span::styled(
                format!("{}  ", a.label),
                Style::default().fg(fg).bg(pal.field),
            ),
            Span::styled(a.key, Style::default().fg(fg).bg(pal.field)),
            Span::styled(" ", field),
        ]
    });
    let action_width = action.as_ref().map_or(0, Action::width);

    if app.draft.is_empty() {
        app.composer_top = 0;
        let mut spans = vec![prompt("›")];
        if let Some(action) = action_spans {
            let gap = (inner.width as usize)
                .saturating_sub(MARK_COL)
                .saturating_sub(action_width as usize);
            spans.push(Span::styled(" ".repeat(gap), field));
            spans.extend(action);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).style(field), inner);
        place_caret(
            frame,
            inner.x + MARK_COL as u16,
            inner.y,
            app.tick,
            app.motion,
        );
        return;
    }

    let layout = &composer.layout;
    app.composer_width = composer.width;
    let (cursor_row, cursor_col) = layout.position(app.draft.cursor());
    let height = inner.height.max(1) as usize;
    let last_top = layout.row_count().saturating_sub(height);
    let mut top = app.composer_top.min(last_top);
    top = top.min(cursor_row);
    top = top.max((cursor_row + 1).saturating_sub(height));
    app.composer_top = top;

    let mut lines: Vec<Line<'static>> = (top..(top + height).min(layout.row_count()))
        .map(|i| {
            let mut line = Line::from(Span::styled(
                layout.row_text(i),
                Style::default().fg(pal.label).bg(pal.field),
            ));
            line.spans.insert(
                0,
                if i == 0 {
                    prompt("›")
                } else {
                    Span::styled(" ".repeat(MARK_COL), field)
                },
            );
            line
        })
        .collect();

    // The action, or `N lines`, flush right on the first row: whole or not
    // at all.
    if let Some(first) = lines.first_mut() {
        let used: usize = first.spans.iter().map(|s| s.content.width()).sum();
        let room = (inner.width as usize).saturating_sub(used);
        if let Some(action) = action_spans {
            if room >= GROUP_GAP + action_width as usize {
                first.spans.push(Span::styled(
                    " ".repeat(room - action_width as usize),
                    field,
                ));
                first.spans.extend(action);
            }
        } else if layout.row_count() > 1 {
            let count = format!("{} lines ", layout.row_count());
            if room >= GROUP_GAP + count.width() {
                first
                    .spans
                    .push(Span::styled(" ".repeat(room - count.width()), field));
                first.spans.push(Span::styled(
                    count,
                    Style::default().fg(pal.label2).bg(pal.field),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)).style(field), inner);
    place_caret(
        frame,
        inner.x + (MARK_COL + cursor_col) as u16,
        inner.y + (cursor_row - top) as u16,
        app.tick,
        app.motion,
    );
}

/// Whether the caret is showing at `tick`: always, under reduced motion.
fn caret_on(tick: u64, motion: Motion) -> bool {
    motion == Motion::Reduced || (tick / CARET_TICKS).is_multiple_of(2)
}

/// The caret is the terminal's cursor (an accent bar, set up by `run.rs`):
/// the design's 2px bar between cells has no glyph. Blinks on
/// `--caret-period` via `caret_on`, not the terminal's own blink.
fn place_caret(frame: &mut Frame, x: u16, y: u16, tick: u64, motion: Motion) {
    if caret_on(tick, motion) {
        frame.set_cursor_position((x, y));
    }
}

/// A footer key: glyph, a space, verb.
#[derive(Clone, Copy)]
pub(super) struct KeyHint {
    pub glyph: &'static str,
    pub verb: &'static str,
}

impl KeyHint {
    const fn new(glyph: &'static str, verb: &'static str) -> Self {
        Self { glyph, verb }
    }
}

/// What the footer opens with after the mark column's glyph.
enum Status {
    Ready,
    /// The working line (`ui::working`), and no keys: `esc` always stops.
    Working,
    Waiting,
    /// No status word; the row opens with the keys.
    None,
}

/// The footer's content: only keys that work in the current state.
struct Footer {
    status: Status,
    keys: Vec<KeyHint>,
    /// `/ Commands`, right-flush one group gap before the context bar
    /// (frame A).
    aside: Option<KeyHint>,
}

impl Footer {
    fn new(status: Status, keys: Vec<KeyHint>) -> Self {
        Self {
            status,
            keys,
            aside: None,
        }
    }
}

/// Escape as every frame names it: the word, not `⎋`.
const ESC: &str = "esc";

/// The footer for `app`'s state. Space toggles the turn's work on an empty
/// field but is deliberately unnamed (frames B, C, J): the disclosure's
/// `›`/`⌄` says it opens.
fn footer_state(app: &App) -> Footer {
    if app.is_working() {
        return Footer::new(Status::Working, Vec::new());
    }
    // Frame E's list keys; a dismissible list adds `esc Close`.
    let choose = [KeyHint::new("↑↓", "Choose"), KeyHint::new("↩", "Select")];
    let dismissible = || {
        let mut keys = choose.to_vec();
        keys.push(KeyHint::new(ESC, "Close"));
        keys
    };
    match &app.mode {
        // The agent's question cannot be dismissed; "Chat about this" is an
        // option on the list (frame E).
        Mode::Question(asking) if matches!(asking.asker, Asker::Agent { .. }) => {
            Footer::new(Status::Waiting, choose.to_vec())
        }
        // A list the developer opened (`/resume`, `/model`, `/theme`): no
        // status word, as frame F's command list.
        Mode::Question(_) => Footer::new(Status::None, dismissible()),
        // Frame F: no status word, and a command is run, not selected.
        Mode::Commands(_) => Footer::new(
            Status::None,
            vec![
                KeyHint::new("↑↓", "Choose"),
                KeyHint::new("↩", "Run"),
                KeyHint::new(ESC, "Close"),
            ],
        ),
        // A waiting review with a turn running is the working line, above,
        // so the review reads as the turn it is inside. Nothing running:
        // `esc` leaves the review (`App::handle_review_key`).
        Mode::Review(r) if r.waiting() => {
            Footer::new(Status::None, vec![KeyHint::new(ESC, "Close")])
        }
        Mode::Review(r) if r.confirm.is_some() => Footer::new(Status::None, dismissible()),
        // Shift, Tab and Space are words: the closed glyph table has no
        // mark for them. The mouse is named because it is the main way to
        // select (ADR 0010).
        Mode::Review(r) if r.keys_shown => {
            let mut keys = vec![
                KeyHint::new("↑↓", "Scroll"),
                KeyHint::new("Click, drag or Shift ↑↓", "Select"),
            ];
            if r.file().has_folds() && !r.commenting() {
                keys.push(KeyHint::new("Space", "Show All Lines"));
            }
            keys.extend([
                KeyHint::new("↩", "Comment"),
                KeyHint::new("⌃↩", "Approve"),
                KeyHint::new("Tab", "Next file"),
                KeyHint::new(ESC, "Discard"),
            ]);
            Footer::new(Status::None, keys)
        }
        Mode::Review(_) => Footer::new(Status::None, vec![KeyHint::new("?", "Keys")]),
        // The turn is running but waits on the developer.
        Mode::Conversation if app.answering.is_some() => Footer::new(
            Status::Waiting,
            vec![KeyHint::new("↩", "Send"), KeyHint::new(ESC, "Back")],
        ),
        Mode::Conversation if !app.draft.is_empty() => {
            Footer::new(Status::Ready, vec![KeyHint::new("↩", "Send")])
        }
        // Frame J: after a saved turn, the context bar alone.
        Mode::Conversation if just_saved(app) => Footer::new(Status::None, Vec::new()),
        Mode::Conversation => Footer {
            status: Status::Ready,
            keys: Vec::new(),
            aside: Some(KeyHint::new("/", "Commands")),
        },
    }
}

/// Frame J's `Send  ↩`: offered on an idle field after a saved turn only,
/// ready once there is a draft.
fn send_action(app: &App) -> Option<Action> {
    let idle = matches!(app.mode, Mode::Conversation)
        && app.answering.is_none()
        && !(app.turn_active || app.awaiting_turn);
    (idle && just_saved(app)).then(|| Action {
        label: "Send".into(),
        key: "↩",
        ready: !app.draft.is_empty(),
    })
}

/// Whether the current turn holds a `Saved` review row.
fn just_saved(app: &App) -> bool {
    app.this_turn().iter().any(|e| {
        matches!(
            e,
            LogEntry::Review {
                outcome: ReviewOutcome::Saved { .. }
            }
        )
    })
}

/// The footer, in `label2`: the state glyph in the mark column (`working`
/// while a turn runs, else a `label3` `○`), the status and key groups
/// `--group-gap` apart, and the context bar flush right.
pub(super) fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let pal = app.theme.palette();
    let Footer {
        status,
        keys,
        aside,
    } = footer_state(app);
    let dim = Style::default().fg(pal.label2);
    // Never accent (frames A–J): blue is for the ready action in the field.
    let group = |key: KeyHint| {
        vec![
            Span::styled(key.glyph, dim),
            Span::styled(format!(" {}", key.verb), dim),
        ]
    };

    let line = matches!(status, Status::Working).then(|| app.activity.line(app.tick));
    let mark = match &line {
        Some(line) => working::mark(line, app.tick, app.motion, pal),
        None => Span::styled(
            format!("{:<MARK_COL$}", "○"),
            Style::default().fg(pal.label3),
        ),
    };
    let mut groups: Vec<Vec<Span<'static>>> = match status {
        Status::Ready => vec![vec![Span::styled("Ready", dim)]],
        Status::Waiting => vec![vec![Span::styled("Waiting for you", dim)]],
        Status::Working | Status::None => Vec::new(),
    };
    let status_groups = groups.len();
    groups.extend(keys.into_iter().map(group));

    // When the row is short: the context bar is never cut; the aside is
    // dropped first, then trailing key groups, whole.
    let row = (area.width as usize).saturating_sub(MARGIN_X * 2 + MARK_COL);
    let bar = context_bar(app.status.context_percent(), pal);
    let span_w = |spans: &[Span]| spans.iter().map(|s| s.content.width()).sum::<usize>();
    let groups_w = |groups: &[Vec<Span<'static>>]| {
        groups.iter().map(|g| span_w(g)).sum::<usize>() + GROUP_GAP * groups.len().saturating_sub(1)
    };
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(aside) = aside {
        let aside = group(aside);
        if groups_w(&groups) + GROUP_GAP + span_w(&aside) + GROUP_GAP + span_w(&bar) <= row {
            right.extend(aside);
            right.push(Span::raw(" ".repeat(GROUP_GAP)));
        }
    }
    right.extend(bar);
    let right_w = span_w(&right);
    let budget = row.saturating_sub(GROUP_GAP).saturating_sub(right_w);

    let left = match &line {
        Some(line) => working::words(line, app.motion, pal, budget),
        None => {
            while groups.len() > status_groups && groups_w(&groups) > budget {
                groups.pop();
            }
            truncate_spans(groups.join(&Span::raw(" ".repeat(GROUP_GAP))), budget)
        }
    };
    let gap = row.saturating_sub(span_w(&left)).saturating_sub(right_w);

    let spans: Vec<Span<'static>> = [Span::raw(" ".repeat(MARGIN_X)), mark]
        .into_iter()
        .chain(left)
        .chain(std::iter::once(Span::raw(" ".repeat(gap))))
        .chain(right)
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// `██████████ 41%`, coloured by `Palette::gauge` (`ContextBar.jsx`), with
/// no label: the footer gives it the row (frame `W1`). `None` draws empty
/// at `0%`, as the launch frame does.
pub(super) fn context_bar(percent: Option<u8>, pal: &Palette) -> Vec<Span<'static>> {
    let pct = percent.unwrap_or(0);
    let filled = gauge_filled(pct);
    let colours = pal.gauge(filled);
    let mut spans: Vec<Span<'static>> = colours
        .iter()
        .take(GAUGE_SEGMENTS)
        .map(|colour| Span::styled(GAUGE_CELL.to_string(), Style::default().fg(*colour)))
        .collect();
    spans.push(Span::styled(
        format!(" {pct}%"),
        Style::default().fg(pal.label2),
    ));
    spans
}

/// The comment field, two rows replacing the field while a selection is
/// commented on: the label on `--select`, the draft on `--field`.
pub(super) fn draw_comment_field(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    lines: &str,
    location: &str,
    draft: &str,
    cursor: usize,
) {
    let pal = app.theme.palette();
    let inner = Rect {
        x: area.x + MARGIN_X as u16,
        width: area.width.saturating_sub(MARGIN_X as u16 * 2),
        ..area
    };
    let [label_row, draft_row] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    let on_select = Style::default().bg(pal.select);
    let edge = |bg: Color| Span::styled("▎", Style::default().fg(pal.accent).bg(bg));
    let left = vec![
        edge(pal.select),
        Span::styled(
            "Commenting on ",
            Style::default().fg(pal.label).bg(pal.select),
        ),
        Span::styled(
            lines.to_string(),
            Style::default()
                .fg(pal.label)
                .bg(pal.select)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {location}"),
            Style::default().fg(pal.label2).bg(pal.select),
        ),
    ];
    let right = vec![Span::styled(
        format!("{ESC} "),
        Style::default().fg(pal.label2).bg(pal.select),
    )];
    frame.render_widget(Block::new().style(on_select), label_row);
    frame.render_widget(
        Paragraph::new(justify(left, right, inner.width as usize, pal.select)).style(on_select),
        label_row,
    );

    let on_field = Style::default().bg(pal.field);
    frame.render_widget(Block::new().style(on_field), draft_row);
    let mut row = Line::from(vec![
        edge(pal.field),
        Span::styled(
            draft.to_string(),
            Style::default().fg(pal.label).bg(pal.field),
        ),
    ]);
    let used: usize = row.spans.iter().map(|s| s.content.width()).sum();
    let gap = (inner.width as usize)
        .saturating_sub(used)
        .saturating_sub(2);
    row.spans.push(Span::styled(" ".repeat(gap), on_field));
    row.spans.push(Span::styled(
        "↩ ",
        Style::default().fg(pal.accent).bg(pal.field),
    ));
    frame.render_widget(Paragraph::new(row).style(on_field), draft_row);
    // `cursor` counts chars; the caret needs display cells.
    let column: usize = draft
        .chars()
        .take(cursor)
        .map(|c| c.width().unwrap_or(0))
        .sum();
    place_caret(
        frame,
        draft_row.x + 1 + column as u16,
        draft_row.y,
        app.tick,
        app.motion,
    );
}

fn justify(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
    bg: Color,
) -> Line<'static> {
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let left = if left_w + right_w + 1 > width {
        truncate_spans(left, width.saturating_sub(right_w + 1))
    } else {
        left
    };
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let mut spans = left;
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(left_w).saturating_sub(right_w)),
        Style::default().bg(bg),
    ));
    spans.extend(right);
    Line::from(spans)
}
