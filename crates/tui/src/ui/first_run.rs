//! The design system's first-run screens — `1a`, `1b`, `1c`.
//!
//! Three bands, like every other screen: a 3-row top bar, the body, and the
//! 5-row bottom band (`--bar-bottom-h`). The body is one blank row, the
//! positioning line, three blank rows (`--section-gap-h`), and then the step
//! spine.
//!
//! The bottom band is the *same* band the session has, which it was not
//! before. This screen used to end in a 3-row `--bar-keys-h` footer of its
//! own; the lantern-gold repaint deleted that token and `cells.css` now says
//! of `--bar-bottom-h`: "blank, prompt or keys, blank, status, blank — every
//! frame". So the keys take the row a prompt would, and a status row under
//! them says `○  waiting` — which is true, and is what makes first run read
//! as the app's own opening state rather than as an installer in front of it.
//!
//! There is no wordmark. It was a reverse-video row above the positioning
//! line until the same repaint cut it: the brand is the plain `Aldwin` in
//! the top bar, and gold is "never on a fill larger than the wordmark" now
//! describes a fill this screen no longer draws.
//!
//! # The spine
//!
//! Turn 14 stopped paginating this screen. **All three steps are on screen
//! from the start**, as one vertical list, so the shape of the flow is
//! visible before any of it is answered — and so the nesting is legible for
//! free: the settled `provider anthropic` row sits directly above a list of
//! that provider's models, which is the whole explanation of why those
//! models and not others. It needs no tree and no counter to say so.
//!
//! Each step is one row — glyph on the 3-cell margin, name on the body
//! column (cell 13), content on `STEP_CONTENT_COL` (cell 29) — in one of
//! three states, which the closed glyph vocabulary already had words for:
//!
//! | State | Glyph | Name | Content | Rows |
//! | --- | --- | --- | --- | --- |
//! | settled | `●` `done` | `label` | its answer, in `text` | 1 |
//! | open | `▌` `mark` | `speaker_you` | its purpose, then its list | 2 + list |
//! | pending | `○` `mark_idle` | `dim` | what it will ask, in `dim` | 1 |
//!
//! One blank row parts one step from the next. The 3-row `--section-gap-h`
//! survives exactly once, above the first step.
//!
//! The `step n/m` counter this screen used to carry is gone: the glyphs are
//! the progress indicator now, which is why the design system's own note
//! says the sequence "needs no progress bar and no step counter". The label
//! column is consequently empty on every row of this screen — the one screen
//! in the system where that is true.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::TOP_BAR_ROWS;
use super::grid::{elide, Ctx, GROUP_GAP, MARGIN_X, OPTION_LABEL_COL, STEP_CONTENT_COL, STEP_MARK_COL};
use crate::first_run::{AccessTier, FirstRun, Step};
use crate::palette::Palette;

/// Rows the footer takes; the top bar takes the session's own
/// [`TOP_BAR_ROWS`], and the body gets the rest.
const FOOTER_ROWS: u16 = 5;

/// Where the harness's answers land. Stated plainly rather than implied,
/// per the design system's Content Fundamentals. A directory, not a single
/// file, because the answers land in two files inside it.
///
/// The reference writes `config → ~/.aldwin/config.toml`. There is no such
/// file — the answers are `provider.yaml` and `permissions.yaml` — so the
/// directory is what is true. See `baseline.json`'s
/// `frame-names-files-and-a-command-the-product-does-not-have`.
const CONFIG_LOCATION: &str = "config → ~/.aldwin/";

/// The top bar's right group before any model is chosen.
const NO_MODEL: &str = "no model";

/// The `more` row's own name and purpose. It is not a provider, so it is
/// not in the catalogue the caller hands in.
const MORE_LABEL: &str = "more";
const MORE_PURPOSE: &str = "the full provider list";

/// What each question is for, in one row of prose beside the open step.
///
/// `model` names `/model` because that command exists and does what the
/// sentence says. The design's `access` copy also promised "/access changes
/// it later"; there is no `/access`, so that clause is dropped rather than
/// shipped false.
const PROVIDER_PROSE: &str = "Where the model runs. Each one needs its own key.";
const MODEL_PROSE: &str = "Which model this session starts with. /model changes it later.";
const ACCESS_PROSE: &str = "Which actions run without asking.";

/// What a step still to come says it will ask, in one dim row. Shorter and
/// flatter than the prose above, because a step that is not taking keys is
/// previewing a question rather than posing one.
const MODEL_PREVIEW: &str = "which model, once the provider is set";
const ACCESS_PREVIEW: &str = "what runs without asking";
/// Never drawn in the shipped flow — `provider` is always the first step
/// when it is asked at all, so it is never pending. Present so that
/// [`step_preview`] is total over [`Step`] rather than guessing.
const PROVIDER_PREVIEW: &str = "where the model runs";

/// Which of the three states a step is in on this frame.
///
/// Derived from the step's position relative to the live one rather than
/// stored, so it cannot disagree with `FirstRun::step()`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StepState {
    Settled,
    Open,
    Pending,
}

pub(crate) fn draw(frame: &mut Frame, state: &FirstRun, pal: &Palette) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(pal.ground)), area);

    let [top, body, footer] =
        Layout::vertical([Constraint::Length(TOP_BAR_ROWS), Constraint::Min(1), Constraint::Length(FOOTER_ROWS)]).areas(area);

    draw_top_bar(frame, top, &state.cwd, pal);
    let ctx = Ctx::new(pal, body.width);
    frame.render_widget(Paragraph::new(Text::from(body_lines(state, ctx))), body);
    draw_footer(frame, footer, state, ctx);
}

