//! The deterministic gates.
//!
//! Every one of these reads the **declared cells** — what the app said it was
//! drawing, recovered from its own byte stream by the proxy — not the
//! picture. Reading colour off a PNG cannot work: rasterization antialiases
//! every glyph edge into values that belong to no palette, and a pixel says
//! nothing about which run of text is a label rather than body.
//!
//! A gate is zero-tolerance and is never averaged into a score. A frame with
//! truncated text is not eighty percent unbroken.
//!
//! Before reporting anything, each gate consults the baseline file. What it
//! finds there is correct *by decision* and cites the ADR or spec entry that
//! made it so — ADR 0002's markdown table really does draw box-drawing
//! glyphs the closed table forbids. An entry that cannot name its authority
//! is a bug being silenced, and `Baseline::load` refuses the file.

use serde::{Deserialize, Serialize};

use crate::baseline::Baseline;
use crate::contrast;
use crate::design::Design;
use crate::facts::Facts;
use crate::geometry::Theme;
use crate::regions::{Map, Zone};
use crate::vt::{Color, Grid};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub gate:   String,
    pub row:    u16,
    pub col:    u16,
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub violations: Vec<Violation>,
    /// Exceptions that actually suppressed something, so a run says out loud
    /// which carve-outs it leaned on rather than leaving them in a file
    /// nobody reopens.
    pub applied:    Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn by_gate(&self) -> Vec<(&str, usize)> {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for v in &self.violations {
            match counts.iter_mut().find(|(g, _)| *g == v.gate) {
                Some((_, n)) => *n += 1,
                None => counts.push((v.gate.as_str(), 1)),
            }
        }
        counts
    }
}

/// Run every gate that reads a single frame.
///
/// `regression` is not here: it compares two revisions rather than inspecting
/// one frame, and lives in [`crate::regression`].
pub fn run(grid: &Grid, map: &Map, theme: Theme, design: &Design, baseline: &Baseline, facts: &Facts) -> Report {
    let mut report = Report::default();

    let exempt_glyphs: Vec<(char, &str)> = baseline
        .exceptions
        .iter()
        .filter(|e| e.gate == "breakages")
        .flat_map(|e| e.glyphs.chars().map(move |c| (c, e.authority.as_str())))
        .collect();

    for (row, col, cell) in grid.cells() {
        if cell.is_continuation() {
            continue;
        }
        let (fg, bg) = cell.effective();

        // --- breakages: a cell the app never painted ----------------------
        // The design system requires every glyph inside a frame to come from
        // the palette; a default colour is the terminal's, not the app's.
        if bg == Color::Default || (fg == Color::Default && cell.ch != ' ') {
            report.violations.push(Violation {
                gate: "breakages".into(),
                row,
                col,
                detail: format!("unpainted cell {:?} — the terminal's default colour shows through", cell.ch),
            });
        }

        // --- breakages: a glyph from outside the closed table --------------
        if cell.ch != ' ' && !cell.ch.is_ascii() && !design.marks.contains(&cell.ch) {
            match exempt_glyphs.iter().find(|(c, _)| *c == cell.ch) {
                Some((_, authority)) => {
                    let note = format!("breakages: {:?} allowed by {authority}", cell.ch);
                    if !report.applied.contains(&note) {
                        report.applied.push(note);
                    }
                }
                None => report.violations.push(Violation {
                    gate: "breakages".into(),
                    row,
                    col,
                    detail: format!("{:?} is not in the design system's glyph table", cell.ch),
                }),
            }
        }

        // --- breakages: a wide glyph with nowhere to put its second half ---
        if col + 1 == grid.cols && unicode_width::UnicodeWidthChar::width(cell.ch).unwrap_or(1) == 2 {
            report.violations.push(Violation {
                gate: "breakages".into(),
                row,
                col,
                detail: format!("{:?} is two cells wide and the row has one left", cell.ch),
            });
        }

        // --- colour: every declared value belongs to the theme -------------
        for (which, colour) in [("foreground", fg), ("background", bg)] {
            match colour {
                Color::Rgb(r, g, b) if design.role_names(theme, (r, g, b)).is_empty() => {
                    // A dimmed cell is a role blended toward a ground, which
                    // is by construction not a palette value. Reconstructing
                    // the blend keeps the gate honest about the difference
                    // between "dimmed" and "off-palette".
                    match design.dimmed(theme, (r, g, b)) {
                        Some((role, alpha)) => {
                            let note = format!("colour: --tui-{role} dimmed {alpha}% toward a ground (a panel is open)");
                            if !report.applied.contains(&note) {
                                report.applied.push(note);
                            }
                        }
                        None => report.violations.push(Violation {
                            gate: "colour".into(),
                            row,
                            col,
                            detail: format!("{which} #{r:02x}{g:02x}{b:02x} is not a {theme}-theme role, or a role dimmed toward a ground"),
                        }),
                    }
                }
                Color::Indexed(n) => report.violations.push(Violation {
                    gate: "colour".into(),
                    row,
                    col,
                    detail: format!("{which} is ANSI colour {n}; the design system is authored in truecolor"),
                }),
                _ => {}
            }
        }
    }

    content(grid, &mut report);
    layout(grid, map, &mut report, baseline);
    role_pairing(grid, map, theme, design, &mut report);
    contrast_gate(facts, map, &mut report, baseline);
    report
}

