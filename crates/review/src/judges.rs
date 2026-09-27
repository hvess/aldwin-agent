//! Which of stages 6 to 8's judges a change needs, and where each one stands.
//!
//! The judges themselves are subagents in the review skill. Which ones run is
//! read off the staged diff, never chosen, and a verdict is recorded only
//! through [`Assignment::record`], so `run.json` cannot hold a pass for a
//! judge the change never called for. A pass carries across runs on
//! unchanged inputs ([`RunState::carry_from`], Decision 17).

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::{Fingerprint, SNAPSHOT};
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

    /// The part of the staged diff this judge reads, as pathspecs; empty is
    /// the whole diff. Must include its sources and its prompt (the review
    /// skill), so a change to what it judges by voids a carried pass.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Judge;
    /// assert!(Judge::Code.reads().is_empty());
    /// assert!(Judge::Rust.reads().contains(&"*.rs"));
    /// ```
    pub fn reads(self) -> &'static [&'static str] {
        match self {
            Judge::Code => &[],
            Judge::Rust => &["*.rs", ".claude/skills/rust/", ".claude/skills/review/"],
            // All of the review crate: it captures the frames, and
            // `baseline.json` holds the settled contradictions.
            Judge::Frames => &[
                SNAPSHOT,
                "crates/review/",
                ".claude/design/",
                ".claude/adr/",
                ".claude/skills/review/",
            ],
        }
    }

    /// How many subagents read for this judge in one pass, their findings
    /// merged into one verdict. Two for the code judge (Decision 17).
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::Judge;
    /// assert_eq!(Judge::Code.readers(), 2);
    /// assert_eq!(Judge::Frames.readers(), 1);
    /// ```
    pub fn readers(self) -> usize {
        match self {
            Judge::Code => 2,
            Judge::Rust | Judge::Frames => 1,
        }
    }
}

/// What a carried pass appends to its reason, and what a verdict read in
/// its place removes.
const CARRIED: &str = "; passed in ";

/// Where one judge stands in a run. One enum, not a "required" flag beside
/// an optional verdict, so a verdict cannot attach to an uncalled judge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Standing {
    /// The staged diff does not call for this judge.
    NotRequired,
    /// Called for, and its verdict is not written yet.
    Pending,
    /// Its verdict is written and has no findings.
    Passed,
    /// Not read in this run: it passed in an earlier one on the same inputs
    /// ([`RunState::carry_from`]).
    Carried,
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
/// Keep the fields private: a standing moves only through [`new`],
/// [`record`] and a carried pass ([`RunState::carry_from`]), so no caller can
/// write a pass for a judge the change never called for.
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
    /// The fingerprint of what it reads ([`Judge::reads`]); `None` carries
    /// nothing.
    inputs: Option<Fingerprint>,
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
            inputs: None,
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
    /// It replaces a carried pass: a reading in this run outranks one in an
    /// earlier run, and a finding must never hide behind the older pass.
    ///
    /// # Errors
    ///
    /// [`Error::Review`] when the change never called for this judge.
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
        if self.standing == Standing::Carried {
            if let Some(at) = self.reason.rfind(CARRIED) {
                self.reason.truncate(at);
            }
        }
        self.standing = if passed {
            Standing::Passed
        } else {
            Standing::Failed
        };
        Ok(())
    }

    /// Takes `earlier`'s pass, from the run named `run`, when it judged
    /// exactly these inputs. Only a pending judge takes one, and only a pass
    /// written in that run: a finding is always re-read, and a carried pass
    /// would name a run that may be gone.
    fn carry(&mut self, earlier: &Assignment, run: &str) {
        let same =
            self.judge == earlier.judge && self.inputs.is_some() && self.inputs == earlier.inputs;
        if same && self.standing == Standing::Pending && earlier.standing == Standing::Passed {
            self.standing = Standing::Carried;
            self.reason = format!("{}{CARRIED}{run} on the same inputs", self.reason);
        }
    }
}

/// One run's state, saved beside its report as `run.json` for `judge`, and
/// copied into the pass record `gate` reads once the run passes.
///
/// Keep the fields private and [`RunState::assess`] the only constructor:
/// every run must hold all three judges as the staged diff assigned them, or
/// a run missing one would pass without it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    tree: String,
    stages_passed: bool,
    assignments: [Assignment; 3],
}

