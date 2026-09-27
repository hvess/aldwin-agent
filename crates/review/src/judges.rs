//! Which of stages 6 to 8's judges a change needs, and where each one stands.
//!
//! The judges are subagents and belong to the skill. What lives here is
//! everything about them that must not depend on anyone remembering: which of
//! them a change needs is read off the staged diff, never chosen, and a
//! verdict can be recorded only through [`Assignment::record`], never by hand
//! — so `run.json` cannot say a judge passed that the change never called
//! for.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// One of the three subagents that judge what no command can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Judge {
    /// Stage 6: the diff against `quality-gate`, the Key Constraints and the
    /// ADRs.
    Code,
    /// Stage 7: the diff against the `rust` skill's rules that no lint checks.
    Rust,
    /// Stage 8: the changed scenes' frames against the design system.
    Frames,
}

impl Judge {
    /// The three, in stage order.
    pub const ALL: [Judge; 3] = [Judge::Code, Judge::Rust, Judge::Frames];

    /// The stage this judge is.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Judge;
    /// assert_eq!(Judge::Rust.stage(), 7);
    /// ```
    pub fn stage(self) -> u8 {
        match self {
            Judge::Code => 6,
            Judge::Rust => 7,
            Judge::Frames => 8,
        }
    }

    /// The judge for a stage number, if that stage is a judge.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Judge;
    /// assert_eq!(Judge::from_stage(8), Some(Judge::Frames));
    /// assert_eq!(Judge::from_stage(5), None);
    /// ```
    pub fn from_stage(stage: u8) -> Option<Judge> {
        Judge::ALL.into_iter().find(|j| j.stage() == stage)
    }

    /// The judge's name as the report heads its section.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Judge;
    /// assert_eq!(Judge::Code.title(), "code judge");
    /// ```
    pub fn title(self) -> &'static str {
        match self {
            Judge::Code => "code judge",
            Judge::Rust => "Rust judge",
            Judge::Frames => "frames judge",
        }
    }
}

/// Where one judge stands in a run. One value rather than a "required" flag
/// beside an optional verdict, so a verdict cannot be attached to a judge
/// the change never called for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Standing {
    /// The staged diff does not call for this judge.
    NotRequired,
    /// Called for, and its verdict is not written yet.
    Pending,
    /// Its verdict is written and has no findings.
    Passed,
    /// Its verdict is written and has findings.
    Failed,
}

impl Standing {
    /// Whether the change calls for this judge at all.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Standing;
    /// assert!(Standing::Pending.required());
    /// assert!(!Standing::NotRequired.required());
    /// ```
    pub fn required(self) -> bool {
        self != Standing::NotRequired
    }
}

/// Whether one judge runs for this change, why, and what it concluded.
///
/// The fields are private so a standing moves only through [`new`] and
/// [`record`]: a caller that could set it directly could write a pass for a
/// judge the change never called for, which is the state [`Standing`]
/// exists to rule out.
///
/// [`new`]: Assignment::new
/// [`record`]: Assignment::record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    /// Which judge.
    judge: Judge,
    /// Where it stands.
    standing: Standing,
    /// The sentence that says why it runs or why it does not.
    reason: String,
}

impl Assignment {
    /// A judge the change calls for, still pending, or one it does not.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge, Standing};
    /// let rust = Assignment::new(Judge::Rust, false, "no Rust source changed");
    /// assert_eq!(rust.standing(), Standing::NotRequired);
    /// ```
    pub fn new(judge: Judge, required: bool, reason: impl Into<String>) -> Self {
        Assignment {
            judge,
            standing: if required {
                Standing::Pending
            } else {
                Standing::NotRequired
            },
            reason: reason.into(),
        }
    }

    /// Which judge this is.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge};
    /// let code = Assignment::new(Judge::Code, true, "a crate changed");
    /// assert_eq!(code.judge(), Judge::Code);
    /// ```
    pub fn judge(&self) -> Judge {
        self.judge
    }