/// The same 3-row identity band every screen opens with, on `bar`: the
/// plain word `Aldwin`, the working directory starting on the body column,
/// and `no model` flush to the right margin.
///
/// The right group is the session bar's own — `model · gauge · cost` — in
/// its unanswered state, which is how the reference draws it (`1a`–`1c`:
/// `no model · ██████████ 0% · $0.00`, every span `--tui-dim`). Aldwin
/// tracks neither a context gauge nor a cost, so it draws the one fact of
/// the three it actually has. The version used to sit here; no frame in the
/// reference carries one any more, and `aldwin --version` is where it lives.
///
/// The bar carries no `▌` — "the name is the brand, and a pip there
/// indicated nothing".
fn draw_top_bar(frame: &mut Frame, area: Rect, cwd: &str, pal: &Palette) {
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar)), area);
    let Some(row) = area.height.checked_sub(2).map(|_| Rect { y: area.y + 1, height: 1, ..area }) else { return };

    // Composed by `chrome::identity_bar_row`, which the session bar also
    // uses — this comment used to say the two "must not disagree" while both
    // laid their groups out by hand, and they had drifted. The right group
    // is dropped whole on a frame too narrow to hold it beside a readable
    // path, never clipped into half a fact.
    let right = vec![vec![Span::styled(NO_MODEL, Style::default().fg(pal.dim).bg(pal.bar))], Vec::new()];
    // `quiet` for the path, the same rung the session bar gives the
    // identical string. `no model` beside it is `dim`: an absence is the
    // quieter fact of the two.
    let line = super::chrome::identity_bar_row(area.width as usize, cwd, pal.quiet, right, pal);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(pal.bar)), row);
}

/// Where a step stands relative to the one taking keys.
///
/// A step the developer has already passed collapses to the row carrying
/// its answer. That is not only tidier: the frame is 36 rows and does not
/// scroll, and three open lists — the expanded provider catalogue included
/// — do not fit in it. Collapsing a settled list is what buys the open one
/// its rows, and it is measured by `the_expanded_screen_still_fits_the_frame`.
fn state_of(state: &FirstRun, step: Step) -> StepState {
    match (state.position(step), state.position(state.step())) {
        (Some((at, _)), Some((now, _))) if at < now => StepState::Settled,
        (Some((at, _)), Some((now, _))) if at > now => StepState::Pending,
        _ => StepState::Open,
    }
}

/// The step's name, in the order the spine draws them.
fn step_name(step: Step) -> &'static str {
    match step {
        Step::Provider => "provider",
        Step::Model => "model",
        Step::Access => "access",
    }
}

fn step_prose(step: Step) -> &'static str {
    match step {
        Step::Provider => PROVIDER_PROSE,
        Step::Model => MODEL_PROSE,
        Step::Access => ACCESS_PROSE,
    }
}

fn step_preview(step: Step) -> &'static str {
    match step {
        Step::Provider => PROVIDER_PREVIEW,
        Step::Model => MODEL_PREVIEW,
        Step::Access => ACCESS_PREVIEW,
    }
}

/// What a settled step collapses to: the answer it was given.
///
/// `None` only for a step whose answer cannot be named — a catalogue row
/// that has gone missing under the selection. The step still draws, with an
/// empty content column, rather than vanishing out of the spine and
/// renumbering everything below it.
fn step_answer(state: &FirstRun, step: Step) -> Option<String> {
    match step {
        Step::Provider => state.chosen_provider().map(|p| p.id.clone()),
        Step::Model => state.chosen_model().map(|m| m.id.clone()),
        Step::Access => Some(AccessTier::ORDER[state.access].label().to_string()),
    }
}

/// The open step's option rows. Empty for any step that is not open — a
/// settled step shows its answer and a pending one shows nothing at all.
fn step_options(state: &FirstRun, step: Step, ctx: Ctx) -> Vec<Line<'static>> {
    let items: Vec<Opt> = match step {
        Step::Provider => {
            let visible = state.visible_providers();
            let mut items: Vec<Opt> =
                visible.iter().enumerate().map(|(i, choice)| Opt::new(&choice.id, &choice.purpose, i == state.provider)).collect();
            if state.shows_more() {
                items.push(Opt::new(MORE_LABEL, MORE_PURPOSE, state.provider == visible.len()).trailing("→"));
            }
            items
        }
        Step::Model => state.visible_models().iter().enumerate().map(|(i, choice)| Opt::new(&choice.id, &choice.purpose, i == state.model)).collect(),
        Step::Access => AccessTier::ORDER.iter().enumerate().map(|(i, tier)| Opt::new(tier.label(), tier.purpose(), i == state.access)).collect(),
    };
    option_rows(&items, ctx)
}

fn body_lines(state: &FirstRun, ctx: Ctx) -> Vec<Line<'static>> {
    // One blank row, then the positioning line — the reference's body band
    // is `padding-top: var(--row)` and its first child is the sentence.
    let mut lines = vec![Line::default(), positioning_line(ctx)];
    // `--section-gap-h`: three blank rows, used once — the steps themselves
    // are one blank row apart.
    lines.extend([Line::default(), Line::default(), Line::default()]);

    // Every step this run asks is drawn, in order, whatever state it is in
    // — that is the whole point of the spine. `state.steps` is what the
    // caller asked for, so a run that skips the provider question simply
    // has a shorter one.
    for (i, step) in state.steps.iter().enumerate() {
        if i > 0 {
            // One blank row between steps. The three-row `--section-gap-h`
            // above is used once and never again on this screen.
            lines.push(Line::default());
        }
        lines.extend(step_rows(state, *step, ctx));
    }
    lines
}

/// One step: its own row, plus — when it is the open one — a blank row and
/// its list.
fn step_rows(state: &FirstRun, step: Step, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let status = state_of(state, step);
    let (glyph, glyph_fg, name_fg, content_fg, content) = match status {
        StepState::Settled => {
            (
                "●",
                pal.done,
                pal.label,
                pal.text,
                step_answer(state, step).unwrap_or_default(),
            )
        }
        StepState::Open => ("▌", pal.mark, pal.speaker_you, pal.body, step_prose(step).to_string()),
        StepState::Pending => ("○", pal.mark_idle, pal.dim, pal.dim, step_preview(step).to_string()),
    };

    let name = step_name(step);
    // The glyph sits at the margin in a 10-cell field, so the name lands on
    // the body column; the name sits in the shared 16-cell option field, so
    // the content lands on cell 29. Both derived — see `grid`.
    let name_pad = OPTION_LABEL_COL.saturating_sub(name.width());
    let room = (ctx.width as usize).saturating_sub(STEP_CONTENT_COL).saturating_sub(MARGIN_X);
    let mut rows = vec![Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled(glyph.to_string(), Style::default().fg(glyph_fg)),
        Span::raw(" ".repeat(STEP_MARK_COL - 1)),
        Span::styled(name.to_string(), Style::default().fg(name_fg)),
        Span::raw(" ".repeat(name_pad)),
        Span::styled(elide(&content, room), Style::default().fg(content_fg)),
    ])];

    if status == StepState::Open {
        rows.push(Line::default());
        rows.extend(step_options(state, step, ctx));
    }
    rows
}

