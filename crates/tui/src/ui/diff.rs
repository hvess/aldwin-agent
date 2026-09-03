//! Unified-diff parsing and rendering — shared by the Edit approval card
//! and by a ```diff fence in assistant prose, which is why it is its own
//! module rather than a section of either.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::grid::Ctx;
use super::row::Row;

/// One line of a unified diff (`mjolnir_tools::diff::unified`'s output),
/// tagged by its leading marker (` `/`+`/`-`). The `--- path`/`+++ path`
/// header pair is pulled out separately by `parse_body` since it's shown
/// once as a label, not per line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Context,
    Added,
    Removed,
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
/// all. `mjolnir_tools::diff::unified` emits no `@@ -a,b +c,d @@` hunk
/// header (it diffs a single already-replaced hunk, not a whole file), so
/// there's no absolute file offset to anchor on — these are relative to the
/// start of the shown diff, numbered from 1 on each side, the same
/// convention a hunk header's own numbers use relative to itself. Context
/// lines advance both counters (they exist on both sides); removed lines
/// only the old counter; added lines only the new one — mirroring the
/// two-column gutter GitHub and most diff UIs show.
pub(super) fn number_lines(body: Vec<(Kind, String)>) -> Vec<DiffLine> {
    let mut old_no = 1usize;
    let mut new_no = 1usize;
    body.into_iter()
        .map(|(kind, text)| {
            let (o, n) = match kind {
                Kind::Context => (Some(old_no), Some(new_no)),
                Kind::Removed => (Some(old_no), None),
                Kind::Added => (None, Some(new_no)),
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
        if line.kind != Kind::Context {
            let start = i.saturating_sub(CONTEXT_RADIUS);
            let end = (i + CONTEXT_RADIUS).min(n.saturating_sub(1));
            for k in &mut keep[start..=end] {
                *k = true;
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
const GUTTER: usize = 5;

/// The `+ ` / `- ` sign that follows the gutter, before the code itself.
const SIGN: usize = 2;

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
    Span::styled(format!("{n:>width$} ", width = GUTTER - 1), Style::default().fg(ctx.pal.dim).bg(bg))
}

/// Renders one diff line, prefixed with its old/new line-number gutter.
/// Added/removed lines get their semantic `add_bg`/`del_bg` tint (which
/// wins over `diff_box`, the surface the quoted diff sits on) so a change
/// reads as a coloured row at a glance, not just a leading +/- character —
/// `InlineDiff.jsx`'s own row treatment; context lines get the plain
/// `diff_box` fill and `context` text colour, since only the changed lines'
/// brighter tint should compete for attention.
///
/// Sign and code text are two different tokens in the source
/// (`--tui-add`/`--tui-del` for the sign, `--tui-add-code`/`--tui-del-code`
/// for the code itself) — kept as separate spans rather than one combined
/// colour so both read exactly as `InlineDiff.jsx` does.
fn render_line(line: &DiffLine, row: Row, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let (marker, sign_fg, code_fg, bg) = match line.kind {
        Kind::Added => ("+ ", pal.add, pal.add_code, pal.add_bg),
        Kind::Removed => ("- ", pal.del, pal.del_code, pal.del_bg),
        Kind::Context => ("  ", pal.diff_box, pal.context, pal.diff_box),
    };
    let spans = vec![
        gutter(line.old_no, line.new_no, bg, ctx),
        Span::styled(marker.to_string(), Style::default().fg(sign_fg).bg(bg)),
        Span::styled(line.text.clone(), Style::default().fg(code_fg).bg(bg)),
    ];
    // The tinted rows keep the box's own `│` sides but swap the field they
    // sit on, which is what `Row`'s "spans carry their own bg" contract is
    // for.
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

/// A quoted diff as `InlineDiff.jsx`'s own real bordered box: top edge, one
/// row per shown line, bottom edge.
///
/// Everything `budget` leaves out is replaced in place by an in-box marker,
/// so the box always closes. Before this, a diff too tall for its panel was
/// cut by `decision::clamp_panel`'s blind row budget, which chopped the box
/// mid-way and left a `┌───┐` with no `└───┘` on screen.
pub(super) fn boxed(body: &[DiffLine], budget: Budget, row: Row, ctx: Ctx) -> Vec<Line<'static>> {
    let shown = if budget.collapse_context { collapse_context(body) } else { body.iter().map(Shown::Line).collect() };
    // Indented to [`CODE_COLUMN`] so a note lines up with the code it
    // stands in for, past an empty gutter — the reference's own `81 more
    // lines` row.
    let marker = |text: String| row.build(vec![Span::styled(format!("{}{text}", " ".repeat(CODE_COLUMN)), Style::default().fg(ctx.pal.dim).bg(row.fill()))], ctx);

    let mut rows: Vec<Line<'static>> = Vec::new();
    for item in &shown {
        match item {
            Shown::Line(line) => rows.extend(render_line(line, row, ctx)),
            Shown::Elided(count) => rows.extend(marker(format!("⋯ {count} unchanged line{} ⋯", if *count == 1 { "" } else { "s" }))),
        }
    }

    // Two of the budget go to the box's own edges, and at least one more to
    // the marker whenever anything is dropped. The count is of rendered
    // rows, which is one per diff line except where a line was wide enough
    // to wrap.
    if let Some(max) = budget.max_rows {
        let body_budget = max.saturating_sub(2);
        if rows.len() > body_budget {
            let keep = body_budget.saturating_sub(1);
            let hidden = rows.len() - keep;
            rows.truncate(keep);
            rows.extend(marker(format!("⋯ {hidden} more line{} not shown ⋯", if hidden == 1 { "" } else { "s" })));
        }
    }

    let mut out = vec![row.border(true, ctx)];
    out.append(&mut rows);
    out.push(row.border(false, ctx));
    out
}
