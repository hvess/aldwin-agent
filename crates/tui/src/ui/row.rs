//! The one filled-row primitive every surface in the UI is built from.
//!
//! A card row, a diff row inside its recessed field, a selectable option
//! row flush to the frame's left edge and a blank spacer are all the same
//! shape:
//!
//! ```text
//! │← margin →│← pad →│ content … fill │← margin →│
//!  surround      bg                     surround
//! ```
//!
//! Three numbers (margin, pad, bg) describe every one of them, and the
//! content width they leave is `Row::avail`, computed once. Adding a surface
//! is a new constructor, not another copy of the wrap → measure →
//! pad-to-width loop.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::grid::{Ctx, MARGIN_X};
use super::wrap::wrap_line;

/// Geometry and fill of one full-width row. `Copy`, so a caller holds one
/// `Row` describing a surface and stamps out every row of it.
#[derive(Clone, Copy)]
pub(super) struct Row {
    /// Cells held off each edge of the column, painted in `surround` —
    /// how far a box sits in from the surface it is quoted inside.
    margin:   usize,
    surround: Color,
    /// Cells of `bg` between the edge and the content — the reference's
    /// `padding: 0 27px` on every card row.
    pad:      usize,
    /// The surface this row fills, edge to edge.
    bg:       Color,
}

impl Row {
    /// A band inset by `MARGIN_X` on each side — the echoed prompt's
    /// `margin: 0 3ch` — with `surround` painted in the margins and no
    /// padding of its own, so content starts at the band's edge.
    pub fn band(bg: Color, surround: Color) -> Self {
        Self { margin: MARGIN_X, surround, pad: 0, bg }
    }

    /// A row inside a field — a fenced code block on `tint`. It has no
    /// outline: nothing inside a window is stroked, so the only thing
    /// marking the field's extent is the step between its own ground and
    /// the surface it is quoted on.
    pub fn field(bg: Color) -> Self {
        Self { margin: 0, surround: Color::Reset, pad: 0, bg }
    }

    /// Overrides how far the row sits in from each edge of its column —
    /// frame E's option row, `margin: 0 1ch` inside the question panel.
    pub fn inset(self, cells: usize) -> Self {
        Self { margin: cells, ..self }
    }

    /// Overrides the cells of fill held between the row's edge and its
    /// content. `CommandBlock.jsx` is the one surface that isn't on the
    /// grid's own `MARGIN_X`: it is a field inset by `MARGIN_X` whose
    /// *contents* start two cells further in (`padding-left: 18px`), so the
    /// `$` lands on cell 5.
    pub fn pad(self, cells: usize) -> Self {
        Self { pad: cells, ..self }
    }

    /// Paints this row's own fill onto every span that didn't already ask
    /// for a background of its own.
    ///
    /// Spans that *do* carry one (a diff row's `add_row` fill) are left
    /// exactly as they are, which is what lets a caller mix a semantic tint
    /// into an otherwise-plain row. Done here because as the caller's job
    /// it was quietly missed at three sites, and a span with no `bg` shows
    /// the frame's `ground` through the panel — reported as "the title
    /// 'permission' has a dark background."
    fn on_field(self, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        spans
            .into_iter()
            .map(|span| if span.style.bg.is_some() { span } else { Span::styled(span.content, span.style.bg(self.bg)) })
            .collect()
    }

    /// Cells left for content once margins and padding are taken.
    fn avail(self, width: u16) -> usize {
        (width as usize).saturating_sub(2 * self.margin).saturating_sub(2 * self.pad)
    }

    /// Wraps `spans` to fit and returns one fully-built row per wrapped
    /// line — each already `ctx.width` cells wide, inset, padded and
    /// filled, so nothing downstream needs to wrap it again.
    ///
    /// A span that already carries a `bg` keeps it, so a caller mixing a
    /// semantic tint — a diff row's `add_bg` over the box's own `diff_box`
    /// — still reads correctly; one that doesn't gets this row's fill (see
    /// [`Row::on_field`]).
    pub fn build(self, spans: Vec<Span<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
        let avail = self.avail(ctx.width);
        wrap_line(Line::from(spans), avail).into_iter().map(|line| self.assemble(line.spans, ctx)).collect()
    }

    /// Like [`Row::build`], for a row whose first span is a glyph column:
    /// the rest wraps to what is left after `indent` cells, and every
    /// continuation row is indented by `indent` so the text keeps one left
    /// edge under itself rather than stepping back under the glyph.
    pub fn build_indented(self, mut spans: Vec<Span<'static>>, indent: usize, ctx: Ctx) -> Vec<Line<'static>> {
        if spans.is_empty() {
            return vec![self.blank(ctx)];
        }
        let glyph = spans.remove(0);
        let avail = self.avail(ctx.width).saturating_sub(indent).max(1);
        let field = Style::default().bg(self.bg);
        wrap_line(Line::from(spans), avail)
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                let mut content = Vec::with_capacity(line.spans.len() + 1);
                content.push(if i == 0 { glyph.clone() } else { Span::styled(" ".repeat(indent), field) });
                content.extend(line.spans);
                self.assemble(content, ctx)
            })
            .collect()
    }

    /// A blank filled row — a leading/trailing spacer inside a card so its
    /// content doesn't sit flush against the card's own top/bottom edge.
    /// Always exactly one row (empty content never wraps), so this stays
    /// single-`Line` for its many `push` call sites.
    pub fn blank(self, ctx: Ctx) -> Line<'static> {
        self.assemble(Vec::new(), ctx)
    }

    /// Wraps already-fitted `content` in this row's margins, padding and
    /// fill, out to the column's full width.
    fn assemble(self, content: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
        let width = ctx.width as usize;
        let content = self.on_field(content);
        let content_width: usize = content.iter().map(|s| s.content.width()).sum();
        let leading = self.margin + self.pad;
        let fill = width.saturating_sub(leading).saturating_sub(content_width).saturating_sub(self.margin);

        let mut spans = Vec::with_capacity(content.len() + 4);
        let surround = Style::default().bg(self.surround);
        let field = Style::default().bg(self.bg);
        if self.margin > 0 {
            spans.push(Span::styled(" ".repeat(self.margin), surround));
        }
        if self.pad > 0 {
            spans.push(Span::styled(" ".repeat(self.pad), field));
        }
        spans.extend(content);
        spans.push(Span::styled(" ".repeat(fill), field));
        if self.margin > 0 {
            spans.push(Span::styled(" ".repeat(self.margin), surround));
        }
        Line::from(spans)
    }
}

/// A full-width band of `bg`, one row tall and carrying no glyph — the
/// design system's replacement for every freestanding rule. Turn 13 settled
/// separators as "a full row of a different ground, never a rule", and this
/// is that row: the tonal step between the band and what sits either side
/// of it *is* the boundary, so there is nothing to draw into the cells.
///
/// Painted as spaces rather than left empty because a `Line` shorter than
/// the column would leave the cells past its end unstyled, and the
/// render-snapshot suite asserts every painted cell carries a palette
/// colour rather than the terminal's own default.
///
/// Not a `Row`: it fills the column edge to edge with no margin, padding or
/// content of its own.
pub(super) fn band_row(bg: Color, ctx: Ctx) -> Line<'static> {
    Line::from(Span::styled(" ".repeat(ctx.width as usize), Style::default().bg(bg)))
}
