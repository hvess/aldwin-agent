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
//! | [`markdown`] | LLM-authored markdown: fences, block prefixes, inline delimiters |
//! | [`diff`] | unified-diff parsing and its bordered-box rendering |
//! | [`transcript`] | the conversation log and the welcome hero |
//! | [`decision`] | the pending-approval / permission panel and its resolved cards |
//! | [`chrome`] | the top bar, the status line and the composer |
//!
//! The one discipline that spans all of them: a row is wrapped exactly
//! once, by [`wrap`] or by [`row::Row`], *before* it is inset or filled —
//! never afterwards by a `Paragraph`'s own `Wrap`, which knows nothing
//! about the insets already applied and would strand continuation rows flush
//! against the frame edge. See mjolnir-tui.md's Progress notes for the two
//! bugs that discipline exists to prevent from recurring.

mod chrome;
mod decision;
mod diff;
mod grid;
mod markdown;
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

pub(crate) use transcript::row_count as log_row_count;

/// `--bar-top-h: 60px` — 3 cells. The reference's `1px` border below it is
/// not a fourth row; see the note on borders further down this file.
pub(super) const TOP_BAR_ROWS: u16 = 3;

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

    // Three bands: a 3-row identity bar, the conversation log, and the
    // bottom bar. No border rows between them — see the note on borders
    // further down this file for why the surface change is the edge.
    //
    // The top bar is a structural element from the design system's
    // reference screens — every one of the five (session/permission/review/
    // commands/first-run) opens with a persistent 3-row identity bar plus a
    // a `border-bottom` inside that band (`tokens/cells.css`'s
    // `--bar-top-h`, `TopBar.jsx`'s `borderBottom`). An earlier pass had
    // folded identity
    // into a single status line right above the input; the source design
    // puts identity back at the top and leaves that line for live turn
    // activity only.
    //
    // The bottom bar is `BottomBar.jsx` exactly as the reference lays it
    // out: five rows — blank, composer, blank, status, blank. The status line sits *below* the composer, not above it; an
    // earlier pass had the two swapped (reported directly: "the status line
    // is above the text field input, but ... it is below in the designs").
    //
    // While a decision is pending the panel takes those rows instead:
    // "input is disabled while a permission is pending: there is nothing to
    // type into, so the prompt row is not drawn at all."
    let input_height = chrome::input_height(&app.input);
    // Three bands, each carrying its own edge inside itself (see the note
    // on borders further down this file). 3 cells for the top bar
    // (`--bar-top-h`), and `BottomBar.jsx`'s blank/composer/blank/status/
    // blank for the bottom one (`--bar-bottom-h`), the composer's own
    // height apart.
    //
    // The panel is the one exception and takes a row for its edge: its
    // first row is the title band, which carries text, so there is no spare
    // cell for a half block there. The row above gets it instead, on
    // `ground`, which is also what the handoff describes — "a one-cell
    // accent-700 rule along its top edge".
    let bottom_height = if pending { panel_height + 1 } else { input_height + 4 };
    let [top_bar_area, log_area, bottom_area] =
        Layout::vertical([Constraint::Length(TOP_BAR_ROWS), Constraint::Min(1), Constraint::Length(bottom_height)]).areas(area);

    chrome::draw_top_bar(frame, top_bar_area, app);

    // No drawn border and no title — the reference shows no box anywhere
    // around the conversation, just filled cards floating directly on the
    // frame background. A right-aligned "live"/"scrolled" title badge used
    // to live here but per explicit developer feedback it was meaningless
    // noise in the corner of the screen — removed outright, not replaced,
    // so this is a plain background fill with nothing reserving a title row.
    let log_block = Block::new().style(Style::default().bg(pal.ground));
    // `Block::inner` is a pure function of the block's border/title config
    // and the outer rect — computed exactly once here, and this same `Rect`
    // is what both `App::render_width`/`render_height` (cached for scroll
    // math between draws) and the log's own content pass use. There must
    // never be a second, independently-derived "inner width" anywhere else
    // in this call graph — see mjolnir-tui.md's scrolling-fix and
    // wrapped-row-scroll-math Progress notes for the two real bugs that came
    // from exactly this kind of divergence before.
    let log_inner = log_block.inner(log_area);
    app.render_width = log_inner.width;
    app.render_height = log_inner.height;
    app.scroll.set_viewport_height(log_inner.height as usize, app.total_lines());

    transcript::draw_log(frame, log_area, log_inner, log_block, app);

    if pending {
        // The transcript recedes while a decision is open — the reference
        // puts the whole conversation column at `opacity:.35` in both of
        // its panel scenes, so the panel reads as the one live surface
        // rather than as another card competing with the history above it.
        // Applied as a post-pass over the already-drawn cells rather than
        // by threading a second faded palette through every render arm: the
        // effect is uniform over the region by definition, so compositing
        // it once here can't drift from the panel's own colours the way a
        // parallel palette would.
        fade_area(frame, log_area, palette::PANEL_TRANSCRIPT_OPACITY);
        // No edge row above the panel. The design system's permission
        // screen is explicit that "the tonal step off the transcript is the
        // whole boundary — there is no rule along its top edge", so the
        // panel takes the full band and announces itself with its own
        // ground plus the risen title row inside it.
        decision::draw_panel(frame, bottom_area, panel_lines, pal);
    } else {
        // blank / composer / blank / status / blank — the reference's own
        // five rows, on its own ground one step off the transcript. No
        // rule above it: the step *is* the boundary, so all five rows are
        // painted `bar_bottom` and the first is simply blank.
        frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), bottom_area);
        let [_pad_top, composer_area, _pad_mid, status_area, _pad_bottom] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(input_height), Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
                .areas(bottom_area);
        chrome::draw_input(frame, composer_area, app);
        chrome::draw_status_line(frame, status_area, app);
    }
}

// Nothing inside a frame is stroked. Boundaries are tonal.
//
// This used to be a long note working out how to render a 1px CSS border
// in a cell grid — `─` glyphs, one-eighth blocks, `BorderType::QuadrantOutside`,
// and finally `SGR 4` underlines carrying `underline_color`. All of it is
// gone, because the design system stopped having borders at all: Turn 13
// removed every rule, pane divider and box outline and made a band's step
// on the seven-rung ground ladder the thing that separates it from its
// neighbour.
//
// That is a straightforwardly better fit for a terminal than any of the
// shapes above were. The handoff says so directly: "Nothing is stroked, so
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

/// Composites every already-drawn cell in `area` toward its own background
/// at `alpha`, the way CSS `opacity` would. Each cell fades toward *its
/// own* `bg`, not one shared ground, so a cell sitting on a card or a diff
/// band recedes against that surface rather than against the frame behind
/// it. Backgrounds themselves are left alone: they are the surfaces being
/// faded onto, and dissolving them too would erase the card edges the fade
/// is supposed to preserve.
fn fade_area(frame: &mut Frame, area: Rect, alpha: f32) {
    let buf = frame.buffer_mut();
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = palette::fade(cell.fg, cell.bg, alpha);
        }
    }
}
