//! Markdown from the LLM: fence splitting, per-line block prefixes, tables,
//! and the inline pass.
//!
//! Hand-rolled, not a CommonMark crate: a block parser reflows paragraphs
//! and normalizes blank lines, which breaks the line-for-line render redone
//! on every streamed delta.
//!
//! Enter through [`render_prose`], not [`render_line`]: a table's rows must
//! be measured together.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::grid::{truncate_spans, Ctx, MARK_COL};
use super::row::band_row;
use super::wrap::wrap_line;

/// One piece of assistant text: prose or a fenced code block.
pub(super) enum Segment {
    Prose(String),
    Code { lang: String, body: String },
}

/// Splits on ` ``` ` fences; the opening fence may carry a language tag. An
/// unterminated fence is code to the end of the text: while streaming, its
/// close has not arrived yet.
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
                segments.push(Segment::Code {
                    lang: lang.trim().to_string(),
                    body,
                });
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

/// Renders one prose segment, each line fitted to `ctx.width` (see
/// [`super::transcript::Transcript`]). A table is consumed as a block; every
/// other line goes through [`render_line`].
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

/// A column's alignment, from the delimiter row (`:--`, `--:`, `:-:`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

/// One GFM pipe table as source text; the inline pass runs later, per cell.
struct Table {
    header: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<String>>,
}

/// One table cell after the inline pass, so it is measured at its rendered
/// width, not its source width.
type Cell = Vec<Span<'static>>;

/// Cells between a column's `│` and its content, each side.
const CELL_PAD: usize = 1;

/// Reads a table off the front of `lines`, with the number of lines consumed.
/// As in GFM, only the delimiter row commits a table, so a half-streamed
/// table renders as prose until it arrives.
fn parse_table(lines: &[&str]) -> Option<(Table, usize)> {
    let header = split_row(lines.first()?)?;
    let aligns = parse_delimiter(lines.get(1)?, header.len())?;

    let mut rows = Vec::new();
    let mut consumed = 2;
    // Ends at the first line with no pipe.
    while let Some(cells) = lines.get(consumed).and_then(|l| split_row(l)) {
        rows.push(cells);
        consumed += 1;
    }
    Some((
        Table {
            header,
            aligns,
            rows,
        },
        consumed,
    ))
}

/// Splits `| a | b |` into trimmed cells, or `None` with no pipe. Outer
/// pipes are optional; `\|` is a literal pipe.
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

/// Parses the `|---|:--:|---:|` row into alignments. `None` (prose) unless
/// every cell is dashes with optional end colons, one per header cell.
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

/// Lays the table out on `ctx.width` as a drawn grid, rules in `label3`.
///
/// The only stroked component (ADR 0002, `MARKS_BY_EXCEPTION`); the box
/// glyphs must not spread to any other boundary, which stays a band.
///
/// Below `4n + 1` cells for `n` columns (every column at one cell), rows are
/// clipped with `…` and lose the right edge; never close the box early or
/// drop columns.
fn render_table(table: &Table, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let columns = table.header.len();

    // Header in `label`, not bold (ADR 0002): the design reserves weight 600
    // for the review title, a file path, a question and "Aldwin".
    let header: Vec<Cell> = table
        .header
        .iter()
        .map(|cell| parse_inline(cell, Style::default().fg(pal.label), ctx))
        .collect();
    let body: Vec<Vec<Cell>> = table
        .rows
        .iter()
        .map(|row| {
            (0..columns)
                .map(|i| {
                    parse_inline(
                        row.get(i).map(String::as_str).unwrap_or(""),
                        Style::default().fg(pal.label),
                        ctx,
                    )
                })
                .collect()
        })
        .collect();

    let widths = column_widths(&header, &body, ctx.width as usize);
    let mut lines = Vec::with_capacity(body.len() + 4);
    lines.push(rule_line(['┌', '┬', '┐'], &widths, ctx));
    lines.push(row_line(&header, &widths, &table.aligns, ctx));
    lines.push(rule_line(['├', '┼', '┤'], &widths, ctx));
    lines.extend(
        body.iter()
            .map(|row| row_line(row, &widths, &table.aligns, ctx)),
    );
    lines.push(rule_line(['└', '┴', '┘'], &widths, ctx));
    lines
}

/// One horizontal rule from `[left, junction, right]`. Each run must be
/// `width + 2 * CELL_PAD` to meet `row_line`'s `│`.
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
    Line::from(truncate_spans(
        vec![Span::styled(rule, Style::default().fg(ctx.pal.label3))],
        ctx.width as usize,
    ))
}

