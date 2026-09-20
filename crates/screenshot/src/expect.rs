//! The design's stated geometry, as assertions the harness executes.
//!
//! # Why this exists
//!
//! For five runs the exit condition was "minimum score ≥ 90 across 72
//! frames", where the score was a 0–100 judgement from a fresh blind model
//! with no anchors. That quantity is not calibrated and never was: on
//! `run-1789850385` one judge gave `empty` 64 and `first_run` 86 on
//! substantially the same finding set, and on `run-1789829088` the *means*
//! rose while the minimum fell 58 → 48. Gating on the minimum of an
//! uncalibrated statistic is gating on the harshest reading of the harshest
//! judge, which is why the loop has never once exited.
//!
//! `HANDOFF.md`'s screen sections are not prose about a design — they are
//! bulleted lists of assertions. `5a` states fifteen of them. Almost all are
//! checkable against the declared grid the harness already parses. Moving
//! them here makes the check reproducible bit-for-bit, makes two runs
//! comparable for the first time, and makes a new deviation arrive with its
//! `HANDOFF.md` citation already attached instead of waiting for a judge to
//! find it and a human to hunt down the line.
//!
//! # What it deliberately does not do
//!
//! An assertion suite checks conformance to what somebody **wrote down**. It
//! cannot find what nobody enumerated — on `run-1789850385` that was Class A
//! items 37 and 42, neither of which any table here would have contained. So
//! this does not retire the blind judge; it demotes it to advisory and takes
//! the numbers off the gate. See `.claude/skills/screenshot/SKILL.md`.
//!
//! # Why the tables are Rust and not a data file
//!
//! Three reasons, in order of weight. The vocabulary of [`Check`] is closed
//! by the enum, so an assertion the checker cannot execute cannot be written
//! at all. A malformed table is a compile error rather than a run that dies
//! after three minutes of capture. And no new dependency: the workspace has
//! no TOML parser, and adding one to express a fifty-line table is the wrong
//! trade.
//!
//! Every number these tables use is **read from `tokens/cells.css`** through
//! [`Design::cells`], never written here. A citation names the `HANDOFF.md`
//! line that states the rule; the arithmetic comes from the tokens.

use serde::{Deserialize, Serialize};

use crate::design::Design;
use crate::geometry::Size;
use crate::regions::Map;
use crate::vt::{Color, Grid};

