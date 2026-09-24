//! Word-wrapping for one logical line, done *before* anything is inset or
//! filled.
//!
//! This exists instead of leaning on a `Paragraph`'s own
//! `Wrap { trim: false }` because `Wrap` has no concept of the label-column
//! inset `grid::with_label_column` applies afterward, nor of the
//! padding/border columns `row::Row` adds: it treats one logical `Line`'s
//! spans as a single continuous run of styled graphemes, so a wrapped
//! continuation row it produced came out flush against the panel edge
//! instead of under the rest of the turn's content (reported as: "the first
//! line of text is correctly in line, but when the text wraps onto a second
//! line, it doesn't respect the padding").
//!
//! Wrapping here means every row handed downstream is already ≤ its column
//! width and already fully inset and filled on its own, so the wrap happens
//! exactly once. Both `Row` and the prose path depend on that discipline;
//! see `aldwin-tui.md`'s Progress notes for the bugs it exists to prevent
//! from recurring.
//!
//! For the transcript there is no longer any downstream wrapper to fall back
//! on at all — `transcript::draw_log` renders a plain slice of already-sized
//! rows — so a builder that skips this step gets truncation, not a
//! badly-placed fold.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Cells a tab expands to — `draft::sanitize`'s own four, so pasted and
/// model-written indentation agree.
const TAB_WIDTH: usize = 4;

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
    // One per `char`, and `Copy`: this runs over every character of every
    // rebuilt transcript entry, and a `String` per character was the single
    // largest allocation count in a streamed reply's re-render.
    #[derive(Clone, Copy)]
    struct Grapheme {
        ch: char,
        style: Style,
        width: usize,
        is_space: bool,
    }

    if max_width == 0 {
        return vec![line];
    }

    let mut graphemes: Vec<Grapheme> =
        Vec::with_capacity(line.spans.iter().map(|s| s.content.len()).sum());
    for span in &line.spans {
        let style = span.style;
        for ch in span.content.chars() {
            match ch {
                // A tab has no width of its own and ratatui draws no cell
                // for it, so left in place it deleted a Go or Makefile
                // line's indentation outright. Expanded to the same four
                // spaces `draft::sanitize` gives a typed one.
                '\t' => graphemes.extend(
                    [Grapheme {
                        ch: ' ',
                        style,
                        width: 1,
                        is_space: true,
                    }; TAB_WIDTH],
                ),
                // Any other control character is dropped: ratatui skips it
                // too, but `UnicodeWidthStr` counts it as one cell, so
                // `Row::assemble` measured a cell nothing drew and left the
                // row's fill one cell short of its right edge.
                ch if ch.is_control() => {}
                ch => graphemes.push(Grapheme {
                    ch,
                    style,
                    width: ch.width().unwrap_or(0),
                    is_space: ch.is_whitespace(),
                }),
            }
        }
    }
    if graphemes.is_empty() {
        return vec![Line::default()];
    }

    // Finished rows, and the one being filled.
    let mut rows: Vec<Vec<Grapheme>> = Vec::new();
    let mut row: Vec<Grapheme> = Vec::new();
    let mut row_width = 0usize;
    let mut i = 0;

    // The line's own genuine leading whitespace (if any) is kept as literal
    // content on the first row — only whitespace a wrap decision below
    // introduces at a row break gets dropped, so a hand-indented prose line
    // keeps its indentation.
    if graphemes[0].is_space {
        let end = graphemes
            .iter()
            .position(|g| !g.is_space)
            .unwrap_or(graphemes.len());
        row_width = graphemes[..end].iter().map(|g| g.width).sum();
        row.extend_from_slice(&graphemes[..end]);
        i = end;
    }

    // Greedy fill: walk whitespace/non-whitespace runs in order, breaking
    // before whichever run would overflow the current row. A run of
    // whitespace is only ever kept mid-row (never used to open one), so a
    // wrapped row never starts with the space that caused the break — the
    // convention ratatui's own word-wrapper follows.
    while i < graphemes.len() {
        let is_space = graphemes[i].is_space;
        let start = i;
        while i < graphemes.len() && graphemes[i].is_space == is_space {
            i += 1;
        }
        let run = &graphemes[start..i];
        let run_width: usize = run.iter().map(|g| g.width).sum();

        if is_space {
            if !row.is_empty() {
                if row_width + run_width > max_width {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0;
                } else {
                    row_width += run_width;
                    row.extend_from_slice(run);
                }
            }
            continue;
        }

        if row_width > 0 && row_width + run_width > max_width {
            rows.push(std::mem::take(&mut row));
            row_width = 0;
        }
        if run_width > max_width {
            // A single word wider than the whole row (e.g. a long URL):
            // hard-break it character by character rather than overflowing.
            for g in run {
                if row_width > 0 && row_width + g.width > max_width {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0;
                }
                row_width += g.width;
                row.push(*g);
            }
        } else {
            row_width += run_width;
            row.extend_from_slice(run);
        }
    }
    rows.push(row);

    rows.into_iter()
        .map(|row| {
            // A wrap decision can leave trailing whitespace dangling at a
            // row's end (the space that caused the break, kept out of the
            // *next* row but already appended to this one); trim it so it
            // doesn't count toward width for anyone measuring this row.
            let end = row.iter().rposition(|g| !g.is_space).map_or(0, |i| i + 1);
            let mut spans: Vec<Span<'static>> = Vec::new();
            // Consecutive characters of one style become one span.
            for run in row[..end].chunk_by(|a, b| a.style == b.style) {
                spans.push(Span::styled(
                    run.iter().map(|g| g.ch).collect::<String>(),
                    run[0].style,
                ));
            }
            Line::from(spans)
        })
        .collect()
}
