//! The one filled-row primitive every surface in the UI is built from.
//!
//! A card row, a diff row inside its bordered box, a selectable option row
//! flush to the frame's left edge and a blank spacer are all the same
//! shape:
//!
//! ```text
//! │← margin →│B│← pad →│ content … fill │B│← margin →│
//!  surround        bg                        surround
//! ```
//!
//! Four numbers (margin, border, pad, bg) describe every one of them.
//! Before this they were eight separate functions — `filled_line`,
//! `flush_line`, `boxed_line`, `card_line`, `card_padding_line`,
//! `card_rule`, `card_footer_line`, `diff_box_border` — each re-deriving
//! the same wrap → measure → pad-to-width loop and each computing its own
//! available content width from the same formula. `diff_box_border` and
//! `boxed_line` in particular had to agree on `width - 2*inset - 2` in two
//! places for a box's top edge to line up with its own sides; here that
//! number is `Row::avail`, computed once.
//!
//! Adding a surface is therefore a new constructor, not a ninth copy of the
//! loop.

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
    /// Whether the edge columns just inside the margin are a drawn `│`
    /// border (`InlineDiff.jsx`: `border: 1px solid var(--tui-line)`)
    /// rather than more fill.
    bordered: bool,
    /// Cells of `bg` between the edge and the content — the reference's
    /// `padding: 0 27px` on every card row.
    pad:      usize,
    /// The surface this row fills, edge to edge.
    bg:       Color,
    /// The surface the *box* is made of, which [`Row::with_fill`] leaves
    /// alone. A diff row swaps `bg` for an `add_bg`/`del_bg` tint, but the
    /// `│` sides it keeps belong to the box, not to the row: in the
    /// reference the tint is a background on the row *inside* a
    /// `border: 1px solid var(--tui-line)`, so the border never takes it.
    /// Drawing the sides on `bg` instead tinted them green or red, which
    /// was reported directly as "the borders are not aligned with the
    /// background at all."
    field:    Color,
}

impl Row {
    /// A card/panel content row: the grid's `MARGIN_X` padding on both
    /// sides, filled edge to edge in `bg`, no border.
    pub fn card(bg: Color) -> Self {
        Self { margin: 0, surround: Color::Reset, bordered: false, pad: MARGIN_X, bg, field: bg }
    }

    /// A row with no padding at all, content starting in cell 0 — the one
    /// row type the reference deliberately runs flush to the frame's own
    /// left edge, so a selectable option's `▌` mark lands in cell 0.
    pub fn flush(bg: Color) -> Self {
        Self { margin: 0, surround: Color::Reset, bordered: false, pad: 0, bg, field: bg }
    }

    /// A row inside a real drawn one-cell box — `│` sides, square corners
    /// from [`Row::border`].
    pub fn boxed(bg: Color) -> Self {
        Self { margin: 0, surround: Color::Reset, bordered: true, pad: 0, bg, field: bg }
    }

    /// Holds the row off each edge by `cells`, painting that strip in
    /// `surround`. The reference nests a diff box two ways: inside the
    /// permission card it rides the card's own `padding: 0 27px` margin
    /// (`.inset(MARGIN_X, pal.bar)`, so the strip reads as card rather than
    /// as diff), while inside a turn's body column it sits flush with no
    /// second margin of its own — the body column's `CONTENT_INDENT` is
    /// already the only offset it needs.
    pub fn inset(self, cells: usize, surround: Color) -> Self {
        Self { margin: cells, surround, ..self }
    }

    /// Overrides the cells of fill held between the row's edge and its
    /// content. `CommandBlock.jsx` is the one surface that isn't on the
    /// grid's own `MARGIN_X`: it is a field inset by `MARGIN_X` whose
    /// *contents* start two cells further in (`padding-left: 18px`), so the
    /// `$` lands on cell 5.
    pub fn pad(self, cells: usize) -> Self {
        Self { pad: cells, ..self }
    }

    /// The same geometry over a different fill — a diff row keeps its box's
    /// `│` sides and margins but swaps the field they sit on for a semantic
    /// `add_bg`/`del_bg` tint. `field` is deliberately not swapped; see it.
    pub fn with_fill(self, bg: Color) -> Self {
        Self { bg, ..self }
    }

    /// The surface this row fills, for a caller styling content to sit on
    /// it.
    pub fn fill(self) -> Color {
        self.bg
    }

    /// Paints this row's own fill onto every span that didn't already ask
    /// for a background of its own.
    ///
    /// Spans that *do* carry one (a diff row's `add_bg` tint) are left
    /// exactly as they are, which is what lets a caller mix a semantic tint
    /// into an otherwise-plain row. Before this, carrying the fill was the
    /// caller's job on every span — an obligation three separate call sites
    /// (the panel's title, its footer key hints, the command block's `$`)
    /// had quietly failed, so those spans rendered on whatever the frame's
    /// canvas happened to hold underneath: `ground`. In the dark theme that
    /// is a near-black hole punched through a panel; in the light theme it
    /// is a white box around the word `permission`. Reported as "the title
    /// 'permission' has a dark background."
    fn on_field(self, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        spans
            .into_iter()
            .map(|span| if span.style.bg.is_some() { span } else { Span::styled(span.content, span.style.bg(self.bg)) })
            .collect()
    }