impl RunState {
    const FILE: &'static str = "run.json";

    /// The run of `tree`, each judge assigned from the staged `paths` and the
    /// snapshot's `changed` scenes and fingerprinted by `inputs`; and the
    /// scenes stage 8 looks at. Every snapshot scene is a capture scene (a
    /// test in `scene.rs`), so the changed scenes are the ones to capture.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::RunState;
    /// let paths = vec!["crates/tui/src/app.rs".to_string()];
    /// let changed = BTreeSet::from(["launch".to_string()]);
    /// let (state, scenes) = RunState::assess("4b825dc", true, &paths, &changed, |_| None);
    /// assert!(state.assignments().iter().all(|a| a.standing().required()));
    /// assert_eq!(scenes, ["launch"]);
    /// ```
    pub fn assess(
        tree: impl Into<String>,
        stages_passed: bool,
        paths: &[String],
        changed: &BTreeSet<String>,
        inputs: impl Fn(Judge) -> Option<Fingerprint>,
    ) -> (Self, Vec<String>) {
        let (mut assignments, scenes) = assign(paths, changed);
        for assignment in &mut assignments {
            assignment.inputs = inputs(assignment.judge);
        }
        let state = Self {
            tree: tree.into(),
            stages_passed,
            assignments,
        };
        (state, scenes)
    }

    /// The staged tree this run reviewed, from `git write-tree`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::RunState;
    /// let (state, _) = RunState::assess("4b825dc", true, &[], &BTreeSet::new(), |_| None);
    /// assert_eq!(state.tree(), "4b825dc");
    /// ```
    pub fn tree(&self) -> &str {
        &self.tree
    }

    /// Whether stages 1 to 5 all passed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::RunState;
    /// let (state, _) = RunState::assess("t", false, &[], &BTreeSet::new(), |_| None);
    /// assert!(!state.stages_passed());
    /// ```
    pub fn stages_passed(&self) -> bool {
        self.stages_passed
    }

    /// The three judges, in stage order, and where each one stands.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::{Judge, RunState};
    /// let (state, _) = RunState::assess("t", true, &[], &BTreeSet::new(), |_| None);
    /// let judges: Vec<Judge> = state.assignments().iter().map(|a| a.judge()).collect();
    /// assert_eq!(judges, Judge::ALL);
    /// ```
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    /// Records `judge`'s verdict: passed with no findings, failed with any.
    ///
    /// # Errors
    ///
    /// [`Error::Review`] when the change never called for `judge`, or the
    /// run holds no assignment for it (only a hand-edited `run.json`).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::{Judge, RunState};
    /// let paths = vec!["Cargo.toml".to_string()];
    /// let (mut state, _) = RunState::assess("t", true, &paths, &BTreeSet::new(), |_| None);
    /// assert!(!state.passed());
    /// state.record(Judge::Code, true)?;
    /// assert!(state.passed());
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn record(&mut self, judge: Judge, passed: bool) -> Result<()> {
        self.assignments
            .iter_mut()
            .find(|a| a.judge() == judge)
            .ok_or_else(|| {
                Error::Review(format!(
                    "this run holds no assignment for stage {}",
                    judge.stage()
                ))
            })?
            .record(passed)
    }

    /// Carries each pending judge's pass from `earlier`, the run named
    /// `run`, where that judge read the same fingerprinted inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::{Judge, RunState, Standing};
    /// let paths = vec!["crates/core/src/lib.rs".to_string()];
    /// let (mut earlier, _) = RunState::assess("t1", true, &paths, &BTreeSet::new(), |_| None);
    /// earlier.record(Judge::Rust, true)?;
    /// let (mut now, _) = RunState::assess("t2", true, &paths, &BTreeSet::new(), |_| None);
    /// now.carry_from(&earlier, "run-1");
    /// assert_eq!(now.assignments()[1].standing(), Standing::Pending);
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn carry_from(&mut self, earlier: &RunState, run: &str) {
        for (now, then) in self.assignments.iter_mut().zip(&earlier.assignments) {
            now.carry(then, run);
        }
    }

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
    /// println!("reviewed tree {}", state.tree());
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
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::RunState;
    /// let dir = tempfile::tempdir()?;
    /// let (state, _) = RunState::assess("4b825dc", true, &[], &BTreeSet::new(), |_| None);
    /// state.save(dir.path())?;
    /// assert_eq!(RunState::load(dir.path())?.tree(), "4b825dc");
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn save(&self, dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(dir.join(Self::FILE), text)?;
        Ok(())
    }

