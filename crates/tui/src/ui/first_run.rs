//! The design system's screen `5d` — first run.
//!
//! Three bands, like every other screen: a 3-row top bar, the body, and a
//! 3-row footer (`--bar-keys-h`). The body is the wordmark, the positioning
//! line, and the two question sections, parted by three blank rows
//! (`--section-gap-h`).
//!
//! Each section is its name in the label column with `step n/m` beneath it,
//! and, in the body column, one row of prose saying what the question is
//! for, a blank row, then the option rows. Every left-hand word sits on the
//! same 8-cell label column the transcript uses, so a first-run step and a
//! conversation turn line up on one edge — which is the design system's
//! stated reason for having one label column at all.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::chrome::{brand_pad, BRAND};
use super::grid::{with_label_column, Ctx, CONTENT_INDENT, MARGIN_X};
use super::row::band_row;
use crate::first_run::{AccessTier, FirstRun, Step};
use crate::palette::Palette;

/// Rows the top bar and the footer take. The body gets the rest.
const TOP_BAR_ROWS: u16 = 3;
const FOOTER_ROWS: u16 = 3;

/// The option name field, in cells. One width for every list in the system
/// — the provider list and the access list are the same control, so they
/// share it (`--option-label-col`).
const OPTION_LABEL_COL: usize = 16;

/// `--group-gap`: 6 cells part two unrelated groups inside a bar — here,
/// the footer's two key hints. Deliberately *not* what sits between the
/// brand and the working directory; that is a pad to the body column (see
/// `chrome::brand_pad`).
const GROUP_GAP: usize = 6;

/// Where the harness's answers land. Stated plainly rather than implied,
/// per the design system's Content Fundamentals. A directory, not a single
/// file, because the two answers land in two files inside it.
const CONFIG_LOCATION: &str = "config → ~/.mjolnir/";

/// The `more` row's own name and purpose. It is not a provider, so it is
/// not in the catalogue the caller hands in.
const MORE_LABEL: &str = "more";
const MORE_PURPOSE: &str = "the full provider list";

/// What each question is for, in one row of body-column prose above its
/// options.
///
/// `provider` names `/model` because that command exists and does what the
/// sentence says. The design's `access` copy also promised "/access changes
/// it later"; there is no `/access`, so that clause is dropped rather than
/// shipped false — the same call the model step's prose used to require.
const PROVIDER_PROSE: &str = "Where the model runs. /model picks a model once the session starts.";
const ACCESS_PROSE: &str = "Which actions run without asking.";

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
/// plain word `mjolnir`, the working directory starting on the body column,
/// and the version flush to the right margin — what the reference's own `5d`
/// top bar carries.
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
        Span::styled(BRAND, on_bar(pal.text)),
        // The cwd lands on the body column, cell 13 — not `--group-gap`
        // away. See `chrome::brand_pad`; the session bar draws the same
        // thing, and the two must not disagree.
        Span::styled(" ".repeat(brand_pad()), Style::default().bg(pal.bar)),
        Span::styled(cwd.clone(), on_bar(pal.dim)),
    ];
    let used = MARGIN_X + BRAND.chars().count() + brand_pad() + cwd.chars().count();
    let gap = (area.width as usize).saturating_sub(used).saturating_sub(version.chars().count()).saturating_sub(MARGIN_X);
    spans.push(Span::styled(" ".repeat(gap), Style::default().bg(pal.bar)));
    spans.push(Span::styled(version, on_bar(pal.dim)));
    spans.push(Span::styled(" ".repeat(MARGIN_X), Style::default().bg(pal.bar)));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(pal.bar)), row);
}

fn body_lines(state: &FirstRun, ctx: Ctx) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default(), wordmark(ctx), Line::default(), positioning_line(ctx)];
    lines.extend(section_gap(ctx));

    if let Some((n, m)) = state.position(Step::Provider) {
        let visible = state.visible_providers();
        let mut rows: Vec<Line<'static>> = visible
            .iter()
            .enumerate()
            .map(|(i, choice)| option_row(&choice.id, &choice.purpose, i == state.provider, ctx))
            .collect();
        if state.shows_more() {
            rows.push(more_row(state.provider == visible.len(), ctx));
        }
        lines.extend(step_section("provider", (n, m), state.step() == Step::Provider, PROVIDER_PROSE, rows, ctx));
        lines.extend(section_gap(ctx));
    }

    if let Some((n, m)) = state.position(Step::Access) {
        let rows: Vec<Line<'static>> = AccessTier::ORDER
            .iter()
            .enumerate()
            .map(|(i, tier)| option_row(tier.label(), tier.purpose(), i == state.access, ctx))
            .collect();
        lines.extend(step_section("access", (n, m), state.step() == Step::Access, ACCESS_PROSE, rows, ctx));
    }

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