/// Elided rather than left to be clipped by the frame's edge. On a narrow
/// terminal it used to end mid-word with nothing marking it — the same
/// silent-truncation fault the identity bar had, on a line whose whole
/// content is one sentence.
fn positioning_line(ctx: Ctx) -> Line<'static> {
    let room = (ctx.width as usize).saturating_sub(MARGIN_X * 2);
    Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled(
            elide("The leverage of a model, without handing over the keys.", room),
            Style::default().fg(ctx.pal.dim),
        ),
    ])
}

/// One option in a step's list, before it is a row: the list has to be
/// measured as a whole before any row of it can be drawn (see
/// [`option_rows`]).
struct Opt<'a> {
    name:     &'a str,
    purpose:  &'a str,
    selected: bool,
    /// A glyph at the row's right edge — only the `more` row has one.
    trailing: Option<&'static str>,
}

impl<'a> Opt<'a> {
    fn new(name: &'a str, purpose: &'a str, selected: bool) -> Self {
        Self { name, purpose, selected, trailing: None }
    }

    fn trailing(mut self, glyph: &'static str) -> Self {
        self.trailing = Some(glyph);
        self
    }

    /// Cells this row would like: the mark, its two spaces, the name field
    /// and the purpose, plus a trailing glyph where there is one.
    fn width(&self) -> usize {
        3 + OPTION_LABEL_COL.max(self.name.width()) + self.purpose.width()
    }
}

/// A step's list of options, every row the same width — which is the
/// **list's** width, not the frame's.
///
/// The band a selected row paints used to run from the content column to
/// the frame's right margin, so its size was a property of the terminal
/// rather than of the control: 47 cells at 80 columns, 87 at 120, and 167
/// at 200, where an option row whose content ends at cell 63 became the
/// largest coloured area in the frame. The rule it broke is that the accent
/// is "a mark or a line, never a filled field", and the `→` on the `more`
/// row rode the same edge — 127 cells from the label it belongs to.
///
/// The width is taken from the widest row rather than from a stated
/// constant. `5c`'s 48-cell command list is the nearest thing the design
/// system states, and it is **not** this list: measured against the
/// reference's own copy, `anthropic` + `claude models · ANTHROPIC_API_KEY`
/// needs 52 cells and the detail column it hangs on starts at cell 48, so a
/// 48-cell list would elide the design's own row at the design's own frame
/// width. A list wide enough for its content and no wider is the honest
/// derivation, and it keeps every row in the list rectangular — nothing
/// ragged, which is what a band must not be.
fn option_rows(items: &[Opt], ctx: Ctx) -> Vec<Line<'static>> {
    // What the frame can give, which on a narrow terminal is less than the
    // list wants; the purpose elides into it rather than wrapping the row.
    let room = (ctx.width as usize).saturating_sub(STEP_CONTENT_COL).saturating_sub(MARGIN_X);
    // A trailing `→` sits in the last cell before the right margin, so on a
    // frame too narrow for both it is the *list* that gives way, not the
    // margin. Without this the arrow was pushed into the margin at 80
    // columns and the `layout` gate caught it — the margin is the one thing
    // in the grid nothing may enter.
    let arrowed = items.iter().any(|item| item.trailing.is_some());
    let width = items.iter().map(Opt::width).max().unwrap_or(0).min(room.saturating_sub(usize::from(arrowed)));
    items.iter().map(|item| row(item, width, room, ctx)).collect()
}

/// The one option row shape in the system: an idle or selected `▌`, two
/// spaces, the name in a 16-cell field, then a purpose statement saying
/// what picking it does. The `more` row that ends a collapsed provider list
/// is the same shape, with its name one step quieter than a real option —
/// it is a way of asking the question again, not an answer to it — and a
/// `→` at the row's right edge saying the list continues.
///
/// Selection is the accent `▌` *and* the band together, never one alone.
fn row(opt: &Opt, width: usize, room: usize, ctx: Ctx) -> Line<'static> {
    let Opt { name, purpose, selected, trailing } = *opt;
    let pal = ctx.pal;
    let quiet_name = trailing.is_some() && !selected;
    // The selected row's *purpose* stays `quiet`, exactly as on an idle row
    // (`1a`: `anthropic` inherits `--tui-text`, its purpose is `--tui-quiet`
    // on the band). Selection is the band and the mark; lifting the purpose
    // as well made the selected row the only one whose two columns read at
    // one weight, which flattened the very distinction the list is built on.
    let (mark_fg, bg, name_fg, purpose_fg) = if selected {
        (pal.mark, pal.band, pal.accent_text, pal.quiet)
    } else if quiet_name {
        (pal.mark_idle, pal.ground, pal.label, pal.dim)
    } else {
        (pal.mark_idle, pal.ground, pal.body, pal.quiet)
    };
    let field = Style::default().bg(bg);

    // The row hangs on the step's content column, so it lines up under the
    // step's own purpose line rather than under its name. The prefix is
    // *unstyled*: the band starts at the mark, not at the frame margin.
    let mut spans = vec![
        Span::raw(" ".repeat(STEP_CONTENT_COL)),
        Span::styled("▌", Style::default().fg(mark_fg).bg(bg)),
        Span::styled("  ".to_string(), field),
    ];
    let pad = OPTION_LABEL_COL.saturating_sub(name.width());
    spans.push(Span::styled(name.to_string(), Style::default().fg(name_fg).bg(bg)));
    spans.push(Span::styled(" ".repeat(pad), field));

    // The mark, its two spaces, the name field and any trailing glyph are
    // fixed, so the purpose gets what is left of the list's width — and on
    // a frame narrower than the design's 120 that can be nothing at all,
    // which is why it is elided rather than allowed to wrap the row.
    let trailing = trailing.unwrap_or("");
    let fixed = 3 + name.width() + pad;
    let purpose = elide(purpose, width.saturating_sub(fixed));
    spans.push(Span::styled(purpose.clone(), Style::default().fg(purpose_fg).bg(bg)));

    // Fill to the list's width so the band is a band, not a ragged
    // highlight ending wherever the purpose text happens to stop.
    let used = fixed + purpose.width();
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), field));
    // The `→` is the one thing on this row the handoff places explicitly —
    // "`more` … with a `→` flush to the 3-cell right margin" — so it sits
    // at the frame's margin rather than at the list's edge, outside the
    // band and on the screen's own ground. It rode the band's edge for one
    // pass on the reading that an affordance belongs to the row it marks;
    // two blind judges measured it against that sentence instead, and the
    // sentence is the reference. What the design does *not* state is how
    // wide this list is — see the conformance spec's Class B entry.
    if !trailing.is_empty() {
        let pad = room.saturating_sub(width).saturating_sub(trailing.width());
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(trailing.to_string(), Style::default().fg(if selected { pal.text } else { pal.label })));
    }
    Line::from(spans)
}

