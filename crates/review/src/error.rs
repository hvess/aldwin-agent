//! The crate's one error type.
//!
//! A variant says what failed — a file, git, the design, a frame, a gate —
//! and its `Display` is the sentence whoever ran the loop reads: `main` prints
//! that and nothing else. The sentences are the interface; the variants are
//! there so a caller that needs to tell a refusal from a broken machine can.

use thiserror::Error;

/// Everything a stage, a capture or the gate can fail with.
///
/// # Examples
///
/// ```
/// use aldwin_review::{scene, Error};
/// let unknown = scene::script("nowhere").unwrap_err();
/// assert!(matches!(unknown, Error::Scene(_)));
/// assert!(unknown.to_string().contains("unknown scene"));
/// ```
#[derive(Debug, Error)]
pub enum Error {
    /// A file could not be read or written, or a process spawned.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// `run.json`, a pass record, a judge's verdict or sway's tree is not the
    /// JSON it should be.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// The clock is before the Unix epoch, so a run directory has no name.
    #[error(transparent)]
    SystemTime(#[from] std::time::SystemTimeError),

    /// `git` ran and exited unsuccessfully.
    #[error("git {command} failed: {stderr}")]
    Git {
        /// The arguments git was given, space-separated.
        command: String,
        /// What git said, trimmed.
        stderr: String,
    },

    /// The design source is malformed, disagrees with itself, or generates a
    /// file rustfmt rejects.
    #[error("{0}")]
    Design(String),

    /// A PNG could not be decoded or encoded.
    #[error("{0}")]
    Png(String),

    /// sway or `swaymsg` did not do what was asked, or did not come up.
    #[error("{0}")]
    Compositor(String),

    /// A frame could not be taken, or its grid and picture disagree.
    #[error("{0}")]
    Capture(String),

    /// A scene is unknown, could not be seeded, or names a key that does not
    /// exist.
    #[error("{0}")]
    Scene(String),

    /// `baseline.json` is invalid, or the machine disagrees with it.
    #[error("{0}")]
    Baseline(String),

    /// The app under capture could not be built.
    #[error("{0}")]
    Build(String),

    /// A pass was asked to be recorded for a run that has not passed.
    #[error("the run has not passed; nothing to record")]
    NotPassed,

    /// Something was staged after the review ran, so the tree it reviewed is
    /// not the one about to be recorded.
    #[error("the index is tree {now}, but this run reviewed {reviewed}; something was staged after the review. Run the loop again.")]
    IndexMoved {
        /// The tree the index holds now.
        now: String,
        /// The tree the run reviewed.
        reviewed: String,
    },

    /// Stage 10: the tree being committed has no review record.
    #[error("no passing review for tree {tree}. An agent's commit runs the whole loop first: run /review, and commit exactly what it reviewed.")]
    NoRecord {
        /// The staged tree.
        tree: String,
    },

    /// Stage 10: the tree being committed has a record, and it did not pass.
    #[error("the review recorded for tree {tree} did not pass; run /review again")]
    RecordFailed {
        /// The staged tree.
        tree: String,
    },

    /// A judge's verdict has nowhere to go in the report.
    #[error("{0}")]
    Report(String),

    /// The review ran and its answer is no: the tree is not reviewable, a
    /// stage failed, a judge has findings, or the review is not finished.
    #[error("{0}")]
    Review(String),
}

/// A `Result` whose error is this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
