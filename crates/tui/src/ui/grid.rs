//! The design system's cell grid, and the render context every builder in
//! `ui` is handed instead of a loose `(pal, width)` pair.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
/// * `LABEL_COL_WIDTH` — `--label-col: 72px` = 8 cells, the speaker /
///   meta-label column. It "carries every left-hand word in the system —
///   speaker, step, field name, file name".
/// * `LABEL_GUTTER` — `--label-gutter: 18px` = 2 cells.
/// * `CONTENT_INDENT` — cell 13, where body text in a turn always starts.
///
/// There is deliberately no `--body-col` token to check `CONTENT_INDENT`
/// against: the design system removed it, because cell 13 is a
/// *consequence* of the three values above rather than an independent
/// fact, and a fourth statement of it could only ever drift from them.
/// `cells.css` says so in place: "Do not add one; it would be a fourth
/// statement of a position the other three already fix." Deriving it here
/// the same way is what keeps this file honest against that.
pub(super) use crate::tokens::MARGIN_X;
use crate::tokens::{LABEL_COL_WIDTH, LABEL_GUTTER};

/// Body text's column — **derived here and nowhere else**, because
/// `cells.css` deliberately declares no `--body-col` and says why: it would
/// be "a fourth statement of a position the other three already fix".
pub(super) const CONTENT_INDENT: usize = MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER;

/// `--option-label-col` — 16 cells, an option row's name field. One width
/// for every list in the system: the provider list, the model list, the
/// access list and the command list are one control, so they share it.
pub(super) use crate::tokens::OPTION_LABEL_COL;

/// `--group-gap` — 6 cells, what parts two *unrelated* groups inside a bar:
/// the identity group from the model group in the top bar, `review changes`
/// from `3 files` in the review bar, one key hint from the next in a footer.
///
/// Deliberately *not* what sits between the brand and the working directory
/// — those are one group, and the three cells there are a pad to the body
/// column (see `chrome::brand_pad`). Facts *within* a group ride the tighter
/// ` · ` rhythm instead. Aldwin shipped six cells in the identity group for
/// three weeks on a misreading of the handoff prose; see `.claude/design/
/// HANDOFF.md`'s "the gap here is not `--group-gap`" note.
pub(super) use crate::tokens::GROUP_GAP;

/// `--step-mark-col` — 10 cells, the field a first-run step's glyph sits
/// in. Derived, not stated: the glyph is at the margin and the step's *name*
/// is at the body column, so this field is exactly what separates them.
pub(super) use crate::tokens::STEP_MARK_COL;

/// `--step-content-col` — cell 29, where a first-run step's content starts,
/// whatever that content is: a settled answer, an open step's purpose line,
/// a pending step's preview, or the option rows themselves. One column for
/// all four is what makes the three steps read as a single vertical spine
/// rather than as three stacked forms.
///
/// Derived from the three landmarks it is made of, like `CONTENT_INDENT`
/// above, so moving the option name field moves this with it.
pub(super) use crate::tokens::STEP_CONTENT_COL;

/// The two facts every line builder in `ui` needs and neither of which it
/// can derive on its own: which theme's colours to draw in, and how many
/// cells the column it is filling is wide.
///
/// Narrowing is explicit (`ctx.narrow(..)` / `ctx.body()`): a builder
/// filling a column inside another one says so at the call site, which is
/// where the two width-divergence bugs recorded in `aldwin-tui.md` would
/// have been visible.
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
    fn narrow(self, width: u16) -> Self {
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
                // The label starts at the margin, and the body column
                // still lands on `CONTENT_INDENT` however long the label is.
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
        // `unwrap_or(1)`: the one-cell reading `str::width` gives a control
        // character, so this agrees with the measurement above.
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
///
/// The span-level counterpart of [`elide`], for a group built from several
/// differently-toned facts rather than one string — the status line's
/// activity/model/turn/tools run. Same rule as `elide`: `max == 0` yields
/// nothing at all rather than a bare `…`, which on a column that narrow is a
/// cell spent saying nothing.
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
        // `< max`, not `<= max - 1`: the last cell is reserved for the `…`.
        if used + w < max {
            used += w;
            out.push(span);
            continue;
        }
        // The span that overruns is cut mid-way; `elide` would append a
        // second `…`, so the budget is taken here and the glyph added once.
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

/// Right-flushes `right` against `left` within `width` columns —
/// `ToolLine.jsx`'s own shape (glyph/name/target on the left, a result
/// summary flush to the right edge).
///
/// When the two sides do not both fit, the **right** group is elided to
/// what is left after the left group and one space, so the line is never
/// longer than `width` — ratatui would clip it at the frame edge with no
/// `…` (a summary once read `17 fil`). A shortened summary is still true, so
/// the right group elides rather than being dropped whole; the left group is
/// never cut here, because its glyph and tool name are what identify the row.
pub(super) fn justified_line(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
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
