//! What git says about the change under review.
//!
//! The crate's only caller of `git`; every failure is [`Error::Git`]. Reads
//! the staged tree, never the working one: it is what a commit records and
//! what a pass is keyed by.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The snapshot stage 5 checks; its changed scenes decide whether stage 8
/// runs, and on which frames (Decision 5).
pub(crate) const SNAPSHOT: &str = "crates/tui/tests/snapshots/render.snap";

/// `git` in the workspace, failing with git's own message.
fn git(root: &Path, args: &[&str]) -> Result<String> {
    git_fed(root, args, None)
}

/// [`git`], with `input` on its stdin when there is some.
fn git_fed(root: &Path, args: &[&str], input: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(Error::Git {
            command: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository's git directory, shared by every worktree, as an absolute
/// path.
///
/// # Errors
///
/// When git cannot say where its directory is.
pub(crate) fn common_dir(root: &Path) -> Result<PathBuf> {
    let common = PathBuf::from(git(root, &["rev-parse", "--git-common-dir"])?.trim());
    Ok(if common.is_absolute() {
        common
    } else {
        root.join(common)
    })
}

/// The short hash of `HEAD`, which the report names the run by.
///
/// # Errors
///
/// When there is no `HEAD` yet, as before the first commit.
pub fn head(root: &Path) -> Result<String> {
    Ok(git(root, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// The tree the next commit would record.
///
/// Inside a pre-commit hook this is the index git is about to commit,
/// `git commit -a`'s temporary one included: `git` inherits the hook's
/// `GIT_INDEX_FILE`.
///
/// # Errors
///
/// When git cannot write the tree, as in a merge with unresolved paths.
pub fn staged_tree(root: &Path) -> Result<String> {
    Ok(git(root, &["write-tree"])?.trim().to_string())
}

/// Paths whose working copy differs from the index, untracked files included.
///
/// Must be empty for a review: it builds and judges the working tree but
/// records the staged one (Decision 15).
///
/// # Errors
///
/// When `git status` cannot run.
pub fn unstaged(root: &Path) -> Result<Vec<String>> {
    Ok(
        git(root, &["status", "--porcelain=v1", "--untracked-files=all"])?
            .lines()
            .filter(|l| l.as_bytes().get(1).is_some_and(|&y| y != b' '))
            .map(|l| l[3..].to_string())
            .collect(),
    )
}

/// Paths the next commit changes against `HEAD`.
///
/// # Errors
///
/// When `git diff` cannot run.
pub fn staged_paths(root: &Path) -> Result<Vec<String>> {
    Ok(git(root, &["diff", "--cached", "--name-only"])?
        .lines()
        .map(str::to_string)
        .collect())
}

/// The staged diff, as the judges read it.
///
/// # Errors
///
/// When `git diff` cannot run.
pub fn staged_diff(root: &Path) -> Result<String> {
    git(root, &["diff", "--cached"])
}

/// Git's hash of `HEAD` and part of the staged diff: equal fingerprints mean
/// the same bytes in that part of the tree (Decision 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// A hash as git gave it.
    pub(crate) fn new(hash: impl Into<String>) -> Self {
        Fingerprint(hash.into())
    }
}

/// The [`Fingerprint`] of `HEAD` and the staged diff limited to `pathspecs`
/// (the whole diff when empty). `HEAD` is included because a commit can
/// change what a judge reads outside the diff while leaving its text alone.
///
/// # Errors
///
/// When `git diff` or `git hash-object` cannot run.
pub fn fingerprint(root: &Path, pathspecs: &[&str]) -> Result<Fingerprint> {
    let args: Vec<&str> = ["diff", "--cached", "--"]
        .into_iter()
        .chain(pathspecs.iter().copied())
        .collect();
    // Empty before the first commit.
    let head = head(root).unwrap_or_default();
    let diff = format!("{head}{}", git(root, &args)?);
    let hash = git_fed(root, &["hash-object", "--stdin"], Some(&diff))?;
    Ok(Fingerprint::new(hash.trim()))
}

/// Scenes whose snapshot the next commit changes, by name.
///
/// # Errors
///
/// When the staged snapshot cannot be read.
pub fn changed_scenes(root: &Path) -> Result<BTreeSet<String>> {
    // No snapshot at `HEAD` means every scene is new.
    let before = git(root, &["show", &format!("HEAD:{SNAPSHOT}")]).unwrap_or_default();
    let after = git(root, &["show", &format!(":{SNAPSHOT}")])?;
    Ok(changed_in(&before, &after))
}

/// The scene a snapshot line names, when it is a section's `=== <theme>
/// <scene> <size>` header. The only parser of that header.
pub(crate) fn scene_of(line: &str) -> Option<&str> {
    line.strip_prefix("=== ")?.split_whitespace().nth(1)
}

/// The scenes whose sections changed, appeared or went away between two
/// snapshots; a section runs from its header to the next.
fn changed_in(before: &str, after: &str) -> BTreeSet<String> {
    fn sections(text: &str) -> BTreeMap<&str, Vec<&str>> {
        let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut current = None;
        for line in text.lines() {
            if scene_of(line).is_some() {
                current = Some(line);
            }
            if let Some(header) = current {
                out.entry(header).or_default().push(line);
            }
        }
        out
    }
    let (before, after) = (sections(before), sections(after));
    before
        .keys()
        .chain(after.keys())
        .filter(|header| before.get(*header) != after.get(*header))
        .filter_map(|header| scene_of(header))
        .map(str::to_string)
        .collect()
}

/// A throwaway repository with one staged file, for the tests here and in
/// `gate`. `Repo::git` removes `GIT_DIR` and `GIT_INDEX_FILE` so it never
/// touches an enclosing repository; the crate's own `git` does not remove
/// them.
#[cfg(test)]
pub(crate) mod fixture {
    use std::path::Path;
    use std::process::Command;

    pub(crate) struct Repo(tempfile::TempDir);

    impl Repo {
        pub(crate) fn new() -> Self {
            let repo = Repo(tempfile::tempdir().unwrap());
            repo.git(&["init", "--quiet"]);
            repo.write("a.txt", "one\n");
            repo.git(&["add", "a.txt"]);
            repo
        }

        pub(crate) fn root(&self) -> &Path {
            self.0.path()
        }

        pub(crate) fn write(&self, name: &str, text: &str) {
            std::fs::write(self.root().join(name), text).unwrap();
        }

        pub(crate) fn git(&self, args: &[&str]) {
            let status = Command::new("git")
                .current_dir(self.root())
                .env_remove("GIT_DIR")
                .env_remove("GIT_INDEX_FILE")
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Repo;
    use super::*;

    const BEFORE: &str = "\
=== Dark launch 80x24
caret None
row a
=== Dark working 80x24
caret None
row b
";

    #[test]
    fn a_scene_whose_section_changed_is_named_and_no_other() {
        let after = BEFORE.replace("row b", "row B");
        let changed = changed_in(BEFORE, &after);
        assert_eq!(changed, BTreeSet::from(["working".to_string()]));
    }

    #[test]
    fn a_scene_that_appears_or_goes_away_counts_as_changed() {
        let after = format!("{BEFORE}=== Light plan 80x24\nrow c\n");
        assert_eq!(
            changed_in(BEFORE, &after),
            BTreeSet::from(["plan".to_string()])
        );
        assert_eq!(
            changed_in(&after, BEFORE),
            BTreeSet::from(["plan".to_string()])
        );
    }

    #[test]
    fn an_unchanged_snapshot_changes_no_scene() {
        assert!(changed_in(BEFORE, BEFORE).is_empty());
    }

    /// Decision 17: a judge's pass carries on this fingerprint.
    #[test]
    fn a_fingerprint_moves_only_with_what_it_covers() {
        let repo = Repo::new();
        repo.write("lib.rs", "fn a() {}\n");
        repo.git(&["add", "lib.rs"]);
        let rust = fingerprint(repo.root(), &["*.rs"]).unwrap();
        let whole = fingerprint(repo.root(), &[]).unwrap();
        assert_eq!(rust.0.len(), 40, "{rust:?}");

        repo.write("a.txt", "two\n");
        repo.git(&["add", "a.txt"]);
        assert_eq!(fingerprint(repo.root(), &["*.rs"]).unwrap(), rust);
        assert_ne!(fingerprint(repo.root(), &[]).unwrap(), whole);

        // A commit moves every fingerprint, even where the diff reads the
        // same.
        repo.git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "c",
        ]);
        repo.write("lib.rs", "fn a() {}\n");
        let empty_before = fingerprint(repo.root(), &["*.rs"]).unwrap();
        repo.write("b.rs", "fn b() {}\n");
        repo.git(&["add", "b.rs"]);
        repo.git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "d",
        ]);
        assert_ne!(fingerprint(repo.root(), &["*.rs"]).unwrap(), empty_before);
    }

    #[test]
    fn unstaged_names_what_the_index_does_not_hold() {
        let repo = Repo::new();
        assert!(unstaged(repo.root()).unwrap().is_empty());
        repo.write("a.txt", "two\n");
        repo.write("b.txt", "new\n");
        assert_eq!(unstaged(repo.root()).unwrap(), ["a.txt", "b.txt"]);
    }
}
