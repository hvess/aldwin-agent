//! The filled-row primitive every surface is built from:
//!
//! ```text
//! │← margin →│← pad →│ content … fill │← margin →│
//!  surround      bg                     surround
//! ```
//!
//! A new surface is a new constructor, not another wrap-and-pad loop.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::grid::{Ctx, MARGIN_X};
use super::wrap::wrap_line;

/// Geometry and fill of one full-width row; one `Row` describes a surface.
#[derive(Clone, Copy)]
pub(super) struct Row {
    /// Cells off each edge of the column, painted in `surround`.
    margin: usize,
    surround: Color,
    /// Cells of `bg` on each side between the margin and the content.
    pad: usize,
    bg: Color,
}

impl Row {
    /// A band inset `MARGIN_X` each side (the echoed prompt's
    /// `margin: 0 var(--margin-x)`), no padding.
    pub fn band(bg: Color, surround: Color) -> Self {
        Self {
            margin: MARGIN_X,
            surround,
            pad: 0,
            bg,
        }
    }

    /// A row filling the column edge to edge, such as a fenced code block on
    /// `tint`. Never outlined: nothing inside a window is stroked.
    pub fn field(bg: Color) -> Self {
        Self {
            margin: 0,
            surround: Color::Reset,
            pad: 0,
            bg,
        }
    }

    /// Overrides the margin, e.g. frame E's option row (`margin: 0 1ch`).
    pub fn inset(self, cells: usize) -> Self {
        Self {
            margin: cells,
            ..self
        }
    }

    /// Overrides the padding, e.g. `CommandBlock.jsx`'s `padding-left: 18px`
    /// that puts its `$` on cell 5.
    pub fn pad(self, cells: usize) -> Self {
        Self { pad: cells, ..self }
    }

    /// Gives `bg` to every span without one; a span with its own (inline
    /// code's `tint`) keeps it. Done here, not by callers: an unfilled span
    /// shows the window's ground through the band.
    fn on_field(self, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        spans
            .into_iter()
            .map(|span| {
                if span.style.bg.is_some() {
                    span
                } else {
                    Span::styled(span.content, span.style.bg(self.bg))
                }
            })
            .collect()
    }

    /// Cells left for content once margins and padding are taken.
    fn avail(self, width: u16) -> usize {
        (width as usize)
            .saturating_sub(2 * self.margin)
            .saturating_sub(2 * self.pad)
    }

    /// Wraps `spans` and returns one built row per wrapped line, each
    /// `ctx.width` cells wide and filled (see [`Row::on_field`]).
    pub fn build(self, spans: Vec<Span<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
        let avail = self.avail(ctx.width);
        wrap_line(Line::from(spans), avail)
            .into_iter()
            .map(|line| self.assemble(line.spans, ctx))
            .collect()
    }

    /// Like [`Row::build`], but the first span is a glyph `indent` cells
    /// wide and continuation rows are indented to align under the text.
    /// Empty `spans` yields one blank row.
    pub fn build_indented(
        self,
        mut spans: Vec<Span<'static>>,
        indent: usize,
        ctx: Ctx,
    ) -> Vec<Line<'static>> {
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
                content.push(if i == 0 {
                    glyph.clone()
                } else {
                    Span::styled(" ".repeat(indent), field)
                });
                content.extend(line.spans);
                self.assemble(content, ctx)
            })
            .collect()
    }

    /// One blank filled row.
    pub fn blank(self, ctx: Ctx) -> Line<'static> {
        self.assemble(Vec::new(), ctx)
    }

    /// Surrounds already-fitted `content` with margins, padding and fill to
    /// the column's full width.
    fn assemble(self, content: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
        let width = ctx.width as usize;
        let content = self.on_field(content);
        let content_width: usize = content.iter().map(|s| s.content.width()).sum();
        let leading = self.margin + self.pad;
        let fill = width
            .saturating_sub(leading)
            .saturating_sub(content_width)
            .saturating_sub(self.margin);

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

/// A full-width row of `bg` with no glyph: the separator, since nothing is
/// stroked. Painted as spaces because the render-snapshot tests require
/// every cell to carry a palette colour.
pub(super) fn band_row(bg: Color, ctx: Ctx) -> Line<'static> {
    Line::from(Span::styled(
        " ".repeat(ctx.width as usize),
        Style::default().bg(bg),
    ))
}