/// Contrast, against the two floors the design states about itself.
///
/// The arithmetic and the provenance of both floors are in
/// [`crate::contrast`]. What is here is the policy: ink is measured against
/// the ground it is actually painted on, a band is measured against the band
/// below it, and a pair that is correct by decision is exempted **by name**
/// with an authority, never by moving a floor.
///
/// Three judges on `run-1789850385` each computed a subset of these by hand
/// from sampled pixels, and three of the conformance catalogue's Class C
/// items are contrast findings that arrived that way. None of it needed a
/// model.
fn contrast_gate(facts: &Facts, map: &Map, report: &mut Report, baseline: &Baseline) {
    let exempt: Vec<(&str, &str)> = baseline
        .exceptions
        .iter()
        .filter(|e| e.gate == "contrast")
        .flat_map(|e| e.pairs.iter().map(move |p| (p.as_str(), e.authority.as_str())))
        .collect();

    for adjacency in &facts.adjacencies {
        // A mark answers to the floor the design sets for marks, and a
        // dimmed span to none at all — see `contrast::floor_for`, which
        // carries the reasoning and the arithmetic behind both.
        let Some(floor) = contrast::floor_for(&adjacency.ink) else { continue };
        if adjacency.ratio >= floor {
            continue;
        }
        let pair = format!("{} on {}", adjacency.ink, adjacency.ground);
        match exempt.iter().find(|(p, _)| *p == pair) {
            Some((_, authority)) => {
                let note = format!("contrast: {pair} at {:.3}:1 allowed by {authority}", adjacency.ratio);
                if !report.applied.contains(&note) {
                    report.applied.push(note);
                }
            }
            None => report.violations.push(Violation {
                gate: "contrast".into(),
                row: adjacency.first_at.0,
                col: adjacency.first_at.1,
                detail: format!(
                    "{pair} is {:.3}:1, under the {floor}:1 the design states for it (HANDOFF.md:109); {} span(s)",
                    adjacency.ratio, adjacency.spans
                ),
            }),
        }
    }

    // A band against the band below it. Since Turn 13 nothing inside a frame
    // is stroked, so this step is the entire boundary — below the floor there
    // is not a faint boundary, there is none.
    for pair in map.bands.windows(2) {
        let (upper, lower) = (&pair[0], &pair[1]);
        let (Some(a), Some(b)) = (hex_rgb(&upper.ground), hex_rgb(&lower.ground)) else { continue };
        let ratio = contrast::ratio(a, b);
        if ratio >= contrast::GROUND_FLOOR {
            continue;
        }
        let name = |band: &crate::regions::Band| band.role.clone().unwrap_or_else(|| band.ground.clone());
        let label = format!("--tui-{} against --tui-{}", name(upper), name(lower));
        match exempt.iter().find(|(p, _)| *p == label) {
            Some((_, authority)) => {
                let note = format!("contrast: {label} at {ratio:.3}:1 allowed by {authority}");
                if !report.applied.contains(&note) {
                    report.applied.push(note);
                }
            }
            None => report.violations.push(Violation {
                gate: "contrast".into(),
                row: lower.from,
                col: 0,
                detail: format!(
                    "{label} is {ratio:.3}:1, under the {}:1 the design claims for every adjacency (HANDOFF.md:74) — and the step is the whole boundary",
                    contrast::GROUND_FLOOR
                ),
            }),
        }
    }
}

