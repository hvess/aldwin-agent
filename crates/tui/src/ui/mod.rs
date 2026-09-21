//! The ratatui frontend: `App`'s state turned into a frame, once per
//! redraw.
//!
//! This module owns the frame's *band layout* and nothing else. Everything
//! it composes lives in a submodule with one job:
//!
//! | module | job |
//! |--------|-----|
//! | [`grid`] | the design system's cell grid, and the `Ctx` (`palette` + column width) every builder takes |
//! | [`wrap`] | word-wrapping one logical line, before anything is inset or filled |
//! | [`row`] | the single filled-row primitive every card, box and option row is built from |
//! | [`markdown`] | LLM-authored markdown: fences, block prefixes, tables, inline delimiters |
//! | [`diff`] | unified-diff parsing and its bordered-box rendering |
//! | [`transcript`] | the conversation log and the welcome hero |
//! | [`decision`] | the pending-approval / permission panel and its resolved cards |
//! | [`picker`] | the model picker's panel — what bare `/model` opens |
//! | [`chrome`] | the top bar, the status line and the composer |
//!
//! The one discipline that spans all of them: a row is wrapped exactly
//! once, by [`wrap`] or by [`row::Row`], *before* it is inset or filled —
//! never afterwards by a `Paragraph`'s own `Wrap`, which knows nothing
//! about the insets already applied and would strand continuation rows flush
//! against the frame edge. See aldwin-tui.md's Progress notes for the two
//! bugs that discipline exists to prevent from recurring.
//!
//! In the transcript that is now load-bearing rather than merely tidy: the
//! log's own `Paragraph` no longer wraps *at all*, so a builder that hands
//! it an over-wide row gets that row truncated rather than folded. See
//! [`transcript::Transcript`] for why the second wrapper went away and what
//! it bought.

mod chrome;
mod decision;
mod diff;
pub(crate) mod first_run;
mod grid;
mod markdown;
pub(crate) mod picker;
mod row;
mod transcript;
mod wrap;

#[cfg(test)]
mod tests;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::app::App;
use crate::palette;
use grid::Ctx;

pub(crate) use transcript::Transcript;

/// `--bar-top-h: 60px` — 3 cells, first run's bar included.
const TOP_BAR_ROWS: u16 = 3;