/// Each column as wide as its widest rendered cell, then the widest shrunk
/// one cell at a time until the row fits `avail`. Not proportional: a
/// narrow column (a count, `yes`/`no`) stays intact.
fn column_widths(header: &[Cell], body: &[Vec<Cell>], avail: usize) -> Vec<usize> {
    let mut widths: Vec<usize> = header.iter().map(|cell| span_width(cell)).collect();
    for row in body {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(span_width(cell));
        }
    }

    // Per column: content, two pads and one `│`; plus the closing `│`.
    let room = avail.saturating_sub(widths.len() * (2 * CELL_PAD + 1) + 1);
    let mut total: usize = widths.iter().sum();
    while total > room {
        // `max_by_key` yields the last maximum, so ties shrink right to
        // left and the leftmost column, which names the row, shrinks last.
        let Some((i, _)) = widths.iter().enumerate().max_by_key(|(_, w)| **w) else {
            break;
        };
        if widths[i] <= 1 {
            // All columns at one cell and still too wide: `row_line` and
            // `rule_line` clip the assembled row.
            break;
        }
        widths[i] -= 1;
        total -= 1;
    }
    widths
}

/// One table row, each cell elided and padded to its column. The last
/// column is padded too, so the closing `│` lines up with the rule's corner.
fn row_line(cells: &[Cell], widths: &[usize], aligns: &[Align], ctx: Ctx) -> Line<'static> {
    let rule = Style::default().fg(ctx.pal.label3);
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

/// Renders one prose line (fences are already split out). Styling is
/// modifiers only: never underline (a stroke), never the accent (blue is
/// the developer's, not the agent's prose).
pub(super) fn render_line(line: &str, ctx: Ctx) -> Line<'static> {
    let pal = ctx.pal;
    let base = Style::default().fg(pal.label);
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];

    if is_hr(trimmed_start) {
        // A band, not a run of `─`: the glyph table is closed.
        return band_row(pal.tint, ctx);
    }
    if let Some(rest) = parse_heading(trimmed_start) {
        // A heading is plain prose: no underline (a stroke), no weight 600
        // (reserved as for table headers). Inline `**bold**` still applies.
        return Line::from(parse_inline(rest, base, ctx));
    }
    // A quote is only indented by `MARK_COL`: `▎` is reserved for the
    // developer's selection.
    if let Some(rest) = trimmed_start.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let mut spans = vec![Span::raw(format!("{indent}{}", " ".repeat(MARK_COL)))];
        spans.extend(parse_inline(rest, base.add_modifier(Modifier::ITALIC), ctx));
        return Line::from(spans);
    }
    // `·`: the closed glyph table has no bullet.
    if let Some(rest) = parse_bullet(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}· "), base)];
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

/// Whether the `_` at `rest[0]` sits between two alphanumerics: then it is a
/// literal, as in CommonMark, so `ANTHROPIC_API_KEY` keeps its underscores.
fn intraword(before: &str, rest: &str) -> bool {
    let previous = before.chars().last();
    let following = rest[1..].chars().next();
    matches!(previous, Some(c) if c.is_alphanumeric())
        && matches!(following, Some(c) if c.is_alphanumeric())
}

/// Recursive-descent inline pass: `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, `~~strike~~`, `[text](url)`; nesting is by recursion.
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
                // On `--tint` like a fenced block; ink alone read as prose.
                // No padding cells, so nothing after it shifts.
                spans.push(Span::styled(
                    stripped[..end].to_string(),
                    Style::default().fg(ctx.pal.label).bg(ctx.pal.tint),
                ));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(
                    &stripped[..end],
                    base.add_modifier(Modifier::BOLD),
                    ctx,
                ));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("~~") {
            if let Some(end) = stripped.find("~~") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(
                    &stripped[..end],
                    base.add_modifier(Modifier::CROSSED_OUT),
                    ctx,
                ));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if rest.starts_with('*') || (rest.starts_with('_') && !intraword(&buf, rest)) {
            let delim = &rest[..1];
            let stripped = &rest[1..];
            if let Some(end) = stripped.find(delim) {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(
                    &stripped[..end],
                    base.add_modifier(Modifier::ITALIC),
                    ctx,
                ));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((label, url, remainder)) = parse_link(rest) {
                flush(&mut buf, base, &mut spans);
                // No underline: it is a stroke.
                spans.push(Span::styled(label.to_string(), base));
                if !url.is_empty() && url != label {
                    spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(ctx.pal.label3),
                    ));
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

fn parse_heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].strip_prefix(' ')
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

/// CommonMark's thematic break: 3+ of one of `-`, `*`, `_`, spaces allowed.
fn is_hr(line: &str) -> bool {
    let mut marks = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = marks.next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    let mut count = 1;
    marks.all(|c| {
        count += 1;
        c == first
    }) && count >= 3
}
