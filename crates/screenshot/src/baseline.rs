//! The baseline file — what is correct by decision.
//!
//! It holds three things, and the spec is explicit that they are different
//! questions on different clocks: the measured cell and pinned font (this
//! machine's), the **exceptions** (design deviations a gate must not report),
//! and the **masks** (cells too volatile to compare).
//!
//! An exception with no `authority` is not an exception. It is an unfixed bug
//! being silenced, and `Baseline::load` rejects the file rather than let one
//! through.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geometry::Cell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// The file's own header line. Carried through so rewriting the baseline
    /// does not quietly delete the sentence explaining what it is.
    #[serde(rename = "//", default)]
    pub comment:    String,
    pub cell:       Cell,
    pub font:       String,
    #[serde(default)]
    pub exceptions: Vec<Exception>,
    #[serde(default)]
    pub masks:      Vec<Mask>,
}

/// A design deviation that is correct by decision. `authority` cites the ADR
/// or spec entry that made it one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exception {
    pub gate:      String,
    pub scope:     String,
    pub authority: String,
    #[serde(default)]
    pub note:      String,
    /// The glyphs this exception allows, for a `breakages` entry. Prose says
    /// why; this is what the gate actually consults.
    #[serde(default)]
    pub glyphs:    String,
    /// The ink-on-ground pairs this exception allows, for a `contrast` entry,
    /// each written exactly as the gate reports it — `"--tui-dim on
    /// --tui-diff-box"`. A pair rather than a threshold on purpose: raising a
    /// floor would silence every adjacency at once, including the ones nobody
    /// has looked at, which is the suppression dump the `authority` rule
    /// exists to prevent.
    #[serde(default)]
    pub pairs:     Vec<String>,
}

/// Cells that legitimately differ between two captures of the same state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mask {
    pub scene:  String,
    pub reason: String,
    pub rows:   Vec<u32>,
    pub cols:   Vec<u32>,
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

        for e in &baseline.exceptions {
            if e.authority.trim().is_empty() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("exception for gate {:?} scope {:?} cites no authority — that is an unfixed bug, not an exception", e.gate, e.scope),
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
