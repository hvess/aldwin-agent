//! The LLM-authored-markdown reader: fenced-block splitting, per-line block
//! prefixes, tables, and the inline delimiter pass.
//!
//! Hand-rolled rather than pulling in a CommonMark crate — a real
//! block-level parser normalizes blank lines and reflows paragraphs, which
//! would fight the line-for-line streaming render that happens on every
//! delta. Per-line block-prefix detection (heading, list, blockquote, rule)
//! plus a recursive-descent inline pass covers what LLMs actually emit.
//!
//! A table is the one construct here that a line cannot render on its own —
//! a column is only as wide as the widest cell *anywhere* in the block, so
//! the rows have to be measured together. [`render_prose`] is therefore the
//! entry point rather than [`render_line`]: it groups a table's rows and
//! hands every other line to the per-line path unchanged.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::grid::{truncate_spans, Ctx};
use super::row::band_row;
use super::wrap::wrap_line;

/// One piece of assistant text — either prose or a fenced code block.
pub(super) enum Segment {
    Prose(String),
    Code { lang: String, body: String },
}

/// Splits on ` ``` ` fences (optionally followed by a language tag on the
/// opening fence). An unterminated fence — the closing ` ``` ` hasn't
/// streamed in yet — still renders as code up to the end of the buffer
/// rather than falling back to prose, since re-rendering happens on every
/// delta and the fence will close on a later redraw.
pub(super) fn split_code_fences(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut prose = String::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        match line.trim_start().strip_prefix("```") {
            Some(lang) => {
                if !prose.is_empty() {
                    segments.push(Segment::Prose(std::mem::take(&mut prose)));
                }
                let mut body = String::new();
                for line in lines.by_ref() {
                    if line.trim() == "```" {
                        break;
                    }
                    body.push_str(line);
                    body.push('\n');
                }
                segments.push(Segment::Code { lang: lang.trim().to_string(), body });
            }
            None => {
                prose.push_str(line);
                prose.push('\n');
            }
        }
    }
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
    segments
}

/// Renders one prose segment — every line already fitted to `ctx.width`, so
/// nothing downstream wraps (see [`super::transcript::Transcript`] on why
/// that equivalence is load-bearing).
///
/// Ordinary lines go one at a time through [`render_line`]. A table is the
/// exception: it is consumed as a block, because its column widths are a
/// property of every row at once.
pub(super) fn render_prose(text: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let src: Vec<&str> = text.lines().collect();
    let mut out: Vec<Line<'static>> = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if let Some((table, consumed)) = parse_table(&src[i..]) {
            out.extend(render_table(&table, ctx));
            i += consumed;
            continue;
        }
        out.extend(wrap_line(render_line(src[i], ctx), ctx.width as usize));
        i += 1;
    }
    out
}

/// Which edge a column's cells are flush to — the delimiter row's `:`
/// markers (`:--` left, `--:` right, `:-:` centre).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

/// One GFM pipe table, still as source text: the inline pass runs later, on
/// the cells, so a `**bold**` cell is measured at its rendered width rather
/// than its source width.
struct Table {
    header: Vec<String>,
    aligns: Vec<Align>,
    rows:   Vec<Vec<String>>,
}

/// One table cell, already through the inline pass — so it is measured at
/// the width it will actually occupy rather than at its source width, which
/// a `**bold**` or `` `code` `` cell overstates by four cells or two.
type Cell = Vec<Span<'static>>;

/// Cells of clearance between a column's rule and its content, on each
/// side. One, so a column occupies `width + 2` cells between its two `│`.
/// Two would double every interior boundary's cost — on a 120-column frame
/// a five-column table would spend 15 cells on air — and a drawn rule needs
/// far less breathing room than a rule-less layout did, since the rule
/// itself is what parts the columns now.
const CELL_PAD: usize = 1;

/// Reads a table off the front of `lines`, returning it with the number of
/// lines it consumed.
///
/// A header row alone is not a table — the delimiter row underneath it is
/// what commits, exactly as GFM has it. That is also what makes this safe
/// under streaming: a half-arrived table renders as prose (its header line
/// still contains literal pipes) until the delimiter lands, and snaps into
/// columns on the next delta.
fn parse_table(lines: &[&str]) -> Option<(Table, usize)> {
    let header = split_row(lines.first()?)?;
    let aligns = parse_delimiter(lines.get(1)?, header.len())?;

    let mut rows = Vec::new();
    let mut consumed = 2;
    // The table runs until the first line that is not a row — a blank line,
    // or ordinary prose with no pipe in it.
    while let Some(cells) = lines.get(consumed).and_then(|l| split_row(l)) {
        rows.push(cells);
        consumed += 1;
    }
    Some((Table { header, aligns, rows }, consumed))
}

