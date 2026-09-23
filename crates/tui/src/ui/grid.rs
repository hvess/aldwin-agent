//! The design system's cell grid, and the render context every builder in
//! `ui` is handed instead of a loose `(pal, width)` pair.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::palette::Palette;

/// The grid, in cells (`tokens/layout.css`, where `1ch` is a cell and
/// `--row: 24px` is a row):
///
/// * `MARGIN_X` — `--margin-x: 3ch`. The field, the footer and a plan step
///   start here; so does every band's left edge.
/// * `MARK_COL` — `--mark-col: 2ch`. The prompt `›`, a step's `✓ ● ○`, a
///   selection's `▎`: the glyph sits in the first cell and the second is
///   the gap.
/// * `BODY_X` — `--body-x: 5ch`. Prose, a disclosure, the launch card. The
///   design states it as "margin + mark column" and declares it anyway, so
///   it is emitted as stated and checked here against the sum.
///
/// There is no label column any more. The speaker was an 8-cell word in
/// the previous system; in this one the echoed prompt sits on its own
/// ground with a `›`, and the agent's prose needs no name.
pub(super) use crate::tokens::{
    BODY_X, COMMAND_COL, DETAIL_COL, FACT_COL, GROUP_GAP, GUTTER_LN, MARGIN_X, MARK_COL, NUMBER_COL, PANE_GAP, SIGN_COL, TREE_W,
};

const _: () = assert!(BODY_X == MARGIN_X + MARK_COL, "layout.css states --body-x as margin + mark column");

/// The two facts every line builder in `ui` needs and neither of which it
/// can derive on its own: which theme's colours to draw in, and how many
/// cells the column it is filling is wide.
///
/// Narrowing is explicit (`ctx.narrow(..)` / `ctx.body()`): a builder
/// filling a column inside another one says so at the call site, which is
/// where the width-divergence bugs recorded in `aldwin-tui.md` would have
/// been visible.
#[derive(Clone, Copy)]
pub(super) struct Ctx<'a> {
    pub pal:   &'a Palette,
    /// Cells available to whatever is being built — the *column's* width,
    /// not necessarily the frame's.
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

    /// The prose column: from `BODY_X` to one `MARGIN_X` short of the right
    /// edge, so wrapped text and filled blocks end where the field does
    /// rather than running into the frame's edge.
    pub fn body(self) -> Self {
        self.narrow(self.width.saturating_sub(BODY_X as u16).saturating_sub(MARGIN_X as u16))
    }
}

/// Prefixes every row with `BODY_X` blank cells — where prose lands.
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

/// A row whose first cells are a glyph in the mark column at the margin,
/// then `content` at `BODY_X`: a plan step, the echoed prompt, a status row.
pub(super) fn marked(glyph: Span<'static>, content: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = Vec::with_capacity(content.len() + 3);
    spans.push(Span::raw(" ".repeat(MARGIN_X)));
    let glyph_width = glyph.content.width();
    spans.push(glyph);
    spans.push(Span::raw(" ".repeat(MARK_COL.saturating_sub(glyph_width))));
    spans.extend(content);
    Line::from(spans)
}

/// Truncates to `max` *display cells* with a trailing `…` — the design's
/// own elision glyph, and the one way a hand-composed row (which has no
/// wrapper of its own) is allowed to handle content wider than its column.
/// `max == 0` yields nothing at all rather than a bare `…`.
pub(super) fn elide(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
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

/// Truncates a run of *styled* spans to `max` display cells, appending the
/// system's own `…` in the style of the span it had to cut.
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
        let keep = elide(&span.content, max - used);
        if !keep.is_empty() {
            out.push(Span::styled(keep, span.style));
        } else {
            out.push(Span::styled("…", span.style));
        }
        return out;
    }
    out
}

/// Right-flushes `right` against `left` within `width` columns — the
/// design's fact rows: verb and target on the left, `412 lines` flush right.
///
/// When the two sides do not both fit, the **right** group is elided to
/// what is left after the left group and one space, so the line is never
/// longer than `width`. The left group is never cut here, because it is
/// what identifies the row.
pub(super) fn justified(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
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

/// Pads `text` to exactly `cells` display cells, eliding if it is wider —
/// a fixed column such as the launch card's fact label or a detail row's verb.
pub(super) fn column(text: &str, cells: usize) -> String {
    let text = elide(text, cells);
    let pad = cells.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(pad))
}
