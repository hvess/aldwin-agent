//! The baseline: the measured cell, the pinned font, and the design's own
//! contradictions.
//!
//! The first two are facts about this machine. The third is the honest part.
//!
//! `HANDOFF.md` is prose written across fifteen turns over a token layer that
//! was rebuilt twice underneath it, and in places it states one thing and its
//! own CSS states another. Stage 3 cannot be run against a reference that
//! contradicts itself without somewhere to record where. **Each entry here is
//! a bug in the design system, not in the app** — it is removed when the
//! design is fixed upstream, not when the app changes.
//!
//! An entry with no `design_says` and `design_also_says` is not a
//! contradiction, it is an unfixed bug wearing a costume, and [`Baseline::load`]
//! rejects the file rather than let one through.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geometry::Cell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    #[serde(rename = "//", default)]
    pub comment:        String,
    pub cell:           Cell,
    pub font:           String,
    #[serde(default)]
    pub contradictions: Vec<Contradiction>,
}

/// One place the design system disagrees with itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub id:               String,
    /// What the design states in one place.
    pub design_says:      String,
    /// What it states in another, or what its own rendered frame shows.
    pub design_also_says: String,
    /// Which half the app follows, and why that is the defensible one.
    pub app_follows:      String,
    /// Glyphs this contradiction licenses, for the closed-table check.
    #[serde(default)]
    pub glyphs:           String,
}

impl Baseline {
    pub fn path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("baseline.json")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let baseline: Baseline =
            serde_json::from_str(&text).map_err(|e| Error::new(ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;

        for c in &baseline.contradictions {
            if c.design_says.trim().is_empty() || c.design_also_says.trim().is_empty() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "contradiction {:?} does not state both halves — an entry that cannot name what the design says twice is not a contradiction, it is an unfixed bug",
                        c.id
                    ),
                ));
            }
        }
        Ok(baseline)
    }

    pub fn save(&self) -> Result<()> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        std::fs::write(Self::path(), json)
    }
}
