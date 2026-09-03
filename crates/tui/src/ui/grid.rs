//! The design system's cell grid, and the render context every builder in
//! `ui` is handed instead of a loose `(pal, width)` pair.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::palette::Palette;

/// The design system's grid, in cells (`tokens/cells.css`, confirmed
/// against every measurement in the reference frames themselves):
///
/// * `MARGIN_X` — `--margin-x: 27px` = 3 cells. *Every* content row in a
///   frame carries this left/right margin: transcript turns, the top bar,
///   the status line, the decision panel's own text rows, its footer. The
///   one deliberate exception is a selectable option row, which the
///   reference renders flush to the frame's left edge so its `▌` mark sits
///   in cell 0 (see `decision::option_rows`).
/// * `LABEL_COL_WIDTH` — `--label-col: 108px` = 12 cells, the speaker /
///   meta-label column.
/// * `LABEL_GUTTER` — `--label-gutter: 18px` = 2 cells.
/// * `CONTENT_INDENT` — `--body-col: 153px` = 17 cells from the frame
///   edge, which is exactly `MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER`;
///   body text in a turn always starts here.
///
/// An earlier pass used 10/2 with no margin at all, so every transcript row
/// started 5 cells left of where the grid puts it — reported directly as
/// "the chat rows themselves appear misaligned and do not follow the
/// cell/grid system."
pub(super) const MARGIN_X: usize = 3;
const LABEL_COL_WIDTH: usize = 12;
const LABEL_GUTTER: usize = 2;
pub(super) const CONTENT_INDENT: usize = MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER;

/// The two facts every line builder in `ui` needs and neither of which it
/// can derive on its own: which theme's colours to draw in, and how many
/// cells the column it is filling is wide.
///
/// Carried as one `Copy` value rather than as a trailing `(pal, width)`
/// pair on every signature — the pair was threaded by hand through 25-odd
/// functions in inconsistent positions, so a builder that wanted one more
/// piece of render-wide context could not get it without editing all of
/// them. Narrowing is explicit (`ctx.narrow(..)` / `ctx.body()`): a builder
/// filling a column inside another one says so at the call site, which is
/// where the two historical width-divergence bugs recorded in
/// `mjolnir-tui.md` would have been visible.
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

    /// The turn body column: everything left of `CONTENT_INDENT` belongs to
    /// the margin and the label column, and `MARGIN_X` more is held back on
    /// the right — the reference's turn container is `padding: 0 27px`, a
    /// margin on *both* sides, so prose wraps and filled blocks (code
    /// fences, diff boxes) end one margin short of the frame's edge rather
    /// than running into it.
    pub fn body(self) -> Self {
        self.narrow(self.width.saturating_sub(CONTENT_INDENT as u16).saturating_sub(MARGIN_X as u16))
    }
}

/// Lays `lines` out under `Turn.jsx`'s label column: `label` (if any) sits
/// on the first row only, left-padded to `CONTENT_INDENT`; every other row
/// — the first row too, when `label` is `None` — gets a blank
/// `CONTENT_INDENT` prefix instead, so a tool-activity/retry/error/notice
/// entry (which continues the previous turn rather than starting a new one;
/// see `transcript::render_entry`) lines its content up under whichever
/// speaker's turn it belongs to without repeating that speaker's name.
pub(super) fn with_label_column(lines: Vec<Line<'static>>, label: Option<(&str, Color)>) -> Vec<Line<'static>> {
    let blank = " ".repeat(CONTENT_INDENT);
    let margin = " ".repeat(MARGIN_X);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let mut spans = Vec::with_capacity(line.spans.len() + 2);
            match (i, label) {
                // The label starts at the 3-cell margin — cell 3, not cell
                // 0 — and the body column still lands on cell 17 regardless
                // of how long the label itself is.
                (0, Some((text, color))) => {
                    let pad = (LABEL_COL_WIDTH + LABEL_GUTTER).saturating_sub(text.width());
                    spans.push(Span::raw(margin.clone()));
                    spans.push(Span::styled(text.to_string(), Style::default().fg(color)));
                    spans.push(Span::raw(" ".repeat(pad)));
                }
                _ => spans.push(Span::raw(blank.clone())),
            }
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Truncates to `max` *display cells* with a trailing `…` — the design
/// system's own elision glyph, and the one way a hand-composed row (which
/// has no wrapper of its own) is allowed to handle content wider than its
/// column. `max == 0` yields nothing at all rather than a bare `…`, which
/// on a column that narrow is a character spent saying nothing.
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
        let w = c.to_string().width();
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Right-flushes `right` against `left` within `width` columns —
/// `ToolLine.jsx`'s own shape (glyph/name/target on the left, a result
/// summary flush to the right edge). Falls back to a single-space gap
/// rather than clipping when the two sides don't leave room to space apart
/// properly.
pub(super) fn justified_line(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let gap = width.saturating_sub(left_w).saturating_sub(right_w).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}