    /// Whether any judge written in this run has findings, whatever the last
    /// verdict was.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::{Judge, RunState};
    /// let paths = vec!["crates/core/src/lib.rs".to_string()];
    /// let (mut state, _) = RunState::assess("t", true, &paths, &BTreeSet::new(), |_| None);
    /// state.record(Judge::Code, false)?;
    /// state.record(Judge::Rust, true)?;
    /// assert!(state.has_findings());
    /// # Ok::<(), aldwin_review::Error>(())
    /// ```
    pub fn has_findings(&self) -> bool {
        self.assignments
            .iter()
            .any(|a| a.standing == Standing::Failed)
    }

    /// Whether every deterministic stage passed and every judge the change
    /// called for passed or was carried.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use aldwin_review::judges::RunState;
    /// let docs = vec![".claude/spec/aldwin-review.md".to_string()];
    /// let (docs_only, _) = RunState::assess("t", true, &docs, &BTreeSet::new(), |_| None);
    /// assert!(docs_only.passed());
    /// ```
    pub fn passed(&self) -> bool {
        self.stages_passed
            && self.assignments.iter().all(|a| {
                matches!(
                    a.standing,
                    Standing::NotRequired | Standing::Passed | Standing::Carried
                )
            })
    }
}

/// Which judges the change calls for, in stage order, and the scenes stage 8
/// is to look at — [`RunState::assess`]'s reading of the staged diff.
fn assign(paths: &[String], changed: &BTreeSet<String>) -> ([Assignment; 3], Vec<String>) {
    // The gate's hooks and settings count as code for the code judge.
    let code_changed = paths.iter().any(|p| {
        p.starts_with("crates/")
            || p.starts_with(".githooks/")
            || p.starts_with(".claude/hooks/")
            || ["Cargo.toml", "Cargo.lock", ".claude/settings.json"].contains(&p.as_str())
    });
    let rust_changed = paths.iter().any(|p| p.ends_with(".rs"));
    let scenes: Vec<String> = changed.iter().cloned().collect();
    let frames_reason = if scenes.is_empty() {
        "no scene's snapshot changed".to_string()
    } else {
        format!("the snapshot changed in {}", scenes.join(", "))
    };
    let assignments = [
        Assignment::new(
            Judge::Code,
            code_changed,
            if code_changed {
                "a crate, the workspace manifest or the gate's hooks changed"
            } else {
                "no crate, no workspace manifest and no gate hook changed"
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
        Assignment::new(Judge::Frames, !scenes.is_empty(), frames_reason),
    ];
    (assignments, scenes)
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
        let (assignments, scenes) = assign(&paths, &BTreeSet::new());
        assert!(required(&assignments).is_empty());
        assert!(scenes.is_empty());
    }

    #[test]
    fn a_change_to_the_gate_itself_calls_for_the_code_judge() {
        for path in [
            ".githooks/pre-commit",
            ".claude/hooks/commit-guard.sh",
            ".claude/settings.json",
        ] {
            let (assignments, _) = assign(&[path.to_string()], &BTreeSet::new());
            assert_eq!(required(&assignments), vec![Judge::Code], "{path}");
        }
    }

    #[test]
    fn a_manifest_change_calls_for_the_code_judge_but_not_the_rust_one() {
        let paths = vec!["Cargo.toml".to_string()];
        let (assignments, _) = assign(&paths, &BTreeSet::new());
        assert_eq!(required(&assignments), vec![Judge::Code]);
    }

    #[test]
    fn frames_are_judged_for_exactly_the_scenes_whose_snapshot_changed() {
        let paths = vec!["crates/tui/src/app.rs".to_string(), SNAPSHOT.to_string()];
        let changed = BTreeSet::from(["launch".to_string(), "working".to_string()]);
        let (assignments, scenes) = assign(&paths, &changed);
        assert_eq!(
            required(&assignments),
            vec![Judge::Code, Judge::Rust, Judge::Frames]
        );
        assert_eq!(scenes, ["launch", "working"]);
        assert!(assignments[2].reason().contains("launch, working"));

        let (assignments, scenes) = assign(&paths, &BTreeSet::new());
        assert!(scenes.is_empty());
        assert_eq!(assignments[2].standing(), Standing::NotRequired);
    }

    #[test]
    fn a_run_passes_only_when_every_required_judge_has_passed() {
        let paths = ["crates/core/src/lib.rs".to_string()];
        let (mut state, _) = RunState::assess("t", true, &paths, &BTreeSet::new(), |_| None);
        assert!(!state.passed(), "judges pending");
        state.record(Judge::Code, true).unwrap();
        assert!(!state.passed(), "the Rust judge is still pending");
        state.record(Judge::Rust, true).unwrap();
        assert!(state.passed());

        let (failed_stages, _) = RunState::assess("t", false, &[], &BTreeSet::new(), |_| None);
        assert!(!failed_stages.passed());
    }

    /// Regression: a hand-edited run missing a judge passed without it.
    #[test]
    fn a_run_missing_a_judge_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["crates/core/src/lib.rs".to_string()];
        let (state, _) = RunState::assess("t", true, &paths, &BTreeSet::new(), |_| None);
        state.save(dir.path()).unwrap();

        let file = dir.path().join(RunState::FILE);
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        json["assignments"].as_array_mut().unwrap().pop();
        std::fs::write(&file, json.to_string()).unwrap();

        assert!(matches!(RunState::load(dir.path()), Err(Error::Json(_))));
    }

    /// Decision 17: only a pass carries, only onto a pending judge, only for
    /// identical inputs.
    #[test]
    fn a_pass_carries_only_onto_the_same_inputs() {
        let paths = ["crates/core/src/lib.rs".to_string()];
        let assess = |rust: Option<&str>| {
            let inputs = |judge: Judge| match judge {
                Judge::Rust => rust.map(Fingerprint::new),
                _ => Some(Fingerprint::new("diff")),
            };
            RunState::assess("t", true, &paths, &BTreeSet::new(), inputs).0
        };
        let mut earlier = assess(Some("rust"));
        earlier.record(Judge::Code, false).unwrap();
        earlier.record(Judge::Rust, true).unwrap();

        let mut same = assess(Some("rust"));
        same.carry_from(&earlier, "run-1");
        assert_eq!(
            same.assignments()[0].standing(),
            Standing::Pending,
            "a finding is re-read"
        );
        assert_eq!(same.assignments()[1].standing(), Standing::Carried);
        assert!(same.assignments()[1].reason().contains("run-1"));

        let mut reread = same.clone();
        reread.record(Judge::Rust, false).unwrap();
        assert_eq!(
            reread.assignments()[1].standing(),
            Standing::Failed,
            "a verdict read now replaces the carried pass"
        );
        assert!(
            !reread.assignments()[1].reason().contains("run-1"),
            "and its reason no longer names the run it came from"
        );

        // Carried on again, it would name run-2, where nothing was read.
        let mut next = assess(Some("rust"));
        next.carry_from(&same, "run-2");
        assert_eq!(next.assignments()[1].standing(), Standing::Pending);

        let mut moved = assess(Some("rust changed"));
        moved.carry_from(&earlier, "run-1");
        assert_eq!(moved.assignments()[1].standing(), Standing::Pending);

        let mut unknown = assess(None);
        let mut unknown_before = assess(None);
        unknown_before.record(Judge::Rust, true).unwrap();
        unknown.carry_from(&unknown_before, "run-1");
        assert_eq!(unknown.assignments()[1].standing(), Standing::Pending);
    }

    /// Regression: `judge` read the last verdict's pass as the run's.
    #[test]
    fn an_earlier_judges_findings_outlast_a_later_pass() {
        let paths = ["crates/core/src/lib.rs".to_string()];
        let (mut state, _) = RunState::assess("t", true, &paths, &BTreeSet::new(), |_| None);
        state.record(Judge::Code, false).unwrap();
        assert!(state.has_findings());
        state.record(Judge::Rust, true).unwrap();
        assert!(state.has_findings(), "a later pass does not clear it");
        assert!(!state.passed());
    }

    #[test]
    fn a_verdict_cannot_be_recorded_for_a_judge_that_was_not_required() {
        let mut frames = Assignment::new(Judge::Frames, false, "no scene's snapshot changed");
        assert!(matches!(frames.record(true), Err(Error::Review(_))));
        assert_eq!(frames.standing(), Standing::NotRequired);
    }
}