/// Blank rows held back at the top and bottom of the transcript band, so
/// conversation text never sits flush against a chrome bar.
///
/// `cells.css`: "spacing inside a frame is blank rows, never padding" — so
/// this is a row of the transcript's own ground, not an inset with a
/// different tone, and it is taken out of the band's *inner* rect while the
/// band itself still fills edge to edge. The reference frames space the
/// transcript off both bars this way; aldwin had the body band running
/// straight into them, reported directly as "the chat doesn't have any top
/// and bottom padding and it means the text touches the top and bottom
/// bars".
///
/// One row, not more: the bottom bar already opens with a blank row of its
/// own (`BottomBar.jsx`'s blank/composer/blank/status/blank), so the gap
/// under the last turn reads as two rows while the gap under the top bar
/// reads as one — which is what the reference does, the top bar being the
/// denser edge.
const LOG_PAD_ROWS: u16 = 1;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let pal = app.theme.palette();
    let area = frame.area();
    let ctx = Ctx::new(pal, area.width);
    // Opaque canvas, drawn first and under everything else — without this,
    // every gap between panels renders as the terminal's own background,
    // which is exactly the "transparent app" look the redesign replaced.
    // See `Palette::ground`'s doc comment for the tier this belongs to.
    frame.render_widget(Block::default().style(Style::default().bg(pal.ground)), area);

    // The panel's lines (and therefore its height) are computed once, up
    // front — the same discipline `log_inner`'s width/height follow below.
    // There must never be a second, independently-derived height for what
    // the panel actually renders, or the two can drift the way `log_inner`'s
    // own comment describes two real historical bugs happening.
    //
    // The height counts *wrapped* rows, not `panel_lines.len()`: a prompt's
    // title can easily be wider than the frame (an arbitrarily long shell
    // command, say), and the panel's own `Paragraph` wraps rather than
    // truncates, same as the log panel's render path.
    let panel_lines = decision::panel_lines(app, ctx, area.height);
    let panel_height = decision::row_count(&panel_lines, area.width) as u16;
    let pending = panel_height > 0;

    // The model picker takes the same band on the same terms — it is a
    // question waiting on an answer, and there is nothing to type into
    // while it is open. A pending decision outranks it: that one is the
    // agent blocked on the developer, and `App::handle_key` routes keys the
    // same way round, so what is on screen is always what the next key
    // resolves.
    let picker_lines = if pending { Vec::new() } else { picker::panel_lines(app, ctx) };
    let picker_height = decision::row_count(&picker_lines, area.width) as u16;
    let picking = picker_height > 0;

    // Three bands: a 3-row identity bar (`--bar-top-h`), the conversation
    // log, and the bottom bar. No border rows between them — the surface
    // change is the edge (see the note on borders further down this file).
    //
    // The bottom bar is `BottomBar.jsx` exactly as the reference lays it
    // out: five rows — blank, composer, blank, status, blank. The status
    // line sits *below* the composer, not above it.
    //
    // While a decision is pending the panel takes those rows instead:
    // "input is disabled while a permission is pending: there is nothing to
    // type into, so the prompt row is not drawn at all."
    //
    // Measured once for the whole frame, and only when the composer is
    // actually on screen: a pending decision or the picker takes the band
    // instead, and wrapping a draft nobody can see is pure cost on a large
    // one. See `chrome::Composer` for why there is exactly one of these.
    let composer = (!pending && !picking).then(|| chrome::Composer::new(&app.input, area.width));
    // The panel takes exactly its own rows — there is no edge above it to
    // reserve one for, and a spare row would show as a stray `bar` row
    // *below* the footer, since the panel renders from the top of its rect.
    let bottom_height = match &composer {
        // The composer's own rows plus `BottomBar.jsx`'s four fixed ones.
        Some(composer) => composer.height() + 4,
        None if pending => panel_height,
        None => picker_height,
    };
    let [top_bar_area, log_area, bottom_area] =
        Layout::vertical([Constraint::Length(TOP_BAR_ROWS), Constraint::Min(1), Constraint::Length(bottom_height)]).areas(area);

    chrome::draw_top_bar(frame, top_bar_area, app);

    // No drawn border and no title — the reference shows no box anywhere
    // around the conversation, so this is a plain background fill with
    // nothing reserving a title row.
    let log_block = Block::new().style(Style::default().bg(pal.ground));
    // `Block::inner` is a pure function of the block's border/title config
    // and the outer rect — computed exactly once here, and this same `Rect`
    // is what `App::render_width` (cached for scroll
    // math between draws) and the log's own content pass use. There must
    // never be a second, independently-derived "inner width" anywhere else
    // in this call graph — see aldwin-tui.md's scrolling-fix and
    // wrapped-row-scroll-math Progress notes for the two real bugs that came
    // from exactly this kind of divergence before.
    let log_inner = pad_rows(log_block.inner(log_area), LOG_PAD_ROWS);
    app.render_width = log_inner.width;
    // Measured, scrolled and rendered off one cache, synced once (see
    // `App::transcript_view`) — the count is the row list's own length, so
    // there is no second pass that could disagree with what is drawn, and
    // only the viewport is ever materialised.
    let mut visible = app.transcript_view(log_inner.height as usize);
    // Bottom-anchored, like `2a`'s body band (`justify-content: flex-end`):
    // a conversation shorter than the band hangs off its *bottom* edge, not
    // its top. Drawn from the top, the first few turns of a session sat
    // glued under the identity bar with the gap below them — and then, the
    // moment the transcript outgrew the band, jumped down to rest on the
    // composer instead. Nothing is added to the row count: these blank rows
    // are layout, not transcript, and `ScrollState` must keep measuring the
    // conversation rather than the space around it.
    //
    // The empty state is the exception, and it is the reference's: `1d`'s
    // body band is a plain column, so its two facts sit under the top bar
    // (see `transcript::hero_lines`). The two bands differ because they are
    // answering different questions — a transcript grows toward where the
    // next turn is typed; a resting screen has nothing growing in it.
    let anchored_to_bottom = !app.log.is_empty();
    if let Some(pad) = (log_inner.height as usize).checked_sub(visible.len()).filter(|&p| p > 0 && anchored_to_bottom) {
        let mut anchored = vec![ratatui::text::Line::default(); pad];
        anchored.append(&mut visible);
        visible = anchored;
    }

    transcript::draw_log(frame, log_area, log_inner, log_block, visible);

    if pending {
        // The transcript recedes while a decision is open, so the panel
        // reads as the one live surface rather than as another card
        // competing with the history above it. The reference (`3a`, `3b`)
        // does this by *recolouring* the conversation to the three
        // `--tui-scrim-*` roles — "a recolour, never alpha" — where it used
        // to set `opacity:.45` on the column.
        //
        // Applied as a post-pass over the already-drawn cells rather than
        // by threading a second palette through every render arm: the
        // effect is uniform over the region by definition, so doing it once
        // here can't drift from the transcript's own colours the way a
        // parallel set of builders would.
        scrim_area(frame, log_area, pal);
        // No edge row above the panel. The design system's permission
        // screen is explicit that "the tonal step off the transcript is the
        // whole boundary — there is no rule along its top edge", so the
        // panel takes the full band and announces itself with its own
        // ground plus the risen title row inside it.
        decision::draw_panel(frame, bottom_area, panel_lines, pal);
    } else if picking {
        // Same treatment as a pending decision, for the same reason: the
        // panel is the one live surface while it is open.
        scrim_area(frame, log_area, pal);
        decision::draw_panel(frame, bottom_area, picker_lines, pal);
    } else if let Some(composer) = &composer {
        // blank / composer / blank / status / blank — the reference's own
        // five rows, on its own ground one step off the transcript. No
        // rule above it: the step *is* the boundary, so all five rows are
        // painted `bar_bottom` and the first is simply blank.
        frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), bottom_area);
        let [notice_area, composer_area, _pad_mid, status_area, _pad_bottom] = Layout::vertical([
            Constraint::Length(1),
            // The same height the band above was sized from — one
            // `Composer`, so the two cannot disagree.
            Constraint::Length(composer.height()),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(bottom_area);
        // The band's first row is blank — unless the last turn failed, in
        // which case it carries the notice (`2d`). It takes the blank's row
        // rather than adding one, so the band is five rows either way and
        // nothing above it moves when a turn fails.
        chrome::draw_notice(frame, notice_area, app);
        chrome::draw_input(frame, composer_area, &mut *app, composer);
        chrome::draw_status_line(frame, status_area, app);
    }
}

