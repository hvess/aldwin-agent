//! The session contract.
//!
//! A session states what it is for before it runs anything: a **goal**, a
//! **focus set** — the regions the change is allowed to alter — and the
//! **scenes** the goal touches. Everything downstream is judged against those
//! three, so they exist as a file rather than as something said in
//! conversation: preflight checks the file is there, the regression gate reads
//! the focus set out of it, the judge is handed the goal from it, and the
//! report prints all of it back at the end.
//!
//! The scores live here too. They are produced by an agent that never sees the
//! code, and this is the handover: the judge writes them in, the report reads
//! them out, and the crate computes the verdict from both rather than letting
//! anything narrate its own result.

use std::io::Result;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub goal:     String,
    /// Scene names, optionally narrowed — `conversation,approval:10-14`.
    pub focus:    String,
    pub scenes:   Vec<String>,
    pub baseline: String,
    #[serde(default)]
    pub iterations: Vec<Iteration>,
    #[serde(default)]
    pub scores:     Vec<Score>,
    /// The regression gate's outcome, written by `run`.
    ///
    /// It lives here rather than in a frame's gate report because it is the
    /// one gate that compares two revisions instead of inspecting a frame —
    /// and because the verdict has to see it. It did not, at first: a session
    /// that moved snapshot regions outside its focus set still passed, which
    /// disarmed the one check standing between the loop and chasing its score
    /// by editing something it was not asked to touch.
    #[serde(default)]
    pub regression: Option<Regression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Regression {
    pub baseline:    String,
    pub moved:       usize,
    /// Sections the diff touched that no focus entry accounts for.
    pub unaccounted: Vec<String>,
}

/// What changed on one pass of the loop, in the agent's own words. It sits
/// beside the measurements in the report, never wrapped around them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Iteration {
    pub n:       u32,
    pub changed: String,
}

/// One frame, as judged. 0–100 each; the threshold is the **minimum** across
/// every frame in the run, never the mean.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub scene:     String,
    pub size:      String,
    pub theme:     String,
    pub spatial:   u32,
    pub component: u32,
    #[serde(default)]
    pub note:      String,
}

impl Session {
    pub fn path(run: &Path) -> PathBuf {
        run.join("session.json")
    }

    pub fn load(run: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(Self::path(run))?;
        serde_json::from_str(&text).map_err(|e| std::io::Error::other(format!("session.json: {e}")))
    }

    pub fn save(&self, run: &Path) -> Result<()> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        std::fs::write(Self::path(run), json)
    }

    /// The lowest of every judged number in the run. A mean would let six
    /// frames averaging ninety hide one frame at forty.
    pub fn minimum(&self) -> Option<u32> {
        self.scores.iter().flat_map(|s| [s.spatial, s.component]).min()
    }
}
