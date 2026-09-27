//! The baseline: the measured cell, the pinned font, and the design's own
//! contradictions.
//!
//! The first two are facts about this machine. The third is the honest part.
//!
//! The design's prose, its tokens and its frames state one thing in one place
//! and another elsewhere, and where the developer has decided against the
//! design — or where the design disagrees with Apple's Human Interface
//! Guidelines, the usability reference the `ux` skill names — the app follows
//! one half. The loop cannot be run against such a reference without
//! somewhere to record which. **Each entry here is a bug in the design
//! system, not in the app** — it is removed when the design is fixed
//! upstream or the decision reversed, not when the app changes.
//!
//! An entry with no `design_says` and `design_also_says` is not a
//! contradiction, it is an unfixed bug wearing a costume, and [`Baseline::load`]
//! rejects the file rather than let one through.

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
    /// `rustc --version` this baseline's results were produced on. Stage 1
    /// fails when the toolchain moves: clippy's lint set changes between
    /// releases, so a stage 2 failure on untouched code is a real
    /// possibility and deserves to be named rather than puzzled over.
    #[serde(default)]
    pub toolchain: String,
    /// Where the design disagrees with itself, the HIG or a decision.
    #[serde(default)]
    pub contradictions: Vec<Contradiction>,
}

/// One place the design system disagrees with itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    /// A short kebab-case name for the entry, quoted wherever it is cited.
    pub id: String,
    /// What the design states in one place.
    pub design_says: String,
    /// What it states in another, what its own rendered frame shows, or what
    /// the HIG says against it.
    pub design_also_says: String,
    /// Which half the app follows, and why that is the defensible one.
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