/// One question: its name in the label column, `step n/m` beneath it, and
/// its prose, a blank row and its option rows in the body column.
///
/// The counter is `step n/m`, which is exactly 8 cells — the label column's
/// full width, and the reason the design writes it with a slash. An earlier
/// pass shortened it to `1 of 2` while trying to fit `step 1 of 2` into the
/// same column; the reference's own wording fits, so it is used.
///
/// Both steps state their number, as the reference does — the counter says
/// how long the screen is, which is as much use on the question already
/// answered as on the live one. Only the *active* step's label takes the
/// accent; an inactive one stays on the neutral label step.
fn step_section(
    name: &'static str,
    (number, total): (usize, usize),
    active: bool,
    prose: &'static str,
    rows: Vec<Line<'static>>,
    ctx: Ctx,
) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let label_fg = if active { pal.speaker_you } else { pal.label };

    let mut body = vec![
        Line::from(Span::styled(prose, Style::default().fg(pal.body))),
        Line::default(),
    ];
    body.extend(rows);

    let mut out = with_label_column(body, Some((name, label_fg)));

    // The counter belongs on the row *under* the label, in the label
    // column — the same two-row shape a transcript turn uses for its
    // speaker and its time. That row is the blank one between the prose
    // and the options, so the counter replaces the blank label-column
    // prefix `with_label_column` left there rather than displacing an
    // option row.
    let text = format!("step {number}/{total}");
    let pad = CONTENT_INDENT.saturating_sub(MARGIN_X).saturating_sub(text.chars().count());
    let mut spans =
        vec![Span::raw(" ".repeat(MARGIN_X)), Span::styled(text, Style::default().fg(pal.dim)), Span::raw(" ".repeat(pad))];
    if out.len() > 1 {
        spans.extend(out[1].spans.clone().into_iter().skip(1));
        out[1] = Line::from(spans);
    } else {
        out.push(Line::from(spans));
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
    row(name, purpose, selected, None, ctx)
}

/// The `more` row, which ends the collapsed provider list: the same shape,
/// with its name one step quieter than a real option — it is a way of
/// asking the question again, not an answer to it — and a `→` flush to the
/// right margin saying the list continues.
fn more_row(selected: bool, ctx: Ctx) -> Line<'static> {
    row(MORE_LABEL, MORE_PURPOSE, selected, Some("→"), ctx)
}

