//! The design system's screen `5d` — first run.
//!
//! Three bands, like every other screen: a 3-row top bar, the body, and a
//! 3-row footer (`--bar-keys-h`). The body is the wordmark, the positioning
//! line, and the two question sections, parted by three blank rows
//! (`--section-gap-h`).
//!
//! Every left-hand word sits on the same 8-cell label column the transcript
//! uses, so a first-run step and a conversation turn line up on one edge —
//! which is the design system's stated reason for having one label column
//! at all.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::grid::{with_label_column, Ctx, CONTENT_INDENT, MARGIN_X};
use super::row::band_row;
use crate::first_run::{AccessTier, FirstRun, Step, MODELS};
use crate::palette::Palette;

/// Rows the top bar and the footer take. The body gets the rest.
const TOP_BAR_ROWS: u16 = 3;
const FOOTER_ROWS: u16 = 3;

/// The option name field, in cells. One width for every list in the system
/// — the model list and the access list are the same control, so they share
/// it (`--option-label-col`).
const OPTION_LABEL_COL: usize = 16;

/// `--group-gap`: 6 cells part two unrelated groups inside a bar.
const GROUP_GAP: usize = 6;

/// Where the harness's answers land. Stated plainly rather than implied,
/// per the design system's Content Fundamentals. A directory, not a single
/// file, because the two answers land in two files inside it.
const CONFIG_LOCATION: &str = "config → ~/.mjolnir/";

pub(crate) fn draw(frame: &mut Frame, state: &FirstRun, pal: &Palette) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(pal.ground)), area);

    let [top, body, footer] =
        Layout::vertical([Constraint::Length(TOP_BAR_ROWS), Constraint::Min(1), Constraint::Length(FOOTER_ROWS)]).areas(area);

    draw_top_bar(frame, top, pal);
    let ctx = Ctx::new(pal, body.width);
    frame.render_widget(Paragraph::new(Text::from(body_lines(state, ctx))), body);
    draw_footer(frame, footer, ctx);
}

/// The same 3-row identity band every screen opens with, on `bar`: the
/// plain word `mjolnir`, a 6-cell group gap, the working directory, and the
/// version flush to the right margin — what the reference's own `5d` top bar
/// carries.
///
/// The wordmark below is a different thing and deliberately not repeated
/// here ("It is not in the top bar"), and the bar carries no `▌` either —
/// "the name is the brand, and a pip there indicated nothing".
fn draw_top_bar(frame: &mut Frame, area: Rect, pal: &Palette) {
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar)), area);
    let Some(row) = area.height.checked_sub(2).map(|_| Rect { y: area.y + 1, height: 1, ..area }) else { return };
    let on_bar = |fg| Style::default().fg(fg).bg(pal.bar);

    let cwd = crate::app::current_dir_display().unwrap_or_default();
    let version = format!("v{}", crate::version::VERSION);
    let mut spans = vec![
        Span::styled(" ".repeat(MARGIN_X), Style::default().bg(pal.bar)),
        Span::styled("mjolnir", on_bar(pal.text)),
        // `--group-gap`: 6 cells between two unrelated groups.
        Span::styled(" ".repeat(GROUP_GAP), Style::default().bg(pal.bar)),
        Span::styled(cwd.clone(), on_bar(pal.dim)),
    ];
    let used = MARGIN_X + "mjolnir".chars().count() + GROUP_GAP + cwd.chars().count();
    let gap = (area.width as usize).saturating_sub(used).saturating_sub(version.chars().count()).saturating_sub(MARGIN_X);
    spans.push(Span::styled(" ".repeat(gap), Style::default().bg(pal.bar)));
    spans.push(Span::styled(version, on_bar(pal.dim)));
    spans.push(Span::styled(" ".repeat(MARGIN_X), Style::default().bg(pal.bar)));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(pal.bar)), row);
}

fn body_lines(state: &FirstRun, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let mut lines = vec![Line::default(), wordmark(ctx), Line::default(), positioning_line(ctx)];
    lines.extend(section_gap(ctx));

    if let Some((n, m)) = state.position(Step::Model) {
        let rows: Vec<Line<'static>> =
            MODELS.iter().enumerate().map(|(i, spec)| option_row(spec.label, spec.purpose, i == state.model, ctx)).collect();
        lines.extend(step_section("model", (n, m), state.step() == Step::Model, rows, ctx));
        lines.extend(section_gap(ctx));
    }

    if let Some((n, m)) = state.position(Step::Access) {
        let rows: Vec<Line<'static>> = AccessTier::ORDER
            .iter()
            .enumerate()
            .map(|(i, tier)| option_row(tier.label(), tier.purpose(), i == state.access, ctx))
            .collect();
        lines.extend(step_section("access", (n, m), state.step() == Step::Access, rows, ctx));
    }

    let _ = pal;
    lines
}