fn hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let v = value.strip_prefix('#')?;
    if v.len() != 6 {
        return None;
    }
    Some((u8::from_str_radix(&v[0..2], 16).ok()?, u8::from_str_radix(&v[2..4], 16).ok()?, u8::from_str_radix(&v[4..6], 16).ok()?))
}

/// Content is not supposed to be in the margin.
///
/// The grid puts a 3-cell gutter at either edge and a band's *ground* runs
/// through it; its glyphs do not. That makes a glyph in the margin the visible
/// symptom of the defect this UI keeps producing — a row that ran into its
/// neighbour and clipped, `6c1ab32` and `1f125d4` both — without needing to
/// know what the row was trying to say.
///
/// The decision panel is the documented exception: the handoff puts its option
/// rows "flush to the frame's left edge", so it carries a baseline entry
/// rather than a special case here.
fn layout(grid: &Grid, map: &Map, report: &mut Report, baseline: &Baseline) {
    let option_rows_may_be_flush = baseline.exceptions.iter().any(|e| e.gate == "layout");

    for (row, col, cell) in grid.cells() {
        if cell.ch == ' ' || cell.is_continuation() || map.zone(col) != Zone::Margin {
            continue;
        }
        // A row whose option list is flush left is exempt only on its left
        // margin — nothing licences running off the right.
        let flush_left = col < crate::regions::MARGIN_X && option_rows_may_be_flush && is_option_row(grid, row);
        if flush_left {
            let note = "layout: option rows are flush to the frame's left edge (.claude/design/HANDOFF.md, screen 5a)".to_string();
            if !report.applied.contains(&note) {
                report.applied.push(note);
            }
            continue;
        }
        report.violations.push(Violation {
            gate: "layout".into(),
            row,
            col,
            detail: format!("{:?} is in the {}-cell margin", cell.ch, crate::regions::MARGIN_X),
        });
    }
}

/// A decision panel's option row, in the exact shape the handoff specifies for
/// screen `5a`: the mark in cell 0, then the number, then two cells before the
/// label — "option text starts at cell 6".
///
/// Matched precisely rather than loosely. "A `▌` and a digit somewhere in the
/// first six cells" would hand the margin carve-out to any transcript row that
/// happened to open with a speaker mark and a number.
fn is_option_row(grid: &Grid, row: u16) -> bool {
    grid.get(row, 0).ch == '▌'
        && grid.get(row, 1).ch == ' '
        && grid.get(row, 2).ch == ' '
        && grid.get(row, 3).ch.is_ascii_digit()
        && grid.get(row, 4).ch == ' '
}

/// A band's ground must be a ground, and what sits on it must not be.
///
/// This is the weakest useful form of the design's role pairing, and it is
/// deliberately not stronger: the map knows which band a cell is in and which
/// column zone it sits in, but the design does not enumerate which ink role
/// belongs on which band, so anything more specific would be invented here
/// rather than imported. What it does catch is the two ways a role can be
/// used as the wrong *kind* of thing — a band painted with an ink colour, or
/// text painted in a ground.
fn role_pairing(grid: &Grid, map: &Map, theme: Theme, design: &Design, report: &mut Report) {
    // Only the ink ramp is flagged as a band. A diff row, a selection band and
    // a recessed field all paint rows with roles that are not ground rungs and
    // are entirely correct — `--tui-del-row` is a band by design. What is
    // never correct is a band painted in a colour whose job is to be read.
    for band in &map.bands {
        if let Some(role) = &band.role {
            if Design::INK.contains(&role.as_str()) {
                report.violations.push(Violation {
                    gate: "role pairing".into(),
                    row: band.from,
                    col: 0,
                    detail: format!("rows {}..{} are a band of --tui-{role}, an ink role — ink is for glyphs, not fields", band.from, band.to),
                });
            }
        }
    }

    for (row, col, cell) in grid.cells() {
        if cell.ch == ' ' || cell.is_continuation() {
            continue;
        }
        if let Color::Rgb(r, g, b) = cell.effective().0 {
            if design.is_only_ground(theme, (r, g, b)) {
                let names = design.role_names(theme, (r, g, b)).join("/");
                report.violations.push(Violation {
                    gate: "role pairing".into(),
                    row,
                    col,
                    detail: format!("{:?} is painted in --tui-{names}, a ground rung, not an ink role", cell.ch),
                });
            }
        }
    }
}

