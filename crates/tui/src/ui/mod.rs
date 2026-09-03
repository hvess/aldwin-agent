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
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::palette;
use grid::Ctx;

pub(crate) use transcript::row_count as log_row_count;

/// `--bar-top-h: 60px` — 3 cells. The `1px` border below it in the
/// reference is *inside* this band, not a fourth row: see [`edge_row`].
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

    // Five bands: a 3-row identity bar and its rule, the conversation log,
    // a rule, and the bottom bar.
    //
    // The top bar is a structural element from the design system's
    // reference screens — every one of the five (session/permission/review/
    // commands/first-run) opens with a persistent 3-row identity bar plus a
    // 1-row rule below it (`tokens/cells.css`'s `--bar-top-h`,
    // `TopBar.jsx`'s `borderBottom`). An earlier pass had folded identity
    // into a single status line right above the input; the source design
    // puts identity back at the top and leaves that line for live turn
    // activity only.
    //
    // The bottom bar is `BottomBar.jsx` exactly as the reference lays it
    // out: a `line` edge, then five rows — blank, composer, blank, status,
    // blank. The status line sits *below* the composer, not above it; an
    // earlier pass had the two swapped (reported directly: "the status line
    // is above the text field input, but ... it is below in the designs").
    //
    // While a decision is pending the panel takes those rows instead:
    // "input is disabled while a permission is pending: there is nothing to
    // type into, so the prompt row is not drawn at all."
    let input_height = chrome::input_height(&app.input);
    // Both bars carry their own border inside their own band (see
    // [`edge_row`]), so neither costs a row: 3 for the top bar
    // (`--bar-top-h`), and `BottomBar.jsx`'s blank/composer/blank/status/
    // blank for the bottom one (`--bar-bottom-h`), the composer's own
    // height apart.
    //
    // The panel is the one exception. Its first row is the title band,
    // which carries text, so there is no spare cell edge to draw its
    // `border-top: 1px solid var(--tui-modal-line)` against — it takes the
    // row above instead, drawn on `ground` so the accent hairline sits
    // flush against the top of the band with transcript above it.
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
        let [panel_edge_area, panel_area] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(bottom_area);
        frame.render_widget(edge_row(Edge::Bottom, pal.modal_line, pal.ground, area.width), panel_edge_area);
        decision::draw_panel(frame, panel_area, panel_lines, pal);
    } else {
        // blank / composer / blank / status / blank — `BottomBar.jsx`'s own
        // five rows, on its own raised ground. The first blank row also
        // carries the bar's `border-top`, against the top edge of its own
        // cell, so the border touches the transcript above it rather than
        // floating half a cell down inside the bar.
        frame.render_widget(Block::new().style(Style::default().bg(pal.bar_bottom)), bottom_area);
        let [pad_top, composer_area, _pad_mid, status_area, _pad_bottom] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(input_height), Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
                .areas(bottom_area);
        frame.render_widget(edge_row(Edge::Top, pal.line, pal.bar_bottom, area.width), pad_top);
        chrome::draw_input(frame, composer_area, app);
        chrome::draw_status_line(frame, status_area, app);
    }
}

/// Which edge of its own cell a border row is drawn against.
///
/// `─` is the wrong glyph for this and was the bug: it draws through the
/// *middle* of its cell, so a border row rendered with it leaves half a
/// cell of its own background on the far side. Measured on the top bar,
/// that put the line 10px above the bar's bottom edge — "the border is not
/// aligned cleanly with the bottom of the component". A CSS border is the
/// last pixel of its band, touching the neighbour with nothing in between.
///
/// The one-eighth blocks are the glyphs that actually do that: `▁` fills
/// the bottom ~2.5px of its cell and `▔` the top, which at a 20px cell is
/// about as close to the reference's 1px hairline as a terminal gets. Both
/// are Block Elements (U+2580–U+259F), the same range as the `█` and `▌`
/// the design system's own glyph table already mandates, so a terminal that
/// can draw those can draw these.
#[derive(Clone, Copy)]
enum Edge {
    /// A `border-bottom`: the line sits on the last row of its own band.
    Bottom,
    /// A `border-top`: the line sits on the first row of its own band.
    Top,
}

/// One full-width structural border, drawn against `edge` of a single row
/// that otherwise belongs to `bg` — so the border costs no row of its own.
/// `--bar-top-h: 60px` and `--bar-bottom-h: 101px` are 3 and 5 cells; the
/// extra `1px` in each is the border, which is why it has to live inside
/// the band rather than beside it.
fn edge_row(edge: Edge, fg: Color, bg: Color, width: u16) -> Paragraph<'static> {
    let glyph = match edge {
        Edge::Bottom => "▁",
        Edge::Top => "▔",
    };
    Paragraph::new(Line::from(Span::styled(glyph.repeat(width as usize), Style::default().fg(fg).bg(bg))))
}

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