/// Where a screen's rows are counted from.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    /// The first row whose text has `text` starting exactly at `col`.
    Row { col: Col, text: &'static str },
    /// The first row of the first band whose ground resolves to this role.
    BandStart(&'static str),
}

/// A column, either a literal or — normally — a `cells.css` token, optionally
/// with other tokens added to it. Literals are a last resort and each one
/// that appears below says why it is not a token.
#[derive(Debug, Clone, Copy)]
pub enum Col {
    At(u16),
    Token(&'static str),
    Sum(&'static [&'static str]),
}

impl Col {
    fn resolve(self, design: &Design) -> Option<u16> {
        match self {
            Col::At(n) => Some(n),
            Col::Token(name) => design.cells.get(name).copied(),
            Col::Sum(names) => names.iter().map(|n| design.cells.get(*n).copied()).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Check {
    /// No glyphs anywhere on the row.
    Blank,
    /// The row has this text starting exactly at this column.
    TextAt { col: Col, text: &'static str },
    /// The row has a glyph at this column, whatever it is.
    GlyphAt { col: Col },
    /// The row's last glyph ends flush against the 3-cell right margin.
    RightFlush,
    /// The declared foreground at this column resolves to this `--tui-` role.
    InkAt { col: Col, role: &'static str },
    /// The band behind this row resolves to this ground role.
    BandIs(&'static str),
    /// `first` and `second` both appear on the row, in that order, with
    /// exactly `cells` blank cells between them.
    GapBetween { first: &'static str, second: &'static str, cells: Col },
    /// `first` and `second` both appear on the row, in that order, parted by
    /// exactly this string. The design's within-group separator is ` · `; a
    /// run of spaces where it belongs is a deviation even when the columns
    /// happen to look tidy.
    PartedBy { first: &'static str, second: &'static str, by: &'static str },
}

#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// Rows below the anchor; negative is above it.
    pub at:    i32,
    pub check: Check,
    pub cite:  &'static str,
}

/// Which frame sizes a screen's assertions are checked at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Applies {
    /// Every size. Column landmarks are size-independent: the grid is the
    /// same 3/8/2 scheme at 80 columns as at 200.
    EverySize,
    /// The 120×36 design frame only. Row *counts* are not size-independent —
    /// at 80×24 an 18-row panel would leave six rows for the whole
    /// conversation — and the design specifies one frame, so asserting an
    /// absolute height anywhere else would be asserting something the design
    /// never said.
    DesignFrame,
}

pub struct Screen {
    /// The design system's own name for it, as `HANDOFF.md` heads the section.
    pub name:    &'static str,
    pub scenes:  &'static [&'static str],
    pub applies: Applies,
    pub anchor:  Anchor,
    /// The construct occupies exactly this many rows, from the anchor to the
    /// frame's last row inclusive, where the count comes from this
    /// `cells.css` token.
    pub height:  Option<(&'static str, &'static str)>,
    pub rules:   &'static [Rule],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub screen: String,
    pub rule:   String,
    pub cite:   String,
    pub row:    Option<u16>,
    pub detail: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub checked:  usize,
    pub failures: Vec<Failure>,
    /// Screens whose anchor was not on this frame, so none of their rules
    /// ran. Reported rather than silently skipped: a screen that stops being
    /// anchored is a screen that stopped being checked.
    pub skipped:  Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// `4a` — the session screen. Its top bar and grid are on every frame the
/// harness captures, which is why this table is `EverySize` and anchored on
/// the brand rather than on a band.
static SESSION: Screen = Screen {
    name:    "4a session",
    scenes:  &[],
    applies: Applies::EverySize,
    anchor:  Anchor::Row { col: Col::Token("margin-x"), text: "mjolnir" },
    height:  None,
    rules:   &[
        Rule {
            at:    0,
            check: Check::InkAt { col: Col::Token("margin-x"), role: "text" },
            cite:  "HANDOFF.md:254 — `mjolnir` in primary text on the 3-cell margin",
        },
        Rule {
            at:    0,
            check: Check::BandIs("bar"),
            cite:  "HANDOFF.md:253 — top bar on the chrome tone `--t-bar`",
        },
        // The working directory lands on the body column. `HANDOFF.md:254`
        // says "6 cells" and `HANDOFF.md:389` says "three spaces"; the
        // reference contradicts itself and the frames follow `14d`, which is
        // also what the 3/8/2 grid derives. Recorded as conformance Class C 8
        // and asserted here in the form the grid produces.
        Rule {
            at:    0,
            check: Check::InkAt { col: Col::Sum(&["margin-x", "label-col", "label-gutter"]), role: "quiet" },
            cite:  "HANDOFF.md:389 — cwd in `--t-quiet` on the body column (and Class C 8)",
        },
    ],
};

/// `5a` — the permission prompt. Thirty of the catalogue's frames.
static PERMISSION: Screen = Screen {
    name:    "5a permission",
    scenes:  &["prompt", "prompt_path", "prompt_scoped", "approval", "approval_large"],
    applies: Applies::DesignFrame,
    anchor:  Anchor::BandStart("panel-title"),
    height:  Some(("panel-permission-h", "HANDOFF.md:271 + cells.css --panel-permission-h — the panel is 18 rows")),
    rules:   &[
        Rule {
            at:    0,
            check: Check::TextAt { col: Col::Token("margin-x"), text: "permission" },
            cite:  "HANDOFF.md:273 — `permission` in accent text on the 3-cell margin, no glyph",
        },
        Rule {
            at:    0,
            check: Check::InkAt { col: Col::Token("margin-x"), role: "accent-text" },
            cite:  "HANDOFF.md:273 — the title in accent text",
        },
        Rule {
            at:    0,
            check: Check::RightFlush,
            cite:  "HANDOFF.md:273 — right-aligned the tool name",
        },
        Rule {
            at:    1,
            check: Check::Blank,
            cite:  "HANDOFF.md:274 — blank row, then the sentence",
        },
        Rule {
            at:    3,
            check: Check::Blank,
            cite:  "HANDOFF.md:275 — blank row, then the command block",
        },
    ],
};

/// `14d` — the empty session, the resting state.
static EMPTY: Screen = Screen {
    name:    "14d empty",
    scenes:  &["empty"],
    applies: Applies::EverySize,
    anchor:  Anchor::Row { col: Col::Token("margin-x"), text: "in" },
    height:  None,
    rules:   &[
        Rule {
            at:    0,
            check: Check::InkAt { col: Col::Token("margin-x"), role: "label" },
            cite:  "HANDOFF.md:395 — three facts on the ordinary 8-cell label column",
        },
        Rule {
            at:    1,
            check: Check::TextAt { col: Col::Token("margin-x"), text: "provider" },
            cite:  "HANDOFF.md:396 — `in`, `provider`, `access`, in that order",
        },
        Rule {
            at:    2,
            check: Check::TextAt { col: Col::Token("margin-x"), text: "access" },
            cite:  "HANDOFF.md:396 — `in`, `provider`, `access`, in that order",
        },
        // Three permission states rather than `14d`'s single tier word is a
        // deliberate departure with its reasoning in `transcript.rs` — see
        // the conformance spec's "What a judge will raise again" item 4. What
        // is *not* decided is how they are parted: these are three facts in
        // one group, and the design parts those with ` · `.
        Rule {
            at:    2,
            check: Check::PartedBy { first: "read:deny", second: "shell:deny", by: " · " },
            cite:  "HANDOFF.md:396 / IMPORT.md — within-group facts are parted by ` · `",
        },
        Rule {
            at:    4,
            check: Check::TextAt { col: Col::Token("margin-x"), text: "Ask for a change, or / for commands." },
            cite:  "HANDOFF.md:397 — the guidance line, verbatim",
        },
    ],
};

/// `5d` — first run. The step spine is the one place the design states a
/// column built from three tokens, so it is the strongest test that
/// `cells.css` is being derived rather than guessed.
static FIRST_RUN: Screen = Screen {
    name:    "5d first run",
    scenes:  &["first_run"],
    applies: Applies::EverySize,
    anchor:  Anchor::Row { col: Col::Sum(&["margin-x", "label-col", "label-gutter"]), text: "provider" },
    height:  None,
    rules:   &[
        // The step's name sits at margin + --step-mark-col, which resolves to
        // 3 + 10 = 13. That it coincides with the body column is a property
        // of the tokens, not a coincidence to hard-code.
        Rule {
            at:    0,
            check: Check::TextAt { col: Col::Sum(&["margin-x", "step-mark-col"]), text: "provider" },
            cite:  "HANDOFF.md:322 — step name in the label column, content at --step-content-col",
        },
        Rule {
            at:    0,
            check: Check::InkAt { col: Col::Sum(&["margin-x", "step-mark-col"]), role: "speaker-you" },
            cite:  "HANDOFF.md:324 — the active step's label in `--t-accent-you`",
        },
    ],
};

static SCREENS: &[&Screen] = &[&SESSION, &PERMISSION, &EMPTY, &FIRST_RUN];

// ---------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------

/// A row as text, with the grid column each character came from.
pub fn row_with_columns(grid: &Grid, row: u16) -> (String, Vec<u16>) {
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

/// The column a character index sits in. Not the same as the index: `▌`, `·`
/// and `…` are three bytes and one cell each, and a wide glyph is one
/// character and two cells.
fn column_of(columns: &[u16], char_index: usize) -> Option<u16> {
    columns.get(char_index).copied()
}

fn text_starts_at(grid: &Grid, row: u16, col: u16, text: &str) -> bool {
    let (line, columns) = row_with_columns(grid, row);
    let Some(index) = columns.iter().position(|c| *c == col) else { return false };
    line.chars().skip(index).take(text.chars().count()).eq(text.chars())
}

pub fn check(grid: &Grid, map: &Map, design: &Design, scene: &str, size: Size) -> Outcome {
    let mut outcome = Outcome::default();
    for screen in SCREENS {
        if !screen.scenes.is_empty() && !screen.scenes.contains(&scene) {
            continue;
        }
        if screen.applies == Applies::DesignFrame && size != Size::Medium {
            continue;
        }
        check_screen(screen, grid, map, design, &mut outcome);
    }
    turn_breaks(grid, map, &mut outcome);
    outcome
}

fn find_anchor(anchor: Anchor, grid: &Grid, map: &Map, design: &Design) -> Option<u16> {
    match anchor {
        Anchor::Row { col, text } => {
            let col = col.resolve(design)?;
            (0..grid.rows).find(|row| text_starts_at(grid, *row, col, text))
        }
        Anchor::BandStart(role) => map.bands.iter().find(|b| b.role.as_deref() == Some(role)).map(|b| b.from),
    }
}

fn check_screen(screen: &Screen, grid: &Grid, map: &Map, design: &Design, outcome: &mut Outcome) {
    let Some(anchor) = find_anchor(screen.anchor, grid, map, design) else {
        outcome.skipped.push(screen.name.to_string());
        return;
    };

    if let Some((token, cite)) = screen.height {
        outcome.checked += 1;
        if let Some(expected) = design.cells.get(token).copied() {
            let actual = grid.rows - anchor;
            if actual != expected {
                outcome.failures.push(Failure {
                    screen: screen.name.into(),
                    rule:   format!("height == --{token} ({expected} rows)"),
                    cite:   cite.into(),
                    row:    Some(anchor),
                    detail: format!("the construct runs rows {anchor}..{} — {actual} rows, not {expected}", grid.rows - 1),
                });
            }
        }
    }

    for rule in screen.rules {
        let row = anchor as i32 + rule.at;
        if row < 0 || row >= grid.rows as i32 {
            continue;
        }
        outcome.checked += 1;
        if let Some(detail) = failed(rule.check, grid, map, design, row as u16) {
            outcome.failures.push(Failure {
                screen: screen.name.into(),
                rule:   describe(rule.check, design),
                cite:   rule.cite.into(),
                row:    Some(row as u16),
                detail,
            });
        }
    }
}

fn describe(check: Check, design: &Design) -> String {
    let col = |c: Col| c.resolve(design).map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    match check {
        Check::Blank => "the row is blank".into(),
        Check::TextAt { col: c, text } => format!("{text:?} starts at cell {}", col(c)),
        Check::GlyphAt { col: c } => format!("a glyph at cell {}", col(c)),
        Check::RightFlush => "the last glyph is flush to the 3-cell right margin".into(),
        Check::InkAt { col: c, role } => format!("the ink at cell {} is --tui-{role}", col(c)),
        Check::BandIs(role) => format!("the band is --tui-{role}"),
        Check::GapBetween { first, second, cells } => format!("{first:?} and {second:?} are {} cells apart", col(cells)),
        Check::PartedBy { first, second, by } => format!("{first:?} and {second:?} are parted by {by:?}"),
    }
}

/// `None` when the rule holds; the measured reality when it does not.
fn failed(check: Check, grid: &Grid, map: &Map, design: &Design, row: u16) -> Option<String> {
    let (line, columns) = row_with_columns(grid, row);
    match check {
        Check::Blank => line.chars().any(|c| c != ' ').then(|| format!("the row reads {:?}", line.trim())),

        Check::TextAt { col, text } => {
            let col = col.resolve(design)?;
            (!text_starts_at(grid, row, col, text)).then(|| {
                let at = columns.iter().position(|c| *c == col).map(|i| line.chars().skip(i).take(text.chars().count()).collect::<String>());
                match at {
                    Some(found) => format!("cell {col} reads {found:?}, not {text:?}"),
                    None => format!("the row has no cell {col}; it reads {:?}", line.trim()),
                }
            })
        }

        Check::GlyphAt { col } => {
            let col = col.resolve(design)?;
            (grid.get(row, col).ch == ' ').then(|| format!("cell {col} is blank"))
        }

        Check::RightFlush => {
            let last = columns.last().copied()?;
            let expected = grid.cols - crate::regions::MARGIN_X - 1;
            (last != expected).then(|| format!("the last glyph is at cell {last}, not {expected}"))
        }

        Check::InkAt { col, role } => {
            let col = col.resolve(design)?;
            let want = design.roles(crate::geometry::Theme::Dark).get(role).copied();
            // Compare by role name rather than by value, so the check reads
            // the same in both themes and a role that shares a ramp step with
            // another is reported as the pair it is.
            match grid.get(row, col).effective().0 {
                Color::Rgb(r, g, b) => {
                    let names = design_role_names(design, (r, g, b), map);
                    (!names.contains(&role)).then(|| {
                        let found = if names.is_empty() { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("--tui-{}", names.join("/")) };
                        format!("cell {col} is {found}, not --tui-{role}")
                    })
                }
                other => {
                    let _ = want;
                    Some(format!("cell {col} is {other:?}, not a palette role"))
                }
            }
        }

        Check::BandIs(role) => {
            let band = map.band(row)?;
            (band.role.as_deref() != Some(role)).then(|| {
                format!("the band is {}", band.role.clone().unwrap_or_else(|| band.ground.clone()))
            })
        }

        Check::GapBetween { first, second, cells } => {
            let want = cells.resolve(design)?;
            let (a, b) = (line.find(first)?, line.find(second)?);
            let end = column_of(&columns, line[..a].chars().count() + first.chars().count())?;
            let start = column_of(&columns, line[..b].chars().count())?;
            (start - end != want).then(|| format!("{} cells apart, not {want}", start - end))
        }

        Check::PartedBy { first, second, by } => {
            let a = line.find(first)?;
            let b = line.find(second)?;
            if b < a {
                return Some(format!("{second:?} comes before {first:?}"));
            }
            let between = &line[a + first.len()..b];
            (between != by).then(|| format!("parted by {between:?}, not {by:?}"))
        }
    }
}

/// Role names for a colour, ignoring the theme by checking both — the checker
/// runs once per frame and a frame is one theme, but the tables are written
/// once for both and a role's *name* is what they assert.
fn design_role_names<'a>(design: &'a Design, rgb: (u8, u8, u8), _map: &Map) -> Vec<&'a str> {
    let dark = design.role_names(crate::geometry::Theme::Dark, rgb);
    if !dark.is_empty() {
        return dark;
    }
    design.role_names(crate::geometry::Theme::Light, rgb)
}

/// The turn break carries its own padding, and gives it up only under
/// pressure.
///
/// `HANDOFF.md:258` — "Turns are separated by a blank row, a 1-row band of
/// the composer tone, and another blank row." The app drops those two blanks
/// when the viewport is too short to hold them, which is deliberate and
/// documented twice (`transcript::Transcript::viewport`,
/// `decision::panel_lines`): spacing is cheaper than structure.
///
/// What is checked here is the **trigger**, not the rule. If the frame still
/// has a spare blank row in the same band — a row carrying no glyphs that is
/// not itself part of a separator — then the pressure the degradation exists
/// for was not there, and the separator gave up its padding for nothing. That
/// is conformance Class A item 35, and it is exactly the shape a judge cannot
/// check by eye because it requires counting the whole band's spare rows.
fn turn_breaks(grid: &Grid, map: &Map, outcome: &mut Outcome) {
    let has_glyphs = |row: u16| (0..grid.cols).any(|c| grid.get(row, c).ch != ' ');

    for band in map.bands.iter().filter(|b| b.role.as_deref() == Some("break")) {
        outcome.checked += 1;
        let above = band.from.checked_sub(1);
        let below = (band.to + 1 < grid.rows).then_some(band.to + 1);
        let padded = above.map(|r| !has_glyphs(r)).unwrap_or(false) && below.map(|r| !has_glyphs(r)).unwrap_or(false);
        if padded {
            continue;
        }

        // Spare rows are the blank ones the transcript left at its *top*,
        // above any content at all. Counting every blank ground row would be
        // wrong and was, in a first draft of this check: a blank row between
        // prose and a tool group is structure the design asks for
        // (`HANDOFF.md:259`), not slack, and counting those claimed five rows
        // were available at 80×24 when two were.
        let ground_rows: Vec<u16> = map
            .bands
            .iter()
            .filter(|b| b.role.as_deref() == Some("ground"))
            .flat_map(|b| b.from..=b.to)
            .collect();
        let spare = ground_rows.iter().take_while(|row| !has_glyphs(**row)).count();

        if spare >= 2 {
            outcome.failures.push(Failure {
                screen: "4a transcript".into(),
                rule:   "a turn break is blank / band / blank".into(),
                cite:   "HANDOFF.md:258 — a blank row, a 1-row band, another blank row".into(),
                row:    Some(band.from),
                detail: format!(
                    "the break at row {} has no padding, and the transcript opens with {spare} unused blank rows that would have paid for it",
                    band.from
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables are only as good as the tokens they resolve through, and a
    /// silently-missing token would turn every rule using it into a skip.
    #[test]
    fn every_token_the_tables_use_resolves() {
        let design = Design::load().expect("design system loads");
        for token in ["margin-x", "label-col", "label-gutter", "step-mark-col", "step-content-col", "panel-permission-h", "group-gap"] {
            assert!(design.cells.contains_key(token), "cells.css has no --{token}");
        }
        assert_eq!(design.cells.get("margin-x"), Some(&3));
        assert_eq!(design.cells.get("label-col"), Some(&8));
        assert_eq!(design.cells.get("label-gutter"), Some(&2));
        assert_eq!(design.cells.get("group-gap"), Some(&6));
        assert_eq!(design.cells.get("panel-permission-h"), Some(&18));
        // The derived ones, which are the whole reason this is parsed rather
        // than restated: a sum of three tokens, one of which is itself a sum.
        assert_eq!(design.cells.get("step-mark-col"), Some(&10));
        assert_eq!(design.cells.get("step-content-col"), Some(&29));
        assert_eq!(design.cells.get("diff-code-col"), Some(&11));
    }

    /// Body text lands on cell 13 as a consequence of the three tokens, which
    /// is what `CLAUDE.md` forbids restating and what every table above
    /// derives instead.
    #[test]
    fn the_body_column_is_derived_not_declared() {
        let design = Design::load().expect("design system loads");
        assert!(!design.cells.contains_key("body-col"), "cells.css must not declare a --body-col");
        let derived = Col::Sum(&["margin-x", "label-col", "label-gutter"]).resolve(&design);
        assert_eq!(derived, Some(crate::regions::BODY_COL));
    }
}