/// Splits one `| a | b |` row into trimmed cells, or `None` if the line
/// carries no pipe at all. Leading and trailing pipes are optional (LLMs
/// emit both forms); `\|` is a literal pipe and does not split.
fn split_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);

    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('|') => cur.push('|'),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    Some(cells)
}

/// The `|---|:--:|---:|` row, which both commits the table and fixes each
/// column's alignment. Every cell must be dashes with optional end colons,
/// and there must be exactly one per header cell — a mismatch means this
/// was never a table, so it falls back to prose rather than guessing.
fn parse_delimiter(line: &str, columns: usize) -> Option<Vec<Align>> {
    let cells = split_row(line)?;
    if cells.len() != columns || columns == 0 {
        return None;
    }
    cells
        .iter()
        .map(|cell| {
            let left = cell.starts_with(':');
            let right = cell.ends_with(':');
            let dashes = cell.trim_start_matches(':').trim_end_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (left, right) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Lays the table out on `ctx.width`, as a drawn grid.
///
/// **This is the design system's one stroked component, and the exception is
/// deliberate** — see `.claude/adr/0002-markdown-tables-are-drawn.md`. Turn
/// 13's rule ("nothing inside a frame is stroked; every boundary is a step
/// on the ground ladder") and the closed glyph table (`tokens::MARKS`, none of it box-drawing)
/// still hold everywhere else in this crate, and a boundary that separates
/// one *region* from another — a turn break, a markdown `---`, a panel from
/// its bar — is still a band. What a table needs is different in kind: a
/// two-dimensional grid of boundaries, one per column, repeated down every
/// row. The ground ladder has no way to express that (a ladder is one
/// dimension), and the first build of this proved it — column position
/// alone held the shape only until a cell was empty or a neighbouring
/// column was narrow, at which point the rows read as ragged prose.
///
/// The rules are `--tui-quiet`, the tier below `dim`: present enough to
/// carry the structure, quiet enough that the cells stay the thing being
/// read.
///
/// # The one case where the box does not close
///
/// A column can be shrunk to one cell but no further, so a table needs at
/// least `3n + 1` cells for `n` columns. Below that — 15-odd columns on a
/// narrow terminal — the assembled rows are clipped to the column with the
/// system's `…`, and the right-hand edge of the box goes with them.
///
/// That is deliberate over the two alternatives. Closing the box anyway
/// would draw a `┐` claiming an edge that is not where the table ends, and
/// dropping the columns that do not fit would silently discard the
/// developer's data. A clipped edge with a visible `…` says "there is more
/// here than fits", which is the true statement. Pinned by
/// `a_table_with_more_columns_than_cells_clips_rather_than_lying`.
fn render_table(table: &Table, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let columns = table.header.len();

    let header: Vec<Cell> = table.header.iter().map(|cell| parse_inline(cell, Style::default().fg(pal.label), ctx)).collect();
    let body: Vec<Vec<Cell>> = table
        .rows
        .iter()
        .map(|row| (0..columns).map(|i| parse_inline(row.get(i).map(String::as_str).unwrap_or(""), Style::default().fg(pal.body), ctx)).collect())
        .collect();

    let widths = column_widths(&header, &body, ctx.width as usize);
    let mut lines = Vec::with_capacity(body.len() + 4);
    lines.push(rule_line(['┌', '┬', '┐'], &widths, ctx));
    lines.push(row_line(&header, &widths, &table.aligns, ctx));
    lines.push(rule_line(['├', '┼', '┤'], &widths, ctx));
    lines.extend(body.iter().map(|row| row_line(row, &widths, &table.aligns, ctx)));
    lines.push(rule_line(['└', '┴', '┘'], &widths, ctx));
    lines
}

/// One horizontal rule — `[left, junction, right]` picking which of the
/// three it is. Every column's run is its content width plus the cell's own
/// two pad cells, so a rule meets its neighbouring row's `│` exactly.
fn rule_line(corners: [char; 3], widths: &[usize], ctx: Ctx) -> Line<'static> {
    let [left, junction, right] = corners;
    let mut rule = String::from(left);
    for (i, width) in widths.iter().copied().enumerate() {
        if i > 0 {
            rule.push(junction);
        }
        rule.extend(std::iter::repeat_n('─', width + 2 * CELL_PAD));
    }
    rule.push(right);
    Line::from(truncate_spans(vec![Span::styled(rule, Style::default().fg(ctx.pal.quiet))], ctx.width as usize))
}

/// Each column as wide as its widest *rendered* cell, then shrunk — widest
/// column first — until the whole row fits the column it is being drawn in.
/// Shrinking one cell at a time rather than scaling proportionally keeps a
/// narrow column (`yes`/`no`, a count) intact while the prose column gives
/// up the cells, which is nearly always the right trade.
fn column_widths(header: &[Cell], body: &[Vec<Cell>], avail: usize) -> Vec<usize> {
    let mut widths: Vec<usize> = header.iter().map(|cell| span_width(cell)).collect();
    for row in body {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(span_width(cell));
        }
    }

    // Every column costs its content plus two pad cells, and there is one
    // more rule than there are columns (`│ a │ b │`).
    let room = avail.saturating_sub(widths.len() * (2 * CELL_PAD + 1) + 1);
    let mut total: usize = widths.iter().sum();
    while total > room {
        // `max_by_key` yields the *last* maximum, so tied columns give up
        // cells right to left — the leftmost column is the one that names
        // the row, and it is the last that should lose its text.
        let Some((i, _)) = widths.iter().enumerate().max_by_key(|(_, w)| **w) else { break };
        if widths[i] <= 1 {
            // Every column is down to a single cell and it still does not
            // fit; `row_line` truncates the assembled row rather than
            // letting it overhang the body column.
            break;
        }
        widths[i] -= 1;
        total -= 1;
    }
    widths
}

/// One table row: `│`, then each cell elided to its column and padded to
/// that column's edge, then the closing `│`.
///
/// Every column is padded to its full width, the last one included — unlike
/// a rule-less layout, where the trailing run would be nothing but spaces at
/// the row's end. Here it holds the closing rule on the same column the rule
/// rows put their corner, and a row one cell short of that would leave the
/// box visibly unclosed.
fn row_line(cells: &[Cell], widths: &[usize], aligns: &[Align], ctx: Ctx) -> Line<'static> {
    let rule = Style::default().fg(ctx.pal.quiet);
    let pad = |n: usize| Span::raw(" ".repeat(n));
    let mut spans: Vec<Span<'static>> = vec![Span::styled("│", rule)];

    for (i, width) in widths.iter().copied().enumerate() {
        let cell = truncate_spans(cells.get(i).cloned().unwrap_or_default(), width);
        let slack = width.saturating_sub(span_width(&cell));
        let (before, after) = match aligns.get(i).copied().unwrap_or(Align::Left) {
            Align::Left => (0, slack),
            Align::Right => (slack, 0),
            Align::Center => (slack / 2, slack - slack / 2),
        };
        spans.push(pad(CELL_PAD + before));
        spans.extend(cell);
        spans.push(pad(after + CELL_PAD));
        spans.push(Span::styled("│", rule));
    }
    Line::from(truncate_spans(spans, ctx.width as usize))
}

