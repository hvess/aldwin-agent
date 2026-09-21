//! Unified-diff parsing and rendering — shared by the Edit approval card
//! and by a ```diff fence in assistant prose, which is why it is its own
//! module rather than a section of either.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::grid::Ctx;
use super::row::Row;

/// One line of a unified diff (`aldwin_tools::diff::unified`'s output),
/// tagged by its leading marker (` `/`+`/`-`). The `--- path`/`+++ path`
/// header pair is pulled out separately by `parse_body` since it's shown
/// once as a label, not per line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Context,
    Added,
    Removed,
    /// A `@@ -a,b +c,d @@` hunk header. Never produced by
    /// `aldwin_tools::diff::unified`, which emits no header at all — this
    /// is a diff the *model* wrote inside a ```diff fence, where the header
    /// is ordinary text in the reply and carries the only absolute line
    /// numbers there are. It used to fall through to `Context`, which drew
    /// it as a line of the file, gave it a gutter number, and numbered
    /// everything under it from 1 as though the header were not there.
    Hunk,
}

/// One diff body line plus the line number(s) it carries on each side of
/// the change — see [`number_lines`].
pub(super) struct DiffLine {
    pub kind:   Kind,
    pub text:   String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
}

/// How many lines of unmodified context to keep immediately before/after a
/// change — per explicit developer feedback that in an approval card,
/// unchanged lines are only relevant this close to what actually changed; a
/// longer run of context collapses to a single elision marker instead of
/// listing every line, and pure context isn't coloured at all (see
/// [`render_line`]) — only the changed lines are, so they're the only thing
/// competing for attention.
const CONTEXT_RADIUS: usize = 2;

/// Splits a unified diff into its path (from the `--- path` header line;
/// `+++ path` names the same path, so it's dropped) and its body lines,
/// each tagged with the kind its leading marker encodes.
pub(super) fn parse_body(diff: &str) -> (Option<String>, Vec<(Kind, String)>) {
    let mut path = None;
    let mut body = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            path.get_or_insert_with(|| rest.to_string());
        } else if line.starts_with("+++ ") {
            // Same path as the "---" line — nothing new to show.
        } else if line.starts_with("@@") {
            body.push((Kind::Hunk, line.to_string()));
        } else if let Some(rest) = line.strip_prefix('+') {
            body.push((Kind::Added, rest.to_string()));
        } else if let Some(rest) = line.strip_prefix('-') {
            body.push((Kind::Removed, rest.to_string()));
        } else {
            body.push((Kind::Context, line.strip_prefix(' ').unwrap_or(line).to_string()));
        }
    }
    (path, body)
}

/// Assigns old-file/new-file line numbers to a parsed diff body — per
/// explicit developer feedback that diffs rendered with no line numbers at
/// all. Context lines advance both counters (they exist on both sides);
/// removed lines only the old counter; added lines only the new one —
/// mirroring the two-column gutter GitHub and most diff UIs show.
///
/// Where the counters *start* depends on what the diff carries.
/// `aldwin_tools::diff::unified` emits no `@@ -a,b +c,d @@` header (it
/// diffs a single already-replaced hunk, not a whole file), so there is no
/// absolute file offset to anchor on and the numbers are relative to the
/// start of the shown diff, from 1 on each side — the same convention a
/// hunk header's own numbers use relative to itself.
///
/// A ```diff fence in assistant prose is a different surface sharing this
/// one path, and it often does carry a header. Then the header's own
/// offsets anchor the counters, and each subsequent header re-anchors them,
/// so the gutter reads the file's line numbers rather than a count of the
/// rows on screen. Without this the `fenced_diff` scene numbered a seven-
/// line hunk `1, 2, 3, 2, 3, 4, 5` under a header claiming line 12.
pub(super) fn number_lines(body: Vec<(Kind, String)>) -> Vec<DiffLine> {
    let mut old_no = 1usize;
    let mut new_no = 1usize;
    body.into_iter()
        .map(|(kind, text)| {
            if kind == Kind::Hunk {
                if let Some((old, new)) = hunk_offsets(&text) {
                    old_no = old;
                    new_no = new;
                }
                return DiffLine { kind, text, old_no: None, new_no: None };
            }
            let (o, n) = match kind {
                Kind::Context => (Some(old_no), Some(new_no)),
                Kind::Removed => (Some(old_no), None),
                Kind::Added => (None, Some(new_no)),
                Kind::Hunk => unreachable!("returned above"),
            };
            if o.is_some() {
                old_no += 1;
            }
            if n.is_some() {
                new_no += 1;
            }
            DiffLine { kind, text, old_no: o, new_no: n }
        })
        .collect()
}

