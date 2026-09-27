//! The staged changeset (ADR 0009 §4): every edit of a turn lands here, and
//! nothing lands on disk until the developer approves the review.
//!
//! An overlay rather than a write-then-revert: `edit` replaces text in the
//! *staged* content of a file, `read` returns the staged content when there
//! is one, and `run` never sees any of it — a staged changeset is reviewed
//! before any run that would observe it (`Dispatcher::before_step`). That is
//! what keeps "nothing is saved until you approve" literally true rather
//! than true-after-an-undo, and it is why there is no undo to build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aldwin_core::{ChangedFile, Changeset};

use crate::error::ToolError;
use crate::paths::Workspace;

/// One staged file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The path as the model named it — what the review shows.
    pub rel: String,
    /// The file as it was on disk when first staged; `None` for a file that
    /// did not exist. Re-checked at write time: a file that moved underneath
    /// the review is not overwritten (`Staging::write_all`).
    pub before: Option<String>,
    pub after: String,
}

#[derive(Debug, Default)]
struct Inner {
    files: BTreeMap<PathBuf, Staged>,
    /// Comments left at the last review of this changeset, and not yet
    /// reported as resolved. Counted so the Saved row can say how many the
    /// approve closed.
    pending_comments: usize,
}

/// The changeset of the current turn, shared by every tool and the
/// dispatcher. Locked for the whole of a read-modify-stage, so two edits to
/// one file in the same step cannot interleave.
#[derive(Debug)]
pub struct Staging {
    inner: Mutex<Inner>,
    /// What each staged path is resolved through again at write time.
    workspace: Workspace,
}

impl Staging {
    /// An empty changeset whose writes are resolved through `workspace`.
    pub fn new(workspace: Workspace) -> Self {
        Self {
            inner: Mutex::default(),
            workspace,
        }
    }

    /// Whether nothing is staged — the review has nothing to open on.
    pub fn is_empty(&self) -> bool {
        self.lock().files.is_empty()
    }

    /// The staged content of `resolved`, if any edit has touched it.
    pub fn current(&self, resolved: &Path) -> Option<String> {
        self.lock().files.get(resolved).map(|s| s.after.clone())
    }

