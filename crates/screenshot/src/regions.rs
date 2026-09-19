//! The region map — what part of the frame a cell belongs to.
//!
//! Derived from the frame, not declared in a file. The design system already
//! specifies both halves of it, so a list maintained by hand would be a
//! restatement that goes stale:
//!
//! * **Bands** come from the ground ladder. Since Turn 13 nothing inside a
//!   frame is stroked — a band is distinguished from its neighbour by its step
//!   on `--color-ground-0…6`, so a run of rows sharing one ground *is* a band,
//!   and the role that ground resolves to is what the band is.
//! * **Columns** come from the grid: a 3-cell margin, an 8-cell label column,
//!   a 2-cell gutter, so body text lands on cell 13. `CLAUDE.md` is explicit
//!   that there is deliberately no `--body-col` token — derive it, never
//!   restate it — which is exactly what happens here.
//!
//! One honest limit. Regions are inferred from the same rules the app is
//! supposed to be following, so a band drawn in the wrong place is a band this
//! map will confidently mislabel rather than notice. That circularity is why
//! the map is written into the run directory beside the frame: it is a
//! judgement the harness made, and it should be visible rather than silently
//! underpinning six gate results. The independent check stays the rendered
//! handoff at 120×36.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::design::Design;
use crate::geometry::Theme;
use crate::vt::{Color, Grid};

/// The grid, from `tokens/cells.css`. Derived downstream, never restated:
/// `BODY_COL` is a function of the three below and has no token of its own.
pub const MARGIN_X: u16 = 3;
pub const LABEL_COL_WIDTH: u16 = 8;
pub const LABEL_GUTTER: u16 = 2;
pub const BODY_COL: u16 = MARGIN_X + LABEL_COL_WIDTH + LABEL_GUTTER;

/// Where a cell sits across the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    /// The 3-cell gutter at either edge. A band's *ground* runs through it;
    /// its glyphs are not supposed to.
    Margin,
    Label,
    Gutter,
    Body,
}

/// A run of rows sharing one ground.
#[derive(Debug, Clone, Serialize)]
pub struct Band {
    pub from:   u16,
    pub to:     u16,
    /// The `--tui-*` role the ground resolves to, or `None` for a ground that
    /// is not in the palette at all — which the colour gate reports
    /// separately.
    pub role:   Option<String>,
    pub ground: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Map {
    pub bands: Vec<Band>,
    pub cols:  u16,
    pub rows:  u16,
}

impl Map {
    pub fn zone(&self, col: u16) -> Zone {
        if col < MARGIN_X || col + MARGIN_X >= self.cols {
            Zone::Margin
        } else if col < MARGIN_X + LABEL_COL_WIDTH {
            Zone::Label
        } else if col < BODY_COL {
            Zone::Gutter
        } else {
            Zone::Body
        }
    }

    pub fn band(&self, row: u16) -> Option<&Band> {
        self.bands.iter().find(|b| row >= b.from && row <= b.to)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}×{} cells\n", self.cols, self.rows));
        out.push_str(&format!(
            "margin {MARGIN_X}  label {}..{}  gutter {}..{}  body {BODY_COL}..{}\n\n",
            MARGIN_X,
            MARGIN_X + LABEL_COL_WIDTH,
            MARGIN_X + LABEL_COL_WIDTH,
            BODY_COL,
            self.cols - MARGIN_X
        ));
        for band in &self.bands {
            let name = band.role.clone().unwrap_or_else(|| "(not a palette ground)".into());
            out.push_str(&format!("rows {:>3}..{:<3} {:<22} {}\n", band.from, band.to, name, band.ground));
        }
        out
    }
}

/// Infer the bands of a frame.
///
/// A row's ground is the background its cells mostly carry — "mostly" because
/// a selection band, a diff line or an inline field paints a few cells of its
/// own without making the row a band of its own.
pub fn derive(grid: &Grid, theme: Theme, design: &Design) -> Map {
    let mut bands: Vec<Band> = Vec::new();

    for row in 0..grid.rows {
        let ground = dominant_background(grid, row);
        let hex = match ground {
            Some((r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            None => "(unset)".to_string(),
        };
        // Among aliases, prefer the name that describes a *surface*. Taking
        // the first alphabetically labelled the dark scrim `reverse-ink`,
        // because `--tui-reverse-ink` shares its value and sorts earlier —
        // and `role_pairing` then tests that label against the ink ramp, so an
        // alias one letter luckier would report a legitimate band as ink.
        let role = ground.and_then(|rgb| {
            let names = design.role_names(theme, rgb);
            names
                .iter()
                .find(|n| !Design::INK.contains(n))
                .or_else(|| names.first())
                .map(|s| s.to_string())
        });

        match bands.last_mut() {
            Some(last) if last.ground == hex => last.to = row,
            _ => bands.push(Band { from: row, to: row, role, ground: hex }),
        }
    }

    Map { bands, cols: grid.cols, rows: grid.rows }
}

fn dominant_background(grid: &Grid, row: u16) -> Option<(u8, u8, u8)> {
    let mut counts: BTreeMap<(u8, u8, u8), usize> = BTreeMap::new();
    for col in 0..grid.cols {
        if let Color::Rgb(r, g, b) = grid.get(row, col).effective().1 {
            *counts.entry((r, g, b)).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(rgb, _)| rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_column_is_derived_from_the_grid_not_restated() {
        // `CLAUDE.md`: "The grid is 3-cell margin, 8-cell label column, 2-cell
        // gutter, so body text lands on cell 13. There is deliberately no
        // --body-col token; derive it, never restate it."
        assert_eq!(BODY_COL, 13);
    }

    #[test]
    fn zones_cover_both_margins() {
        let map = Map { bands: vec![], cols: 120, rows: 36 };
        assert_eq!(map.zone(0), Zone::Margin);
        assert_eq!(map.zone(2), Zone::Margin);
        assert_eq!(map.zone(3), Zone::Label);
        assert_eq!(map.zone(12), Zone::Gutter);
        assert_eq!(map.zone(13), Zone::Body);
        // 117..119 are the right margin, so 116 is the last content column.
        assert_eq!(map.zone(116), Zone::Body);
        assert_eq!(map.zone(117), Zone::Margin);
        assert_eq!(map.zone(119), Zone::Margin);
    }
}