// Nothing inside a frame is stroked. Boundaries are tonal.
//
// Turn 13 removed every rule, pane divider and box outline and made a
// band's step on the seven-rung ground ladder the thing that separates it
// from its neighbour. The handoff says so directly: "Nothing is stroked, so
// nothing needs a `Block::bordered()` — build each band as a rect with its
// own `Style::bg` and let the tonal step do the work", and of separator
// rows, "in a terminal that is a single `Style::bg` on a one-row rect, so
// nothing here needs approximating". A background colour is exact in a
// cell grid in a way a hairline never was, and it costs no glyph, no row
// and no `SGR 58` support.
//
// The one thing this now depends on is the ladder keeping its spacing —
// `palette.css`: "Changing a step's lightness removes a boundary." See
// `palette.rs`.

/// Holds `n` rows back at the top and bottom of `area`. Saturating on both
/// counts: a terminal short enough that the padding would consume the whole
/// band keeps at least one usable row and simply loses the gap, rather than
/// producing a zero-height rect nothing can render into.
fn pad_rows(area: Rect, n: u16) -> Rect {
    let taken = n.saturating_mul(2);
    if area.height <= taken {
        return area;
    }
    Rect { y: area.y + n, height: area.height - taken, ..area }
}

/// Recolours every already-drawn cell in `area` to the dimmed-transcript
/// roles — see [`palette::Palette::scrimmed`] for which ink goes where.
///
/// Foregrounds only. A cell's background is the surface it sits on, and the
/// reference leaves those alone: a turn break behind an open panel is still
/// a turn break. Blank cells are skipped because a foreground nothing draws
/// with is not a colour on screen, and recolouring it would only make the
/// buffer harder to read in a failing test.
fn scrim_area(frame: &mut Frame, area: Rect, pal: &palette::Palette) {
    let buf = frame.buffer_mut();
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if cell.symbol().trim().is_empty() {
                continue;
            }
            cell.fg = pal.scrimmed(cell.fg);
        }
    }
}