/// `  M J O L N I R  ` in reverse video — the accent as the ground, the desk
/// as the ink, letters one space apart, the whole run padded by one space at
/// each end. One row, never a block: a multi-row block-character wordmark
/// was built and cut because "at 15px it dominated a frame whose whole
/// argument is that nothing shouts".
///
/// This is one of exactly two places the accent is allowed to be a filled
/// field; the selection band is the other.
fn wordmark(ctx: Ctx) -> Line<'static> {
    let letters: String = "MJOLNIR".chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled(format!(" {letters} "), Style::default().fg(ctx.pal.reverse_ink).bg(ctx.pal.reverse_bg)),
    ])
}

fn positioning_line(ctx: Ctx) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled("The leverage of a model, without handing over the keys.", Style::default().fg(ctx.pal.dim)),
    ])
}

/// `--section-gap-h`: three blank rows between first-run sections.
fn section_gap(_ctx: Ctx) -> Vec<Line<'static>> {
    vec![Line::default(), Line::default(), Line::default()]
}

/// One question: its name in the label column, `n of m` beneath it, and its
/// option rows in the body column.
///
/// The counter is `1 of 2`, not `step 1 of 2`: the label column is 8 cells
/// wide and the longer form overflowed it into the first option row. The
/// reference wrote "step 2 of 3" when that column was 12 cells.
///
/// Only the *active* step's label takes the accent; an inactive one stays
/// on the neutral label step, and only the active step states its number —
/// a counter under a question already answered is noise.
fn step_section(name: &'static str, (number, total): (usize, usize), active: bool, rows: Vec<Line<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let label_fg = if active { pal.speaker_you } else { pal.label };
    let mut out = with_label_column(rows, Some((name, label_fg)));
    if active {
        // The counter belongs on the row *under* the label, in the label
        // column — the same two-row shape a transcript turn uses for its
        // speaker and its time.
        // Padded out to the body column by hand: this row replaces the one
        // `with_label_column` would have left blank, so it owes the same
        // `CONTENT_INDENT` cells before the option row resumes.
        let text = format!("{number} of {total}");
        let pad = CONTENT_INDENT.saturating_sub(MARGIN_X).saturating_sub(text.chars().count());
        let counter = Line::from(vec![
            Span::raw(" ".repeat(MARGIN_X)),
            Span::styled(text, Style::default().fg(pal.dim)),
            Span::raw(" ".repeat(pad)),
        ]);
        // Row 0 already carries the label; the counter goes on row 1, which
        // `with_label_column` left blank. Splicing rather than appending
        // keeps it beside the first option rather than under the last.
        if out.len() > 1 {
            let mut spans = counter.spans;
            let existing = out[1].spans.clone();
            // Drop the blank label-column span the helper inserted and put
            // the counter in its place, keeping the option row beside it.
            spans.extend(existing.into_iter().skip(1));
            out[1] = Line::from(spans);
        } else {
            out.push(counter);
        }
    }
    out
}

/// The one option row shape in the system: an idle or selected `▌`, two
/// spaces, the name in a 16-cell field, then a purpose statement saying
/// what picking it does.
///
/// Selection is the accent `▌` *and* the band together, never one alone.
/// The band is a real accent fill and stops at the right margin, so it
/// reads as belonging to the body column rather than to the whole frame.
fn option_row(name: &str, purpose: &str, selected: bool, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let (mark_fg, bg, name_fg) =
        if selected { (pal.mark, pal.band, pal.text) } else { (pal.mark_idle, pal.ground, pal.body) };
    let field = Style::default().bg(bg);

    let mut spans = vec![Span::styled("▌", Style::default().fg(mark_fg).bg(bg)), Span::styled("  ".to_string(), field)];
    let pad = OPTION_LABEL_COL.saturating_sub(name.chars().count());
    spans.push(Span::styled(name.to_string(), Style::default().fg(name_fg).bg(bg)));
    spans.push(Span::styled(" ".repeat(pad), field));
    spans.push(Span::styled(purpose.to_string(), Style::default().fg(pal.quiet).bg(bg)));

    // Fill to the right margin so the band is a band, not a ragged
    // highlight ending wherever the purpose text happens to stop.
    let used: usize = 3 + name.chars().count() + pad + purpose.chars().count();
    let width = (ctx.width as usize).saturating_sub(CONTENT_INDENT).saturating_sub(MARGIN_X);
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), field));
    Line::from(spans)
}

