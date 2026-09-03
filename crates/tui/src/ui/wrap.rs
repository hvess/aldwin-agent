//! Word-wrapping for one logical line, done *before* anything is inset or
//! filled.
//!
//! This exists instead of leaning on the log paragraph's own
//! `Wrap { trim: false }` (`transcript::draw_log`) because `Wrap` has no
//! concept of the label-column inset `grid::with_label_column` applies
//! afterward, nor of the padding/border columns `row::Row` adds: it treats
//! one logical `Line`'s spans as a single continuous run of styled
//! graphemes, so a wrapped continuation row it produced came out flush
//! against the panel edge instead of under the rest of the turn's content
//! (reported as: "the first line of text is correctly in line, but when the
//! text wraps onto a second line, it doesn't respect the padding").
//!
//! Wrapping here means every row handed downstream is already ≤ its column
//! width and already fully inset and filled on its own — `Wrap` downstream
//! never has to split anything, so the wrap happens exactly once. Both
//! `Row` and the prose path depend on that discipline; see `mjolnir-tui.md`'s
//! Progress notes for the two bugs it exists to prevent from recurring.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Word-wraps one logical `Line` to `max_width` display columns, breaking
/// only at whitespace and preserving each span's style across a break, into
/// however many `Line`s it takes.
///
/// Doesn't hang-indent list/blockquote markers under wrapped continuation
/// text (a wrapped `• ` bullet's second row starts at the same column every
/// other prose row does, not under the first row's text) — only the flat
/// inset every prose row gets from `with_label_column` regardless of what
/// produced it.
pub(super) fn wrap_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    #[derive(Clone)]
    struct Grapheme {
        text:     String,
        style:    Style,
        width:    usize,
        is_space: bool,
    }

    if max_width == 0 {
        return vec![line];
    }

    let graphemes: Vec<Grapheme> = line
        .spans
        .into_iter()
        .flat_map(|span| {
            let style = span.style;
            span.content.chars().map(move |ch| Grapheme { text: ch.to_string(), style, width: ch.width().unwrap_or(0), is_space: ch.is_whitespace() }).collect::<Vec<_>>()
        })
        .collect();
    if graphemes.is_empty() {
        return vec![Line::default()];
    }

    let mut rows: Vec<Vec<Grapheme>> = vec![Vec::new()];
    let mut row_width = 0usize;
    let mut i = 0;

    // The line's own genuine leading whitespace (if any) is kept as literal
    // content on the first row — only whitespace a wrap decision below
    // introduces at a row break gets dropped, so a rare hand-indented prose
    // line doesn't lose that indentation just because it happens to be
    // short enough to fit on one row anyway.
    if graphemes[0].is_space {
        let end = graphemes.iter().position(|g| !g.is_space).unwrap_or(graphemes.len());
        row_width = graphemes[..end].iter().map(|g| g.width).sum();
        rows[0].extend_from_slice(&graphemes[..end]);
        i = end;
    }

    // Greedy fill: walk whitespace/non-whitespace runs in order, breaking
    // before whichever run would overflow the current row. A run of
    // whitespace is only ever kept mid-row (never used to open one), so a
    // wrapped row never starts with the space that caused the break — same
    // "don't start a wrapped line with the space that broke it" convention
    // ratatui's own word-wrapper follows.
    while i < graphemes.len() {
        let is_space = graphemes[i].is_space;
        let start = i;
        while i < graphemes.len() && graphemes[i].is_space == is_space {
            i += 1;
        }
        let run = &graphemes[start..i];
        let run_width: usize = run.iter().map(|g| g.width).sum();

        if is_space {
            if !rows.last().unwrap().is_empty() {
                if row_width + run_width > max_width {
                    rows.push(Vec::new());
                    row_width = 0;
                } else {
                    row_width += run_width;
                    rows.last_mut().unwrap().extend_from_slice(run);
                }
            }
            continue;
        }

        if row_width > 0 && row_width + run_width > max_width {
            rows.push(Vec::new());
            row_width = 0;
        }
        if run_width > max_width {
            // A single word wider than the whole row (e.g. a long URL):
            // hard-break it grapheme by grapheme rather than overflowing.
            for g in run {
                if row_width > 0 && row_width + g.width > max_width {
                    rows.push(Vec::new());
                    row_width = 0;
                }
                row_width += g.width;
                rows.last_mut().unwrap().push(g.clone());
            }
        } else {
            row_width += run_width;
            rows.last_mut().unwrap().extend_from_slice(run);
        }
    }

    rows.into_iter()
        .map(|row| {
            // A wrap decision can leave trailing whitespace dangling at a
            // row's end (the space that caused the break, kept out of the
            // *next* row but already appended to this one before the
            // overflow check above); trim it so it doesn't count toward
            // width for anyone measuring this row later.
            let end = row.iter().rposition(|g| !g.is_space).map(|i| i + 1).unwrap_or(0);
            let mut spans: Vec<Span<'static>> = Vec::new();
            for g in &row[..end] {
                match spans.last_mut() {
                    Some(Span { content, style }) if *style == g.style => {
                        let mut merged = content.to_string();
                        merged.push_str(&g.text);
                        *content = merged.into();
                    }
                    _ => spans.push(Span::styled(g.text.clone(), g.style)),
                }
            }
            Line::from(spans)
        })
        .collect()
}