    /// Where the judge stands.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge, Standing};
    /// let code = Assignment::new(Judge::Code, true, "a crate changed");
    /// assert_eq!(code.standing(), Standing::Pending);
    /// ```
    pub fn standing(&self) -> Standing {
        self.standing
    }

    /// The sentence that says why the judge runs or why it does not.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge};
    /// let code = Assignment::new(Judge::Code, true, "a crate changed");
    /// assert_eq!(code.reason(), "a crate changed");
    /// ```
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Records the judge's verdict: passed with no findings, failed with any.
    ///
    /// # Errors
    ///
    /// [`Error::Review`] when the change never called for this judge — there
    /// is no verdict to record for a judge that did not run.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge, Standing};
    /// let mut code = Assignment::new(Judge::Code, true, "a crate changed");
    /// code.record(true)?;
    /// assert_eq!(code.standing(), Standing::Passed);
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn record(&mut self, passed: bool) -> Result<()> {
        if !self.standing.required() {
            return Err(Error::Review(format!(
                "stage {} was not required for this change, so it has no verdict to record",
                self.judge.stage()
            )));
        }
        self.standing = if passed {
            Standing::Passed
        } else {
            Standing::Failed
        };
        Ok(())
    }
}

/// What one run of the loop knows about itself, kept beside its report as
/// `run.json` so `judge` can find it and, once every required judge has
/// passed, copied into the pass record `gate` looks for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    /// The staged tree this run reviewed, from `git write-tree`.
    pub tree: String,
    /// Whether stages 1 to 5 all passed.
    pub stages_passed: bool,
    /// The three judges and where each one stands.
    pub assignments: Vec<Assignment>,
}

impl RunState {
    const FILE: &'static str = "run.json";