/// Three rows — blank, keys, blank — on the composer's own step, with the
/// keys left and where the answers land flush right.
fn draw_footer(frame: &mut Frame, area: Rect, ctx: Ctx) {
    let pal = ctx.pal;
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), area);
    let Some(row) = area.height.checked_sub(2).map(|_| Rect { y: area.y + 1, height: 1, ..area }) else { return };

    let key = |k: &str, verb: &str| {
        vec![
            Span::styled(k.to_string(), Style::default().fg(pal.mark).bg(pal.bar_bottom)),
            Span::styled(format!(" {verb}"), Style::default().fg(pal.quiet).bg(pal.bar_bottom)),
        ]
    };
    let mut left = vec![Span::styled(" ".repeat(MARGIN_X), Style::default().bg(pal.bar_bottom))];
    for (i, (k, verb)) in [("↑↓", "choose"), ("⏎", "continue")].into_iter().enumerate() {
        if i > 0 {
            left.push(Span::styled("      ", Style::default().bg(pal.bar_bottom)));
        }
        left.extend(key(k, verb));
    }
    let used: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let width = area.width as usize;
    let gap = width.saturating_sub(used).saturating_sub(CONFIG_LOCATION.chars().count()).saturating_sub(MARGIN_X);
    left.push(Span::styled(" ".repeat(gap), Style::default().bg(pal.bar_bottom)));
    left.push(Span::styled(CONFIG_LOCATION.to_string(), Style::default().fg(pal.dim).bg(pal.bar_bottom)));
    left.push(Span::styled(" ".repeat(MARGIN_X), Style::default().bg(pal.bar_bottom)));

    frame.render_widget(Paragraph::new(Line::from(left)).style(Style::default().bg(pal.bar_bottom)), row);
}