    /// Applies `change` to the file's current content — staged if staged,
    /// otherwise what is on disk (`None` when the file does not exist) —
    /// and stages the result. The lock is held across the read, so a
    /// concurrent edit of the same file sees this one's output.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Io`] when the file exists but cannot be read,
    /// and passes on any error `change` returns. Nothing is staged on error.
    pub async fn edit(
        &self,
        resolved: PathBuf,
        rel: &str,
        change: impl FnOnce(Option<&str>) -> Result<String, ToolError>,
    ) -> Result<(), ToolError> {
        // The disk read happens outside the lock — it is async and a
        // `std::sync::Mutex` must not be held across an await — and is only
        // used when nothing is staged yet, in which case the lock's job is
        // done by the `entry` check below.
        let on_disk = match tokio::fs::read_to_string(&resolved).await {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ToolError::Io {
                    path: resolved,
                    source,
                })
            }
        };
        let mut inner = self.lock();
        let (before, current) = match inner.files.get(&resolved) {
            Some(staged) => (staged.before.clone(), Some(staged.after.clone())),
            None => (on_disk.clone(), on_disk),
        };
        let after = change(current.as_deref())?;
        inner.files.insert(
            resolved,
            Staged {
                rel: rel.to_string(),
                before,
                after,
            },
        );
        Ok(())
    }

    /// The changeset as the review draws it, in path order.
    pub fn changeset(&self) -> Changeset {
        let inner = self.lock();
        Changeset {
            files: inner
                .files
                .values()
                .map(|s| ChangedFile {
                    path: s.rel.clone(),
                    before: s.before.clone(),
                    after: s.after.clone(),
                })
                .collect(),
        }
    }

    /// The developer left `count` comments on this changeset; the next
    /// approve reports them resolved.
    pub fn note_comments(&self, count: usize) {
        self.lock().pending_comments += count;
    }

    /// Writes every staged file and empties the staging area. Returns the
    /// paths written, the comments closed, and any file that was **not**
    /// written — because it changed on disk since it was staged (the review
    /// showed a diff against content that is no longer there, and writing
    /// over the newer content would clobber a change they never saw), or
    /// because its path no longer resolves to where it was staged.
    ///
    /// The second check holds the workspace boundary again immediately
    /// before each write, not only at the edit: a review can stay open for
    /// minutes, and a directory swapped for a symlink in that time would
    /// otherwise carry an approved write out of the workspace.
    pub async fn write_all(&self) -> Written {
        let (files, comments) = {
            let mut inner = self.lock();
            (
                std::mem::take(&mut inner.files),
                std::mem::take(&mut inner.pending_comments),
            )
        };
        let mut written = Written {
            files: Vec::new(),
            comments_resolved: comments,
            skipped: Vec::new(),
        };
        for (resolved, staged) in files {
            if self.workspace.resolve(&staged.rel).ok().as_ref() != Some(&resolved) {
                written.skipped.push((
                    staged.rel,
                    "its path no longer resolves inside the workspace to where it was staged"
                        .into(),
                ));
                continue;
            }
            let now = match tokio::fs::read_to_string(&resolved).await {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    written.skipped.push((staged.rel, e.to_string()));
                    continue;
                }
            };
            if now != staged.before {
                written
                    .skipped
                    .push((staged.rel, "changed on disk since it was staged".into()));
                continue;
            }
            if let Some(parent) = resolved.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    written.skipped.push((staged.rel, e.to_string()));
                    continue;
                }
            }
            match tokio::fs::write(&resolved, &staged.after).await {
                Ok(()) => written.files.push(staged.rel),
                Err(e) => written.skipped.push((staged.rel, e.to_string())),
            }
        }
        written
    }

    /// Drops everything staged. Returns the paths that were.
    pub fn discard(&self) -> Vec<String> {
        let mut inner = self.lock();
        inner.pending_comments = 0;
        std::mem::take(&mut inner.files)
            .into_values()
            .map(|s| s.rel)
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What an approve did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub files: Vec<String>,
    pub comments_resolved: usize,
    pub skipped: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn an_edit_is_staged_not_written_and_read_sees_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));

        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("old", "new"))
            })
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "old\n",
            "nothing on disk changes"
        );
        assert_eq!(staging.current(&path).as_deref(), Some("new\n"));
        let cs = staging.changeset();
        assert_eq!(cs.files.len(), 1);
        assert_eq!(cs.files[0].before.as_deref(), Some("old\n"));
        assert_eq!(cs.files[0].after, "new\n");
    }

    #[tokio::test]
    async fn a_second_edit_builds_on_the_first() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "a\nb\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("a", "A"))
            })
            .await
            .unwrap();
        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("b", "B"))
            })
            .await
            .unwrap();
        assert_eq!(staging.current(&path).as_deref(), Some("A\nB\n"));
        assert_eq!(
            staging.changeset().files[0].before.as_deref(),
            Some("a\nb\n"),
            "before is the disk, not the first edit"
        );
    }

    #[tokio::test]
    async fn a_new_file_is_staged_with_no_before() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("new/limit.rs");
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "new/limit.rs", |cur| {
                assert!(cur.is_none());
                Ok("fn x() {}\n".into())
            })
            .await
            .unwrap();
        assert_eq!(staging.changeset().files[0].before, None);

        let written = staging.write_all().await;
        assert_eq!(written.files, vec!["new/limit.rs".to_string()]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn x() {}\n",
            "approve creates the directory and the file"
        );
        assert!(staging.is_empty());
    }

    #[tokio::test]
    async fn approve_writes_and_reports_the_comments_it_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        staging.note_comments(2);

        let written = staging.write_all().await;
        assert_eq!(written.files, vec!["f.rs".to_string()]);
        assert_eq!(written.comments_resolved, 2);
        assert!(written.skipped.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    }

    /// The review showed a diff against content that is no longer there.
    #[tokio::test]
    async fn a_file_that_changed_under_the_review_is_not_overwritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        std::fs::write(&path, "someone else\n").unwrap();

        let written = staging.write_all().await;
        assert!(written.files.is_empty());
        assert_eq!(written.skipped[0].0, "f.rs");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else\n");
    }

    #[tokio::test]
    async fn discard_drops_everything_and_names_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        assert_eq!(staging.discard(), vec!["f.rs".to_string()]);
        assert!(staging.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    }

    /// The review can stay open for minutes. A directory swapped for a
    /// symlink in that time — or the file itself, pointing at a file outside
    /// with the same text, so the `before` check passes — must not carry the
    /// approved write out of the workspace.
    #[tokio::test]
    async fn a_symlink_swapped_in_after_staging_does_not_redirect_the_write() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("f.rs"), "old\n").unwrap();
        std::fs::write(outside.path().join("f.rs"), "old\n").unwrap();
        let staging = Staging::new(Workspace::new(&root));
        staging
            .edit(
                root.join("src/new.rs"),
                "src/new.rs",
                |_| Ok("new\n".into()),
            )
            .await
            .unwrap();
        staging
            .edit(root.join("f.rs"), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();

        std::fs::remove_dir(root.join("src")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("src")).unwrap();
        std::fs::remove_file(root.join("f.rs")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("f.rs"), root.join("f.rs")).unwrap();

        let written = staging.write_all().await;
        assert!(written.files.is_empty(), "{written:?}");
        assert_eq!(written.skipped.len(), 2);
        assert!(!outside.path().join("new.rs").exists());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("f.rs")).unwrap(),
            "old\n"
        );
    }
}