fn span_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Renders one prose line (never a fenced-code line — those are already
/// pulled out by `split_code_fences`). Styling is modifiers only
/// (bold/italic/underline/reversed/crossed-out) — aldwin-tui.md reserves
/// the one accent colour for the approval card and focused input.
pub(super) fn render_line(line: &str, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let base = Style::default().fg(pal.body);
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];

    if is_hr(trimmed_start) {
        // A markdown thematic break is a separator, and separators are
        // bands: one row of `break_`, the same treatment a turn break gets.
        // It used to be a 20-cell run of `─`, which is not in the design
        // system's glyph vocabulary at all (`tokens::MARKS`) — and
        // that vocabulary is closed: "if a mark is needed and it is not in
        // that table, do not draw one."
        return band_row(pal.break_, ctx);
    }
    if let Some((level, rest)) = parse_heading(trimmed_start) {
        let style = if level <= 2 { base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { base.add_modifier(Modifier::BOLD) };
        return Line::from(parse_inline(rest, style, ctx));
    }
    if let Some(rest) = trimmed_start.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let mut spans = vec![Span::styled(format!("{indent}▎ "), Style::default().fg(pal.dim))];
        spans.extend(parse_inline(rest, base.add_modifier(Modifier::ITALIC), ctx));
        return Line::from(spans);
    }
    if let Some(rest) = parse_bullet(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}• "), base)];
        spans.extend(parse_inline(rest, base, ctx));
        return Line::from(spans);
    }
    if let Some((marker, rest)) = parse_ordered(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}{marker} "), base)];
        spans.extend(parse_inline(rest, base, ctx));
        return Line::from(spans);
    }
    Line::from(parse_inline(line, base, ctx))
}

