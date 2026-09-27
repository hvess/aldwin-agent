//! Word-wrapping for one logical line, before anything is inset or filled.
//!
//! Not `Paragraph`'s `Wrap`: it knows nothing of the inset `grid::at_body` or
//! `row::Row` adds afterwards, so its continuation rows land flush against
//! the edge (`aldwin-tui.md` Progress notes). `transcript::draw_log` does not
//! wrap, so a builder that skips this step gets truncated rows.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::draft;

/// Word-wraps `line` to `max_width` display cells, keeping each span's
/// style. Breaks at whitespace; a word wider than `max_width` is hard-broken.
/// `max_width == 0` returns `line` unchanged.
///
/// No hanging indent: a continuation row starts at column 0.
pub(super) fn wrap_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    // One `Copy` value per `char`, not a `String`: this runs over every
    // character of a streamed reply on each re-render.
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
                // ratatui draws no cell for a tab, which erases indentation;
                // expand to `draft::TAB` as everywhere else.
                '\t' => graphemes.extend(
                    [Grapheme {
                        ch: ' ',
                        style,
                        width: 1,
                        is_space: true,
                    }; draft::TAB.len()],
                ),
                // Dropped: ratatui draws no cell for it, but `UnicodeWidthStr`
                // counts one, which leaves `Row::assemble`'s fill a cell short.
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

    let mut rows: Vec<Vec<Grapheme>> = Vec::new();
    let mut row: Vec<Grapheme> = Vec::new();
    let mut row_width = 0usize;
    let mut i = 0;

    // The line's own leading whitespace is kept, so an indented line stays
    // indented; only whitespace at a wrap break is dropped.
    if graphemes[0].is_space {
        let end = graphemes
            .iter()
            .position(|g| !g.is_space)
            .unwrap_or(graphemes.len());
        row_width = graphemes[..end].iter().map(|g| g.width).sum();
        row.extend_from_slice(&graphemes[..end]);
        i = end;
    }

    // Greedy fill over whitespace/word runs. Whitespace never opens a row,
    // as in ratatui's own word-wrapper.
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
            // A word wider than a row (a long URL): hard-break per character.
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
            // Trailing whitespace is trimmed so it does not count toward
            // the row's measured width.
            let end = row.iter().rposition(|g| !g.is_space).map_or(0, |i| i + 1);
            let mut spans: Vec<Span<'static>> = Vec::new();
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