    /// Reads the state `review` left in `dir`.
    ///
    /// # Errors
    ///
    /// When `dir` holds no `run.json`, or it does not parse.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use aldwin_review::judges::RunState;
    /// let state = RunState::load(Path::new("target/review-frames/run-1790488849"))?;
    /// println!("reviewed tree {}", state.tree);
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn load(dir: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(dir.join(Self::FILE))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Writes the state into `dir`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::RunState;
    /// let dir = tempfile::tempdir()?;
    /// let state = RunState { tree: "4b825dc".into(), stages_passed: true, assignments: vec![] };
    /// state.save(dir.path())?;
    /// assert_eq!(RunState::load(dir.path())?.tree, "4b825dc");
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn save(&self, dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(dir.join(Self::FILE), text)?;
        Ok(())
    }

    /// Whether the run has passed as a whole: every deterministic stage, and
    /// every judge the change called for.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::RunState;
    /// let docs_only = RunState { tree: "t".into(), stages_passed: true, assignments: vec![] };
    /// assert!(docs_only.passed());
    /// ```
    pub fn passed(&self) -> bool {
        self.stages_passed
            && self
                .assignments
                .iter()
                .all(|a| matches!(a.standing, Standing::NotRequired | Standing::Passed))
    }
}

/// Which judges the change calls for, read off what it touches, and the
/// scenes stage 8 is to look at.
///
/// `catalogue` is capture's list of scenes. It is not the snapshot's: the
/// snapshot draws scenes capture has no script for, and a change that moves
/// only those has nothing stage 8 can look at — which is said in the reason
/// rather than passed over.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
/// use aldwin_review::judges::assign;
/// let paths = vec!["crates/tui/src/app.rs".to_string()];
/// let changed = BTreeSet::from(["launch".to_string()]);
/// let (assignments, scenes) = assign(&paths, &changed, &["launch"]);
/// assert!(assignments.iter().all(|a| a.standing().required()));
/// assert_eq!(scenes, ["launch"]);
/// ```
pub fn assign(
    paths: &[String],
    changed: &BTreeSet<String>,
    catalogue: &[&str],
) -> (Vec<Assignment>, Vec<String>) {
    let crate_changed = paths
        .iter()
        .any(|p| p.starts_with("crates/") || p == "Cargo.toml" || p == "Cargo.lock");
    let rust_changed = paths.iter().any(|p| p.ends_with(".rs"));
    let captured: Vec<String> = changed
        .iter()
        .filter(|s| catalogue.contains(&s.as_str()))
        .cloned()
        .collect();
    let uncaptured: Vec<&str> = changed
        .iter()
        .map(String::as_str)
        .filter(|s| !catalogue.contains(s))
        .collect();

    let frames_reason = match (captured.is_empty(), uncaptured.is_empty()) {
        (true, true) => "no scene's snapshot changed".to_string(),
        (true, false) => format!(
            "the snapshot changed only in scenes capture does not draw ({}), so there are no frames to judge",
            uncaptured.join(", ")
        ),
        (false, _) => format!("the snapshot changed in {}", captured.join(", ")),
    };
    let assignments = vec![
        Assignment::new(
            Judge::Code,
            crate_changed,
            if crate_changed {
                "a crate or the workspace manifest changed"
            } else {
                "no crate and no workspace manifest changed"
            },
        ),
        Assignment::new(
            Judge::Rust,
            rust_changed,
            if rust_changed {
                "Rust source changed"
            } else {
                "no Rust source changed"
            },
        ),
        Assignment::new(Judge::Frames, !captured.is_empty(), frames_reason),
    ];
    (assignments, captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::SNAPSHOT;

    fn required(assignments: &[Assignment]) -> Vec<Judge> {
        assignments
            .iter()
            .filter(|a| a.standing().required())
            .map(Assignment::judge)
            .collect()
    }

    #[test]
    fn a_docs_only_change_calls_for_no_judge() {
        let paths = vec![".claude/spec/aldwin-review.md".to_string()];
        let (assignments, scenes) = assign(&paths, &BTreeSet::new(), &["launch"]);
        assert!(required(&assignments).is_empty());
        assert!(scenes.is_empty());
    }

    #[test]
    fn a_manifest_change_calls_for_the_code_judge_but_not_the_rust_one() {
        let paths = vec!["Cargo.toml".to_string()];
        let (assignments, _) = assign(&paths, &BTreeSet::new(), &["launch"]);
        assert_eq!(required(&assignments), vec![Judge::Code]);
    }

    #[test]
    fn frames_are_judged_only_for_changed_scenes_capture_can_draw() {
        let paths = vec!["crates/tui/src/app.rs".to_string(), SNAPSHOT.to_string()];
        let changed = BTreeSet::from(["launch".to_string(), "working".to_string()]);
        let (assignments, scenes) = assign(&paths, &changed, &["launch", "plan"]);
        assert_eq!(
            required(&assignments),
            vec![Judge::Code, Judge::Rust, Judge::Frames]
        );
        assert_eq!(scenes, vec!["launch".to_string()]);

        let only_uncaptured = BTreeSet::from(["working".to_string()]);
        let (assignments, scenes) = assign(&paths, &only_uncaptured, &["launch"]);
        assert!(scenes.is_empty());
        let frames = &assignments[2];
        assert_eq!(frames.standing(), Standing::NotRequired);
        assert!(frames.reason().contains("working"), "{}", frames.reason());
    }

    #[test]
    fn a_run_passes_only_when_every_required_judge_has_passed() {
        let (assignments, _) = assign(
            &["crates/core/src/lib.rs".to_string()],
            &BTreeSet::new(),
            &[],
        );
        let mut state = RunState {
            tree: "t".into(),
            stages_passed: true,
            assignments,
        };
        assert!(!state.passed(), "judges pending");
        for a in &mut state.assignments {
            if a.standing().required() {
                a.record(true).unwrap();
            }
        }
        assert!(state.passed());
        state.stages_passed = false;
        assert!(!state.passed());
    }

    /// The flag-and-option pair this replaced could carry a pass for a judge
    /// the change never called for.
    #[test]
    fn a_verdict_cannot_be_recorded_for_a_judge_that_was_not_required() {
        let mut frames = Assignment::new(Judge::Frames, false, "no scene's snapshot changed");
        assert!(matches!(frames.record(true), Err(Error::Review(_))));
        assert_eq!(frames.standing(), Standing::NotRequired);
    }
}
