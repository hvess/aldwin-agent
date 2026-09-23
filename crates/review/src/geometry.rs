//! The three sizes, and the cell they are measured in.
//!
//! A size is stated in **cells** and only ever converted to pixels through a
//! cell that was measured on this machine (see `capture::measure_cell`). The
//! probe that preceded this crate assumed 8×18 for a cell that is 8×19, got a
//! window holding 34 rows instead of 36, and produced frames that were wrong
//! in every row while looking entirely correct. That is why nothing here
//! carries a pixel constant.

use std::fmt;
use std::str::FromStr;

/// One terminal cell, in pixels, as foot actually lays it out for the pinned
/// font. Measured per run; never hardcoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub w: u32,
    pub h: u32,
}

/// The three standard geometries, each carrying a different question.
///
/// * `Small` — 80×24, the universal floor. Chosen for **vertical** pressure,
///   not narrow width: at 24 rows `COMPOSER_MAX_ROWS = 10` takes 42% of the
///   frame and `ui::decision::clamp_panel` starts discarding rows.
/// * `Medium` — 104×32, a terminal the size of the design's window body (880px wide, 28 rows)
///   (`tokens/cells.css`). The only size with a reference to check against.
/// * `Large` — 200×50, a maximized terminal. Its job is the opposite of
///   small's: prove nothing stretches that shouldn't.
///
/// Widths near the mid-50s are deliberately avoided: `ui/decision.rs`
/// computes the option-detail column's survival from actual label and detail
/// widths, so a size on that boundary would flip with a one-word copy change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    pub fn cells(self) -> (u32, u32) {
        match self {
            Size::Small => (80, 24),
            Size::Medium => (104, 32),
            Size::Large => (200, 50),
        }
    }

    pub fn pixels(self, cell: Cell) -> (u32, u32) {
        let (cols, rows) = self.cells();
        (cols * cell.w, rows * cell.h)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
        })
    }
}

impl FromStr for Size {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "small" => Ok(Size::Small),
            "medium" => Ok(Size::Medium),
            "large" => Ok(Size::Large),
            other => Err(format!("unknown size {other:?} (small|medium|large)")),
        }
    }
}

/// Both themes are captured for every scene. Theme is read once at startup
/// from **global** config (`cli/src/bootstrap.rs`: `config.global_tui().theme`
/// through `Theme::from_config`), so the two themes are two seeded configs —
/// not two renders of one capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Light];
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        })
    }
}

impl FromStr for Theme {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dark" => Ok(Theme::Dark),
            "light" => Ok(Theme::Light),
            other => Err(format!("unknown theme {other:?} (dark|light)")),
        }
    }
}