/// The two starting line numbers in a `@@ -12,7 +12,9 @@` header — the old
/// file's and the new file's. `None` for anything that does not parse,
/// which leaves the counters where they were rather than guessing: a header
/// the model mistyped is still a header, and a wrong number in the gutter
/// is worse than a continued one.
fn hunk_offsets(text: &str) -> Option<(usize, usize)> {
    let inner = text.trim_start_matches('@').split("@@").next()?;
    let mut sides = inner.split_whitespace();
    let start = |side: &str, sign: char| -> Option<usize> { side.strip_prefix(sign)?.split(',').next()?.parse().ok() };
    let old = start(sides.next()?, '-')?;
    let new = start(sides.next()?, '+')?;
    Some((old, new))
}

/// `+84` / `+11 -2` — a parsed diff's own stat, in the diff colours, for a
/// tool line's right-flush summary slot. The reference shows exactly this
/// beside a `write`/`edit` row (`Turn.jsx`), and omits the side that is
/// zero rather than printing `-0`.
pub(super) fn stat_spans(body: &[DiffLine], ctx: Ctx) -> Vec<Span<'static>> {
    let added = body.iter().filter(|l| l.kind == Kind::Added).count();
    let removed = body.iter().filter(|l| l.kind == Kind::Removed).count();
    let mut spans = Vec::new();
    if added > 0 {
        spans.push(Span::styled(format!("+{added}"), Style::default().fg(ctx.pal.add)));
    }
    if removed > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(format!("-{removed}"), Style::default().fg(ctx.pal.del)));
    }
    spans
}

/// Drops a unified diff's `a/` or `b/` side marker from a path, so a header
/// line's `--- a/src/page.rs` reads as the file the developer knows —
/// `src/page.rs` — wherever the path is shown as a target rather than as
/// part of the diff text.
pub(super) fn strip_prefix(path: &str) -> String {
    path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")).unwrap_or(path).to_string()
}

/// What to draw for each body line: the line itself, or a marker standing
/// in for a run of lines that were left out.
enum Shown<'a> {
    Line(&'a DiffLine),
    Elided(usize),
}

/// Collapses every run of unchanged context further than [`CONTEXT_RADIUS`]
/// from a change into a single [`Shown::Elided`] marker.
fn collapse_context(body: &[DiffLine]) -> Vec<Shown<'_>> {
    let n = body.len();
    let mut keep = vec![false; n];
    for (i, line) in body.iter().enumerate() {
        match line.kind {
            // A header is always shown — it is what says where in the file
            // the rows under it are — but it anchors no context of its own:
            // it is not a change, and keeping two unchanged lines either
            // side of it collapses nothing.
            Kind::Hunk => keep[i] = true,
            Kind::Context => {}
            Kind::Added | Kind::Removed => {
                let start = i.saturating_sub(CONTEXT_RADIUS);
                let end = (i + CONTEXT_RADIUS).min(n.saturating_sub(1));
                for k in &mut keep[start..=end] {
                    *k = true;
                }
            }
        }
    }

    let mut shown = Vec::new();
    let mut i = 0;
    while i < n {
        if keep[i] {
            shown.push(Shown::Line(&body[i]));
            i += 1;
        } else {
            let start = i;
            while i < n && !keep[i] {
                i += 1;
            }
            shown.push(Shown::Elided(i - start));
        }
    }
    shown
}

/// `--gutter-line-no-inline: 45px` — 5 cells, the width `tokens/cells.css`
/// names for a diff gutter inside the transcript, laid out in the reference
/// as a 4-cell right-aligned number plus one cell of separation
/// (`flex: 0 0 45px; text-align: right; padding-right: 9px`).
use crate::tokens::GUTTER_LINE_NO_INLINE as GUTTER;