/// The mechanical part of the Content Fundamentals.
///
/// Three rules, all of them things a model should never have been asked to
/// look for by eye:
///
/// * **Third person.** The agent is written about, not as. This was the
///   original gate.
/// * **No contractions.** The design system's copy has none anywhere, in any
///   of its five screens. `run-1789850385` found
///   `deciding doesn't require scrolling them` shipping in every `prompt*`
///   frame at 80×24 (conformance Class A item 40).
/// * **A count and its pronoun agree.** The same string reads "1 more line …
///   scrolling **them**" whenever exactly one row is hidden, which at 80×24
///   is always. A format string that pluralises its noun and not its pronoun
///   is a defect no amount of reading the source catches, because the source
///   looks right — it is only wrong for one value of `n`.
///
/// Tone and sentence case stay a human judgement.
///
/// The bare pronoun needs more care than the contractions do, and the first
/// version of this gate proved it by firing on the wordmark: `M J O L N I R`
/// is letter-spaced, so it contains a literal `"I "`. Requiring a lowercase
/// word after it separates `I can` from `I R`. That is a heuristic, and the
/// honest fix is the region map — prose is a region, and a wordmark is not —
/// which is also what `role pairing` is waiting on.
fn content(grid: &Grid, report: &mut Report) {
    const CONTRACTIONS: [&str; 6] = ["I'm", "I'll", "I've", "we ", "We ", "our "];
    /// Any contraction, not only a first-person one. The apostrophe is the
    /// signal; the list is what an English contraction can end in.
    const ENCLITICS: [&str; 6] = ["n't", "'re", "'ll", "'ve", "'s ", "'d "];

    for row in 0..grid.rows {
        // Byte offsets are not columns. `row_text` builds a String from chars
        // and drops continuation cells, so the two diverge the moment a row
        // holds a multi-byte glyph — and `▌`, `·`, `…` are three bytes each
        // and in every frame. Reporting the byte offset put the magenta
        // outline on a cell that had not failed.
        let (text, columns) = row_with_columns(grid, row);
        let bytes = text.as_bytes();
        let column_of = |at: usize| columns.get(text[..at].chars().count()).copied().unwrap_or(0);
        let starts_word = |at: usize| at == 0 || !bytes[at - 1].is_ascii_alphanumeric();

        for form in CONTRACTIONS {
            // Every occurrence, not the first: `find` stopped at the `our` in
            // `your`, the guard rejected it, and the genuine one later in the
            // same row was never looked at.
            for (at, _) in text.match_indices(form) {
                if starts_word(at) {
                    report.violations.push(Violation {
                        gate: "content".into(),
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
                    gate: "content".into(),
                    row,
                    col: column_of(at),
                    detail: "first person \"I\" — the agent is written about in the third person".into(),
                });
            }
        }

        // A contraction anywhere. `'s` and `'d` carry a trailing space so a
        // possessive at the end of a row is not reported as a contraction it
        // is not — and a path like `~/proj` has no apostrophe to begin with.
        for form in ENCLITICS {
            for (at, _) in text.match_indices(form) {
                // The apostrophe has to be inside a word: `'s ` matching the
                // start of a row would be a quotation mark, not a clitic.
                if at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
                    report.violations.push(Violation {
                        gate: "content".into(),
                        row,
                        col: column_of(at),
                        detail: format!("contraction {form:?} — the design system's copy uses none"),
                    });
                }
            }
        }

        // A singular count with a plural pronoun. Narrow on purpose: it fires
        // only when the row literally opens a count at `1 ` and later refers
        // back with a plural, which is the shape a `{n} line{s}` format
        // string produces when its pronoun was not pluralised too.
        if let Some(rest) = singular_count(&text) {
            for plural in [" them", " they ", " these ", " those "] {
                if let Some(at) = rest.find(plural) {
                    let offset = text.len() - rest.len() + at;
                    report.violations.push(Violation {
                        gate: "content".into(),
                        row,
                        col: column_of(offset),
                        detail: format!("a count of 1 followed by {:?} — the noun pluralises and the pronoun does not", plural.trim()),
                    });
                    break;
                }
            }
        }
    }
}

/// The text after a leading `1 <word>` count, or `None` when the row does not
/// open with one. Anchored at the row's first non-space so a `1` inside a
/// path, a version or an option number cannot trigger it.
fn singular_count(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("1 ")?;
    // The next token has to be a bare lowercase word — the noun being
    // counted. `1-5 to pick` and `1  Allow once` both fail here, which is
    // the point: an option number is not a count.
    let word = rest.split(' ').next()?;
    (!word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())).then_some(rest)
}

/// A row as text, plus the grid column each character came from.
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