/// The bottom band: blank, keys, blank, status, blank — `--bar-bottom-h`,
/// the same five rows every frame ends in (see this module's doc comment).
///
/// Keys left on the row a prompt would take; under them `○  waiting` left
/// and where the answers land flush right. **Nothing here is gold.** The
/// hint keys were `--tui-mark` until the repaint; the reference now draws
/// every key in every footer in `--tui-body`, because gold is "spent on one
/// thing per band" and in this band there is nothing open to spend it on —
/// the open step is up in the body, and already has it.
fn draw_footer(frame: &mut Frame, area: Rect, state: &FirstRun, ctx: Ctx) {
    let pal = ctx.pal;
    let on_band = |fg| Style::default().fg(fg).bg(pal.bar_bottom);
    let field = Style::default().bg(pal.bar_bottom);
    frame.render_widget(Block::new().style(field), area);
    // Rows 1 and 3 of the five. A frame too short to hold the whole band
    // keeps the keys and loses the status row, in that order: the keys are
    // what a footer is for.
    let row_at = |n: u16| (area.height > n).then(|| Rect { y: area.y + n, height: 1, ..area });
    let width = area.width as usize;

    // Two hints, as the reference carries. `←` reopens the previous
    // question and stays bound, but is not named here — the frame shows two
    // groups and a third would be one wider than the design's footer. It is
    // in the same position `Esc` has always been: real, and undocumented on
    // screen.
    //
    // The last step's `⏎` says what it does. On every other step it moves
    // on; on the last one it *starts the session*, and the reference
    // changes the verb to say so (`1c`).
    let confirm = if state.index + 1 < state.steps.len() { "continue" } else { "start session" };
    if let Some(row) = row_at(1) {
        let mut keys = vec![Span::styled(" ".repeat(MARGIN_X), field)];
        for (i, (k, verb)) in [("⏎", confirm), ("↑↓", "choose")].into_iter().enumerate() {
            if i > 0 {
                keys.push(Span::styled(" ".repeat(GROUP_GAP), field));
            }
            keys.push(Span::styled(k.to_string(), on_band(pal.body)));
            keys.push(Span::styled(format!(" {verb}"), on_band(pal.quiet)));
        }
        frame.render_widget(Paragraph::new(Line::from(keys)).style(field), row);
    }

    let Some(row) = row_at(3) else { return };
    let mut status = vec![
        Span::styled(" ".repeat(MARGIN_X), field),
        Span::styled("○", on_band(pal.glyph_pending)),
        Span::styled("  waiting", on_band(pal.quiet)),
    ];
    // The config location is a path, so it goes whole or not at all — the
    // same rule the identity bar applies to its right group. It used to be
    // pushed on regardless, which at 44 columns ran the text beside it
    // straight into it and then clipped the path itself.
    let used: usize = status.iter().map(|s| s.content.width()).sum();
    let config = if width.saturating_sub(used).saturating_sub(MARGIN_X) >= CONFIG_LOCATION.width() + GROUP_GAP {
        CONFIG_LOCATION
    } else {
        ""
    };
    let gap = width.saturating_sub(used).saturating_sub(config.width()).saturating_sub(MARGIN_X);
    status.push(Span::styled(" ".repeat(gap), field));
    status.push(Span::styled(config.to_string(), on_band(pal.dim)));
    status.push(Span::styled(" ".repeat(MARGIN_X), field));
    frame.render_widget(Paragraph::new(Line::from(status)).style(field), row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::{sample_providers, SAMPLE_CURATED};
    use crate::palette::DARK;
    // The step *name* column is the ordinary body column — that is the
    // point of the spine, so it is asserted against the same constant every
    // other screen uses rather than a local copy.
    use crate::ui::grid::CONTENT_INDENT;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(state: &FirstRun, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, state, &DARK)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("")
    }

    /// Reads the buffer's *own* width and height rather than the design's
    /// 120×36. These were hardcoded, which was fine while every test
    /// rendered at the frame size and panicked with an out-of-bounds index
    /// the moment one rendered narrower.
    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect()
    }

    fn find_row(buffer: &ratatui::buffer::Buffer, needle: &str) -> u16 {
        (0..buffer.area.height)
            .find(|y| row_text(buffer, *y).contains(needle))
            .unwrap_or_else(|| panic!("no row containing {needle:?}"))
    }

    /// Byte offset converted to a *cell* offset: `▌` is three bytes, so
    /// `str::find` alone would report every column past a mark two cells to
    /// the right of where it actually is. The grid counts cells.
    fn col_of(buffer: &ratatui::buffer::Buffer, y: u16, needle: &str) -> usize {
        let row = row_text(buffer, y);
        let byte = row.find(needle).unwrap_or_else(|| panic!("{needle:?} not on row {y}: {row:?}"));
        row[..byte].chars().count()
    }

    /// Every row that is a *step's own* line: a glyph on the 3-cell margin.
    /// Option rows hang on cell 29 and so are never picked up here, and the
    /// positioning line carries no glyph.
    ///
    /// Body band only. The bottom band's status row is `○  waiting` with its
    /// glyph on the same margin — deliberately, it is the same column — so a
    /// search of the whole frame counts it as a fourth step.
    fn spine_rows(buffer: &ratatui::buffer::Buffer) -> Vec<u16> {
        let body = TOP_BAR_ROWS..36 - FOOTER_ROWS;
        body.filter(|y| row_text(buffer, *y).chars().nth(MARGIN_X).is_some_and(|c| "●▌○".contains(c))).collect()
    }

    /// The row a given step's own line is on. Matched on the step name
    /// sitting at the body column, not merely appearing on the row — the
    /// open `provider` step's prose contains the word "model", and an
    /// earlier version of this helper matched that instead.
    fn step_row(buffer: &ratatui::buffer::Buffer, name: &str) -> u16 {
        spine_rows(buffer)
            .into_iter()
            .find(|y| {
                let row = row_text(buffer, *y);
                row.match_indices(name).any(|(byte, _)| row[..byte].chars().count() == CONTENT_INDENT)
            })
            .unwrap_or_else(|| panic!("no step row for {name:?}"))
    }

    fn glyph_of(buffer: &ratatui::buffer::Buffer, name: &str) -> String {
        buffer[(MARGIN_X as u16, step_row(buffer, name))].symbol().to_string()
    }

    /// There is no wordmark, and gold is not a fill anywhere on this screen
    /// but the one selected row. The reverse-video `A L D W I N` row led this
    /// screen until the lantern-gold repaint cut it; this is the assertion
    /// that fails if it comes back, from either direction — as letters, or as
    /// a gold field with something else written on it.
    #[test]
    fn there_is_no_wordmark_and_the_body_opens_on_the_positioning_line() {
        let buffer = render(&FirstRun::default(), 120, 36);
        assert!(!text(&buffer).contains("A L D W I N"), "the letter-spaced wordmark is gone");
        for y in 0..36u16 {
            let gold = (0..120).filter(|x| buffer[(*x, y)].bg == DARK.reverse_bg).count();
            assert_eq!(gold, 0, "row {y} carries a gold fill; the only fill on this screen is the selection band");
        }

        // `padding-top: var(--row)`, then the sentence: one blank row under
        // the top bar and the positioning line directly after it.
        let line = find_row(&buffer, "The leverage of a model");
        assert_eq!(line, TOP_BAR_ROWS + 1, "one blank row, then the positioning line");
        assert_eq!(col_of(&buffer, line, "The leverage"), MARGIN_X, "on the margin, not the body column");
        assert_eq!(buffer[(MARGIN_X as u16, line)].fg, DARK.dim);

        // Then `--section-gap-h`, three blank rows, and the first step.
        assert_eq!(spine_rows(&buffer)[0], line + 4, "three blank rows part the line from the spine");
    }

    /// All three steps are on screen from the first frame, in order. That is
    /// the whole point of the spine: the shape of the flow is visible before
    /// any of it is answered.
    #[test]
    fn every_step_is_on_screen_from_the_start_in_order() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let (provider, model, access) = (step_row(&buffer, "provider"), step_row(&buffer, "model"), step_row(&buffer, "access"));
        assert!(provider < model && model < access, "the spine reads top to bottom: {provider} {model} {access}");
    }

    /// The three states are the three glyphs the system already has, and
    /// they move together as the form advances. Nothing else says where the
    /// developer is — there is no counter and no progress bar.
    #[test]
    fn the_glyphs_carry_the_sequence_and_no_counter_does() {
        for (index, want) in [(0, ["▌", "○", "○"]), (1, ["●", "▌", "○"]), (2, ["●", "●", "▌"])] {
            let state = FirstRun { index, ..Default::default() };
            let buffer = render(&state, 120, 36);
            let got = ["provider", "model", "access"].map(|n| glyph_of(&buffer, n));
            assert_eq!(got, want.map(String::from), "at step {index}");
            assert!(!text(&buffer).contains("step 1/"), "the `step n/m` counter is gone — the glyphs are the progress");
        }
    }

    /// A settled step collapses to the answer it was given; an open one
    /// expands into its list; a pending one previews what it will ask and
    /// shows no list at all.
    #[test]
    fn a_settled_step_shows_its_answer_and_a_pending_one_shows_no_list() {
        let before = text(&render(&FirstRun::default(), 120, 36));
        assert!(!before.contains("alpha-large"), "no model list before the model step is open: {before:?}");
        assert!(before.contains("which model, once the provider is set"), "it previews instead: {before:?}");
        assert!(before.contains("what runs without asking"), "so does access: {before:?}");
        assert!(!before.contains("every call asks"), "and the access list is not drawn yet: {before:?}");

        let state = FirstRun { index: 1, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let out = text(&buffer);
        assert!(out.contains("alpha-large"), "the open step's list is the chosen provider's models: {out:?}");
        assert!(!out.contains("bravo"), "the settled provider question is down to its answer: {out:?}");
        assert!(!out.contains("the full provider list"), "and to nothing else: {out:?}");

        let provider = step_row(&buffer, "provider");
        assert_eq!(col_of(&buffer, provider, "alpha"), STEP_CONTENT_COL, "the answer sits on the content column");
        assert_eq!(buffer[(STEP_CONTENT_COL as u16, provider)].fg, DARK.text, "an answer is primary text");
    }

    /// Every landmark is a whole number of cells off the grid: the 3-cell
    /// margin for the glyph, the body column (cell 13) for the step name,
    /// and `--step-content-col` (cell 29) for *all four* kinds of content —
    /// a settled answer, an open step's prose, a pending preview, and the
    /// option rows. One column, whatever the step is doing.
    #[test]
    fn every_column_lands_on_the_grid() {
        assert_eq!(STEP_MARK_COL, 10, "--step-mark-col is the label column plus its gutter");
        assert_eq!(STEP_CONTENT_COL, 29, "--step-content-col: margin + step mark col + the 16-cell name field");

        let buffer = render(&FirstRun::default(), 120, 36);

        let provider = step_row(&buffer, "provider");
        assert_eq!(col_of(&buffer, provider, "▌"), MARGIN_X, "the open step's glyph is on the 3-cell margin");
        assert_eq!(col_of(&buffer, provider, "provider"), CONTENT_INDENT, "its name is on the body column, cell 13");
        assert_eq!(col_of(&buffer, provider, "Where the model runs"), STEP_CONTENT_COL, "its prose on cell 29");

        let model = step_row(&buffer, "model");
        assert_eq!(col_of(&buffer, model, "○"), MARGIN_X);
        assert_eq!(col_of(&buffer, model, "model"), CONTENT_INDENT, "one name column for every step");
        assert_eq!(col_of(&buffer, model, "which model"), STEP_CONTENT_COL, "a preview hangs on the same content column");

        let first = find_row(&buffer, "alpha");
        assert_eq!(col_of(&buffer, first, "▌"), STEP_CONTENT_COL, "option rows hang on the content column too");
        assert_eq!(col_of(&buffer, first, "alpha"), STEP_CONTENT_COL + 3, "the mark plus two spaces, then the name");
        assert_eq!(
            col_of(&buffer, first, "alpha models"),
            STEP_CONTENT_COL + 3 + OPTION_LABEL_COL,
            "the purpose starts past the 16-cell name field"
        );
    }

    /// A step row puts nothing between its glyph and its name: the glyph is
    /// alone on the margin and the name is on the body column, with the
    /// 10-cell `--step-mark-col` field between them. That gap is where
    /// `step n/m` used to sit, and the counter is what Turn 14 removed —
    /// so this is the assertion that would fail if it came back.
    ///
    /// Scoped to the spine deliberately. The positioning line starts on the
    /// margin and runs straight through cell 13; it is not label-column
    /// content, it is a full-width row.
    #[test]
    fn a_step_row_carries_nothing_between_its_glyph_and_its_name() {
        for index in 0..3 {
            let buffer = render(&FirstRun { index, ..Default::default() }, 120, 36);
            let rows = spine_rows(&buffer);
            assert_eq!(rows.len(), 3, "three steps, three spine rows, at step {index}");
            for y in rows {
                let row = row_text(&buffer, y);
                let gap: String = row.chars().skip(MARGIN_X + 1).take(STEP_MARK_COL - 1).collect();
                assert!(gap.trim().is_empty(), "row {y} puts {gap:?} between the glyph and the name: {row:?}");
            }
        }
    }

    /// Only the open step carries a selection band, and selection is always
    /// the accent `▌` *and* the band together — never one without the other.
    #[test]
    fn only_the_open_step_carries_a_selection_band() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let banded: Vec<u16> = (0..36u16).filter(|y| (0..120).any(|x| buffer[(x, *y)].bg == DARK.band)).collect();
        assert_eq!(banded.len(), 1, "one banded row, in the one open list");
        assert!(row_text(&buffer, banded[0]).contains("alpha"), "the provider list opens on its first curated row");
        assert!(
            (0..120).any(|x| buffer[(x, banded[0])].symbol() == "▌" && buffer[(x, banded[0])].fg == DARK.mark),
            "the banded row carries the accent mark too"
        );

        let state = FirstRun { index: 2, access: 2, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let banded: Vec<u16> = (0..36u16).filter(|y| (0..120).any(|x| buffer[(x, *y)].bg == DARK.band)).collect();
        assert_eq!(banded.len(), 1, "still one — the two settled steps show answers, not selections");
        assert!(row_text(&buffer, banded[0]).contains("reads and writes run"), "the chosen tier is banded");
    }

    /// A settled step's `●` is `done` — the same neutral a finished tool call
    /// takes, and never the gold mark. See `Palette::done` for why this is
    /// one role where there were briefly two.
    #[test]
    fn a_settled_steps_glyph_uses_the_done_role() {
        let state = FirstRun { index: 1, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let row = step_row(&buffer, "provider");
        assert_eq!(buffer[(MARGIN_X as u16, row)].fg, DARK.done);
        assert_ne!(DARK.done, DARK.mark, "settled is not gold; gold is what is open");
    }

    /// Each open step states what it is for, and only names a command that
    /// exists.
    #[test]
    fn each_step_states_its_purpose_and_promises_only_commands_that_exist() {
        let out = text(&render(&FirstRun::default(), 120, 36));
        assert!(out.contains("Where the model runs."), "{out:?}");
        let model = text(&render(&FirstRun { index: 1, ..Default::default() }, 120, 36));
        assert!(model.contains("/model changes it later"), "{model:?}");
        let access = text(&render(&FirstRun { index: 2, ..Default::default() }, 120, 36));
        assert!(access.contains("Which actions run without asking."), "{access:?}");
        assert!(!access.contains("/access"), "there is no /access command, so the frame must not promise one: {access:?}");
    }

    /// The collapsed list ends on `more`, with a `→` flush to the right
    /// margin saying the list continues.
    #[test]
    fn the_collapsed_provider_list_ends_on_a_more_row_with_a_trailing_arrow() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let more = find_row(&buffer, "more");
        let row = row_text(&buffer, more);
        assert!(row.contains("the full provider list"), "{row:?}");
        let arrow = row.chars().position(|c| c == '→').expect("a trailing arrow");
        // Two facts, and they are separate on purpose (see `row`): the
        // handoff places the arrow "flush to the 3-cell right margin", and
        // the accent band is bounded by the list rather than by the frame.
        assert_eq!(arrow, 120 - MARGIN_X - 1, "the arrow sits against the 3-cell right margin");
        let selected = (0..36).find(|&y| buffer[(STEP_CONTENT_COL as u16, y)].bg == DARK.band).expect("a selected option row");
        let band: Vec<usize> = (0..120).filter(|&x| buffer[(x as u16, selected)].bg == DARK.band).collect();
        assert_eq!(band.first().copied(), Some(STEP_CONTENT_COL), "the band starts at the mark, not at the frame margin");
        assert!(band.len() < 120 - STEP_CONTENT_COL - MARGIN_X, "and the band is narrower than the frame allows — its width is the list's content");
        assert!(!text(&buffer).contains("delta"), "the rest of the catalogue stays behind the row until it is taken");
    }

    /// Taking `more` replaces it with the rest of the catalogue.
    #[test]
    fn expanding_shows_every_provider_and_drops_the_more_row() {
        let state = FirstRun { expanded: true, ..Default::default() };
        let out = text(&render(&state, 120, 36));
        for id in sample_providers().iter().map(|p| p.id.clone()) {
            assert!(out.contains(&id), "expanded, every provider shows: {id} missing");
        }
        assert!(!out.contains("the full provider list"), "`more` has nothing left to reveal: {out:?}");
    }

    /// The bands are 3 / rest / 3 rows, the three-row `--section-gap-h` is
    /// used exactly once, and the steps themselves are one blank row apart.
    #[test]
    fn the_bands_and_the_gaps_are_whole_rows() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let blank = |y: u16| row_text(&buffer, y).trim().is_empty();

        for y in 0..TOP_BAR_ROWS {
            assert_eq!(buffer[(0, y)].bg, DARK.bar, "the top bar is {TOP_BAR_ROWS} rows");
        }
        assert_eq!(buffer[(0, TOP_BAR_ROWS)].bg, DARK.ground, "and the body begins immediately after it");
        for y in (36 - FOOTER_ROWS)..36 {
            assert_eq!(buffer[(0, y)].bg, DARK.bar_bottom, "the footer is {FOOTER_ROWS} rows");
        }

        let positioning = find_row(&buffer, "The leverage of a model");
        let provider = step_row(&buffer, "provider");
        assert_eq!(provider - positioning - 1, 3, "--section-gap-h: three blank rows above the first step");
        for y in (positioning + 1)..provider {
            assert!(blank(y), "and they are genuinely blank");
        }

        let more = find_row(&buffer, "more");
        let model = step_row(&buffer, "model");
        assert_eq!(model - more - 1, 1, "one blank row parts one step from the next, not three");
        assert!(blank(more + 1));
        assert_eq!(step_row(&buffer, "access") - model - 1, 1, "and the same between the other two");
    }

    /// The whole spine has to fit the design's 36 rows with the provider
    /// list expanded, which is the tallest it ever gets — on *every* step,
    /// since which list is open changes as the form advances.
    #[test]
    fn the_expanded_screen_still_fits_the_frame() {
        for index in 0..3 {
            let state = FirstRun { expanded: true, index, ..Default::default() };
            let buffer = render(&state, 120, 36);
            let last = step_row(&buffer, "access");
            let bottom = if index == 2 { find_row(&buffer, "reads and writes run") } else { last };
            assert!(bottom < 36 - FOOTER_ROWS, "on step {index} the spine must clear the footer, not be clipped by it");
        }
        assert_eq!(SAMPLE_CURATED, 3, "the sample is shaped like the real catalogue");
    }

    /// The model list follows the provider selection rather than being
    /// fixed at construction — it is that provider's catalogue.
    #[test]
    fn the_model_list_is_the_selected_providers_own() {
        let state = FirstRun { index: 1, provider: 2, ..Default::default() };
        let out = text(&render(&state, 120, 36));
        assert!(out.contains("charlie-large"), "{out:?}");
        assert!(!out.contains("alpha-large"), "another provider's models are not on offer here: {out:?}");
    }

    /// A frame narrower than the design's 120 keeps the spine's columns and
    /// gives up the purpose text, rather than wrapping a row off the grid.
    /// The names stay readable, which is what a list is for.
    #[test]
    fn a_narrow_frame_elides_the_purpose_and_keeps_the_row_on_one_line() {
        let buffer = render(&FirstRun::default(), 60, 36);
        let row = (0..36u16).map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>()).find(|r| r.contains("alpha")).expect("a provider row");
        assert!(row.chars().count() == 60, "one row, not wrapped: {row:?}");
        assert!(row.contains("alpha"), "the name survives: {row:?}");
        assert!(!row.contains("ALPHA_API_KEY"), "the purpose does not fit and is elided rather than wrapped: {row:?}");
    }

    /// First run's top bar shares `chrome::identity_bar_row` with the
    /// session's, so it inherits the same guarantees: the two groups are
    /// parted by at least `--group-gap`, the version is shown whole or
    /// dropped, and a shortened path carries `…`.
    ///
    /// It had the same defect before they were unified — its own hand-rolled
    /// layout let the working directory run into the version — and no test
    /// covered it, because the frames are authored at 120 columns and it is
    /// only visible below about 56.
    ///
    /// The cwd here is the *real* process directory (`current_dir_display`),
    /// not a fixture, so this asserts the invariants rather than an exact
    /// row — which is the right shape for it either way.
    #[test]
    fn the_top_bar_groups_never_collide_and_never_clip_silently() {
        let version = format!("v{}", crate::version::VERSION);
        for width in [36u16, 44, 52, 60, 80, 120] {
            let buffer = render(&FirstRun::default(), width, 36);
            let row: String = (0..width).map(|x| buffer[(x, 1)].symbol().to_string()).collect();

            assert!(row.starts_with("   Aldwin"), "the brand always renders: {width} -> {row:?}");
            if let Some(at) = row.find('v') {
                assert!(row[at..].starts_with(&version), "a partial version reads as a real one: {width} -> {row:?}");
                let left_end = row[..at].trim_end().chars().count();
                assert!(row[..at].chars().count() - left_end >= GROUP_GAP, "groups too close at {width}: {row:?}");
            }
            let cwd = crate::app::current_dir_display().unwrap_or_default();
            if !cwd.is_empty() && !row.contains(&cwd) {
                assert!(row.contains('…'), "a shortened path must say so: {width} -> {row:?}");
            }
        }
    }

    /// The footer had the identity bar's defect too, and the screenshots of
    /// the *bar* are what showed it: at 44 columns it read
    /// `↑↓ chooseconfig → ~/.mjol`, the key hints running into the config
    /// location and the path then clipped. The location is a path, so it
    /// goes whole or not at all; the key hints are what a footer is for, so
    /// they are what survives.
    ///
    /// The positioning line is checked here for the same reason — it was
    /// ending mid-word at the frame's edge with nothing marking it.
    #[test]
    fn the_footer_and_the_positioning_line_never_clip_silently() {
        for width in [36u16, 44, 52, 60, 80, 120] {
            let buffer = render(&FirstRun::default(), width, 36);
            let read = |y: u16| -> String { (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect() };

            let keys = read(36 - FOOTER_ROWS + 1);
            assert!(keys.contains("⏎ continue"), "the keys always survive: {width} -> {keys:?}");
            assert!(keys.chars().count() <= width as usize, "{width} -> {keys:?}");

            // The config location rides the *status* row now, two rows under
            // the keys. Asserted to be present at the design's own width, so
            // this block cannot go quiet the way it did when the path moved
            // rows and the `if let` below simply stopped matching.
            let status = read(36 - FOOTER_ROWS + 3);
            assert!(status.trim_start().starts_with("○  waiting"), "the status always survives: {width} -> {status:?}");
            if width >= 80 {
                assert!(status.contains("config →"), "there is room for the path at {width}: {status:?}");
            }
            if let Some(at) = status.find("config →") {
                assert!(status[at..].trim_end().ends_with("~/.aldwin/"), "a clipped path reads as a path: {width} -> {status:?}");
                let left_end = status[..at].trim_end().chars().count();
                assert!(status[..at].chars().count() - left_end >= GROUP_GAP, "groups too close at {width}: {status:?}");
            }
            assert!(status.chars().count() <= width as usize, "{width} -> {status:?}");

            let line = read(find_row(&buffer, "The leverage"));
            let full = "The leverage of a model, without handing over the keys.";
            assert!(line.contains(full) || line.contains('…'), "a shortened sentence must say so: {width} -> {line:?}");
        }
    }

    /// The footer names the two keys the reference names, and states where
    /// the answers land — plainly, rather than by implication.
    #[test]
    fn the_footer_names_the_keys_and_where_answers_land() {
        let out = text(&render(&FirstRun::default(), 120, 36));
        assert!(out.contains("⏎ continue"), "the footer states the key then the verb: {out:?}");
        assert!(out.contains("↑↓ choose"), "{out:?}");
        assert!(!out.contains("← back"), "the reference's footer carries two hints; `←` stays bound but unnamed: {out:?}");
        assert!(out.contains("config → ~/.aldwin/"), "where state lives is stated plainly: {out:?}");
    }

    /// The bottom band is the session's own five rows — blank, keys, blank,
    /// status, blank — on `bar_bottom`, not a 3-row footer of this screen's
    /// own. `cells.css`: "blank, prompt or keys, blank, status, blank —
    /// every frame".
    #[test]
    fn the_bottom_band_is_the_same_five_rows_every_frame_ends_in() {
        let buffer = render(&FirstRun::default(), 120, 36);
        assert_eq!(FOOTER_ROWS, 5, "--bar-bottom-h");
        for y in 36 - FOOTER_ROWS..36 {
            assert_eq!(buffer[(60, y)].bg, DARK.bar_bottom, "row {y} is on the bottom band's ground");
        }
        assert_eq!(buffer[(60, 36 - FOOTER_ROWS - 1)].bg, DARK.ground, "and the row above it is the body");

        let read = |y: u16| row_text(&buffer, y);
        let base = 36 - FOOTER_ROWS;
        for blank in [base, base + 2, base + 4] {
            assert!(read(blank).trim().is_empty(), "row {blank} is blank: {:?}", read(blank));
        }
        assert!(read(base + 1).contains("⏎ continue"), "keys on the row a prompt would take");
        assert!(read(base + 3).contains("○  waiting"), "status two rows under them");
    }

    /// Nothing in the bottom band is gold. The hint keys were `--tui-mark`
    /// until the repaint; gold is "spent on one thing per band" and the open
    /// step, up in the body, is what this screen spends it on.
    #[test]
    fn no_key_in_the_bottom_band_is_gold() {
        let buffer = render(&FirstRun::default(), 120, 36);
        for y in 36 - FOOTER_ROWS..36 {
            for x in 0..120u16 {
                let cell = &buffer[(x, y)];
                assert!(cell.symbol().trim().is_empty() || cell.fg != DARK.mark, "({x},{y}) {:?} is gold", cell.symbol());
            }
        }
        let keys = 36 - FOOTER_ROWS + 1;
        assert_eq!(buffer[(MARGIN_X as u16, keys)].fg, DARK.body, "a key is `body`");
        assert_eq!(buffer[(MARGIN_X as u16 + 2, keys)].fg, DARK.quiet, "its verb is `quiet`");
        assert_eq!(buffer[(MARGIN_X as u16, keys + 2)].fg, DARK.glyph_pending, "the waiting `○` is the pending tone");
    }

    /// `⏎` says what it does. On the last step it starts the session, and
    /// the reference changes the verb to say so (`1c`).
    #[test]
    fn the_last_steps_enter_key_says_it_starts_the_session() {
        let first = text(&render(&FirstRun::default(), 120, 36));
        assert!(first.contains("⏎ continue") && !first.contains("start session"), "{first:?}");
        let last = text(&render(&FirstRun { index: 2, ..Default::default() }, 120, 36));
        assert!(last.contains("⏎ start session") && !last.contains("⏎ continue"), "{last:?}");
    }

    /// A selected row is the band and the mark. Its purpose stays `quiet`,
    /// exactly as on an idle row — see `row`.
    #[test]
    fn a_selected_rows_purpose_stays_quiet() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let y = find_row(&buffer, "alpha models");
        let at = col_of(&buffer, y, "alpha models") as u16;
        assert_eq!(buffer[(at, y)].bg, DARK.band, "this is the selected row");
        assert_eq!(buffer[(at, y)].fg, DARK.quiet, "and its purpose is not lifted with it");
        assert_eq!(buffer[(STEP_CONTENT_COL as u16 + 3, y)].fg, DARK.accent_text, "the name is");
    }

    /// No tier may promise that edits run without asking — the one claim
    /// the harness structurally cannot honour.
    #[test]
    fn no_access_row_claims_edits_run_unasked() {
        let out = text(&render(&FirstRun { index: 2, ..Default::default() }, 120, 36));
        assert!(!out.contains("nothing asks"), "the design's original top-tier copy would be false here: {out:?}");
        for tier in AccessTier::ORDER {
            assert!(tier.purpose().contains("ask"), "{} must say what still asks", tier.label());
        }
    }

    #[test]
    #[ignore = "visual aid; run with --ignored to eyeball the screen"]
    fn dump() {
        for state in [
            FirstRun::default(),
            FirstRun { expanded: true, ..Default::default() },
            FirstRun { index: 1, ..Default::default() },
            FirstRun { index: 2, model: 1, ..Default::default() },
        ] {
            println!("\n=== step {:?} ===", state.step());
            let buffer = render(&state, 120, 36);
            for y in 0..36 {
                let banded = (0..120).any(|x| buffer[(x, y)].bg == DARK.band);
                println!("{y:2}|{}|{}", row_text(&buffer, y), if banded { " <- selected" } else { "" });
            }
        }
    }
}