/// The `+ ` / `- ` sign that follows the gutter, before the code itself.
use crate::tokens::DIFF_SIGN_COL as SIGN;

/// Cells from the box's inner edge to the first character of code — what an
/// in-box note (`⋯ 3 unchanged lines ⋯`) indents to, so it starts in the
/// code column rather than in the gutter. The reference does exactly this
/// with its own `81 more lines` row: an empty gutter, then the text.
pub(super) const CODE_COLUMN: usize = GUTTER + SIGN;

/// One right-aligned line number in the reference's own 5-cell gutter.
///
/// One number, not two. An earlier pass showed old and new side by side
/// with a `│` between them, which took 11 cells before the sign — more than
/// twice the grid's allowance — and pushed every line of code past the
/// column the design puts it in: "the diff doesn't match the designs well,
/// it appears to be very misaligned." A unified diff row exists on exactly
/// one side of the change, so the number that side carries is the only one
/// there is to show; a context row exists on both and takes the new-file
/// number, the side the developer is about to be looking at.
fn gutter(old_no: Option<usize>, new_no: Option<usize>, bg: ratatui::style::Color, ctx: Ctx) -> Span<'static> {
    let n = new_no.or(old_no).map(|n| n.to_string()).unwrap_or_default();
    // `label`, not `dim`. The handoff names the role and then says why it
    // is that one: "a 5-cell right-aligned line number in `--t-label` …
    // the neutral label step holds 3.5:1 for the gutter". At `dim` it
    // measured the same value as `--tui-context`, which is what a context
    // row's code is painted in — so on an unchanged row the number and the
    // code it numbers were the identical colour and the gutter stopped
    // reading as a gutter.
    Span::styled(format!("{n:>width$} ", width = GUTTER - 1), Style::default().fg(ctx.pal.label).bg(bg))
}

/// Renders one diff line, prefixed with its old/new line-number gutter.
/// Added/removed lines get their semantic `add_row`/`del_row` fill (which
/// wins over `diff_box`, the surface the quoted diff sits on) so a change
/// reads as a coloured row at a glance, not just a leading +/- character;
/// context lines get the plain `diff_box` fill and `context` text colour,
/// since only the changed lines should compete for attention.
///
/// Those fills are the design system's *resolved solids*, not its
/// `--tui-add-bg`/`--tui-del-bg` rgba tints — a terminal cell has no alpha,
/// and the system ships the opaque pair for exactly this case (see
/// `palette.rs`).
///
/// Sign and code take the **same** token here — `--t-add-code` /
/// `--t-del-code` — rather than the sign/code split the source's review
/// pane uses. This comment used to say the opposite ("kept as separate
/// spans … so both read as the source does"), which read the review pane's
/// rule onto the inline diff. The handoff draws the distinction explicitly
/// and gives a measured reason: "Both the sign and the code take
/// `--t-add-code` here rather than the sign/code split the review pane
/// uses, because a tinted row over the recessed field is the darkest
/// backdrop in the light theme and the mid-lightness sign green measures
/// only **2.7:1** on it; the code colour holds 4.8:1 light and 5.9:1 dark."
/// The review pane's hunk keeps the split because it sits on the much
/// lighter transcript ground — and it is not built, so nothing here needs
/// that branch yet.
fn render_line(line: &DiffLine, row: Row, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    // A hunk header is not a line of the file: no sign, no number in the
    // gutter, and `--tui-hunk-header` rather than the tone the gutter and a
    // context row share — `5b` draws its own `@@ -0,0 +1,84 @@` that way,
    // and it is the one role in the palette named for this.
    // A header is not a row of the file, so it takes neither the gutter nor
    // the sign column: it starts at the field's own left edge, which is
    // where `5b` draws its `@@ -0,0 +1,84 @@ impl RateLimit`. The trailing
    // "N more lines" note is the one in-box row that hangs on the *code*
    // column instead, because it stands in for code.
    if line.kind == Kind::Hunk {
        let spans = vec![Span::styled(line.text.clone(), Style::default().fg(pal.hunk_header).bg(pal.diff_box))];
        return row.with_fill(pal.diff_box).build(spans, ctx);
    }
    let (marker, sign_fg, code_fg, bg) = match line.kind {
        Kind::Added => ("+ ", pal.add_code, pal.add_code, pal.add_row),
        Kind::Removed => ("- ", pal.del_code, pal.del_code, pal.del_row),
        Kind::Context | Kind::Hunk => ("  ", pal.diff_box, pal.context, pal.diff_box),
    };
    let spans = vec![
        gutter(line.old_no, line.new_no, bg, ctx),
        Span::styled(marker.to_string(), Style::default().fg(sign_fg).bg(bg)),
        Span::styled(line.text.clone(), Style::default().fg(code_fg).bg(bg)),
    ];
    // A changed row swaps the field it sits on, which is what `Row`'s
    // "spans carry their own bg" contract is for.
    row.with_fill(bg).build(spans, ctx)
}

