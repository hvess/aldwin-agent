//! `baseline.json`: this machine's measured cell, pinned font and toolchain,
//! and the design's recorded contradictions (`aldwin-review.md` Decision 7).
//!
//! A contradiction is a bug in the design system, not in the app: it is
//! removed when the design is fixed upstream or the decision reversed, never
//! when the app changes. [`Baseline::load`] rejects an entry missing either
//! half.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geometry::Cell;
use crate::{Error, Result};

/// The contents of `baseline.json`: the facts every screenshot is taken
/// against, and the design's recorded contradictions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// The file's own explanation of itself, kept so a save does not drop it.
    #[serde(rename = "//", default)]
    pub comment: String,
    /// The terminal cell in pixels, as `measure` found it for `font`.
    pub cell: Cell,
    /// The font the compositor's terminal is started with, pinned so every
    /// capture draws the same glyphs.
    pub font: String,
    /// `rustc --version` the results were produced on; stage 1 fails when it
    /// moves, since clippy's lints change between releases (Decision 10).
    #[serde(default)]
    pub toolchain: String,
    /// Where the design disagrees with itself, the HIG or a decision.
    #[serde(default)]
    pub contradictions: Vec<Contradiction>,
}

/// One place the design disagrees with itself, the HIG or a decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    /// A short kebab-case name for the entry, quoted wherever it is cited.
    pub id: String,
    /// What the design states in one place.
    pub design_says: String,
    /// What it states in another, what its own rendered frame shows, or what
    /// the HIG says against it.
    pub design_also_says: String,
    /// Which half the app follows, and why.
    pub app_follows: String,
    /// Glyphs this contradiction licenses, for the closed-table check.
    #[serde(default)]
    pub glyphs: String,
}

impl Baseline {
    /// Where the committed `baseline.json` lives, next to this crate's manifest.
    pub fn path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("baseline.json")
    }

    /// Reads the committed baseline from [`Baseline::path`].
    ///
    /// # Errors
    ///
    /// As [`Baseline::load_from`].
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    /// Reads a baseline from `path` and checks every contradiction states
    /// both halves.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not valid baseline JSON, or holds a
    /// contradiction with an empty `design_says` or `design_also_says`.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let baseline: Baseline = serde_json::from_str(&text)
            .map_err(|e| Error::Baseline(format!("{}: {e}", path.display())))?;

        for c in &baseline.contradictions {
            if c.design_says.trim().is_empty() || c.design_also_says.trim().is_empty() {
                return Err(Error::Baseline(format!(
                    "contradiction {:?} does not state both halves — an entry that cannot name what the design says twice is not a contradiction, it is an unfixed bug",
                    c.id
                )));
            }
        }
        Ok(baseline)
    }

    /// Writes this baseline back to [`Baseline::path`], pretty-printed.
    ///
    /// # Errors
    ///
    /// When serialising fails or the file cannot be written.
    pub fn save(&self) -> Result<()> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        std::fs::write(Self::path(), json)?;
        Ok(())
    }
}