/// A one-row band of `break_`, used nowhere on this screen yet but kept
/// importable so a future section separator uses the system's own row.
#[allow(dead_code)]
fn separator(ctx: Ctx) -> Line<'static> {
    band_row(ctx.pal.break_, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::DARK;
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

    /// The wordmark is one row of reverse video — the accent as the ground,
    /// the desk as the ink — and never a block. It is the one place besides
    /// the selection band where the accent is a filled field.
    #[test]
    fn the_wordmark_is_one_reverse_video_row() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let out = text(&buffer);
        assert!(out.contains(" M J O L N I R "), "the wordmark is letters one space apart, padded at each end: {out:?}");

        let row = (0..36u16).find(|y| (0..120).any(|x| buffer[(x, *y)].symbol() == "M")).expect("a wordmark row");
        let cell = &buffer[(MARGIN_X as u16 + 1, row)];
        assert_eq!(cell.bg, DARK.reverse_bg, "the accent is the ground");
        assert_eq!(cell.fg, DARK.reverse_ink, "and the desk colour is the ink");

        let below = (0..120).filter(|x| buffer[(*x, row + 1)].bg == DARK.reverse_bg).count();
        assert_eq!(below, 0, "one row, never a block — the row under it carries no reverse video");
    }

    /// Both lists open with a row selected, and selection is always the
    /// accent `▌` *and* the band together — never one without the other.
    /// `ask` is the preselected access row (see `FirstRun::access`).
    #[test]
    fn both_lists_open_with_a_row_selected() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let banded: Vec<u16> = (0..36u16).filter(|y| (0..120).any(|x| buffer[(x, *y)].bg == DARK.band)).collect();
        assert_eq!(banded.len(), 2, "one banded row per list — the model default and `ask`");
        for y in banded {
            assert!(
                (0..120).any(|x| buffer[(x, y)].symbol() == "▌" && buffer[(x, y)].fg == DARK.mark),
                "row {y} carries the band, so it must carry the accent mark too"
            );
        }
        let row: String = (0..120).map(|x| buffer[(x, banded_ask(&buffer))].symbol()).collect();
        assert!(row.contains("ask"), "the preselected access row is `ask`: {row:?}");
    }

    /// The second banded row — the access list's selection.
    fn banded_ask(buffer: &ratatui::buffer::Buffer) -> u16 {
        (0..36u16).filter(|y| (0..120).any(|x| buffer[(x, *y)].bg == DARK.band)).nth(1).expect("an access selection")
    }

    /// Selecting an access tier lights exactly that row, mark and band
    /// together — never one without the other.
    #[test]
    fn selecting_an_access_tier_bands_that_row_and_marks_it() {
        let state = FirstRun { index: 1, access: 2, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let row = (0..36u16).find(|y| (0..120).any(|x| buffer[(x, *y)].symbol() == "a" && buffer[(x, *y)].bg == DARK.band)).is_some();
        assert!(row, "the chosen tier's row carries the selection band");
        let marks = (0..36u16)
            .filter(|y| (0..120).any(|x| buffer[(x, *y)].symbol() == "▌" && buffer[(x, *y)].fg == DARK.mark))
            .count();
        assert_eq!(marks, 2, "one accent mark per step — the model row and the chosen access row");
    }

    /// Every landmark on this screen is a whole number of cells off the
    /// grid: the 3-cell margin, the 8-cell label column with its 2-cell
    /// gutter (so body text lands on cell 13), and the 16-cell option name
    /// field the model list and the access list share.
    #[test]
    fn every_column_lands_on_the_grid() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let row_text = |y: u16| -> String { (0..120).map(|x| buffer[(x, y)].symbol()).collect() };
        // Byte offset converted to a *cell* offset: `▌` is three bytes, so
        // `str::find` alone would report every column past a mark two cells
        // to the right of where it actually is. The grid counts cells.
        let col_of = |y: u16, needle: &str| -> usize {
            let row = row_text(y);
            let byte = row.find(needle).expect("needle on row");
            row[..byte].chars().count()
        };

        // Anchored on the option row, not on the word "model" — the
        // positioning line contains that word too, 21 cells in.
        let label = (0..36u16).find(|y| row_text(*y).contains("sonnet-5")).expect("the model label row");
        assert_eq!(col_of(label, "model"), MARGIN_X, "a step label starts on the 3-cell margin");
        assert_eq!(col_of(label, "▌"), CONTENT_INDENT, "its option rows start on the body column, cell 13");
        assert_eq!(col_of(label, "sonnet-5"), CONTENT_INDENT + 3, "the mark plus two spaces, then the name");
        assert_eq!(col_of(label, "balanced"), CONTENT_INDENT + 3 + OPTION_LABEL_COL, "the purpose starts past the 16-cell name field");

        let access = (0..36u16).find(|y| row_text(*y).contains("access")).expect("the access label row");
        assert_eq!(col_of(access, "access"), MARGIN_X, "both steps share one label column");
        assert_eq!(col_of(access, "▌"), CONTENT_INDENT);
        assert_eq!(col_of(access, "every tool"), CONTENT_INDENT + 3 + OPTION_LABEL_COL, "and one name field, so the two lists read as one control");

        let mark = (0..36u16).find(|y| row_text(*y).contains("M J O L N I R")).expect("the wordmark");
        assert_eq!(col_of(mark, "M J O L N I R"), MARGIN_X + 1, "the wordmark's own one-space pad sits inside the margin");
    }

    /// `--section-gap-h`: three blank rows between first-run sections, and
    /// the bands are 3 / rest / 3 rows.
    #[test]
    fn the_vertical_bands_and_section_gaps_are_whole_rows() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let row_text = |y: u16| -> String { (0..120).map(|x| buffer[(x, y)].symbol()).collect() };
        let blank = |y: u16| row_text(y).trim().is_empty();

        for y in 0..TOP_BAR_ROWS {
            assert_eq!(buffer[(0, y)].bg, DARK.bar, "the top bar is {TOP_BAR_ROWS} rows");
        }
        assert_eq!(buffer[(0, TOP_BAR_ROWS)].bg, DARK.ground, "and the body begins immediately after it");
        for y in (36 - FOOTER_ROWS)..36 {
            assert_eq!(buffer[(0, y)].bg, DARK.bar_bottom, "the footer is {FOOTER_ROWS} rows");
        }

        let last_model = (0..36u16).rev().find(|y| row_text(*y).contains("haiku")).expect("the last model row");
        let access = (0..36u16).find(|y| row_text(*y).contains("access")).expect("the access label");
        assert_eq!(access - last_model - 1, 3, "three blank rows part the sections");
        for y in (last_model + 1)..access {
            assert!(blank(y), "and they are genuinely blank");
        }
    }

    #[test]
    #[ignore = "visual aid; run with --ignored to eyeball the screen"]
    fn dump() {
        let buffer = render(&FirstRun::default(), 120, 36);
        for y in 0..36 {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
            let banded = (0..120).any(|x| buffer[(x, y)].bg == DARK.band);
            println!("{y:2}|{row}|{}", if banded { " <- selected" } else { "" });
        }
    }

    /// The footer states where the answers land, plainly rather than by
    /// implication.
    #[test]
    fn the_footer_names_the_keys_and_where_answers_land() {
        let out = text(&render(&FirstRun::default(), 120, 36));
        assert!(out.contains("↑↓ choose"), "the footer states the key then the verb: {out:?}");
        assert!(out.contains("⏎ continue"), "{out:?}");
        assert!(out.contains("config → ~/.mjolnir/"), "where state lives is stated plainly: {out:?}");
    }

    /// No tier may promise that edits run without asking — the one claim
    /// the harness structurally cannot honour.
    #[test]
    fn no_access_row_claims_edits_run_unasked() {
        let out = text(&render(&FirstRun::default(), 120, 36));
        assert!(!out.contains("nothing asks"), "the design's original top-tier copy would be false here: {out:?}");
        for tier in AccessTier::ORDER {
            assert!(tier.purpose().contains("ask"), "{} must say what still asks", tier.label());
        }
    }
}