/// How much of a diff a box is allowed to leave out. `Budget::default()`
/// shows every line — what a ```diff fence in assistant prose wants, where
/// the log scrolls and nothing has to fit a fixed band.
#[derive(Clone, Copy, Default)]
pub(super) struct Budget {
    /// Collapse runs of unchanged context further than [`CONTEXT_RADIUS`]
    /// from a change.
    pub collapse_context: bool,
    /// Cap the whole box, its two border rows included.
    pub max_rows:         Option<usize>,
}

/// A quoted diff as the design system's recessed field: one row per shown
/// line, on the field's own ground, with no outline. The step between that
/// ground and the surface the diff is quoted on is the whole boundary — see
/// the note on borders in `super`.
///
/// Everything `budget` leaves out is replaced in place by a marker row
/// rather than by truncation, so the field still says what it dropped. That
/// used to also be what kept the box *closed* when `decision::clamp_panel`
/// cut it — a chopped box left a `┌───┐` with no `└───┘` on screen. There
/// is no longer an edge to lose, but the marker is still the honest thing
/// to show.
pub(super) fn boxed(body: &[DiffLine], budget: Budget, row: Row, ctx: Ctx) -> Vec<Line<'static>> {
    let shown = if budget.collapse_context { collapse_context(body) } else { body.iter().map(Shown::Line).collect() };
    // Indented to [`CODE_COLUMN`] so a note lines up with the code it
    // stands in for, past an empty gutter — the reference's own `81 more
    // lines` row.
    // Plain prose, with no mark of its own. It read `... 3 unchanged lines
    // ...` in U+22EF until a capture caught the obvious: that glyph is not
    // in the closed table, and not among the typographic marks the baseline
    // exempts — those are the ones the design's own screens use (`· … ⏎ ↑↓
    // ← →`), and this was neither. The reference writes exactly this row as
    // `81 more lines` (`4a`) and `73 more added lines below` (`5b`): the
    // empty gutter beside it is what says it is not a line of the file, so
    // a decoration on both ends was saying it a second time in a glyph the
    // system does not have.
    let marker = |text: String| row.build(vec![Span::styled(format!("{}{text}", " ".repeat(CODE_COLUMN)), Style::default().fg(ctx.pal.dim).bg(row.fill()))], ctx);

    let mut rows: Vec<Line<'static>> = Vec::new();
    for item in &shown {
        match item {
            Shown::Line(line) => rows.extend(render_line(line, row, ctx)),
            Shown::Elided(count) => rows.extend(marker(format!("{count} unchanged line{}", if *count == 1 { "" } else { "s" }))),
        }
    }

    // The whole budget is the field's own rows now that it has no edges to
    // pay for — one is still reserved for the marker whenever anything is
    // dropped. The count is of rendered rows, which is one per diff line
    // except where a line was wide enough to wrap.
    if let Some(max) = budget.max_rows {
        if rows.len() > max {
            let keep = max.saturating_sub(1);
            let hidden = rows.len() - keep;
            rows.truncate(keep);
            rows.extend(marker(format!("{hidden} more line{} not shown", if hidden == 1 { "" } else { "s" })));
        }
    }

    rows
}