    /// Cells left for content once margins, borders and padding are taken.
    fn avail(self, width: u16) -> usize {
        (width as usize).saturating_sub(2 * self.margin).saturating_sub(2 * usize::from(self.bordered)).saturating_sub(2 * self.pad)
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

    /// One row of plain `text` in `fg`, on this row's own fill.
    pub fn text(self, text: &str, fg: Color, ctx: Ctx) -> Vec<Line<'static>> {
        self.build(vec![Span::styled(text.to_string(), Style::default().fg(fg).bg(self.bg))], ctx)
    }

    /// A blank filled row — a leading/trailing spacer inside a card so its
    /// content doesn't sit flush against the card's own top/bottom edge.
    /// Always exactly one row (empty content never wraps), so this stays
    /// single-`Line` for its many `push` call sites.
    pub fn blank(self, ctx: Ctx) -> Line<'static> {
        self.assemble(Vec::new(), ctx)
    }

    /// The box's top (`┌─…─┐`) or bottom (`└─…─┘`) edge. Its dash run is
    /// `avail` wide — the same number [`Row::build`] wraps content to, so
    /// an edge can never come out a different width from the sides.
    pub fn border(self, top: bool, ctx: Ctx) -> Line<'static> {
        let (left, right) = if top { ('┌', '┐') } else { ('└', '┘') };
        let bar = format!("{left}{}{right}", "─".repeat(self.avail(ctx.width)));
        let margin = || Span::styled(" ".repeat(self.margin), Style::default().bg(self.surround));
        Line::from(vec![margin(), Span::styled(bar, Style::default().fg(ctx.pal.line).bg(self.field)), margin()])
    }

    /// A single row split left/right — the same shape `KeyHints.jsx` and
    /// `Modal.jsx`'s footer use (key hints on the left, a fact flush
    /// right). Both halves are caller-styled: the panel's title band puts
    /// its badge in `hunk_header` where the footer puts its note in `dim`,
    /// and a single hardcoded colour here got one of the two wrong.
    ///
    /// On a column too narrow for both halves the right-hand group is
    /// dropped outright rather than wrapped: this row is laid out by hand,
    /// so an overlong one would spill onto a row *outside* the panel — the
    /// frame's own ground showing through under a fragment of text. The
    /// keys on the left are what a developer actually needs; the note on
    /// the right is the half that can go.
    pub fn split(self, left: Vec<Span<'static>>, right: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
        let avail = self.avail(ctx.width);
        let left = self.on_field(left);
        let right = self.on_field(right);
        let left_width: usize = left.iter().map(|s| s.content.width()).sum();
        let right_width: usize = right.iter().map(|s| s.content.width()).sum();
        // `<`, not `<=`: at least one cell of gap has to survive between
        // the two halves, or they'd read as one run-on string.
        let right = if left_width + right_width < avail { right } else { Vec::new() };
        let right_width: usize = right.iter().map(|s| s.content.width()).sum();
        let gap = avail.saturating_sub(left_width.min(avail)).saturating_sub(right_width);
        let field = Style::default().bg(self.bg);
        let mut spans = vec![Span::styled(" ".repeat(self.pad), field)];
        spans.extend(left);
        spans.push(Span::styled(" ".repeat(gap.max(1)), field));
        spans.extend(right);
        spans.push(Span::styled(" ".repeat(self.pad), field));
        Line::from(spans)
    }

    /// Wraps already-fitted `content` in this row's margins, borders,
    /// padding and fill, out to the column's full width.
    fn assemble(self, content: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
        let width = ctx.width as usize;
        let content = self.on_field(content);
        let content_width: usize = content.iter().map(|s| s.content.width()).sum();
        let border = usize::from(self.bordered);
        let leading = self.margin + border + self.pad;
        let fill = width.saturating_sub(leading).saturating_sub(content_width).saturating_sub(border + self.margin);

        let mut spans = Vec::with_capacity(content.len() + 6);
        let surround = Style::default().bg(self.surround);
        let edge = Style::default().fg(ctx.pal.line).bg(self.field);
        let field = Style::default().bg(self.bg);
        if self.margin > 0 {
            spans.push(Span::styled(" ".repeat(self.margin), surround));
        }
        if self.bordered {
            spans.push(Span::styled("│", edge));
        }
        if self.pad > 0 {
            spans.push(Span::styled(" ".repeat(self.pad), field));
        }
        spans.extend(content);
        spans.push(Span::styled(" ".repeat(fill), field));
        if self.bordered {
            spans.push(Span::styled("│", edge));
        }
        if self.margin > 0 {
            spans.push(Span::styled(" ".repeat(self.margin), surround));
        }
        Line::from(spans)
    }
}

/// A full-width flat rule inside a card/panel — the design system's own
/// revision log settled every freestanding rule as flat and single-colour,
/// not a fading gradient (see `Palette::rule`). Not a `Row`: it fills the
/// column edge to edge with no margin, padding or content of its own.
pub(super) fn rule_row(fg: Color, bg: Color, ctx: Ctx) -> Line<'static> {
    Line::from(Span::styled("─".repeat(ctx.width as usize), Style::default().fg(fg).bg(bg)))
}
