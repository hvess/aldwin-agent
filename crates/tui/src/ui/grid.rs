//! The design system's cell grid, and `Ctx`, the render context every `ui`
//! builder takes.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::palette::Palette;

/// The grid in cells, from `tokens/layout.css` (`1ch` is a cell, `--row` a row):
///
/// * `MARGIN_X` (`--margin-x`): where the field, the footer, a plan step and
///   every band start.
/// * `MARK_COL` (`--mark-col`): a mark glyph (`›`, a step's mark, `▎`) in the
///   first cell, the gap in the second.
/// * `BODY_X` (`--body-x`): where prose lands. Declared by the design and
///   checked below against `MARGIN_X + MARK_COL`.
///
/// There is no label column: the echoed prompt has its own ground and `›`.
pub(super) use crate::tokens::{
    BODY_X, COMMAND_COL, DETAIL_COL, FACT_COL, GROUP_GAP, GUTTER_LN, MARGIN_X, MARK_COL,
    NUMBER_COL, PANE_GAP, SIGN_COL, TREE_W,
};

const _: () = assert!(
    BODY_X == MARGIN_X + MARK_COL,
    "layout.css states --body-x as margin + mark column"
);

/// The theme's palette and the width of the column being filled.
///
/// Narrow explicitly (`narrow`, `body`) at the call site; an implicit width
/// caused the width-divergence bugs recorded in `aldwin-tui.md`.
#[derive(Clone, Copy)]
pub(super) struct Ctx<'a> {
    pub pal: &'a Palette,
    /// The column's width in cells, not necessarily the frame's.
    pub width: u16,
}

impl<'a> Ctx<'a> {
    pub fn new(pal: &'a Palette, width: u16) -> Self {
        Self { pal, width }
    }

    /// Same palette, a different column width.
    pub fn narrow(self, width: u16) -> Self {
        Self { width, ..self }
    }

    /// The prose column: every frame's `padding: 0 var(--body-x)`, inset
    /// `BODY_X` on both sides.
    pub fn body(self) -> Self {
        self.narrow(self.width.saturating_sub(2 * BODY_X as u16))
    }
}

/// Prefixes every row with `BODY_X` blank cells.
pub(super) fn at_body(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let blank = " ".repeat(BODY_X);
    lines
        .into_iter()
        .map(|line| {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            spans.push(Span::raw(blank.clone()));
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// `glyph` in the mark column, then `content` at `BODY_X`.
pub(super) fn marked(glyph: Span<'static>, content: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = Vec::with_capacity(content.len() + 3);
    spans.push(Span::raw(" ".repeat(MARGIN_X)));
    let glyph_width = glyph.content.width();
    spans.push(glyph);
    spans.push(Span::raw(" ".repeat(MARK_COL.saturating_sub(glyph_width))));
    spans.extend(content);
    Line::from(spans)
}

/// Truncates to `max` display cells with a trailing `…`; `max == 0` yields
/// an empty string. The only way an unwrapped row may handle overflow.
pub(super) fn elide(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    cut(text, max)
}

/// `text`'s first `max - 1` cells and a `…`, even when it would fit;
/// `max == 0` yields an empty string.
fn cut(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(1);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Truncates styled spans to `max` display cells; the `…` takes the style of
/// the span it cuts.
pub(super) fn truncate_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.width()).sum();
    if total <= max {
        return spans;
    }
    if max == 0 {
        return Vec::new();
    }
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    let mut used = 0;
    for span in spans {
        let w = span.content.width();
        if used + w < max {
            used += w;
            out.push(span);
            continue;
        }
        // Later spans are dropped, so the `…` is due even when this one fits.
        out.push(Span::styled(cut(&span.content, max - used), span.style));
        return out;
    }
    out
}

/// `left`, then `right` flushed to `width` with at least one space between.
///
/// Only `right` is elided to fit; `left` identifies the row and is never
/// cut, so a `left` wider than `width` overflows.
pub(super) fn justified(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let room = width.saturating_sub(left_w).saturating_sub(1);
    let right = truncate_spans(right, room);
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let gap = width.saturating_sub(left_w).saturating_sub(right_w).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}

/// Pads or elides `text` to exactly `cells` display cells.
pub(super) fn column(text: &str, cells: usize) -> String {
    let text = elide(text, cells);
    let pad = cells.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(pad))
}
