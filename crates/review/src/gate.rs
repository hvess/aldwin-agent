//! Stage 10 — the record of a passing review, and the check the pre-commit
//! hook runs against it.
//!
//! A pass is recorded against the staged tree, so `gate` asks one question:
//! was exactly this tree reviewed, and did every stage pass?
//!
//! Keyed by tree rather than by commit because the commit does not exist yet
//! when the hook runs, and because any edit after the review changes the tree:
//! a record cannot be carried over to code it did not see.
//!
//! Refusals are [`Error`] variants, so the gate's refusals are typed like the
//! rest of the crate's failures.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::git::{common_dir, staged_tree};
use crate::judges::RunState;
use crate::{Error, Result};

/// Where the pass record for `tree` lives: inside the repository's git
/// directory, so it is never committed and never shows in `git status`.
///
/// # Errors
///
/// When git cannot say where its directory is.
fn record_path(root: &Path, tree: &str) -> Result<PathBuf> {
    Ok(common_dir(root)?
        .join("aldwin-review")
        .join(format!("{tree}.json")))
}

/// Records `state` as the pass for its tree.
///
/// # Errors
///
/// When the run has not passed, when the index has moved since it ran — a
/// record for a tree nobody reviewed is the failure this exists to prevent —
/// or when the record cannot be written.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use aldwin_review::gate;
/// use aldwin_review::judges::RunState;
/// let state = RunState::load(Path::new("target/review-frames/run-1790488849"))?;
/// let record = gate::write_record(Path::new("."), &state)?;
/// println!("recorded in {}", record.display());
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn write_record(root: &Path, state: &RunState) -> Result<PathBuf> {
    if !state.passed() {
        return Err(Error::NotPassed);
    }
    let now = staged_tree(root)?;
    if now != state.tree() {
        return Err(Error::IndexMoved {
            now,
            reviewed: state.tree().to_string(),
        });
    }
    let path = record_path(root, state.tree())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Stage 10: whether the tree about to be committed has a passing review.
///
/// `Ok` carries the record's path; `Err` the sentence the committer reads.
///
/// # Errors
///
/// [`Error::NoRecord`] when no review of this tree was recorded,
/// [`Error::RecordFailed`] when the one recorded did not pass, and the I/O or
/// JSON error itself when a record exists but cannot be read or parsed —
/// a damaged record is not the same news as a missing one.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// match aldwin_review::gate::check(Path::new(".")) {
///     Ok(record) => println!("reviewed: {}", record.display()),
///     Err(refusal) => eprintln!("{refusal}"),
/// }
/// ```
pub fn check(root: &Path) -> Result<PathBuf> {
    let tree = staged_tree(root)?;
    let path = record_path(root, &tree)?;
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(Error::NoRecord { tree }),
        read => read?,
    };
    let state: RunState = serde_json::from_str(&text)?;
    if state.tree() != tree || !state.passed() {
        return Err(Error::RecordFailed { tree });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::git::fixture::Repo;

    /// A run of `repo`'s staged tree that passed every stage and needed no
    /// judge.
    fn passing(repo: &Repo) -> RunState {
        run(repo, true)
    }

    /// A run of `repo`'s staged tree that needed no judge, whose stages 1 to
    /// 5 passed or not.
    fn run(repo: &Repo, stages_passed: bool) -> RunState {
        let tree = staged_tree(repo.root()).unwrap();
        RunState::assess(tree, stages_passed, &[], &BTreeSet::new(), |_| None).0
    }

    #[test]
    fn the_record_lives_in_the_git_directory_under_its_tree() {
        let repo = Repo::new();
        let tree = staged_tree(repo.root()).unwrap();
        let path = record_path(repo.root(), &tree).unwrap();
        assert!(
            path.ends_with(format!(".git/aldwin-review/{tree}.json")),
            "{}",
            path.display()
        );
    }

    #[test]
    fn a_passing_review_of_this_tree_lets_the_commit_through() {
        let repo = Repo::new();
        assert!(matches!(check(repo.root()), Err(Error::NoRecord { .. })));
        let written = write_record(repo.root(), &passing(&repo)).unwrap();
        assert_eq!(check(repo.root()).unwrap(), written);
    }

    /// The failure the record exists to prevent: a pass written for a tree
    /// nobody reviewed.
    #[test]
    fn nothing_is_recorded_once_the_index_has_moved() {
        let repo = Repo::new();
        let reviewed = passing(&repo);
        repo.write("a.txt", "changed after the review\n");
        repo.git(&["add", "a.txt"]);
        assert!(matches!(
            write_record(repo.root(), &reviewed),
            Err(Error::IndexMoved { .. })
        ));
        assert!(matches!(check(repo.root()), Err(Error::NoRecord { .. })));
    }

    #[test]
    fn a_failed_run_is_never_recorded_and_a_failed_record_never_passes() {
        let repo = Repo::new();
        let failed = run(&repo, false);
        assert!(matches!(
            write_record(repo.root(), &failed),
            Err(Error::NotPassed)
        ));

        // A record that says the run failed — written by hand, since
        // `write_record` refuses to — is still a refusal.
        let path = record_path(repo.root(), failed.tree()).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string(&failed).unwrap()).unwrap();
        assert!(matches!(
            check(repo.root()),
            Err(Error::RecordFailed { .. })
        ));
    }

    /// A damaged record used to read as "no passing review", which sent the
    /// committer to review again when the record was the problem.
    #[test]
    fn a_damaged_record_is_reported_as_damaged_not_as_missing() {
        let repo = Repo::new();
        let tree = staged_tree(repo.root()).unwrap();
        let path = record_path(repo.root(), &tree).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not a record").unwrap();
        assert!(matches!(check(repo.root()), Err(Error::Json(_))));
    }
}
