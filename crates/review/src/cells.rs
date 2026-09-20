//! Stage 3, second half — every cell the app paints comes from the design
//! system.
//!
//! The first half ([`crate::tokens`]) proves the app's palette *is* the
//! design's. This proves the app uses nothing else: no colour outside the
//! palette, no glyph outside the closed table, no cell left unpainted.
//!
//! It reads the **declared cells** — what the app said it was drawing,
//! recovered from its own byte stream by the proxy — never the picture.
//! Reading colour off a PNG cannot work: rasterization antialiases every
//! glyph edge into values that belong to no palette.
//!
//! There is no judgement here and there is deliberately nowhere to put one.
//! Whether a band is in the right place, whether a label is the right rung,
//! whether a boundary has enough contrast — all of that is stage 5's, and an
//! earlier version of this harness that tried to answer it mechanically grew
//! a thousand lines of tables encoding one reading of an ambiguous reference.

use serde::{Deserialize, Serialize};

use crate::baseline::Baseline;
use crate::design::Design;
use crate::geometry::Theme;
use crate::vt::{Color, Grid};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub row:    u16,
    pub col:    u16,
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub violations: Vec<Violation>,
    /// Design contradictions this frame actually leaned on, so a run says out
    /// loud which ones are still load-bearing rather than leaving them in a
    /// file nobody reopens.
    pub applied:    Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

pub fn check(grid: &Grid, theme: Theme, design: &Design, baseline: &Baseline) -> Report {
    let mut report = Report::default();
    let allowed: Vec<(char, &str)> = baseline
        .contradictions
        .iter()
        .flat_map(|c| c.glyphs.chars().map(move |ch| (ch, c.id.as_str())))
        .collect();

    for (row, col, cell) in grid.cells() {
        if cell.is_continuation() {
            continue;
        }
        let (fg, bg) = cell.effective();

        // A cell the app never painted: the terminal's default shows through.
        if bg == Color::Default || (fg == Color::Default && cell.ch != ' ') {
            report.violations.push(Violation {
                row,
                col,
                detail: format!("unpainted cell {:?} — the terminal's default colour shows through", cell.ch),
            });
        }

        // A glyph from outside the closed table.
        if cell.ch != ' ' && !cell.ch.is_ascii() && !design.marks.contains(&cell.ch) {
            match allowed.iter().find(|(c, _)| *c == cell.ch) {
                Some((_, id)) => {
                    let note = format!("{:?} allowed by design contradiction {id}", cell.ch);
                    if !report.applied.contains(&note) {
                        report.applied.push(note);
                    }
                }
                None => report.violations.push(Violation {
                    row,
                    col,
                    detail: format!("{:?} is not in the design system's glyph table", cell.ch),
                }),
            }
        }

        // A wide glyph with nowhere to put its second half.
        if col + 1 == grid.cols && unicode_width::UnicodeWidthChar::width(cell.ch).unwrap_or(1) == 2 {
            report.violations.push(Violation {
                row,
                col,
                detail: format!("{:?} is two cells wide and the row has one left", cell.ch),
            });
        }

        // Every declared colour is a palette role, or a role blended toward a
        // ground — the transcript behind an open panel is dimmed, which is by
        // construction not a palette value.
        for (which, colour) in [("foreground", fg), ("background", bg)] {
            match colour {
                Color::Rgb(r, g, b) if design.role_names(theme, (r, g, b)).is_empty() => {
                    if design.dimmed(theme, (r, g, b)).is_none() {
                        report.violations.push(Violation {
                            row,
                            col,
                            detail: format!("{which} #{r:02x}{g:02x}{b:02x} is not a {theme}-theme role, or a role dimmed toward a ground"),
                        });
                    }
                }
                Color::Indexed(n) => report.violations.push(Violation {
                    row,
                    col,
                    detail: format!("{which} is ANSI colour {n}; the design system is authored in truecolor"),
                }),
                _ => {}
            }
        }
    }

    content(grid, &mut report);
    report
}

/// The mechanical part of the Content Fundamentals: third person, and no
/// contractions. Tone and sentence case stay stage 5's.
///
/// The bare pronoun needs more care than the contractions do, and the first
/// version of this proved it by firing on the wordmark: `M J O L N I R` is
/// letter-spaced, so it contains a literal `"I "`. Requiring a lowercase word
/// after it separates `I can` from `I R`.
fn content(grid: &Grid, report: &mut Report) {
    const FIRST_PERSON: [&str; 6] = ["I'm", "I'll", "I've", "we ", "We ", "our "];
    const ENCLITICS: [&str; 6] = ["n't", "'re", "'ll", "'ve", "'s ", "'d "];

    for row in 0..grid.rows {
        let (text, columns) = row_with_columns(grid, row);
        let bytes = text.as_bytes();
        let column_of = |at: usize| columns.get(text[..at].chars().count()).copied().unwrap_or(0);
        let starts_word = |at: usize| at == 0 || !bytes[at - 1].is_ascii_alphanumeric();

        for form in FIRST_PERSON {
            for (at, _) in text.match_indices(form) {
                if starts_word(at) {
                    report.violations.push(Violation {
                        row,
                        col: column_of(at),
                        detail: format!("first person {form:?} — the agent is written about in the third person"),
                    });
                }
            }
        }

        for (at, window) in bytes.windows(3).enumerate() {
            if starts_word(at) && window[0] == b'I' && window[1] == b' ' && window[2].is_ascii_lowercase() {
                report.violations.push(Violation {
                    row,
                    col: column_of(at),
                    detail: "first person \"I\" — the agent is written about in the third person".into(),
                });
            }
        }

        for form in ENCLITICS {
            for (at, _) in text.match_indices(form) {
                if at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
                    report.violations.push(Violation {
                        row,
                        col: column_of(at),
                        detail: format!("contraction {form:?} — the design system's copy uses none"),
                    });
                }
            }
        }
    }
}

/// A row as text, plus the grid column each character came from. Byte offsets
/// are not columns: `▌`, `·` and `…` are three bytes and one cell each.
fn row_with_columns(grid: &Grid, row: u16) -> (String, Vec<u16>) {
    let mut text = String::new();
    let mut columns = Vec::new();
    for col in 0..grid.cols {
        let cell = grid.get(row, col);
        if cell.is_continuation() {
            continue;
        }
        text.push(cell.ch);
        columns.push(col);
    }
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    columns.truncate(text.chars().count());
    (text, columns)
}
