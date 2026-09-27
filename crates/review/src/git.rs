//! What git says about the change under review.
//!
//! This is the one place the crate runs `git`, so every git failure reads the
//! same way: the arguments it was given and what it said, as [`Error::Git`].
//!
//! Everything here reads the staged tree rather than the working one, because
//! the staged tree is what a commit records and what a pass is keyed by.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Error, Result};

/// The snapshot stage 5 checks. Which of its scenes changed is what decides
/// whether stage 8 runs, and on which frames.
pub(crate) const SNAPSHOT: &str = "crates/tui/tests/snapshots/render.snap";

/// `git` in the workspace, failing with git's own message.
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(root).args(args).output()?;
    if !out.status.success() {
        return Err(Error::Git {
            command: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository's git directory — shared by every worktree — as an
/// absolute path.
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
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// let on = aldwin_review::git::head(Path::new("."))?;
/// println!("reviewing on top of {on}");
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn head(root: &Path) -> Result<String> {
    Ok(git(root, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// The tree the next commit would record.
///
/// Inside a pre-commit hook this is the index git is about to commit —
/// including `git commit -a`'s temporary one, since git hands the hook
/// `GIT_INDEX_FILE` and this inherits it.
///
/// # Errors
///
/// When git cannot write the tree, as in a merge with unresolved paths.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// let tree = aldwin_review::git::staged_tree(Path::new("."))?;
/// println!("the next commit records tree {tree}");
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn staged_tree(root: &Path) -> Result<String> {
    Ok(git(root, &["write-tree"])?.trim().to_string())
}

/// Paths whose working copy differs from the index, untracked files included.
///
/// A review builds and judges the working tree but records the staged one, so
/// the two have to be the same thing; this is what says they are not.
///
/// # Errors
///
/// When `git status` cannot run.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// let unstaged = aldwin_review::git::unstaged(Path::new("."))?;
/// assert!(unstaged.is_empty(), "stage what the commit is first: {unstaged:?}");
/// # Ok::<(), aldwin_review::Error>(())
/// ```
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
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// let paths = aldwin_review::git::staged_paths(Path::new("."))?;
/// let rust_changed = paths.iter().any(|p| p.ends_with(".rs"));
/// # Ok::<(), aldwin_review::Error>(())
/// ```
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
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// std::fs::write("change.diff", aldwin_review::git::staged_diff(Path::new("."))?)?;
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn staged_diff(root: &Path) -> Result<String> {
    git(root, &["diff", "--cached"])
}

/// Scenes whose snapshot the next commit changes, by name.
///
/// # Errors
///
/// When the staged snapshot cannot be read.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// for scene in aldwin_review::git::changed_scenes(Path::new("."))? {
///     println!("{scene} moved");
/// }
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn changed_scenes(root: &Path) -> Result<BTreeSet<String>> {
    // No snapshot at `HEAD` means every scene is new.
    let before = git(root, &["show", &format!("HEAD:{SNAPSHOT}")]).unwrap_or_default();
    let after = git(root, &["show", &format!(":{SNAPSHOT}")])?;
    Ok(changed_in(&before, &after))
}

/// The scene a snapshot line names, when it is a section's `=== <theme>
/// <scene> <size>` header. The one reader of that header.
pub(crate) fn scene_of(line: &str) -> Option<&str> {
    line.strip_prefix("=== ")?.split_whitespace().nth(1)
}

/// The scenes whose sections differ between two snapshots.
///
/// A section starts at `=== <theme> <scene> <size>` and runs to the next;
/// one that changed, appeared or went away names its scene.
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

/// A throwaway repository with one staged file, for the tests of the functions
/// that decide whether a commit lands — here and in `gate`. Its git runs with
/// the caller's `GIT_DIR` and `GIT_INDEX_FILE` removed, so a test never
/// touches the repository it happens to run inside.
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

    #[test]
    fn unstaged_names_what_the_index_does_not_hold() {
        let repo = Repo::new();
        assert!(unstaged(repo.root()).unwrap().is_empty());
        repo.write("a.txt", "two\n");
        repo.write("b.txt", "new\n");
        assert_eq!(unstaged(repo.root()).unwrap(), ["a.txt", "b.txt"]);
    }
}
