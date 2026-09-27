//! The three sizes, and the cell they are measured in.
//!
//! A size is in cells, converted to pixels only through a cell measured on
//! this machine (`capture::measure_cell`). Never add a pixel constant: an
//! assumed cell one pixel off yields plausible frames with the wrong row count.

use std::fmt;
use std::str::FromStr;

/// One terminal cell, in pixels, as foot lays it out for the pinned font.
/// Measured per run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

/// The three standard geometries.
///
/// * `Small` — 80×24, the floor, for vertical pressure: the composer's
///   `COMPOSER_MAX_ROWS` (10) takes 42% of it.
/// * `Medium` — 104×32, the design's window body (880px wide, 28 rows,
///   `tokens/cells.css`); the only size with a reference to check against.
/// * `Large` — 200×50, a maximized terminal, to show nothing stretches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 80×24, the floor.
    Small,
    /// 104×32, the design's window body.
    Medium,
    /// 200×50, a maximized terminal.
    Large,
}

impl Size {
    /// Every size, in the order a run captures them.
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    /// Columns and rows.
    pub fn cells(self) -> (u32, u32) {
        match self {
            Size::Small => (80, 24),
            Size::Medium => (104, 32),
            Size::Large => (200, 50),
        }
    }

    /// Width and height in pixels, through a measured `cell`.
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

/// A theme; every scene is captured in both. The app reads it at startup from
/// global config (`cli/src/bootstrap.rs`), so each theme is its own seeded
/// config and its own capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// The dark palette, the app's default.
    Dark,
    /// The light palette, which inverts the ladder of grounds.
    Light,
}

impl Theme {
    /// Both themes, in the order a run captures them.
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