/// An `_` between two word characters is a literal underscore, never an
/// emphasis delimiter.
///
/// CommonMark and GFM both disallow intraword `_` emphasis (and both allow it
/// for `*`), and the reason is exactly the case that broke here:
/// `ANTHROPIC_API_KEY` was rendering as `ANTHROPICAPIKEY` — italic `API`, both
/// underscores eaten. In a harness whose transcript is full of `snake_case`
/// identifiers, env-var names and file paths, silently deleting underscores is
/// worse than never supporting `_italic_` at all. Found by the screenshot
/// harness's `markdown` scene (2026-09-19), which is the first defect it
/// caught that this crate's own tests do not.
fn intraword(before: &str, rest: &str) -> bool {
    let previous = before.chars().last();
    let following = rest[1..].chars().next();
    matches!(previous, Some(c) if c.is_alphanumeric()) && matches!(following, Some(c) if c.is_alphanumeric())
}

/// Recursive-descent inline pass: `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, `~~strike~~`, `[text](url)`. Delimiters nest via recursion
/// (e.g. `**bold *and italic***`) rather than a flat token stream, which
/// keeps this a single small function instead of a tokenizer + AST.
pub(super) fn parse_inline(text: &str, base: Style, ctx: Ctx) -> Vec<Span<'static>> {
    fn flush(buf: &mut String, style: Style, spans: &mut Vec<Span<'static>>) {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    }

    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                flush(&mut buf, base, &mut spans);
                // The code tone *on the quoted-code ground* (`2b`:
                // `background:var(--tui-diff-box);color:var(--tui-code)`).
                // The ground is what does the work. `code` and the `body`
                // prose around it are one rung apart — enough to tell two
                // blocks from each other, not enough to pick one word out
                // of a sentence — so a span that only changed its ink read
                // as prose. On the raised ground it "reads as quoted rather
                // than emphasised", and it is the same ground a fenced
                // block and the inline diff take, so all three sizes of
                // quoted code are visibly one thing.
                //
                // Exactly the span's own cells: no padding cell either
                // side. The reference adds none, and a padded span would
                // shift every word after it off the column it wraps to.
                spans.push(Span::styled(stripped[..end].to_string(), Style::default().fg(ctx.pal.code).bg(ctx.pal.diff_box)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::BOLD), ctx));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("~~") {
            if let Some(end) = stripped.find("~~") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::CROSSED_OUT), ctx));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if rest.starts_with('*') || (rest.starts_with('_') && !intraword(&buf, rest)) {
            let delim = &rest[..1];
            let stripped = &rest[1..];
            if let Some(end) = stripped.find(delim) {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::ITALIC), ctx));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((label, url, remainder)) = parse_link(rest) {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(label.to_string(), base.add_modifier(Modifier::UNDERLINED)));
                if !url.is_empty() && url != label {
                    spans.push(Span::styled(format!(" ({url})"), Style::default().fg(ctx.pal.dim)));
                }
                rest = remainder;
                continue;
            }
        }

        let ch_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        buf.push_str(&rest[..ch_len]);
        rest = &rest[ch_len..];
    }
    flush(&mut buf, base, &mut spans);
    spans
}

fn parse_link(text: &str) -> Option<(&str, &str, &str)> {
    let after_bracket = &text[1..];
    let close = after_bracket.find(']')?;
    let label = &after_bracket[..close];
    let after_label = &after_bracket[close + 1..];
    let after_paren = after_label.strip_prefix('(')?;
    let close_paren = after_paren.find(')')?;
    let url = &after_paren[..close_paren];
    let remainder = &after_paren[close_paren + 1..];
    Some((label, url, remainder))
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(|rest| (hashes as u8, rest))
}

fn parse_bullet(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    line[marker.len_utf8()..].strip_prefix(' ')
}

fn parse_ordered(line: &str) -> Option<(String, &str)> {
    let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
    if digits_end == 0 {
        return None;
    }
    let (digits, rest) = line.split_at(digits_end);
    let sep = rest.chars().next()?;
    if sep != '.' && sep != ')' {
        return None;
    }
    let rest = rest[sep.len_utf8()..].strip_prefix(' ')?;
    Some((format!("{digits}{sep}"), rest))
}

/// A line of 3+ `-`, `*`, or `_` (ignoring interior spaces, so `- - -`
/// counts) and nothing else — CommonMark's thematic break.
fn is_hr(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3 && (stripped.chars().all(|c| c == '-') || stripped.chars().all(|c| c == '*') || stripped.chars().all(|c| c == '_'))
}