fn row(name: &str, purpose: &str, selected: bool, trailing: Option<&str>, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let quiet_name = trailing.is_some() && !selected;
    let (mark_fg, bg, name_fg, purpose_fg) = if selected {
        (pal.mark, pal.band, pal.text, pal.accent_text)
    } else if quiet_name {
        (pal.mark_idle, pal.ground, pal.label, pal.dim)
    } else {
        (pal.mark_idle, pal.ground, pal.body, pal.quiet)
    };
    let field = Style::default().bg(bg);

    let mut spans = vec![Span::styled("▌", Style::default().fg(mark_fg).bg(bg)), Span::styled("  ".to_string(), field)];
    let pad = OPTION_LABEL_COL.saturating_sub(name.chars().count());
    spans.push(Span::styled(name.to_string(), Style::default().fg(name_fg).bg(bg)));
    spans.push(Span::styled(" ".repeat(pad), field));
    spans.push(Span::styled(purpose.to_string(), Style::default().fg(purpose_fg).bg(bg)));

    // Fill to the right margin so the band is a band, not a ragged
    // highlight ending wherever the purpose text happens to stop.
    let trailing = trailing.unwrap_or("");
    let used: usize = 3 + name.chars().count() + pad + purpose.chars().count() + trailing.chars().count();
    let width = (ctx.width as usize).saturating_sub(CONTENT_INDENT).saturating_sub(MARGIN_X);
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), field));
    if !trailing.is_empty() {
        spans.push(Span::styled(trailing.to_string(), Style::default().fg(if selected { pal.text } else { pal.label }).bg(bg)));
    }
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
    for (i, (k, verb)) in [("⏎", "continue"), ("↑↓", "choose")].into_iter().enumerate() {
        if i > 0 {
            left.push(Span::styled(" ".repeat(GROUP_GAP), Style::default().bg(pal.bar_bottom)));
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
    use crate::first_run::{sample_providers, SAMPLE_CURATED};
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

    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..120).map(|x| buffer[(x, y)].symbol()).collect()
    }

    fn find_row(buffer: &ratatui::buffer::Buffer, needle: &str) -> u16 {
        (0..36u16).find(|y| row_text(buffer, *y).contains(needle)).unwrap_or_else(|| panic!("no row containing {needle:?}"))
    }

    /// The wordmark is one row of reverse video — the accent as the ground,
    /// the desk as the ink — and never a block. It is the one place besides
    /// the selection band where the accent is a filled field.
    #[test]
    fn the_wordmark_is_one_reverse_video_row() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let out = text(&buffer);
        assert!(out.contains(" M J O L N I R "), "the wordmark is letters one space apart, padded at each end: {out:?}");

        let row = find_row(&buffer, "M J O L N I R");
        let cell = &buffer[(MARGIN_X as u16 + 1, row)];
        assert_eq!(cell.bg, DARK.reverse_bg, "the accent is the ground");
        assert_eq!(cell.fg, DARK.reverse_ink, "and the desk colour is the ink");

        let below = (0..120).filter(|x| buffer[(*x, row + 1)].bg == DARK.reverse_bg).count();
        assert_eq!(below, 0, "one row, never a block — the row under it carries no reverse video");
    }

    /// Both lists open with a row selected, and selection is always the
    /// accent `▌` *and* the band together — never one without the other.
    /// `ask` is the preselected access row (see `FirstRun`).
    #[test]
    fn both_lists_open_with_a_row_selected() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let banded: Vec<u16> = (0..36u16).filter(|y| (0..120).any(|x| buffer[(x, *y)].bg == DARK.band)).collect();
        assert_eq!(banded.len(), 2, "one banded row per list — the first provider and `ask`");
        for y in &banded {
            assert!(
                (0..120).any(|x| buffer[(x, *y)].symbol() == "▌" && buffer[(x, *y)].fg == DARK.mark),
                "row {y} carries the band, so it must carry the accent mark too"
            );
        }
        assert!(row_text(&buffer, banded[0]).contains("alpha"), "the provider list opens on its first curated row");
        assert!(row_text(&buffer, banded[1]).contains("ask"), "the preselected access row is `ask`");
    }

    /// The provider step is first and the access step second, both stating
    /// their number the way the reference does.
    #[test]
    fn the_provider_step_comes_first_and_both_steps_state_their_number() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let provider = find_row(&buffer, "provider");
        let access = find_row(&buffer, "access");
        assert!(provider < access, "provider is step 1");
        assert_eq!(row_text(&buffer, provider + 1).trim_end(), format!("{}step 1/2", " ".repeat(MARGIN_X)).trim_end());
        assert!(row_text(&buffer, access + 1).contains("step 2/2"), "{:?}", row_text(&buffer, access + 1));
    }

    /// `step n/m` is exactly the label column's 8 cells, so it never
    /// overflows into the body column the way `step 1 of 2` did.
    #[test]
    fn the_counter_fits_the_label_column_exactly() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let row = row_text(&buffer, find_row(&buffer, "provider") + 1);
        let counter: String = row.chars().skip(MARGIN_X).take(8).collect();
        assert_eq!(counter, "step 1/2");
        assert!(row.chars().skip(MARGIN_X + 8).take(2).all(|c| c == ' '), "the 2-cell gutter must stay blank: {row:?}");
    }

    /// Each question states what it is for in one row of body-column prose
    /// above its options, and only names a command that exists.
    #[test]
    fn each_step_states_its_purpose_and_promises_only_commands_that_exist() {
        let out = text(&render(&FirstRun::default(), 120, 36));
        assert!(out.contains("Where the model runs."), "{out:?}");
        assert!(out.contains("/model picks a model"), "{out:?}");
        assert!(out.contains("Which actions run without asking."), "{out:?}");
        assert!(!out.contains("/access"), "there is no /access command, so the frame must not promise one: {out:?}");
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
        assert_eq!(arrow, 120 - MARGIN_X - 1, "the arrow sits against the 3-cell right margin");
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

    /// Selecting an access tier lights exactly that row, mark and band
    /// together — never one without the other.
    #[test]
    fn selecting_an_access_tier_bands_that_row_and_marks_it() {
        let state = FirstRun { index: 1, access: 2, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let banded = (0..36u16).any(|y| (0..120).any(|x| buffer[(x, y)].symbol() == "a" && buffer[(x, y)].bg == DARK.band));
        assert!(banded, "the chosen tier's row carries the selection band");
        let marks = (0..36u16)
            .filter(|y| (0..120).any(|x| buffer[(x, *y)].symbol() == "▌" && buffer[(x, *y)].fg == DARK.mark))
            .count();
        assert_eq!(marks, 2, "one accent mark per step — the chosen provider and the chosen access row");
    }

    /// Every landmark on this screen is a whole number of cells off the
    /// grid: the 3-cell margin, the 8-cell label column with its 2-cell
    /// gutter (so body text lands on cell 13), and the 16-cell option name
    /// field the provider list and the access list share.
    #[test]
    fn every_column_lands_on_the_grid() {
        let buffer = render(&FirstRun::default(), 120, 36);
        // Byte offset converted to a *cell* offset: `▌` is three bytes, so
        // `str::find` alone would report every column past a mark two cells
        // to the right of where it actually is. The grid counts cells.
        let col_of = |y: u16, needle: &str| -> usize {
            let row = row_text(&buffer, y);
            let byte = row.find(needle).expect("needle on row");
            row[..byte].chars().count()
        };

        let label = find_row(&buffer, "provider");
        assert_eq!(col_of(label, "provider"), MARGIN_X, "a step label starts on the 3-cell margin");
        assert_eq!(col_of(label, "Where the model runs"), CONTENT_INDENT, "its prose starts on the body column, cell 13");

        let first = find_row(&buffer, "alpha");
        assert_eq!(col_of(first, "▌"), CONTENT_INDENT, "option rows start on the body column too");
        assert_eq!(col_of(first, "alpha"), CONTENT_INDENT + 3, "the mark plus two spaces, then the name");
        assert_eq!(col_of(first, "alpha models"), CONTENT_INDENT + 3 + OPTION_LABEL_COL, "the purpose starts past the 16-cell name field");

        let access = find_row(&buffer, "access");
        assert_eq!(col_of(access, "access"), MARGIN_X, "both steps share one label column");
        let ask = find_row(&buffer, "every tool");
        assert_eq!(col_of(ask, "▌"), CONTENT_INDENT);
        assert_eq!(col_of(ask, "every tool"), CONTENT_INDENT + 3 + OPTION_LABEL_COL, "and one name field, so the two lists read as one control");

        let mark = find_row(&buffer, "M J O L N I R");
        assert_eq!(col_of(mark, "M J O L N I R"), MARGIN_X + 1, "the wordmark's own one-space pad sits inside the margin");
    }

    /// `--section-gap-h`: three blank rows between first-run sections, and
    /// the bands are 3 / rest / 3 rows.
    #[test]
    fn the_vertical_bands_and_section_gaps_are_whole_rows() {
        let buffer = render(&FirstRun::default(), 120, 36);
        let blank = |y: u16| row_text(&buffer, y).trim().is_empty();

        for y in 0..TOP_BAR_ROWS {
            assert_eq!(buffer[(0, y)].bg, DARK.bar, "the top bar is {TOP_BAR_ROWS} rows");
        }
        assert_eq!(buffer[(0, TOP_BAR_ROWS)].bg, DARK.ground, "and the body begins immediately after it");
        for y in (36 - FOOTER_ROWS)..36 {
            assert_eq!(buffer[(0, y)].bg, DARK.bar_bottom, "the footer is {FOOTER_ROWS} rows");
        }

        let last_provider = find_row(&buffer, "more");
        let access = find_row(&buffer, "access");
        assert_eq!(access - last_provider - 1, 3, "three blank rows part the sections");
        for y in (last_provider + 1)..access {
            assert!(blank(y), "and they are genuinely blank");
        }
    }

    /// The whole screen has to fit the design's 36 rows with the list
    /// expanded, which is the tallest it ever gets.
    #[test]
    fn the_expanded_screen_still_fits_the_frame() {
        let state = FirstRun { expanded: true, ..Default::default() };
        let buffer = render(&state, 120, 36);
        let last_access = find_row(&buffer, "reads and any command run");
        assert!(last_access < 36 - FOOTER_ROWS, "the last option row must clear the footer, not be clipped by it");
        assert_eq!(SAMPLE_CURATED, 3, "the sample is shaped like the real catalogue");
    }

    #[test]
    #[ignore = "visual aid; run with --ignored to eyeball the screen"]
    fn dump() {
        let buffer = render(&FirstRun::default(), 120, 36);
        for y in 0..36 {
            let banded = (0..120).any(|x| buffer[(x, y)].bg == DARK.band);
            println!("{y:2}|{}|{}", row_text(&buffer, y), if banded { " <- selected" } else { "" });
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
